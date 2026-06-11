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

One line above all content (`draw_source_bar`). Left: a bracketed label per `SOURCES` entry. Right (active source only): freshness + self-reported badge.

**Left — per-source label** (leading space, then `[{name}] ` per source):

| State | Render | Color |
|-------|--------|-------|
| Active (loaded) | `[{name}] ` | `Color::Cyan` + `Modifier::BOLD` |
| Loaded, inactive | `[{name}] ` | `Color::DarkGray` |
| Loading | `[{name}] ` + `◐ ` | label DarkGray, `◐` (U+25D0) Yellow |
| Failed | `[{name}] ` + `✗ ` | label DarkGray, `✗` (U+2717) Red |

**Right — active source freshness** (right-aligned, omitted unless the active source is loaded):
- `fetched {relative}` in `Color::DarkGray` (relative time from `SourceMeta.fetched_at`)
- ` self-reported` in `Color::Yellow` when `SourceMeta.verified == false` (LLM Stats)

---

## 3. Source Switcher Keys & Semantics

| Key | Action |
|-----|--------|
| `}` | Next data source |
| `{` | Previous data source |

- `[` / `]` stay **global** PrevTab/NextTab — the braces mirror them (brackets move between tabs, braces between data sources within the tab).
- Switching triggers `rebuild()` against the new `SourceFile` and **clears compare selections and search, and resets sort to the source default** (`multi::default_sort`).
- Selecting a still-loading or failed source shows the standard loading/error state in the content area; sources load progressively (the tab is usable as soon as any source lands).
- Footer hint: ` { } ` (Yellow) + `source`. Help popup has a `Data Source` section (`}` Next / `{` Previous).

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

**Per-source default sort** (`multi::default_sort`): `ReleaseDate` (desc) when any model carries a release date, else `Metric(0)` (first metric, desc). Arena has no dates → defaults to first metric.

**Quick sorts:**

| Key | Sort | Notes |
|-----|------|-------|
| `1` | First metric (`Metric(0)`) | `quick_sort_metric_first` — AA maps to intelligence |
| `2` | Release date | maps to date |
| `3` | First `TokensPerSec` metric | `quick_sort_speed` — **no-op when the source has none** (returns `None`) |

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

Identity block + one section per metric `group` (`groups_in_order`), values formatted by `MetricKind` (`format_metric_value`), with a final source-attribution line. Uses `ScrollablePanel` + `detail_scroll`; `reset_detail_scroll()` on every selection/filter/sort/rebuild.

**Identity block**: display name (White+BOLD), id (DarkGray), then 2-column `ColumnWidths` label-value rows (`[28%, 22%, 28%, 22%]`, 2-space indent): Creator / Released, reasoning / effort / variant (each only when present).

**Label column sizing (metric rows)**: the metric-label column is sized to the source's **longest metric label** + 2-space gap (so values never collide with long labels like "Epoch Capabilities Index"), capped at `width - (indent + 12)` (min 8) so a pathological label can't push values off-panel.

**Direction arrow**: each metric row appends a dim `↑` (higher-is-better) / `↓` (lower-is-better) in `Color::DarkGray`. The arrow **counts toward the label-column budget** — the label is truncated to leave room for `" ↑"` (2 cols).

**Value cell suffixes**:
- Elo cells with a confidence interval append ` ±{ci:.0}`.
- Any cell carrying a per-model `date` appends a dim `(upd {date})`.
- Missing value: em-dash `\u{2014}` in `Color::DarkGray`.

**Section headers**: uniform-kind groups get a dim scale suffix (`group_kind_blurb` + `ui::section_header_line_with_suffix`); mixed-kind groups fall back to the plain `── Title ──` header (`ui::section_header_line`), both filling to panel width with `\u{2500}` in `Color::DarkGray`.

**Source attribution** (final line, after a blank): `Source: {name}` in `Color::Gray` + ` (self-reported)` in `Color::Yellow` when `SourceMeta.verified == false`.

The detail builder (`build_benchmark_detail_lines`) returns owned `Line<'static>` so the compare-mode detail overlay can reuse it.

---

## 9. Glossary Popup

Curated per-benchmark descriptions for the active source. State `show_glossary` + `glossary_scroll`.

- Key: `i` toggles. **Not** `g`/`G` (those are global list-nav jump-first/last).
- **Size**: `centered_rect(60, 70)` — 60% width, 70% height. **Border**: `Color::Cyan`. **Background**: `Clear` first.
- **Title**: `" Benchmark Glossary - i or Esc to close (Up/Down to scroll) "`.
- Content (`build_glossary_lines`): every metric in display order (`groups_in_order` → `metric_indices_in_group`) under the same dash-padded section headers as the detail panel. Per metric:
  1. label (`Color::Gray` + BOLD) + dim direction arrow (DarkGray)
  2. meta line (DarkGray): kind blurb + `updated {date}` when `last_updated` is set (date-portion only — sources emit `YYYY-MM-DD` or RFC3339)
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
| Other | `Color::DarkGray` |

Region grouping key `[5]`: `Color::Yellow` when active, `Color::DarkGray` when not.

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

The **reasoning filter** (`7`) is auto-hidden (key no-op, footer/help row omitted) when no model in the active source carries a reasoning status. The **open-weights filter** (`4`, `SourceFilter`) and O/C indicators read `ModelRow.open_weights`; em-dash where unknown. (`SourceFilter`/`CycleBenchmarkSource` are the open/closed-**weights** filter — distinct from the `{`/`}` data-source switcher.)

---

## 11. H2H Table

Rendered inside `ScrollablePanel` with `.with_wrap(false)`. Sections = metric `groups`, rows = metrics, kind-based formatting (`format_metric_value`).

**Section header rows**:

```
─── Section ────────────────
```

Style: `Color::DarkGray`. Fills to panel width with `\u{2500}`.

**Value formats** follow `MetricKind`: Index/Percentage `{:.1}`/`{:.1}%`, Elo `{:.0}`, TokensPerSec `{:.0}`, Seconds `{:.2}s`, UsdPerMTok `${:.2}`.

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
