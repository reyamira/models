# `transform` — Benchmark Data Pipeline Bin

Offline data-pipeline binary (Cargo feature `pipeline`). Converts raw upstream
benchmark API/data dumps into the v2 `SourceFile` schema that the TUI/CLI
deserialize, writing `data/v2/<id>.json`. **Not built by default** — `cargo
install modelsdev` skips it. Build/run with `--features pipeline`.

Bare `cargo run` is ambiguous (two `[[bin]]` targets); `Cargo.toml` sets
`default-run = "models"`, so always invoke this bin explicitly:

```bash
cargo run --features pipeline --bin transform -- <subcommand> <input> -o <output>
```

## Schema sharing

`main.rs` pulls in the v2 schema via `#[path = "../../benchmarks/schema.rs"] mod
schema;` — the exact same file the app compiles as `crate::benchmarks::schema`.
The crate has no lib target, so this `#[path]` include is what guarantees the
transform output can never drift from what the app reads. Each sub-module uses
`crate::schema::{...}`.

## Subcommands (`main.rs`)

| Subcommand | Input | Output |
|------------|-------|--------|
| `aa <input> -o <out>` | raw AA API response (`{"data": [...]}`) | `data/v2/aa.json` |
| `arena <dir> -o <out>` | directory of 6 board JSONs | `data/v2/arena.json` |
| `epoch <dir> -o <out>` | *unzipped* `benchmark_data.zip` CSV dir | `data/v2/epoch.json` |
| `llmstats <rankings> [--models <m>] -o <out>` | assembled `/v1/rankings` (+ optional `/v1/models`) | `data/v2/llmstats.json` |

Each sub-module exposes `run(...) -> Result<(), String>`. `main` maps `Ok` →
`ExitCode::SUCCESS`, `Err` → prints `error: {err}` to stderr + `FAILURE`.
(`eprintln!` is fine here — this is a CLI bin, never the TUI alternate screen.)

## Per-source quirks

### `aa.rs`
- Mirrors the legacy jq transform in `update-benchmarks.yml` exactly for field
  paths and null-safety (typed serde structs with `#[serde(default)]`, not
  `Value` spelunking).
- `0` is a missing-data **sentinel** for `median_output_tokens_per_second` /
  `median_time_to_first_token_seconds` / `median_time_to_first_answer_token` →
  treated as `None`.
- Reasoning/effort/display-name parsing runs here via the shared
  `schema::parse_name_metadata`.
- Emits 21 metrics across the Indexes / Agentic / Academic / Performance /
  Pricing groups (the radar groups mirror the legacy `RadarPreset` axis sets).

### `arena.rs`
- Ingests only the **6 LLM-relevant boards** (text, vision, code, agent, search,
  document) — media-gen boards are a plan non-goal. The `BOARDS` registry is
  binding: file stem → metric id (`elo_text` … `elo_document`) → label →
  description. All 6 map to a single `"Arena Elo"` group ⇒ exactly one clean
  radar preset (≥3 higher_is_better metrics).
- **Slug-merge:** model rows are merged across boards by `slugify(model_name)`
  (not raw name) — some boards use display names (`"GLM 5.1"`) where others use
  slug-style (`"glm-5.1"`); both slugify to the same id, folding them together.
- **Scoreless drop:** after merge, rows with an empty `scores` map are dropped
  (`retain(|r| !r.scores.is_empty())`) — a model that appeared on a board but had
  a null score contributes nothing.
- A missing board file is a warning (stderr), not an error — the transform
  proceeds with the boards it found.
- `vendor`→creator (slugified), `license` (`"open"`/`"proprietary"`/null)→
  `open_weights`, `score`→value, `ci`→ci, `votes`→votes (per-board head-to-head
  sample size, surfaced as a confidence signal in the detail panel). Board date
  = `last_updated`, falling back to the date part of `fetched_at`. No release
  dates upstream.

### `epoch.rs`
- Input is the **unzipped** ZIP: one wide CSV per benchmark. Uses the `csv`
  crate (the `pipeline`-gated optional dep) because several CSVs carry RFC-4180
  quoted fields with embedded commas AND newlines.
- **Score-column detection:** the prefix is *mostly* `Model version, <score>,
  Release date, …`, but a handful of CSVs insert a categorical column (`Agent`,
  `Tools`, `Scaffold`) before the score. So `find_score_column` locates the
  **first numeric-parseable column after `Model version`**, not a fixed index.
