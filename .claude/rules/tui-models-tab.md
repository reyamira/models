---
description: Models tab design conventions — 3-column layout, RTFO indicators, provider list, model list columns, copy/open keybindings, detail sections
globs:
  - src/tui/models/**
---

# Models Tab Design Conventions

Tab-specific patterns only. For shared colors, borders, focus, search, footer, and scrollbars see `tui-style-guide.md`.

---

## 1. Layout

```
Percentage(20)   -- Providers panel (with filter row)
Percentage(45)   -- Model list
Percentage(35)   -- Right panel (provider detail top + model detail bottom)
```

**Providers panel internal split** (rendered manually — outer block drawn first, inner area split):

```rust
Constraint::Length(1)  -- Filter toggles row
Constraint::Min(0)     -- Provider list (stateful)
```

**Right panel vertical split** (dynamic height — provider detail is auto-sized):

```rust
Constraint::Length(provider_h)  -- Provider card (visual height computed from wrapped lines + 2 borders)
Constraint::Min(0)              -- Model detail (ScrollablePanel)
```

Provider card height is computed as the sum of visual wrapped line heights + 2 (borders). Word-wrap adds +1 slack per wrapped line beyond `div_ceil` estimate.

---

## 2. RTFO Indicators

| Indicator | Active char | Active color | Inactive char | Inactive color |
|-----------|-------------|--------------|---------------|----------------|
| Reasoning | `R` | `Color::Cyan` | `·` | `Color::DarkGray` |
| Tools | `T` | `Color::Yellow` | `·` | `Color::DarkGray` |
| Files | `F` | `Color::Magenta` | `·` | `Color::DarkGray` |
| Open weights | `O` | `Color::Green` | `C` | `Color::Red` |

Total width: **5 chars** — 4 indicator chars + 1 trailing space (`"RTFO "`). The trailing space separates indicators from the model name column.

In the detail panel, capabilities expand to `Yes`/`No` values using the same colors (e.g., Reasoning `Yes` = Cyan, `No` = DarkGray).

---

## 3. Provider List

- **"All" item**: text `Color::Green`, format `"All ({count})"` where count is the filtered model count
- **Category header items** (`ProviderListItem::CategoryHeader`): non-selectable, rendered as:
  ```
  ── Label ──────────────────
  ```
  Color: `cat.color()` + `Modifier::BOLD`. Trailing `\u{2500}` chars fill to inner panel width minus 2.
- **Provider items** (`ProviderListItem::Provider`): single-char category initial + provider ID + count in Gray:
  ```
  {initial} {provider_id} ({count})
  ```
  Initial color: `cat.color()`. Provider ID: default style. Count: `Color::Gray`.
- `find_selectable_index()` skips `CategoryHeader` items — they are never highlighted.

**Provider category colors** (from `ProviderCategory`): Origin=White, Cloud=Cyan, Inference=Yellow, Gateway=Green, Tool=Magenta. These are tab-specific — do not assume fixed colors; use `cat.color()`.

**Filter row** (1-line, rendered as plain `Paragraph` above the list):

```
[5] Cat  [6] Grp
```

- `[5]` key: category color when active (cycles through categories), `Color::DarkGray` when inactive. Label shows `cat.short_label()` when active, `"Cat"` when inactive.
- `[6]` key: `Color::Green` when grouping active, `Color::DarkGray` when not.

**Filter keys**: `1`=reasoning, `2`=tools, `3`=open weights, `4`=free, `5`=provider category (cycles), `6`=group by category

---

## 4. Model List Columns

Fixed column widths (left to right):

| Column | Width | Notes |
|--------|-------|-------|
| Caret | 2 | `"> "` focused / `"  "` unfocused |
| RTFO | 5 | 4 indicator chars + 1 space |
| Model name | dynamic | `inner_width - (2+5+8+8+8+3)`, minimum 10 |
| Input cost | 8 | right-aligned `{:>8}` |
| Output cost | 8 | right-aligned `{:>8}` |
| Context | 8 | right-aligned `{:>8}` |
| Gap spaces | 3 | one leading space per numeric column |

**Header row** — occupies list index 0, offset by +1 in `model_list_state.select()`:
- Default style: `Color::Yellow` + `Modifier::BOLD`
- Actively-sorted column: `Color::Cyan` + `Modifier::BOLD`
- "Input" and "Output" headers share the same style as the active sort column when sorting by cost
- Header leading whitespace is `"  "` (2 spaces, matching unfocused caret width)

**Sort indicator** in model list title:
- Format: ` {arrow}{label}` — prepended space, arrow `\u{2193}`/`\u{2191}`, then label
- Labels: `"date"` (ReleaseDate), `"cost"` (Cost), `"ctx"` (Context)
- `SortOrder::Default` → empty string (no indicator). Note: app launches with `ReleaseDate` descending, so a sort indicator is always visible on startup.

**Model list title format**:
```
" {provider_name} ({count}){sort} "                          -- no query, no filters
" {provider_name} ({count}){sort} [{filters}] "              -- filters active
" {provider_name} ({count}) [/{query}]{sort} "               -- search active
" {provider_name} ({count}) [/{query}] [{filters}]{sort} "   -- both
```
`provider_name` is the selected provider's display name, or `"Models"` when "All" is selected.

---

## 5. Copy / Open Keybindings

- `c` — copy model ID to clipboard
- `C` — copy full model reference (`{provider_id}/{model_id}`)
- `o` — open docs URL in browser
- `A` — open API URL in browser
- `r` — refresh models.dev data (async refetch; state-preserving — keeps
  search/filters/sort and tries to keep the selected provider/model by id; a
  failed refresh keeps the current data). Already-loaded benchmark sources are
  NOT re-enriched.

`o` and `A` hints are shown **conditionally** at the bottom of the provider detail card — only when the corresponding URL exists. Format (inline spans, no block):

```
" o " (Yellow) + "docs" + "  " + " A " (Yellow) + "api"
```

Either hint is omitted entirely if the URL is absent. The gap `"  "` between hints only appears when both are present.

---

## 6. Model Detail Sections

Detail sections rendered in this order, each preceded by a blank line:

1. **Identity** — model name (White + BOLD, DarkGray if deprecated), model ID (DarkGray), then a **Family + optional Status** row. Provider is intentionally **omitted** here — the Provider card directly above always shows the selected model's provider (`provider_detail_lines` keys off `entry.provider_id`), so repeating it is pure duplication. A blank line, then a **description** line (`Color::Gray`, wrapped) from models.dev `description` (~100% coverage — omitted only when absent/empty)
2. **Capabilities** — 2-column `two_pair_line` layout: Reasoning/Tools, Source/Files, Temp/**Structured**. `Structured` renders from `Model.structured_output` (`Option<bool>`, ~49% coverage) via a three-state `cap_val_opt` — Yes (Cyan) / No (DarkGray) / `—` (DarkGray, unknown-when-absent — this is why it lives here and **not** in the compact RTFO row, which is binary-only and stays 4-char `RTFO`). When the model carries `reasoning_options`, a **`Mode:`** line follows (single line, from `Model::reasoning_mode_summary()` — e.g. `budget_tokens 0–24.6k`, `effort`, `toggle`; budget ranges rounded with `format_tokens` like Limits); omitted when empty
3. **Pricing** — 2-column: Input/Output, Cache Read/Cache Write. `Free` = Green. `$0/M` = Green. Then **conditional rows, each rendered only when the model carries that cost** (most models show none): `Thinking: $X/M` (`cost.reasoning` — labeled "Thinking" to disambiguate from the Reasoning *capability*), `Audio In:`/`Audio Out:` (`cost.input_audio`/`output_audio`), and one **tier** line per `cost.tiers[]` entry — `Over {format_tokens(size)}: {in} / {out}` (falls back to `Tier:` when the tier has no size). Legacy `cost.context_over_200k` is intentionally **not** read (subsumed by `tiers`). CLI `models show`/`--json` mirror these (tiers serialized via `Serialize` on `CostTier`/`TierSpec`)
4. **Limits** — 3-column single line: Context / Input / Output (each `width/3` wide)
5. **Modalities** — Input: / Output: label-value pairs (no 2-column layout)
6. **Dates** — 2-column: Released/Knowledge, Updated (when present)

**Section headers** use `section_header_line(width, title)`:
```
── Title ──────────────────────
```
Style: `Color::DarkGray` + `Modifier::BOLD`. Fills to panel inner width with `\u{2500}`.

**2-column layout** (`two_pair_line`): each column is `inner_width / 2` chars. Labels Gray, values colored by type. Padding spaces fill each column to width.

**Deprecated models**: `text_color` = `Color::DarkGray` (instead of White) for all value spans. Status field shown as `"deprecated"` in `Color::Red`.

**Provider card** (top of right panel, separate from model detail):
- Title: `" Provider "`
- Border: always DarkGray (no focus coloring — this panel is not focusable)
- Content: provider name (Cyan + BOLD), Category/Docs/API/Env label-value pairs

---

## 7. Focus States

Three focus positions cycle left/right via `h`/`l`:

```
Focus::Providers  →  Focus::Models  →  Focus::Details
```

- Providers border: Cyan when focused
- Models border: Cyan when focused
- Details (`ScrollablePanel`): Cyan border when focused, scrollable

`reset_detail_scroll()` is called on every navigation, sort, filter, and search change.

---

## 8. Mouse

This tab is the **reference implementation** for TUI mouse support (`handle_models_mouse` + `mouse_tests` in `src/tui/models/`). See style guide §12 for the shared pattern.

- **Cached rects** (`ModelsApp`, written at render time): `provider_list_area` (bare list region below the filter-toggle row), `model_list_area` (the list inner area — the column header is list item 0), `provider_card_area`, `model_detail_area`.
- **Click:** provider row → focus Providers + select (category-header rows are skipped); model row → focus Models + select (item 0 is the header → ignored, `idx - 1` maps to the model); provider card or model detail → focus Details only.
- **Wheel (focus-then-scroll):** over providers → prev/next provider; over models → prev/next model; over the right panel → scroll the model detail.
- The model list renders into the **real** `model_list_state` so `offset()` is valid for click-to-select while scrolled (this is the `ListState` copy gotcha — see CLAUDE.md).
