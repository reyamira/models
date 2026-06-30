# Agents Tab

## Files
- `app.rs` — `AgentsApp` state, `AgentFocus` (List/Details), `AgentSortOrder`, `AgentCategory`, `AgentFilters`
- `render.rs` — `draw_agents_main()` (list + detail), `draw_picker_modal()` (source tracking popup), `draw_add_agent_modal()` (`A` — add a custom agent by name + `owner/repo`, writes `config.agents.custom`)

## Key Patterns
- `AgentsApp` is `Option<AgentsApp>` on `App` — constructed after agents file loads, not at startup
- Changelog search with `n`/`N` match navigation uses `search_matches: Vec<usize>` (line indices into rendered markdown)
- Detail scroll uses `detail_scroll: u16` — counts visual wrapped lines, not logical lines
- Source picker modal intercepts global keys (especially `q`) to prevent accidental quit
- Service health display: agents with status provider mappings show health icon + label in detail panel via `resolve_agent_service_health()`

## Filters & Sort
- Filter keys: `1`=installed only, `2`=CLI only, `3`=open source only
- `AgentSortOrder` variants: Updated (default), Name, Stars, Status — always descending
- Dynamic list width: `max_name_len + 18` (borders + highlight + dot + gap + type + padding)

## Gotchas
- `detail_scroll` is `u16` not `ScrollOffset` — this tab predates the `ScrollOffset` Cell newtype
- Search match indices are tracked during render and converted to visual offsets — positions shift when panel resizes
