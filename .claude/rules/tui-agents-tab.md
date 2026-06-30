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

---

## 7b. Add Agent Modal

Popup for adding a **custom** agent without hand-editing `config.toml` (opened
with `A` — capital, since lowercase `a` opens the tracker picker). Minimal
two-field form; writes a `CustomAgent` to `config.agents.custom`.

- **State** (`AgentsApp`): `show_add_form: bool`, `add_form: AddAgentForm`
  (`{ name, repo, field: AddAgentField (Name|Repo), error: Option<String> }`).
- **Size**: `centered_rect_fixed(min(54, screen_width - 4), min(11, screen_height - 4))`.
- **Border**: `Color::Cyan`. **Title**: `" Add Agent "`.
- **Bottom title**: `" Tab: next field | Enter: save | Esc: cancel "` (centered).
- **Fields**: `Name:` and `Repo:` (each `  {label:<7}` gutter). Active field
  label is `Cyan`+BOLD with a trailing `SLOW_BLINK` `_` cursor; inactive label
  is `Gray`. An empty inactive Repo shows a `DarkGray` `owner/name` placeholder.
- **Keys** (`handle_add_agent_keys`, intercepts all so `q`/`?` don't leak):
  `Tab`/`Up`/`Down` toggle field, `Backspace`, `Char(c)` types into the active
  field, `Enter` saves, `Esc` cancels.
- **Save** (`add_agent_save`): trims; requires a non-empty name and a valid
  `owner/name` slug (`is_valid_repo_slug`: exactly one `/`, non-empty halves,
  `[A-Za-z0-9._-]` only). Id is derived as `name.to_lowercase().replace(' ', "-")`
  — identical to how `AgentsApp::new` derives ids for config-loaded custom
  agents, so a restart re-resolves the same id. **Collision** with an existing
  entry id → inline error, form stays open. On success: push `CustomAgent`,
  `config.set_tracked(id, true)` (so it persists as tracked), `config.save()`
  (rolled back in-memory if the write fails), build a tracked `Loading`
  `AgentEntry`, re-sort entries by name, and queue the GitHub fetch via
  `App.pending_fetches` (same path the tracker's "newly tracked" uses). Minimal
  custom agents carry no `version_command`, so `detect_installed` short-circuits
  (no shell-out). Validation errors set `add_form.error` and return before any
  `config.save()` (filesystem-free).
- **Footer**: ` A ` (Yellow) + `add`. Help: `A — Add a new agent (name + repo)`.

---

## 7c. Update Action (in-app self-update)

Runs an agent's **verified** self-update command as a background subprocess —
no TUI suspension, mirrors the GitHub-fetch async pattern.

- **Registry field**: `Agent.update_command: Vec<String>` (`data/agents.json`,
  `#[serde(default, skip_serializing_if = "Vec::is_empty")]`) — an argv vector,
  **no shell**. Only populated with commands verified from each tool's official
  docs (the 9 CLI agents; IDEs/extensions have none). `AgentEntry::update_command()`
  returns `Option<&[String]>` (None = no update action). Custom agents
  (`CustomAgent::to_agent`) get no update command.
- **Keys**: `u` = update the selected agent; `U` = update **all** agents with
  `update_available()` && a verified updater; `x` = cancel the selected agent's
  in-flight update. `u`/`U` open a **confirm modal** first (update mutates the
  user's system; refresh only reads).
- **`u` gates on installed**: `request_update_selected` refuses (status-bar
  message) when the selected agent has no detected install (`installed.version`
  is `None`) — nothing to update. `U` is already gated via `update_available()`.
- **Confirm modal** (`draw_update_confirm_modal`): Cyan border,
  `centered_rect_fixed(66, …)`, title ` Update Agent(s) `. Lists each target's
  `name` + `$ {argv joined}` in Yellow. Bottom hint is target-count dependent:
  single → ` Enter: background | i: interactive | Esc: cancel `; multi →
  ` Enter: run | Esc: cancel `. `Enter`→`ConfirmUpdate` (background), `i`→
  `ConfirmUpdateInteractive` (suspend-and-run, single only), `Esc`/`q`→
  `CancelUpdate` (`handle_update_confirm_keys`, intercepts all keys). `request_update_selected`
  errors (status bar) when the agent has no updater or is already running;
  `request_update_all` errors when none qualify.
- **Command resolution** (`AgentEntry::resolved_update_command`, install-aware):
  (1) **self-updater** (argv[0] is the agent's own binary, `["claude","update"]`)
  + a detected path → argv[0] replaced with that absolute path so the exact
  detected copy is updated, not whatever PATH resolves first. (2) **JS
  package-manager swap**: an `npm install -g <pkg>` updater whose binary was
  actually installed by a *different* JS PM (bun/pnpm/yarn — inferred from the
  detected path via `infer_install_method`/`InstallMethod`) is rewritten to that
  manager keeping the same package spec (`bun add -g <pkg>`), so it updates the
  copy you run. (3) **Homebrew install** of a package-manager-updated tool →
  `brew upgrade <formula>`, where `<formula>` is parsed from the resolved Cellar
  path (`canonicalize` the bin symlink → `…/Cellar/<formula>/…`). npm installs,
  uv, and unknown/unresolvable paths keep the registry command. The confirm modal
  shows the detected method per target as `(via <method>)` (`UpdateTarget.method`).
- **Execution** (`spawn_agent_update` in `tui/mod.rs`): `tokio::process::Command`
  (needs tokio features `process` + `io-util`), `stdin` null, stdout+stderr
  piped and streamed **line-by-line** over an `mpsc<UpdateEvent>` channel
  (`Output`/`Finished`/`Redetected`). The child is put in its **own process
  group** (`process_group(0)`, Unix) so a tool that opens `/dev/tty` for a prompt
  (sudo) is a background-group reader → SIGTTIN-stopped (caught by the timeout)
  rather than stealing the TUI's keystrokes / corrupting the screen. Bounded by a
  **5-minute timeout** (`tokio::time::timeout` → `start_kill`) since there's no
  TTY for prompts.
  All output is flushed (reader handles awaited) before `Finished`. On success,
  `detect_installed` re-runs via `spawn_blocking` → `Redetected` updates
  `AgentEntry.installed` so the dot flips without a restart. The confirmed
  `(id, argv)` pairs flow `App.pending_updates` → drained in the loop (mirrors
  `pending_fetches`); the agent is looked up at drain time for the re-detect.
- **State** (`AgentsApp`): `show_update_confirm`, `update_targets: Vec<UpdateTarget>`,
  `update_states: HashMap<id, AgentUpdateState>` (`Running`/`Succeeded`/`Failed`),
  `update_logs: HashMap<id, Vec<String>>` (capped at `UPDATE_LOG_CAP` = 200,
  oldest dropped). `confirm_update` marks each target `Running`, resets its log,
  and returns the spawn list. `push_update_output`/`finish_update`/`apply_redetected`
  apply the channel events.
- **Rendering**: list status dot shows `◐` **Magenta** while `Running` (distinct
  from the Yellow GitHub-fetch spinner). Detail panel gains an `Update:` section
  (state line + trailing ≤12 output lines, DarkGray) whenever the selected agent
  has update state/logs. Status bar: `{name} updated` / `{name} update failed —
  see detail panel`.
- **Failure/no-TTY path**: npm-prefix updaters (gemini-cli, qwen-code) can fail
  without a writable global prefix; the captured stderr + non-zero exit surface
  in the detail log and the `Failed` state. On `Failed`, the detail Update
  section adds a `Run manually: $ <cmd>` line (the bare registry command) so the
  user can run it in their own shell — where they have full interactivity for any
  prompt. openclaw's updater restarts its daemon (heaviest side effects) —
  included, shown verbatim in the confirm modal.
- **Interactive (suspend-and-run) path** — the `u` confirm modal offers `i`
  (single-agent only; `U`/update-all stays background). `i` →
  `Message::ConfirmUpdateInteractive` → `AgentsApp::confirm_update_interactive`
  (marks the agent `Running`, returns `(id, argv)`) → `App.pending_interactive_update`
  → drained in `run_app` by `run_interactive_update`: it **suspends the TUI**
  (`disable_raw_mode` + `LeaveAlternateScreen` + `DisableMouseCapture`), runs the
  updater with **inherited stdio** (`std::process::Command::status()` — fully
  interactive: prompts, sudo, menus), waits for a keypress, then **unconditionally
  restores** (`enable_raw_mode` + `EnterAlternateScreen` + `EnableMouseCapture` +
  `terminal.clear()`) and re-detects the version. Runs synchronously on the
  terminal-owning thread; restore uses no `?` so a child error can't wedge the
  screen (the panic hook + `run()` end-cleanup are extra nets). Output goes to the
  real terminal (not captured), so the detail log shows just the summary line.
  `run_interactive_update` manipulates the real terminal → not unit-tested; the
  decision branch (`confirm_update_interactive` single vs multi) is.
- **Cancel a running update** (`x` → `Message::RequestCancelUpdate`): the main
  loop holds a per-agent `oneshot::Sender<()>` (`cancel_signals`, created when the
  update spawns, removed on `Finished`). `x` on a `Running` agent fires it →
  `spawn_agent_update`'s `tokio::select!` (child exit / 5-min timeout / cancel)
  takes the cancel branch → `start_kill` + reap → `Finished(false, "✗ update
  cancelled")`. This frees the user to immediately re-run with `i` (interactive),
  which is the recovery path when a background update turns out to need input.
- **Footer**: ` u ` update, ` U ` update all, ` x ` cancel. Help: `u`/`U`/`x`.

---

## 8. Mouse

`handle_agents_mouse` (in `agents/app.rs`); see style guide §12 for the shared pattern.

- **Cached rects** (`AgentsApp`): `agent_list_area` (the bare list region **below** the `Length(1)` filter-toggle row, so `top_skip = 0`), `detail_area` (the scrollable detail panel).
- The agent list's header (`"St Agent … Type"`) is rendered as **list item 0** (the `agent_list_state` is offset by +1, same as Models). The handler passes `item_count = filtered_entries.len() + 1` to `row_at` and `checked_sub(1)`s the result to map item → agent.
- **Click:** agent row → focus List + `select_agent_at_index`; detail → focus Details only.
- **Wheel (focus-then-scroll):** over the list → prev/next agent; over the detail → adjust `detail_scroll` (a `u16`, clamped at render).
- The list renders into the **real** `agent_list_state` so `offset()` is valid while scrolled (fixed the `ListState` copy gotcha — see CLAUDE.md).
- **Modal mouse:** `modal_popup_open` returns true for `show_picker`, `show_add_form`, **and** `show_update_confirm` so clicks/wheel can't leak to the panels behind. The Add Agent form and the update-confirm modal have no selectable rows — clicks and wheel over them are swallowed (`handle_modal_popup_click`/`handle_modal_popup_mouse` return `None` when `show_add_form || show_update_confirm`).
