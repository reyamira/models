# Benchmarks Module

Multi-source benchmark data: a generic v2 schema, a compile-time source registry,
per-source CDN fetching, registry-driven view helpers, and models.dev trait
matching/enrichment. All views render from per-source metric definitions shipped
in the data files — there are no hardcoded benchmark field names. `BenchmarkStore`
/ `BenchmarkEntry` / `store.rs` are gone (removed at the end of the v2 spine work).

## Module Structure

- **schema.rs** — the v2 wire format, the single source of truth for the shape
  the TUI/CLI deserialize and the transform bin emits.
  - `SourceFile { source: SourceMeta, metrics: Vec<MetricDef>, models: Vec<ModelRow> }`
  - `SourceMeta { id, name, url, fetched_at, verified }` — `verified == false` ⇒
    "self-reported" badge. **All four sources currently set `verified == true`**
    (LLM Stats was flipped to `true` on 2026-06-11 — it aggregates third-party
    results, not provider self-reported numbers), so the badge is generic
    forward-compat machinery with no live user today.
  - `MetricDef { id, label, kind, group, higher_is_better, last_updated, description }`
    — `metrics` order = display order; `group` is the section header in
    Detail/H2H and the radar-preset grouping; `description` is the curated
    glossary blurb (set by the transforms). `higher_is_better` defaults to `true`.
  - `MetricKind` (serde snake_case): `Percentage | Index | Elo | TokensPerSec |
    Seconds | UsdPerMTok` — drives kind-based value formatting and scatter
    log-scale heuristics.
  - `ModelRow { id, name, display_name, creator, creator_name, release_date,
    reasoning_status, effort_level, variant_tag, open_weights, context_window,
    supports_tools, max_output, scores: BTreeMap<String, ScoreCell> }` — `scores`
    is a `BTreeMap` so JSON output is deterministic (required for
    commit-if-changed). `supports_tools` / `max_output` are **runtime-only**
    (backfilled from a models.dev match in `finalize_loaded_source`; the
    transforms always emit them as `None`, so they never appear in the data
    files).
  - `ScoreCell { value, date, ci }` — `ci` carries Arena Elo confidence intervals.
  - **Self-contained on purpose.** This file is compiled both as
    `crate::benchmarks::schema` AND, via a `#[path]` include, into the transform
    bin (the crate has no lib target). Do **not** reference other crate modules
    from it — that guarantees the transform output can never drift from what the
    app reads.
  - Also hosts the shared name-parsing facility (`parse_name_metadata`,
    `ParsedName`, `PAREN_RE`/`DATE_RE`/`EFFORT_ONLY_RE`, `extract_effort`):
    reasoning-status / effort-level / variant-tag extraction from parentheticals
    in a raw model name (e.g. `"Claude (Adaptive Reasoning, Max Effort)"` →
    `Adaptive` / `max` / `display_name="Claude"`). It runs at **transform time**
    (the AA transform `#[path]`-includes this file and calls it), so in the
    `models` binary these items are exercised only by the unit tests — hence the
    item-level `#[allow(dead_code)]`s.

- **multi.rs** — `MultiStore` + the registry-driven view primitives.
  - `MultiStore { sources: Vec<SourceState> }`, `SourceState { descriptor, load }`,
    `SourceLoad = Loading | Loaded(SourceFile) | Failed`. Seeded one `Loading`
    entry per `SOURCES` descriptor; `set_loaded`/`set_failed`/`file`/`file_mut`
    are no-ops on out-of-range indices.
  - `SortKey = ReleaseDate | Name | Metric(usize)` — `Metric(i)` indexes
    `file.metrics`.
  - `ReasoningFilter` (`All → Reasoning → NonReasoning` cycle; `Adaptive` counts
    as reasoning), ported to operate on `ModelRow`.
  - `format_metric_value(kind, value)` — kind-based formatting (AA stores
    percentages as fractions, so `Percentage` ×100): `Percentage` `{:.1}%`,
    `Index` `{:.1}`, `Elo` `{:.0}`, `TokensPerSec` `{:.0}`, `Seconds` `{:.2}s`,
    `UsdPerMTok` `${:.2}`.
  - `groups_in_order(file)` / `metric_indices_in_group(file, group)` — first-
    appearance group order and per-group metric indices.
  - `radar_groups(file)` — groups with ≥3 `higher_is_better` metrics (keeps
    Performance/Pricing off the radar, matching legacy behavior).
  - `default_sort(file)` — `ReleaseDate` if any model has one, else `Metric(0)`
    (Arena has no dates).
  - A unit test (`test_committed_aa_json_deserializes_and_helpers_match`) loads
    the real committed `data/v2/aa.json` and guards the schema↔helper contract
    (21 metrics, plausible model-count band, group order, radar groups).

