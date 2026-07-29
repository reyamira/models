---
description: Models tab design conventions — grouped/flat views, provider modal, lab resolution, RTFO indicators, model list columns, copy/open keybindings, detail sections
globs:
  - src/tui/models/**
---

# Models Tab Design Conventions

Tab-specific patterns only. For shared colors, borders, focus, search, footer, and scrollbars see `tui-style-guide.md`.

---

## 1. Layout & View Modes

```
Percentage(60)   -- Model list (grouped or flat)
Percentage(40)   -- Right panel
```

There is **no provider sidebar** (retired 2026-07-29): its filtering job moved
to the `p` provider modal (§3), its browse job to the grouped list itself.
Focus cycles two panels only: `Focus::Models ↔ Focus::Details`.

**View modes** (`ListMode`, derived by `list_mode()` — never stored):

| Mode | When | Rows |
|------|------|------|
| `Grouped` | All scope, no drill, `flat_view` off (**default**) | one per model name (`ModelGroup`) |
| `Offerings` | `drill_name` is `Some` (Enter on a grouped row) | that group's flat offering rows |
| `Flat` | provider-scoped, or the `V` toggle | flat per-offering rows |

- **Enter** (grouped) → push into the group's offerings; breadcrumb title
  `" Models ▸ {name} ({n} providers) "` (distinct-provider count, singular
  when 1). **Esc** pops back with `selected_group` preserved (guarded by
  `enter_drills_into_group_and_esc_pops_back`); Esc from a provider scope
  returns to All-grouped; only a top-level Esc clears search
  (`escape_back()` before `clear_search` in the `ClearSearch` arm).
- **`V`** toggles Grouped ↔ Flat for the All view, persisted as
  `config.display.flat_models`. Flat-All title: `" Models · flat ({count}) "`.
- Grouped title: `" Models ({group_count}) "`.
- Nav dispatch: `next_model`/`select_model_at_index`/etc. operate on the group
  list in Grouped mode via `nav_len`/`nav_selected`/`nav_select` — event.rs is
  mode-agnostic. Grouped list renders into `group_list_state` (header row at
  item 0, same +1 offset convention); the mouse handler picks the state by
  `list_mode()` and shares the `model_list_area` rect.

**Right panel**: in Grouped mode the detail takes the full height (no provider
card — meaningless for a multi-provider group). In Offerings/Flat modes the
split is unchanged:

```rust
Constraint::Length(provider_h)  -- Provider card (visual height computed from wrapped lines + 2 borders)
Constraint::Min(0)              -- Model detail (ScrollablePanel)
```

Provider card height is computed as the sum of visual wrapped line heights + 2 (borders). Word-wrap adds +1 slack per wrapped line beyond `div_ceil` estimate.

**Grouped row columns** (`draw_grouped_list`): caret 2 + RTFO 5 + name
(min 18) + greedy-kept columns Lab(16) / Providers(11) / Input(12) /
Output(12) / Context(12), sort column survives (same policy as §4). Ranges
render `min–max` (`fmt_cost_range` / `fmt_ctx_range`), collapsing when equal —
and context ranges within 5% collapse to one value (providers spell the same
window as 1,000,000 vs 1,048,576; `1M–1.0M` is noise). RTFO on grouped rows
uses **majority + dim-when-mixed** (`group_cap_char`): the majority value
picks the char; a genuine split renders it DarkGray. Grouped detail: `Lab:`
under the id, a `── Providers (N) ──` section (cheapest first, capped at 10
with an overflow hint) after the description; the rest describes the
representative (first) offering.

**Lab resolution** (`src/labs.rs`): models.dev's canonical `models/` registry
links offerings via `base_model`, but every published endpoint strips it — the
lab is reconstructed: exact name → paren-stripped name → family → id-prefix,
against `models.json` (fetched at startup, 5s timeout, curated-table fallback
offline). Canonical families need **≥2 models** to be trusted (Thinking
Machines' lone "Inkling" claims family `ling`, which would mislabel
InclusionAI's Ling line — guarded by a curated `ling → inclusionai` entry).
`ModelsApp.lab_catalog` is assigned **after** `App::new`, so `tui::run`
re-runs `update_filtered_models` — groups built at construction have no labs.

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

## 3. Provider Modal (`p`)

Search-first provider scoping — the sidebar's replacement. Standard popup
chrome: `Clear` background, Cyan border, `centered_rect_fixed(46, …)`, title
`" Provider ({count}) "`, bottom title `" Enter: browse | Esc: cancel "`.

- **Search is scope-local by design** (user decision): it matches provider
  id/name only, never the models a provider carries — "who sells Claude" is
  answered by the grouped list's Enter drill, not the modal.
- Layout: 1-line query row (`/ {query}_`, SLOW_BLINK cursor) + row list. Row 0
  is always `All models ({total})` (Green); provider rows are
  `{category initial} {id} ({model count})` (initial in `cat.color()`).
  **Typing is immediate** — every printable char (digits included: `302ai`)
  goes to the query, so the modal intercepts all keys
  (`handle_provider_picker_keys`); `q` never quits from inside it.
- Enter (or click — click-to-apply like the sort picker) applies the scope:
  provider rows → provider-scoped Flat view with the Provider card pinned;
  `All models` → back to All-grouped. `picker_rows()` is the single source of
  the row list for render, keys, and mouse alike.
- Renders a **fresh ListState per frame** (deterministic offset for
  `popup_row_at` — see style guide §12); popup inner rect cached in
  `picker_area` (a `Cell`).

**Provider category colors** (from `ProviderCategory`): Origin=White, Cloud=Cyan, Inference=Yellow, Gateway=Green, Tool=Magenta. These are tab-specific — do not assume fixed colors; use `cat.color()`.

**Filter keys** (global on the tab): `1`=reasoning, `2`=tools, `3`=open
weights, `4`=free, `5`=provider category (cycles), `6`=group by category.
`5`/`6` shape which providers the modal lists; there is no filter-toggle row
anymore (the sidebar that hosted it is gone).

---

## 4. Model List Columns

Column widths (left to right):

| Column | Width | Notes |
|--------|-------|-------|
| Caret | 2 | `"> "` focused / `"  "` unfocused |
| RTFO | 5 | 4 indicator chars + 1 space |
| Model | dynamic | remainder after kept columns, minimum 10 — **display name** (`Model.name`), header `"Model"`; id fallback for nameless models |
| Provider | 15 | 1-space gap + left-aligned `{:<14}` display name (truncated with ellipsis) |
| Input cost | 9 | 1-space gap + right-aligned `{:>8}` |
| Output cost | 9 | 1-space gap + right-aligned `{:>8}` |
| Context | 9 | 1-space gap + right-aligned `{:>8}` |

**Column drop policy** (`ModelListColumn` in `render.rs`): columns right of the
name shed greedily from the right of the keep-priority order — **Provider,
Input, Output, Context** — before the name column drops below an 18-char
minimum. Provider is kept first (dropped last) because in the "All" view it is
the differentiator between otherwise-identical duplicate rows. The
**actively-sorted column always survives** the drop (Cost sort keeps Input,
Context sort keeps Context) by replacing the last kept column. Header and rows
render only the kept columns, so nothing ever clips mid-value (the old fixed
layout rendered a 1M-context model as `"1"` at ≤100 total cols). Mirrors the
Benchmarks list `max_cols` policy. Verified by the `*_render_*`-named tests in
`mouse_tests`.

**Name-primary identity**: the identifying column shows the models.dev
**display name**, not the model id. Rationale: the id is the *acting* artifact
(config strings) and stays served by `c`/`C` copy and the detail panel; the
name is the *reading* one — shorter (mean 16.5 vs 20.6 chars), curated
upstream, and consistent with what search matches (search matches id OR name
OR provider, so an id-displaying list showed rows whose visible text didn't
contain the query). Within-provider name collisions are ~1% of rows (28
preview/GA and window-size pairs) — the detail panel disambiguates those.

**Provider column content**: models.dev **display name** (`Provider.name`, e.g.
`Amazon Bedrock`, `Alibaba (China)`, `302.AI`), not the id slug. Falls back to
the id when the provider isn't found. Style follows the row (`style`), never
dimmed.

**Duplicate-row dimming**: consecutive rows for the **same underlying model**
render the name cell in `Color::DarkGray` (first occurrence keeps the default
style)
so the Provider column carries the difference — sameness recedes, difference
advances. The **sameness key is the models.dev display name** (`Model.name`),
not the id: upstream, provider entries inherit `name` verbatim from the
canonical `models/` registry (`base_model` resolution in the models.dev build)
and override it exactly when a variant genuinely differs — `"Claude Opus 5
(EU)"`, `"(Fast)"`, `"(JP)"` stay bright while `us.anthropic.claude-opus-5` /
`claude-opus-5` / `anthropic/claude-opus-5` (all named `"Claude Opus 5"`) dim
as one run. Do **not** add `family` as a guard key — its per-provider
granularity is inconsistent (`qwen` vs `qwen3.7-plus`) and reintroduces misses.
The RTFO cluster dims too, but **only when the four capability flags are also
identical**: a bright RTFO on a dimmed-id row signals that this vendor's
deployment of the "same" model genuinely differs. Selection styling always
wins (a selected row is never dimmed). Adjacency-only: rows compare to their
immediate predecessor in display order, so dimming is a reading aid for runs,
never a global claim. Verified by `consecutive_duplicate_rows_dim_id_and_rtfo`
and `dimming_keys_on_display_name_not_id`.

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
2. **Capabilities** — 2-column `two_pair_line` layout: Reasoning/Tools, Source/Files, Temp/**Structured**. The four RTFO-mirrored fields keep the compact-column's semantic colors (Reasoning Cyan, Tools Yellow, Files Magenta, Source Green/Red); the **new** fields get distinct hues so no single color stacks up in the grid. `Structured` renders from `Model.structured_output` (`Option<bool>`, ~49% coverage) via a three-state `cap_val_opt` — Yes (**Blue**) / No (DarkGray) / `—` (DarkGray, unknown-when-absent — this is why it lives here and **not** in the compact RTFO row, which is binary-only and stays 4-char `RTFO`). When the model carries `reasoning_options`, its **reasoning controls** (the API knobs for *controlling* reasoning — distinct from the Reasoning capability flag and the Thinking price) are appended to the **same 2-column `two_pair_line` grid** as `Label: value` pairs, each with its own non-Cyan color (Budget **LightGreen**, Effort **LightMagenta**, Toggle **LightBlue**, unknown **Blue**). Built by the shared free fn `data::reasoning_controls(&[ReasoningOption]) -> Vec<(String, String)>`: `("Budget", "{min}–{max}")` (`budget_tokens`; ranges rounded via `format_tokens`, `≤max`/`≥min` when only one bound), `("Effort", "{Level, …}")` (title-cased, from the option's `values[]`, `null` "off" entries dropped), `("Toggle", "Yes")`. An unknown future `type` is capitalized with value `"Yes"` (permissive). ~474 models expose 2–3 controls, which flow across the grid like any other pair. Omitted entirely when there are no `reasoning_options`. The **CLI** `models show` prints the same pairs as aligned `Label: value` lines in its Capabilities block, and `--json` emits the raw `reasoning_options` array (via `Serialize` on `ReasoningOption`)
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

## 6b. Glossary Popup (`i`)

`i` toggles a scrollable glossary explaining the tab's fields (mirrors the Benchmarks glossary convention; `i` is free in this tab). State lives on `ModelsApp` (`show_glossary: bool`, `glossary_scroll: ScrollOffset`); `toggle_glossary`/`scroll_glossary_up`/`scroll_glossary_down` are the methods.

- **Content** (`build_glossary_lines(width)` in `render.rs`) is **static** — independent of the selected model. Sections mirror the detail panel, each a `section_header_line`: List column (RTFO), Capabilities, Reasoning controls, Pricing, Limits. Each entry is a term (Gray+BOLD) + a White description line + a blank line.
- **Render** (`draw_glossary`): `centered_rect(60, 70)`, `Clear` background, `ScrollablePanel` with Cyan border, title `" Models Glossary - i or Esc to close (Up/Down to scroll) "`.
- **Message routing**: reuses the shared `ToggleGlossary`/`ScrollGlossaryUp`/`ScrollGlossaryDown` variants — `App::update` dispatches to `models_app` vs `benchmarks_app` by `current_tab`. `event.rs` intercepts glossary keys (`handle_glossary_keys`: `i`/`Esc` close, arrows/`j`/`k` scroll, everything else swallowed so `q` doesn't quit) before the global handler when `models_app.show_glossary`.
- **Mouse**: `modal_popup_open` returns true for `models_app.show_glossary`; the wheel scrolls the glossary, clicks are swallowed (no selectable rows). Footer hint: ` i ` (Yellow) + `glossary`. Help: `i — Open field glossary`.

---

## 7. Focus States

Two focus positions; `h`/`l`/`Tab` toggle:

```
Focus::Models  ↔  Focus::Details
```

- Models border: Cyan when focused
- Details (`ScrollablePanel`): Cyan border when focused, scrollable

`reset_detail_scroll()` is called on every navigation, sort, filter, and search change.

---

## 8. Mouse

This tab is the **reference implementation** for TUI mouse support (`handle_models_mouse` + `mouse_tests` in `src/tui/models/`). See style guide §12 for the shared pattern.

- **Cached rects** (`ModelsApp`, written at render time): `model_list_area` (the list inner area — the column header is list item 0; shared by the grouped and flat lists), `provider_card_area`, `model_detail_area`. `provider_list_area` is reset to `None` every frame (the sidebar is gone; no stale hit-area may linger).
- **Click:** list row → focus Models + select (item 0 is the header → ignored, `idx - 1` maps to the row; grouped mode selects a group). The handler picks state/offset/row-count by `list_mode()` (`group_list_state` vs `model_list_state`, `mouse_row_count()`). Provider card or model detail → focus Details only.
- **Wheel (focus-then-scroll):** over the list → prev/next row (mode-dispatched); over the right panel → scroll the model detail.
- **Provider modal**: `modal_popup_open` includes `show_provider_picker`; wheel → `ProviderPickerNext/Prev`, click → row via `popup_row_at` + click-to-apply.
- Both lists render into their **real** `ListState`s so `offset()` is valid for click-to-select while scrolled (the `ListState` copy gotcha — see CLAUDE.md).
