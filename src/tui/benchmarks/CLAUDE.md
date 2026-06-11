# Benchmarks Tab

Registry-driven multi-source tab: a data-source switcher over 4 sources (AA,
Epoch, Arena, LLM Stats), with every view (Detail, H2H, Scatter, Radar, sort,
sidebar) rendered from per-source `MetricDef`s rather than hardcoded field names.

## Files
- `app.rs` — `BenchmarksApp` state, `BenchmarkFocus` (Creators/List/Details/Compare), `BottomView` (H2H/Scatter/Radar), `CreatorGrouping`, sort/filter types. Key fields: `active_source: usize`, `sort_key: SortKey`, `sort_descending`, `bottom_view`, `scatter_x`/`scatter_y`, `radar_group`, `show_sort_picker`/`sort_picker_selected`, `show_glossary`/`glossary_scroll: ScrollOffset`, `detail_scroll: ScrollOffset`. `MultiStore` itself lives on the top-level `App` (`app.multi_store`), not on `BenchmarksApp` — sub-app methods take `&SourceFile` as a parameter.
- `render.rs` — `draw_benchmarks_main()`, `draw_source_bar()`, `build_benchmark_detail_lines()`, `build_glossary_lines()`, `draw_glossary()`, `draw_sort_picker()`, `compare_colors()` (8-color palette)
- `compare.rs` — `draw_h2h_table_generic()`, `draw_scatter()`
- `radar.rs` — `draw_radar()`, `axes_for_group()`, spoke/polygon math (`MAX_AXES = 6`)

## Source switcher
- 1-line source bar above the existing content (`draw_source_bar`): bracketed label per `SOURCES` entry — active = Cyan+BOLD, loaded-inactive = DarkGray, loading = label + `◐` Yellow, failed = label + `✗` Red. Right-aligned for the active source: `fetched {relative}` (DarkGray) + ` self-reported` (Yellow) when `verified == false`.
- `{` / `}` cycle data source prev/next (tab-local; `[` / `]` stay global PrevTab/NextTab). Switching triggers a `rebuild()` against the new `SourceFile` and clears compare selections/search and resets sort to the source default.
- Sources load progressively; selecting a still-loading/failed source shows the standard loading/error state.

## Glossary popup (`i`)
- State `show_glossary` + `glossary_scroll`. `draw_glossary` renders a `ScrollablePanel` (centered 60% × 70%, Cyan border, `Clear` background) over `build_glossary_lines(file, width)`: every metric in display order under the same dash-padded section headers as the detail panel. Per metric: label (Gray+BOLD) + dim direction arrow; a meta line (kind blurb + `updated {date}` when `last_updated` is set, date-portion only); then the `description` (White) or an em-dash when `None`.
- Key interception: `event.rs::handle_glossary_keys` runs **before** the global handler so `q` doesn't quit — `i`/`Esc` close, arrows/`j`/`k` scroll, everything else is swallowed. Scroll resets on open and on source switch.

## Key Patterns
- Browse mode: Creators panel (left) + model list (center) + detail (right). Compare mode: selected models (left) + visualization (right, switchable via `BottomView`, `v` cycles).
- `compare_colors()` returns 8 colors indexed modulo — H2H columns, scatter points, radar polygons, legend.
- Radar presets are **dynamic**: `multi::radar_groups(file)` = metric groups with ≥3 `higher_is_better` metrics; `axes_for_group` takes the first `MAX_AXES (6)` higher-is-better metrics of the active group. `a` cycles groups. Radar needs ≥3 axes + ≥1 selection to draw.
- Scatter axes are metric-index state (`scatter_x`/`scatter_y`); `x`/`y` cycle the active source's metrics; auto log-scale when value range ratio > 2.5.
- Sort: dynamic `sort_options(file)` = `[ReleaseDate, Name, every metric]` (scrollable picker, `s` opens, `S` toggles direction). Quick sorts: `1` = first metric, `2` = release date, `3` = first `TokensPerSec` metric (no-op when the source has none — `quick_sort_speed` returns `None`). `default_sort` = ReleaseDate when any model has a date, else `Metric(0)`.
- Detail panel uses `ScrollablePanel` + `detail_scroll`; `reset_detail_scroll()` on every selection/filter/sort/rebuild.
- Detail metric rows append a dim direction arrow (`↑`/`↓`) that counts toward the label column budget (label truncated to leave `" ↑"`). Elo cells append ` ±{ci}`; cells with a per-model date append a dim `(upd {date})`. Final line is the source-attribution line (`Source: {name}` + ` (self-reported)` when unverified).
- Uniform-kind groups get a dim scale suffix on the section header (`group_kind_blurb` + `ui::section_header_line_with_suffix`); mixed-kind groups get the plain header.

## Filters & creator grouping
- Reasoning filter (`7`) is auto-hidden (key no-op, footer/help row omitted) when no model in the active source carries a reasoning status. Open-weights filter (`4`, `CycleBenchmarkSource`/`SourceFilter` — the open/closed *weights* filter, **not** the data source) + O/C indicators come from `ModelRow.open_weights` (AA via `apply_model_traits`, others via `enrich_from_models_dev`/`creator_openness`); em-dash where unknown.
- Region grouping (`5`): US, China, Europe, Middle East, South Korea, Canada, Other. Type grouping (`6`): Startup, Giant, Research. Group headers are non-selectable separators (same pattern as Models tab).

## Gotchas
- `SourceFilter`/`CycleBenchmarkSource` are the open/closed-**weights** filter — the **data**-source switcher uses `active_source` + `CycleDataSourcePrev`/`CycleDataSourceNext`. Do not conflate them.
- Compare mode list shows compact rows with reasoning/source indicators (R/AR/NR + O/C) — different format from browse mode.
- Detail/glossary lines are built as `Line<'static>` (owned) so the compare-mode detail overlay can reuse `build_benchmark_detail_lines`.
- Use `line.width()` (unicode-aware) for label-column truncation; the arrow is width-1.