- **sources.rs** — compile-time `SourceDescriptor` registry. `SOURCES` is a
  4-entry `const` slice in display order: `aa` (verified), `epoch` (verified),
  `arena` (verified), `llmstats` (verified). Each entry carries `id`
  (= `data/v2/` filename stem), `name`, `url`, `data_url` (jsDelivr `@main`), and
  `verified`. Only the source list is compiled in; metric definitions stay
  data-driven in the files. `url`/`verified` are part of the binding contract but
  currently `#[allow(dead_code)]` — the UI reads attribution/verification from
  the data file's `SourceMeta` today (see the per-field TODOs).

- **fetch.rs** — `fetch_source(&SourceDescriptor) -> Option<SourceFile>`.
  - Async reqwest GET + JSON deserialize; returns `None` on any error (network,
    non-2xx, parse) — no error payload is carried (keeps the failure path
    data-free, matching `MultiStore::set_failed`).
  - `MODELS_DATA_BASE_URL` env override: when set and non-empty, fetches
    `{base}/{id}.json` instead of `desc.data_url` — a sanctioned dev override for
    serving data files from a local dir or staging host.
  - The TUI spawns 4 of these in parallel at startup; results arrive as
    `Message::DataSourceLoaded(idx, Option<SourceFile>)`.

- **traits.rs** — two distinct models.dev-matching strategies.
  - `apply_model_traits(providers, models)` — **AA only.** Fuzzy Jaro-Winkler
    matching (`MIN_SIMILARITY = 0.85`, via `strsim`) of the AA slug (the
    `ModelRow.id`) against models.dev model IDs, in two stages: (1) creator-scoped
    (AA creator → mapped models.dev provider, e.g. `meta→llama`, `kimi→moonshotai`,
    `aws→amazon-bedrock`), (2) global fallback. Fills `open_weights` /
    `context_window` / `supports_tools` (`tool_call`) / `max_output`
    (`limit.output`) from the matched model, plus `known_creator_openness`
    overrides for creators absent from models.dev (e.g. `ai2→open`,
    `ai21-labs→closed`). Existing populated fields are untouched.
  - `enrich_from_models_dev(providers, models)` — **generic, for the clean-id
    sources** (epoch / arena / llmstats). Matches the source id against models.dev
    ids **exact then normalized only — NO fuzzy/Jaro-Winkler** (clean-id sources
    have consistent naming; fuzzy matching is the AA-specific lesson). `normalize_id`
    lowercases, strips hosting-org prefixes (`fireworks/…`), drops parentheticals,
    repeatedly strips variant suffixes / trailing date stamps (`-2025-12-11`,
    `-202512`) / thinking-budget tags (`-32k`). On a match, fills ONLY empty fields
    (`creator`/`creator_name` from the matched model's host provider with Origin
    preferred, `release_date`, `context_window`, `open_weights`, `supports_tools`,
    `max_output`). Source-provided
    values are never overwritten; unmatched models stay untouched (honest em-dash).
  - `creator_openness(models)` — derives a creator→openness map from model-level
    `open_weights`: `true` if any model is open, `false` if all known-status
    models are closed, absent when no model under that creator has a known status.
    Drives the sidebar O/C indicators and the `4` weights filter.

## Re-exports (mod.rs)

```rust
pub use fetch::fetch_source;
pub use schema::ReasoningStatus;
pub use traits::{apply_model_traits, creator_openness, enrich_from_models_dev};
// schema, sources, multi are `pub mod`s; fetch and traits are private.
```

## Schema sharing into the transform bin

`src/bin/transform/main.rs` does `#[path = "../../benchmarks/schema.rs"] mod schema;`
so the bin and the app deserialize/serialize through the exact same structs.
Keep `schema.rs` free of other-crate references or the bin won't compile.

## Key Gotchas

- New v2 field ⇒ add it to `schema.rs` with `#[serde(default, skip_serializing_if = ...)]`.
  No cache versioning needed (data is fetched fresh every launch); old
  `data/v2/*.json` still deserialize because optional fields are tolerant.
- `ModelRow.scores` and the JSON layout must stay deterministic (`BTreeMap`,
  not `HashMap`) — the transform's commit-if-changed compares serialized output.
- AA percentages are stored as fractions (`0.914`); `format_metric_value`
  multiplies `Percentage` by 100. Do not double-scale.
- `apply_model_traits` (fuzzy) is AA-only; the other three sources use the
  exact/normalized `enrich_from_models_dev`. Don't route a clean-id source
  through the Jaro-Winkler path.
