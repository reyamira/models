# Benchmarks Module

Three-file architecture for AI model benchmarking: store, fetch, traits matching.

## Module Structure

- **store.rs** — `BenchmarkStore`, `BenchmarkEntry`, `ReasoningFilter`
  - `BenchmarkEntry`: single benchmark with 20+ score fields (`intelligence_index`, `coding_index`, `math_index`, `mmlu_pro`, etc.), reasoning status, effort level, variant tags, pricing
  - `parse_metadata()`: regex-based extraction of reasoning status / effort level / variant tag from parentheticals in model name (e.g., `"Claude (Reasoning, High Effort)"` → `reasoning_status=Reasoning`, `effort_level=high`, `display_name="Claude"`)
  - `BenchmarkStore`: thin wrapper; `from_entries()` calls `parse_metadata()` on all entries
  - `ReasoningFilter`: enum with `All` / `Reasoning` / `NonReasoning` and `matches()` helper

- **fetch.rs** — `BenchmarkFetcher`, `BenchmarkFetchResult`
  - Async HTTP client (reqwest) for jsDelivr CDN at `https://cdn.jsdelivr.net/gh/reyamira/models@main/data/benchmarks.json`
  - No caching, no ETag — fetches fresh on every launch
  - Result: `Fresh(Vec<BenchmarkEntry>)` or `Error`

- **traits.rs** — `apply_model_traits()`, `build_open_weights_map()`
  - Jaro-Winkler matching (MIN_SIMILARITY=0.85) of AA benchmark slugs to models.dev model IDs
  - Two-stage matching: (1) creator-scoped (AA creator → mapped models.dev provider), (2) global fallback across all models
  - Creator mappings: `meta→llama`, `kimi→moonshotai`, `aws→amazon-bedrock`, etc.
  - Known creator overrides for providers absent from models.dev (e.g., `ai2→open`, `ai21-labs→closed`)
  - Augments entries with: `reasoning`, `tool_call`, `context_window`, `max_output` from matched models.dev data

## Re-exports (mod.rs)

```rust
pub use fetch::{BenchmarkFetchResult, BenchmarkFetcher};
pub use store::{BenchmarkEntry, BenchmarkStore, ReasoningFilter, ReasoningStatus};
pub use traits::{apply_model_traits, build_open_weights_map};
```

## Key Gotchas

- `BenchmarkEntry` must always derive `Serialize` + `Deserialize`; new fields require `#[serde(default)]`
- AA API uses `0` as sentinel for missing data — upstream `update-benchmarks.yml` jq converts `0 → null`
- `parse_metadata()` uses three LazyLock regexes for parenthetical extraction, date detection, effort keyword matching
- Metadata parsing is destructive: stripped name stored in `display_name` (for UI), reasoning status overrides from base name if not already set in parens
- Matching requires both creator and slug non-empty; unmatched entries absent from the map (no label shown in UI)