- Parses plain fractions (`0.42`), the ECI score (`~62..159`, kind Index), and
  the documented `scorer:mean±stderr` form (`gpt-grader:0.79±0.018` → `0.79`).
  `infer_kind` picks Percentage / Index / Elo from the max observed value for
  stems not in the static `metric_meta` registry.
- **Suffix split:** trailing `_high`/`_low`/`_medium`/`_xhigh`/`_max`/`_minimal`
  → `effort_level` (`xhigh` normalizes to `max`); context/other suffixes →
  `variant_tag`.
- **Dedup:** best (max — every Epoch metric is higher-is-better) score per model
  per benchmark; the winning run's date lands on the `ScoreCell`.
- **Auto-prune (`PRUNE_DAYS = 60`):** a CSV becomes a metric only if its newest
  row date (best run/eval date column, falling back to `Release date`) is ≤60
  days old at transform time. The metric set adapts run-to-run; the data-driven
  UI absorbs it. `metric_meta` is therefore a *superset* lookup (unknown stems →
  humanized label + `"Academic"` group). The static group map keeps each radar
  group (Frontier / Agentic / Academic) at 3–6 higher_is_better metrics.
- `Organization` often lists several contributing orgs comma-separated (e.g.
  `"Google DeepMind,Google"` or `"DeepSeek,Peking University"`); the transform
  takes only the **primary org** (before the first comma) for `creator` (slugified)
  / `creator_name`, so the creators list isn't polluted with composite multi-org
  slugs that never match the region/type tables. `Country` is unused.

### `llmstats.rs`
- The plan's "~11 category scores" are **rankings, not scores.** `/v1/scores` is
  14.6k raw per-benchmark rows; the curated headline signal is the per-category
  TrueSkill `conservative_rating` from `/v1/rankings?category=<cat>`
  (`method: "trueskill"`) — one rating per (model, category). The binding
  `METRICS` registry ingests the 11 plan-named categories.
- **`limit=50` cap:** `/v1/rankings` is hard-capped at 50 (requesting more
  silently returns an empty list — an upstream quirk the workflow's fetch loop
  must respect).
- Rankings carry `organization` (bare slug) and `open_weight`, but NOT
  `release_date` / `context_window`. Those come from the **optional** second
  input `/v1/models` (cursor-paginated), joined on rankings `model_id` == models
  `id`. A model in rankings but absent from `/v1/models` still appears (metadata
  `None`, `creator_name` falls back to the org slug).
- `source.verified = true` ⇒ no self-reported badge. LLM Stats aggregates
  third-party benchmark results and its methodology excludes provider
  self-reported numbers from the ingested rankings, so it is verified like the
  other sources (flipped from `false` on 2026-06-11; plan amendment).

## If-changed write semantics

Every transform writes its output **only when it differs from the existing file
after normalizing `fetched_at` out** (the timestamp changes every run). A no-op
run leaves the file's content/mtime untouched, so the workflow's commit-if-changed
has nothing to stage. If you add another per-run-volatile field, normalize it the
same way (see each file's `unchanged_ignores_fetched_at` /
`if_changed_second_run_leaves_file_untouched` tests).

## Fixture testing pattern

Each transform sub-file carries an inline `#[cfg(test)] mod tests` — there are no
separate integration test crates under `tests/` (that directory holds only
`fixtures/`). Tests load small real-shaped snippets via:

```rust
include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/<source>/<file>"))
```

Fixtures live under `tests/fixtures/`:
- `aa_raw_sample.json`
- `arena/{text,vision,code,agent,search,document}.json`
- `epoch/*.csv` (incl. `malformed.csv` for parse-resilience, `epoch_capabilities_index.csv` for the Index kind, and `*_external.csv` for the internal+external merge)
- `llmstats/{rankings_sample,models_sample}.json`

Tests must run under `--features pipeline` (the bin is feature-gated). The
parsing here is the fiddly part of the pipeline, so it is the most test-covered
code in the repo (aa 23 / arena 31 / epoch 38 / llmstats 19 tests at last count).
The `multi.rs` `test_committed_aa_json_deserializes_and_helpers_match` test
additionally guards the *committed* `data/v2/aa.json` against the schema.
