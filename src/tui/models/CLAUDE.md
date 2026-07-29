# Models Tab

## Files
- `app.rs` — `ModelsApp` state, `Focus` (Providers/Models/Details), `SortOrder`, `Filters`, `ProviderListItem`, `ModelEntry`, `detail_scroll: ScrollOffset`
- `render.rs` — `draw_main()` renders the 3-column layout (providers | model list | detail panel)

## Key Patterns
- `ModelsApp::update_filtered_models(&mut self, providers)` takes `&[(String, Provider)]` param — providers live on `App`, not `ModelsApp`
- Column headers are **sticky** (a fixed line above the list, not item 0) — selection indices map 1:1 (`select(Some(idx))`, no +1)
- `ProviderListItem::CategoryHeader` items are non-selectable — `find_selectable_index()` skips them
- Sort/filter methods (`cycle_sort`, `toggle_reasoning`, etc.) live on `ModelsApp` and call `update_filtered_models` internally
- Detail panel uses `ScrollablePanel` widget with `detail_scroll: ScrollOffset` for scrollable, focus-aware rendering
- Focus navigation uses directional `focus_left()`/`focus_right()` cycling through Providers → Models → Details
- `reset_detail_scroll()` called on every model selection change (navigation, sort, filter, search)
- Provider list items display a category initial prefix (O/C/I/G/T for Origin/Cloud/Inference/Gateway/Tool) at the start of each item instead of an abbreviated label at the end

## Provider Detail Card
- Rendered above model detail in the right panel — border always DarkGray (not focusable)
- Height dynamically computed from wrapped content lines + 2 borders
- Shows: provider name (Cyan+BOLD), category, docs URL, API URL, env var

## Gotchas
- Provider card border is intentionally always DarkGray — it's not in the focus cycle
- Headers are sticky widgets, not list items — never reintroduce the `idx + 1` header offset (it scrolls the header away permanently after `G`-then-`g`)
