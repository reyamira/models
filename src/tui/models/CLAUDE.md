# Models Tab

## Files
- `app.rs` — `ModelsApp` state, `Focus` (Models/Details), `ListMode` (Grouped/Offerings/Flat), `ModelGroup`, `SortOrder`, `Filters`, `ModelEntry`, provider-picker modal state, `detail_scroll: ScrollOffset`
- `render.rs` — `draw_main()` renders the 2-column layout (model list 60% | right panel 40%): `draw_grouped_list` (default), the flat/offerings renderer, the provider picker modal, group detail Providers section

## Key Patterns
- `ModelsApp::update_filtered_models(&mut self, providers)` takes `&[(String, Provider)]` param — providers live on `App`, not `ModelsApp`
- Column headers are **sticky** (a fixed line above the list, not item 0) — selection indices map 1:1 (`select(Some(idx))`, no +1)
- `ProviderListItem::CategoryHeader` items are non-selectable — `find_selectable_index()` skips them
- Sort/filter methods (`cycle_sort`, `toggle_reasoning`, etc.) live on `ModelsApp` and call `update_filtered_models` internally
- Detail panel uses `ScrollablePanel` widget with `detail_scroll: ScrollOffset` for scrollable, focus-aware rendering
- Focus toggles between Models ↔ Details (the provider sidebar is retired; `p` opens the provider modal)
- `reset_detail_scroll()` called on every model selection change (navigation, sort, filter, search)
- Provider modal rows display a category initial prefix (O/C/I/G/T for Origin/Cloud/Inference/Gateway/Tool); `provider_list_items` powers the modal, not a sidebar
- Grouped view state machine: `list_mode()` derives Grouped/Offerings/Flat from scope + `drill_key` + `flat_view`; `drill_name` is only the breadcrumb. Nav methods dispatch via `nav_len`/`nav_selected`/`nav_select`; lab/canonical resolution uses `crate::labs` from the `catalog.json` snapshot (assigned after `App::new` — `tui::run` re-runs `update_filtered_models`)

## Provider Detail Card
- Offerings/Flat modes only (grouped mode gives the detail panel the full height) — border always DarkGray (not focusable)
- Height dynamically computed from wrapped content lines + 2 borders
- Shows: provider name (Cyan+BOLD), category, docs URL, API URL, env var

## Gotchas
- Provider card border is intentionally always DarkGray — it's not in the focus cycle
- Headers are sticky widgets, not list items — never reintroduce the `idx + 1` header offset (it scrolls the header away permanently after `G`-then-`g`)
