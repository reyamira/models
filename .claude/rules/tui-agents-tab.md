---
description: Agents tab design conventions — dynamic list width, status indicators, changelog search, picker modal
globs:
  - src/tui/agents/**
---

# Agents Tab Design Conventions

Tab-specific patterns only. For shared colors, borders, focus, search, footer, and scrollbars see `tui-style-guide.md`.

---

## 1. Layout

```
Length(list_width)  -- Agent list (dynamic, computed from content)
Min(0)              -- Agent detail (ScrollablePanel)
```

**Dynamic list width** formula:

```rust
list_width = max_name_len + 18
// 18 = 2 borders + 2 highlight + 2 (dot+space) + 2 gap + 6 type + 4 padding
```

`max_name_len` is the maximum agent name length across all filtered entries, minimum 5 (for the `"Agent"` header).

**List panel internal split** (outer block drawn manually, inner area split):

```rust
Constraint::Length(1)  -- Filter toggles row
Constraint::Min(0)     -- Agent list (stateful, offset +1 for header row)
```

---

## 2. Agent List Header and Rows

**Header row** (index 0, `model_list_state` offset by +1):
- Style: `Color::DarkGray` + `Modifier::UNDERLINED`
- Format: `"  {:<2} {:<name_width$}  {:>6}"` → `"  St Agent                 Type"`

**Agent rows**:

```
{caret}{status_dot} {name:<max_name_len}  {type:>6}
```

- Caret: `"> "` (focused+selected), `"  "` (unfocused or unselected)
- Status dot: see section 3
- Name: truncated to `max_name_len`, left-aligned with 2-space gap before type
- Type: `"CLI"` / `"IDE"` / `EM_DASH`, right-aligned in 6 chars

---

## 3. Status Indicators

Status dot reflects the agent's fetch state. Only shown for **installed** agents (have `version`). Uninstalled agents show `EM_DASH` in `Color::DarkGray`.

| State | Icon | Color | Condition |
|-------|------|-------|-----------|
| NotStarted | `○` (U+25CB) | `Color::DarkGray` | Fetch not yet started |
| Loading | `◐` (U+25D0) | `Color::Yellow` | GitHub fetch in progress |
| Loaded, up to date | `●` (U+25CF) | `Color::Green` | Loaded, no update |
| Loaded, update available | `●` (U+25CF) | `Color::Blue` | `update_available()` true |
| Failed | `✗` (U+2717) | `Color::Red` | Fetch error |
| Not installed | `—` (`EM_DASH`) | `Color::DarkGray` | No installed version |

The dot style is set **independently** of the row selection style — selected row text uses Yellow+BOLD but the dot retains its status color.

---

## 4. Filter Keys and Sort

**Filter row** (rendered as `filter_toggle_spans` helper):

```
[1] Inst  [2] CLI  [3] OSS
```

Key `[n]` is `Color::Green` when active, `Color::DarkGray` when inactive. Label follows directly after the key with a space.

**Filter keys**: `1`=installed only, `2`=CLI only, `3`=open source only

**Sort labels** (shown in list title as `↓{label}`):
- `"updated"`, `"name"`, `"stars"`, `"status"`

Default sort on launch: `"updated"` (descending).

Sort direction is always descending (`\u{2193}`) in the title — `AgentSortOrder` does not expose direction toggle.

---

## 4b. Refresh

`R` (capital — `r` is Open repo) re-triggers the conditional GitHub fetch for
all **tracked** agents via the same spawn helper as startup: entries flip to
Loading (`◐`), `pending_github_fetches`/`loading_github` are set, and results
flow through the existing `FetchResult` channel. ETag conditional requests
make unchanged repos cheap (NotModified → disk-cache hit). Status message:
`Refreshing agents…`. Footer hint: ` R ` (Yellow) + `refresh`.

## 5. Agent Detail Panel

Content rendered top-to-bottom in this order:

1. **Header line**: `{name}` (White + BOLD) + `"  "` + `"v{version}"` (Cyan)
2. **Repo + Stars**: `{repo}` (Gray) + `"  "` + `"★ {stars_str}"` (Yellow)
3. *(blank line)*
4. **Installed status**: label `"Installed: "` (Gray) + version string + status tag:
   - Up to date: `" (up to date)"` (Green)
   - Update available: `" (update available)"` (Yellow)
   - Not installed: `"Not installed"`, no tag
5. **Latest release**: `"Latest release: "` (Gray) + `"%Y-%m-%d"` date + `" ({relative})"` (Gray)
6. **Release cadence**: `"Release cadence: "` (Gray) + frequency string
7. **Service health** *(conditional — only when agent has a status provider mapping)*:
   - `"Service: "` (Gray) + `"{icon} {health.label()}"` (health style) + `"  ({provider} — {component})"` (Gray)
   - While loading: `"? Loading..."` (DarkGray)
8. **Fetch status** *(only non-Loaded states)*:
   - Loading: `"Loading GitHub data..."` (Yellow)
   - Failed: `"✗ "` (Red) + `"Failed to fetch: {error}"` (Red)
   - NotStarted (tracked): `"Waiting to fetch GitHub data..."` (DarkGray)
9. *(blank line)*
10. **Release History header**: `"Release History:"` (BOLD)
11. **Separator**: `"───────────────────────────────────"` (Gray)
12. **Per-release entries** (for each release):
    - Version line: `"v{version}"` (Cyan + BOLD) + `"  {date}"` (Gray) + optional marker
      - `"  ← INSTALLED"` (Green) for installed version
      - `"  ← NEW"` (Yellow) for new releases
    - Changelog lines (markdown-rendered)
    - Blank line between releases
13. *(blank line)*
14. **Keybinding hints** (inline spans at bottom):
    - `" o "` (Yellow) + `"open docs  "` + `" r "` (Yellow) + `"open repo  "` + `" c "` (Yellow) + `"copy name"`
    - When search active: `"  "` + `" n/N "` (Yellow) + `"next/prev match"`

---

## 6. Changelog Search

- Search applies across all releases' changelogs within the detail panel
- Title format when matches found: `"Details [/{query} {current+1}/{total}]"`
- Title format when query active but no matches: `"Details [/{query}]"`
- Matches highlighted via `changelog_to_lines_highlighted()` — `Color::Black` on `Color::Yellow` background + BOLD (standard search highlight)
- `n`/`N` navigation updates `current_match` and scrolls to center the match on screen using visual (wrapped-line) offsets
- Match line indices are tracked during render, converted to visual offsets via `ScrollablePanelState.visual_offsets`

---

## 7. Source Picker Modal

Popup for tracking/untracking agents (opened with `p`):

- **Size**: `centered_rect_fixed(min(60, screen_width - 4), min(agent_count + 4, screen_height - 4))`
- **Border**: `Color::Cyan`
- **Title**: `" Add/Remove Tracked Agents "`
- **Bottom title**: `" Space: toggle | Enter: save | Esc: cancel "` (centered)

**Item format** (per agent):

```
{checkbox} {name:<20}  {category:<10}  {installed_status}
```

| Part | Width | Style |
|------|-------|-------|
| `[x]` / `[ ]` | 4 | default |
| name | 20, left-aligned, truncated | BOLD |
| category | 10, left-aligned, truncated | DarkGray |
| `"installed"` or `""` | dynamic | Green |

Selected row: Yellow + BOLD (applied via `ListItem::style`, overrides per-span styles).

**Key interception**: modal must intercept `q` before the global handler to prevent accidental quit.
