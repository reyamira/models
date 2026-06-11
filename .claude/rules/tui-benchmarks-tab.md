---
description: Benchmarks tab design conventions — multi-source switcher, browse/compare modes, creator sidebar, H2H table, scatter plot, radar chart, sort picker, glossary popup
globs:
  - src/tui/benchmarks/**
---

# Benchmarks Tab Design Conventions

Tab-specific patterns only. For shared colors, borders, focus, search, footer, and scrollbars see `tui-style-guide.md`.

The Benchmarks tab is **multi-source and registry-driven**: a data-source switcher selects one of 4 sources (Artificial Analysis, Epoch AI, Arena, LLM Stats), and every view renders from per-source `MetricDef`s shipped in the data file — there are no hardcoded benchmark field names. The active source is `BenchmarksApp.active_source` (an index into the compiled-in `SOURCES`); the loaded `SourceFile` is read via `app.multi_store.file(active_source)`.

---

## 1. Top-Level Layout

The benchmarks main area is split source bar + content:

```
Length(1)  -- Source bar (section 2)
Min(0)     -- Content (browse or compare mode)
```

**Browse mode content** (< 2 selections):

```
Percentage(20)  -- Creators sidebar
Percentage(40)  -- Benchmark list
Percentage(40)  -- Detail panel (ScrollablePanel)
```

**Compare mode content** (≥ 2 selections):

```
Length(max(area_width * 30 / 100, 35))  -- Compact list (or creators if toggled)
Min(0)                                   -- Comparison panel
```

Comparison panel internal split:

```
Constraint::Length(1)  -- Subtab bar ([H2H] [Scatter] [Radar])
Constraint::Min(0)     -- Active view
```

---

## 2. Source Bar

One line above all content (`draw_source_bar`). Left: a bracketed label per `SOURCES` entry, then the switch hint. Right (active source only): freshness + self-reported badge.

**Left — per-source label** (leading space, then `[{name}] ` per source):

| State | Render | Color |
|-------|--------|-------|
| Active (loaded) | `[{name}] ` | `Color::Cyan` + `Modifier::BOLD` |
| Loaded, inactive | `[{name}] ` | `Color::DarkGray` |
| Loading | `[{name}] ` + `◐ ` | label DarkGray, `◐` (U+25D0) Yellow |
| Failed | `[{name}] ` + `✗ ` | label DarkGray, `✗` (U+2717) Red |

After the labels: `{ } switch source` in `Color::DarkGray` (mirrors the header's `[/] switch tabs` hint; the footer carries no source-switch hint).

**Source identity appears here only** — the detail panel has no attribution line and switching sources does not flash the source name in the status bar.

**Right — active source freshness** (right-aligned, omitted unless the active source is loaded):
- `fetched {relative}` in `Color::DarkGray` (relative time from `SourceMeta.fetched_at`)
- ` self-reported` in `Color::Yellow` when `SourceMeta.verified == false` (generic mechanism; **no source currently sets `verified == false`** — LLM Stats was flipped to verified on 2026-06-11, so this badge is dormant)

---

## 3. Source Switcher Keys & Semantics

| Key | Action |
|-----|--------|
| `}` | Next data source |
| `{` | Previous data source |
| `r` | Refresh the active source |

- `[` / `]` stay **global** PrevTab/NextTab — the braces mirror them (brackets move between tabs, braces between data sources within the tab).
- Switching is **state-preserving** (`switch_source` → `reset_for_source`): it keeps `search_query`, the open-weights filter, creator grouping, the reasoning filter (reset to `All` only when the target source carries no reasoning metadata), the comparator mode, and a `ReleaseDate`/`Name` sort with direction; a `Metric(i)` sort is source-specific and falls back to the new source's `default_sort`. Per-source view state still resets: selection/creator indices, scrolls, scatter axes, radar group, `bottom_view`, popups.
- Compare selections carry over by **exact `ModelRow.id` match, then normalized match** (`benchmarks::normalize_id` — the enrichment normalizer; the four sources spell the same model differently: `grok-4-3` vs `grok-4.3`, `DeepSeek-V3-1`, `zai-org/GLM-4-7`, dated Arena ids), order-preserving (compare-color stability), deduped (normalization can fold a base model and its `-thinking` variant together); ids with no counterpart drop, and `update_bottom_view` demotes compare→browse when <2 survive.
- `r` is stale-while-revalidate: current data keeps rendering while `fetch_source` re-runs (`Message::RefreshBenchmarkSource` → `Message::DataSourceRefreshed`). Success re-runs enrichment + a state-preserving rebuild (`rebuild_preserving`: keeps sort/search/filters, remaps selections and the focused row by id, stale `Metric(i)` falls back); failure keeps the existing file (`set_failed` is never applied to good data) with status `Failed to refresh {name} — keeping current data`. A `Loading`/`Failed` source stays recoverable via `r`.
- Selecting a still-loading or failed source shows the standard loading/error state in the content area; sources load progressively (the tab is usable as soon as any source lands).
- Footer hints: ` r ` (Yellow) + `refresh` in both browse and compare modes; the source-switch hint lives in the source bar, not the footer. Help popup's `Data Source` section: `}` Next / `{` Previous / `r` Refresh active source.

---

## 4. Subtab Bar (Compare Mode)

Format ` [H2H]  [Scatter]  [Radar] ` (each label space-padded inside brackets):

- Active view: `Color::Cyan` + `Modifier::BOLD`
- Inactive views: `Color::DarkGray`
- `v` cycles views (requires ≥ 2 selections)

---

## 5. Compare Palette

8 colors, indexed modulo 8 (`compare_colors`). Used for selection markers, H2H value columns, scatter points, radar polygons, and legend entries.

```rust
[Red, Green, Blue, Yellow, Magenta, Cyan, LightRed, LightGreen]
```

Selection marker in the compact list: `●` (U+25CF) in the model's compare color.

---

## 6. Sort

Sort keys are **dynamic per source**: `SortKey = ReleaseDate | Name | Metric(i)` where `Metric(i)` indexes `file.metrics`.

**Per-source default sort** (`multi::default_sort`): `ReleaseDate` (desc) when any model carries a release date, else `Metric(0)` (first metric, desc). Arena's committed file has no dates, but runtime models.dev enrichment backfills most of them before the rebuild — so Arena's *runtime* default is ReleaseDate; the `Metric(0)` fallback applies only pre-enrichment.

**Quick sorts** live on the back half of the number row — their targets are per-source, so a stable-shaped footer can't honestly hint them; they are **help-popup-only** (`1`-`3` are deliberately unbound on this tab):

| Key | Sort | Notes |
|-----|------|-------|
| `8` | First metric (`Metric(0)`) | `quick_sort_metric_first` — help line carries the active source's full first-metric label |
| `9` | Release date | maps to date |
| `0` | First `TokensPerSec` metric | `quick_sort_speed` — **no-op when the source has none** (returns `None`); help line hidden then |

**The browse footer carries only stable-shaped hints** (`4 weights`, `5-6 group`, `7 reasoning` when available, `s sort`, `a avg`, `r refresh`, …) — no quick-sort hints, so the footer's shape never changes with the data source.

`s` opens the sort picker, `S` toggles direction. Re-pressing the same quick-sort key toggles direction (`quick_sort`).

**Null-filter semantics:** sorting pushes models missing the sort metric to the end — a model with no score for the active sort key sorts after every model that has one.

**Sort indicator** in the list title: ` {arrow}{label}` — `↓`/`↑` + the sort key's short label (`Date`, `Name`, or the metric's `label`; em-dash when the metric index is stale).

---

## 7. Sort Picker Popup

- Dynamic options = `sort_options(file)` = `[Release Date, Name, every metric in file order]` (scrollable, since metric count varies by source).
- **Size**: `centered_rect_fixed(30, …)` — 30 chars wide, height clamped to fit all options + border.
- **Border**: `Color::Cyan`. **Title**: `" Sort By "`.
- Current sort highlighted with `▼` (descending) / `▲` (ascending) prefix in `Color::Cyan` + `Modifier::BOLD`; other options default with a `Color::DarkGray` prefix space.
- `s` opens, `Enter` confirms, `Esc` (or `s`) cancels. The picker intercepts keys before the global handler.

---

## 8. Detail Panel (Browse Mode)

Identity block + one section per metric `group` (`groups_in_order`), values formatted by `MetricKind` (`format_metric_value`). **No source-attribution line** — source identity lives in the source bar only. Uses `ScrollablePanel` + `detail_scroll`; `reset_detail_scroll()` on every selection/filter/sort/rebuild.

**Panel title** reflects the comparator mode: ` Details · vs field avg `, ` Details · vs peers (±6mo) `, ` Details · rank `, plain ` Details ` when Off.

**Identity block**: display name (White+BOLD), id (DarkGray), then 2-column `ColumnWidths` label-value rows (`[28%, 22%, 28%, 22%]`, 2-space indent), in order: Creator / Released, Reasoning / Effort, Weights / Context, **Region / Type, Tools / Output**, Variant (Variant only when present).

- **Region / Type** derive from the creator slug (`CreatorRegion::from_creator` / `CreatorType::from_creator`, colored by `.color()`) — works for every source. Guarded on a non-empty creator: an unmatched/empty creator shows em-dash for both rather than a misleading `Other`/`Startup`.
- **Tools** (`supports_tools`: Yes Green / No DarkGray / em-dash) and **Output** (`max_output`, via `format_tokens`) are backfilled at runtime from a models.dev match (`finalize_loaded_source`, both the AA `apply_model_traits` and the generic `enrich_from_models_dev` paths) onto `ModelRow.supports_tools`/`max_output`. They populate where the source model matched a models.dev entry — em-dash elsewhere, so coverage is intentionally uneven across sources.

**Label column sizing (metric rows)**: the metric-label column is sized to the source's **longest metric label** + 2-space gap (so values never collide with long labels like "Epoch Capabilities Index"), capped at `width - (indent + 12)` (min 8) so a pathological label can't push values off-panel.

**Direction**: metric rows carry **no** per-metric direction marker — scale and direction live in the section header suffix (see "Section headers" below). The metric-label column is a pure gutter (longest label + 4 spaces).

**Value cell suffixes**:
- Elo cells with a confidence interval append ` ±{ci:.0}`.
- Cells carrying a `votes` sample size (Arena) append a dim ` · {format_tokens(votes)} votes` (DarkGray) — a confidence signal alongside the CI. Per-cell `date`s are **not** rendered in the score rows (dropped in 8828a67; freshness lives in the glossary meta line).
- Missing value: em-dash `\u{2014}` in `Color::DarkGray`.

**Comparator column** (a true aligned column in `Color::DarkGray`: every value cell — value + ±ci + votes — is padded to the model's widest value cell + 2, so comparator cells share one x-position; `MetricRowLayout` + `value_cell_parts`/`value_cell_width` keep render and measurement from drifting): `a` in browse mode cycles `ComparatorMode` — `FieldAvg → PeerAvg → Rank → Off`, default `FieldAvg`, persisted across source switches and refreshes. Formats: `avg {v}` (mean over all models in the source with that metric), `peers({n}) {v}` (±183-day release-date window around the selected model, self-excluded; em-dash when the model is dateless or no peers), `#{rank}/{n}` (1-based, direction-aware via `higher_is_better`; em-dash when the model lacks the value). FieldAvg/PeerAvg still render when the model's own value is missing; Rank cannot. Computation helpers (`field_avg` / `peer_avg` / `rank`) live in `benchmarks/multi.rs` and operate on the **full** model list, never the filtered view. The `a` binding is mode-dependent: browse = comparator cycle, compare-mode Radar view = radar-preset cycle.

**Section headers** (`group_header_suffix`): combine a uniform-kind scale blurb (`group_kind_blurb`) and a uniform-direction blurb (`group_direction_blurb`) into the header suffix — `(kind · dir)` when both uniform (e.g. `── Pricing ($ per 1M tokens · lower is better) ──`), kind alone or direction alone when only one is uniform, and a plain `── Title ──` header when the group is mixed on both (e.g. AA Performance: speed ↑, latency ↓). Suffixed headers use `ui::section_header_line_with_suffix`, plain headers `ui::section_header_line`; both fill to panel width with `\u{2500}` in `Color::DarkGray`.

**Open in browser** (`o`): builds the URL via `SourceDescriptor::model_url`, not `{file.source.url}/models/{id}`. AA / LLM Stats have real `/models/{id}` pages; Epoch uses a normalized slug (last path segment, lowercased, `.` → `-`) with an async HEAD-probe (≤3s) falling back to `https://epoch.ai/data/ai-models` on non-200; Arena has no per-model pages → always `https://arena.ai/leaderboard/text`. Verified live 2026-06-11.

The detail builder (`build_benchmark_detail_lines`) returns owned `Line<'static>` so the compare-mode detail overlay can reuse it.

---

## 9. Glossary Popup

Curated per-benchmark descriptions for the active source. State `show_glossary` + `glossary_scroll`.

- Key: `i` toggles. **Not** `g`/`G` (those are global list-nav jump-first/last).
- **Size**: `centered_rect(60, 70)` — 60% width, 70% height. **Border**: `Color::Cyan`. **Background**: `Clear` first.
- **Title**: `" Benchmark Glossary - i or Esc to close (Up/Down to scroll) "`.
- Content (`build_glossary_lines`): every metric in display order (`groups_in_order` → `metric_indices_in_group`) under the same dash-padded section headers as the detail panel. Per metric:
  1. label (`Color::Gray` + BOLD) — no direction marker
  2. meta line (DarkGray): `{kind blurb} \u{00B7} {direction blurb}` (per-metric direction lives here, so mixed-direction groups stay unambiguous) + `  updated {date}` when `last_updated` is set (date-portion only — sources emit `YYYY-MM-DD` or RFC3339)
  3. description (`Color::White`), or an em-dash line when `description` is `None`
- Blank line between entries; `"No metrics for this source"` (DarkGray) when empty.
- **Key interception**: `handle_glossary_keys` runs before the global handler so `q` is swallowed (doesn't quit). `i`/`Esc` close; arrows / `j` / `k` scroll; all other keys swallowed. Scroll resets on open and on source switch.

---

## 10. Creator Sidebar

**"All" item**: `"All"` in `Color::Green` + `" ({count})"` in default (filtered creator count).

**Group header items** (when grouping active): full-width colored `── Label ──────` header (same pattern as Models tab), colored by group classification + `Modifier::BOLD`.

**Creator items** (ungrouped): `"{name} ({count})"` — name truncated to available width, count in `Color::Gray`. When grouping active, a short colored tag is appended.

**Region grouping colors** (`[5]`):

| Region | Color |
|--------|-------|
| US | `Color::Blue` |
| China | `Color::Red` |
| Europe | `Color::Magenta` |
| Middle East | `Color::Yellow` |
| South Korea | `Color::Cyan` |
| Canada | `Color::Green` |
| India | `Color::Rgb(255, 153, 51)` (saffron) |
| Other | `Color::DarkGray` |

Region grouping key `[5]`: `Color::Yellow` when active, `Color::DarkGray` when not.

**Creator classification** is table-driven (`CreatorClass` / `CREATOR_CLASSES` in `app.rs`): one entity per row carrying its `region`, `ctype`, and every per-source slug alias (the four sources name the same lab differently — `alibaba`/`qwen`, `aws`/`amazon`, `kimi`/`moonshot`/`moonshotai`, plus models.dev provider-id variants like `*-coding-plan` that the runtime enrichment can assign to empty-creator rows). `CreatorRegion::from_creator` / `CreatorType::from_creator` both resolve through this one table (`creator_class`), so region and type can't disagree. `region` is factual (HQ country); `ctype` (Giant/Startup/Research) is a documented convention — Giant = pre-existing large corp where AI isn't core; Research = academic/nonprofit/institute; Startup = AI-first company regardless of size. Unknown slugs fall back to `Other`/`Startup`.

**Type grouping colors** (`[6]`):

| Type | Color |
|------|-------|
| Startup | `Color::Green` |
| Giant | `Color::Blue` |
| Research | `Color::Magenta` |

Type grouping key `[6]`: `Color::Magenta` when active, `Color::DarkGray` when not.

**Filter row**:

```
[5] Rgn  [6] Type       (ungrouped)
[5] Region  [6] Type    (region grouping active — label expands)
```

**Reasoning/Source indicators** in compact list rows:

| Indicator | Chars | Color |
|-----------|-------|-------|
| Reasoning | `"R "` | `Color::Cyan` |
| Adaptive Reasoning | `"AR"` | `Color::Yellow` |
| Non-reasoning | `"NR "` | `Color::DarkGray` |
| Open source | `"O"` | `Color::Green` |
| Closed source | `"C"` | `Color::Red` |

The **reasoning filter** (`7`) is auto-hidden (key no-op, footer/help row omitted) when no model in the active source carries a reasoning status. `reasoning_status` comes from two layers: name-parsing at transform time (parenthetical/suffix markers like `(Reasoning)`/`(Adaptive)`/`_thinking`; AA's API names are the richest, and AA is the only source that emits an explicit `NonReasoning`/`Adaptive`), then a runtime fill from models.dev's `reasoning` capability flag (`true → Reasoning` only, only where name-parsing left `None`; a models.dev `false` is **not** mapped to `NonReasoning` because it's provider-unreliable). The models.dev fill is what makes the filter meaningful on Epoch/Arena/LLM Stats, whose own names rarely mark reasoning. The **open-weights filter** (`4`, `SourceFilter`) and O/C indicators read `ModelRow.open_weights`; em-dash where unknown. (`SourceFilter`/`CycleBenchmarkSource` are the open/closed-**weights** filter — distinct from the `{`/`}` data-source switcher.)

---

## 11. H2H Table

Rendered inside `ScrollablePanel` with `.with_wrap(false)`. Sections = metric `groups`, rows = metrics, kind-based formatting (`format_metric_value`).

**Section header rows**:

```
─── Section ────────────────
```

Style: `Color::DarkGray`. Fills to panel width with `\u{2500}`.

**Value formats** follow `MetricKind`: Index/Percentage `{:.1}`/`{:.1}%`, Elo `{:.0}`, TokensPerSec `{:.0} tok/s`, Seconds `{:.2}s`, UsdPerMTok `${:.2}`.

**Winner highlighting**: best value per row (respecting `higher_is_better`) shown in compare color + `Modifier::BOLD`; non-best values in compare color without bold.

**Wins row**: prefix `"★ Wins"` (Yellow + BOLD), count per model in compare color + BOLD.

**Missing values**: em-dash `\u{2014}` in `Color::DarkGray`.

---

## 12. Scatter Plot

- Axes are metric-index state (`scatter_x`/`scatter_y`); `x`/`y` cycle the active source's metrics.
- Background points: `Color::DarkGray`; selected model points in compare palette colors.
- Average crosshair lines (horizontal + vertical): `Color::DarkGray`.
- Auto log-scale applied per axis when the value range is skewed (ratio > 2.5).

---

## 13. Radar Chart

Presets are **dynamic per source**: `multi::radar_groups(file)` = metric groups with **≥ 3 `higher_is_better` metrics** (in first-appearance order). This keeps Performance/Pricing-style groups off the radar. `axes_for_group` builds axes from the **first `MAX_AXES` (6)** higher-is-better metrics of the active group.

- `a` cycles groups (`radar_group` index). Needs **≥ 3 axes and ≥ 1 selection** to draw; otherwise an empty bordered panel with the group label in the title.
- Spoke lines from center: `Color::DarkGray`. Model polygons: compare palette colors. Axis labels offset ~56 units from center, wrapped at 16 chars.
- Legend uses `ComparisonLegend` (see section 15).

---

## 14. Detail Overlay (Compare Mode)

Full model detail shown as an overlay when `d` is pressed in compare mode (reuses `build_benchmark_detail_lines`):

- **Size**: `centered_rect(60, 75)` — 60% width, 75% height
- **Border**: `Color::Cyan`. **Title**: `" Model Detail (Esc to close) "`. Background: `Clear` first.
- Must intercept global keys (`q`, `?`, etc.) to prevent pass-through.

---

## 15. ComparisonLegend Widget

Used in scatter and radar views. Reusable widget from `src/tui/widgets/comparison_legend.rs`.

- **Average row**: name `"Avg"`, color `Color::Indexed(250)` (light gray), marker `┅` (U+2505)
- **Model rows**: name truncated to fit, compare palette color, marker `●` (U+25CF)
- Value width: 6 chars per column

---

## 16. Loading & Empty States

- Active source still loading / not yet landed: the content area shows the standard loading state (`active_is_loading`); the detail/list panels render `"Loading..."` (Yellow) appropriately.
- Active source failed: standard error state in the content area.
- A source that produced no metrics yields a `"No metrics for this source"` glossary fallback (DarkGray).
