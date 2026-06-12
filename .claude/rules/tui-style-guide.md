---
description: TUI design style guide — layout, colors, typography, interactions, and component patterns for the ratatui terminal UI
globs:
  - src/tui/**
---

# TUI Design Style Guide

This guide defines the visual language and interaction patterns for the Models TUI. All new tabs and components must follow these conventions to maintain visual consistency. When in doubt, reference the Models and Agents tabs as the canonical implementations.

---

## 1. Global Frame

Every tab shares a 3-row vertical layout:

```
Constraint::Length(1)   -- Header (tab bar)
Constraint::Min(0)      -- Main content (tab-specific)
Constraint::Length(1)    -- Footer (keybindings / search bar)
```

The header and footer are rendered by `draw_header()` and `draw_footer()` — tabs only control the main content area.

## 2. Color System

### 2.1 Semantic Colors (core palette)

| Role | Color | Usage |
|------|-------|-------|
| **Focus / Active** | `Color::Cyan` | Focused borders, active tab text, section headers, URLs, interactive highlights |
| **Inactive / Secondary** | `Color::DarkGray` | Unfocused borders, secondary text, inactive filters, missing data placeholders |
| **Labels / Secondary text** | `Color::Gray` | Detail panel label text (e.g., "Provider:", "Installed:", "Latest release:") |
| **Selected / Hint** | `Color::Yellow` | Selected item text, keybinding hints in footer/help, sort indicators, warnings, loading text |
| **Positive / Open** | `Color::Green` | Operational status, open weights, "All" items, active filter keys, "up to date", free pricing |
| **Negative / Closed** | `Color::Red` | Errors, outages, closed weights, deprecated status, fetch failures |
| **Primary Text** | `Color::White` | Model/provider names in detail headers, primary data values |
| **Accent** | `Color::Magenta` | Markdown headers and bullet markers, Files/attach capability indicator |
| **Info** | `Color::Blue` | Maintenance (provider-level), US region, update-available dot |

### 2.2 Capability Indicators (RTFO)

| Capability | Active | Active Color | Inactive | Inactive Color |
|------------|--------|--------------|----------|----------------|
| Reasoning | `R` | `Color::Cyan` | `·` | `Color::DarkGray` |
| Tools | `T` | `Color::Yellow` | `·` | `Color::DarkGray` |
| Files | `F` | `Color::Magenta` | `·` | `Color::DarkGray` |
| Open/Closed | `O` / `C` | `Color::Green` / `Color::Red` | — | — |

### 2.3 Status Health Colors

| State | Icon | Color |
|-------|------|-------|
| Operational | `●` | `Color::Green` |
| Degraded | `◐` | `Color::Yellow` |
| Outage | `✗` | `Color::Red` |
| Maintenance | `◆` | `Color::Blue` |
| Unknown | `?` | `Color::DarkGray` |

### 2.4 Markdown Rendering (detail panels)

| Element | Foreground | Background | Modifier |
|---------|-----------|------------|----------|
| Header (`##`/`###`) | `Color::Magenta` | — | `BOLD` |
| Bullet marker | `Color::Magenta` | — | — |
| Bold (`**text**`) | inherited | — | `BOLD` |
| Inline code | `Color::Yellow` | `Color::Rgb(50, 40, 25)` | — |
| URL | `Color::Cyan` | — | `UNDERLINED` |
| Search highlight | `Color::Black` | `Color::Yellow` | `BOLD` |

### 2.5 Compare Palette (multi-select charts)

```rust
[Red, Green, Blue, Yellow, Magenta, Cyan, LightRed, LightGreen]
```

8 colors, indexed modulo length. Used for selection markers, H2H columns, scatter points, radar polygons, and legend entries.

## 3. Typography

### 3.1 Modifier Usage

| Modifier | Semantic Meaning | Examples |
|----------|-----------------|----------|
| `BOLD` | Emphasis, selected items, section headers, active elements | Active tab, selected row, detail header name, help section headers |
| `UNDERLINED` | Clickable/navigable elements, table header rows | URLs in markdown, list column headers |
| `SLOW_BLINK` | Cursor / active input position | Search bar cursor only |
| `REVERSED` | Strong selection highlight (used sparingly) | — (reserved, not currently used) |

**Not used:** `DIM` (DarkGray color serves the dim role), `ITALIC`

### 3.2 Label/Value Pattern

Detail panels consistently use:
- **Labels:** `Style::default().fg(Color::Gray)` (e.g., "Provider:", "Installed:", "Latest release:")
- **Values:** `Style::default().fg(Color::White)` (or `Color::DarkGray` if deprecated/unavailable)
- **Missing values:** em-dash `\u{2014}` in `Color::DarkGray`

## 4. Borders & Chrome

### 4.1 Panel Borders

All panels use full borders with focus-aware coloring:

```rust
Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(if focused { Color::Cyan } else { Color::DarkGray }))
    .title(title)
```

### 4.2 Titles

- Position: top-left (ratatui default) — never override unless using `.title_bottom()`
- Format: space-padded, e.g., `" Providers (42) "`
- Dynamic content in titles: count, search query `[/{query}]`, sort indicator `↓date`, filter list `[reasoning, open]`
- Bottom titles: used for popup action hints, e.g., `" Space: toggle | Enter: save | Esc: cancel "`

### 4.3 Section Headers (inside detail panels)

```
── Title ──────────────────
```

- Style: `Color::DarkGray` + `Modifier::BOLD`  (for Models tab dash-padded headers)
- Or: `Color::Cyan` + `Modifier::BOLD` plain text (for Agents/Status section labels)
- Separator lines: `\u{2500}` repeated, in `Color::DarkGray`

## 5. Layout Patterns

### 5.1 Multi-Panel Layouts

| Pattern | When to Use | Example |
|---------|------------|---------|
| Percentage 3-column | Browse/overview with sidebar + list + detail | Models: `20% \| 45% \| 35%` |
| Fixed + Fill 2-column | List/detail with predictable list width | Status: `Length(32) \| Min(0)` |
| Dynamic + Fill 2-column | List width adapts to content | Agents: `Length(computed) \| Min(0)` |
| Percentage + Min 2-column | Browse/compare with minimum left width | Benchmarks compare: `Length(max(30%, 35)) \| Min(0)` |

### 5.2 Panel Internal Splits

Common pattern: filter row + scrollable list:
```rust
Constraint::Length(1)  -- Filter toggles / sub-tab bar
Constraint::Min(0)     -- Scrollable content
```

### 5.3 Detail Panel Sections

Stack sections vertically using `Constraint::Length(n)` for fixed-height content and `Constraint::Min(0)` for the final flexible section. Use `Constraint::Length(1)` gaps between sections.

## 6. Interactive Patterns

### 6.1 Selection & Focus

- **Selected item:** `Color::Yellow` + `Modifier::BOLD`
- **Caret indicator:** `"> "` when panel is focused, `"  "` (2 spaces) when unfocused
- **Focus border:** Cyan (focused) / DarkGray (unfocused)
- All three indicators work together to reinforce focus state

### 6.2 Navigation Keybindings

Standard keybindings that every scrollable panel should support:

| Key | Action | Context |
|-----|--------|---------|
| `j` / `Down` | Next item or scroll down | List or detail focus |
| `k` / `Up` | Previous item or scroll up | List or detail focus |
| `g` | Jump to first | List focus |
| `G` | Jump to last | List focus |
| `Ctrl+d` / `PageDown` | Page down | Both list and detail focus |
| `Ctrl+u` / `PageUp` | Page up | Both list and detail focus |
| `h` / `l` / `Tab` | Switch panel focus | All panels |

### 6.3 Scrollbars

Use `ScrollablePanel` for all scrollable content. It handles block, paragraph, scroll clamping, writeback, and scrollbar rendering centrally:

```rust
// Title accepts impl Into<Line<'a>> — styled titles with colored spans are supported
ScrollablePanel::new("Title", lines, &scroll_offset, focused)
    .render(f, area);

// For pre-formatted content that shouldn't wrap:
ScrollablePanel::new("Title", lines, &scroll_offset, focused)
    .with_wrap(false)
    .render(f, area);

// For SoftCard content:
ScrollablePanel::with_cards("Title", cards, &scroll_offset, focused)
    .render(f, area);
```

- Scroll fields use `ScrollOffset` (a `Cell<u16>` newtype) for interior-mutable writeback during render
- `ScrollablePanel` always draws its own `Block::borders(Borders::ALL)` — pass the outer area, not an inner rect
- Scrollbar renders inside the border with `Margin { vertical: 1, horizontal: 0 }`
- Use `.with_wrap(false)` for tabular/pre-formatted content (e.g., H2H table, help popup)
- Use default scrollbar symbols (no custom begin/end symbols)

### 6.4 Search

- Entry: `/` enters search mode
- Display: footer transforms to search bar; query shown in panel title as `[/{query}]`
- Search bar format: `" Search: "` (Cyan) + query + blinking `_` + `" Enter/Esc "` (Yellow) + `"confirm"`
- Exit: `Enter` or `Esc` exits search mode; `Esc` in normal mode clears query
- Match navigation (if applicable): `n` / `N` for next/prev match

**Tab-specific search scopes:**
- **Models**: filters model list by name/provider
- **Agents**: searches changelog content with match highlighting and `n`/`N` navigation
- **Benchmarks**: filters benchmark list by name/creator
- **Status**: filters provider list by name

### 6.5 Sort

- Indicator in panel title: `↓label` (descending) or `↑label` (ascending)
- Sort arrows: `\u{2193}` (down), `\u{2191}` (up)
- Sort picker popup: 30-char fixed width, Cyan border, `" Sort By "` title

## 7. Footer

### 7.1 Layout

```rust
[Constraint::Min(0), Constraint::Length(10)]  // left: hints, right: "? help"
```

### 7.2 Normal Mode

- Left: keybinding hints as `" key "` in Yellow + `"description  "` in default
- Right: `" ? "` in Yellow + `"help "` right-aligned
- Double-space `"  "` between hint groups
- Always include: `q quit`, `/ search`, `? help`
- Tab-specific keys after the common ones

### 7.3 Status Message Override

When `app.status_message` is `Some(...)`, the entire footer shows the message in `Color::Green`. Auto-clears after timeout.

### 7.4 Search Mode

Replaces entire footer with the search bar (see 6.3).

## 8. Help Popup

- Size: `centered_rect(50, 70)` — 50% width, 70% height
- Border: `Color::Cyan`, title: `"{TabName} Help - ? or Esc to close (j/k to scroll)"` where TabName is "Models", "Agents", "Benchmarks", or "Status"
- Background: `Clear` widget rendered first
- Content order: Navigation → Panels → Search → [Tab-specific sections] → Tabs → Other
- Key format: 16-char padded key name in Yellow + description in default
- Scrollable via `ScrollablePanel` widget with `.with_wrap(false)` (see Section 6.3)

## 9. Popups & Overlays

All popups follow these rules:
- Render `Clear` widget to erase background content
- Use `Color::Cyan` borders
- Centering: `centered_rect(pct_x, pct_y)` for responsive sizing, `centered_rect_fixed(w, h)` for fixed sizing
- Must intercept global keys (especially `q`) to prevent accidental quit — intercept before global key handling in `handle_normal_mode`
- Dismiss via `Esc` (cancel) or `Enter` (confirm) as appropriate

## 10. Data Formatting

| Data Type | Format | Missing Value |
|-----------|--------|---------------|
| Dates | `%Y-%m-%d` with `(relative)` suffix | em-dash `\u{2014}` |
| Token counts | `128k`, `1M`, `1.5M` | em-dash |
| Prices (list) | `$100`, `$1.5`, `$0.05`, `$0.001` (tiered by magnitude) | em-dash |
| Prices (detail) | `Free` (Green), `$15/M`, `$2.50/M` | em-dash |
| Star counts | `12.3k`, `1.2m` | — |
| Version strings | `v` prefix: `v1.2.3` | — |
| Percentages | `{:.1}%` (1 decimal) | em-dash |
| Index scores | `{:.1}` (1 decimal) | em-dash |
| Truncation | Append `"..."` when exceeding max width | — |

**Important:** Use `line.width()` (unicode-aware) not `.len()` (byte count) for width calculations. Truncation must respect UTF-8 char boundaries.

## 11. Loading & Error States

### 11.1 Loading

- Show loading indicator when async data hasn't arrived yet
- List panel: loading text in title (e.g., `"refreshing..."`) or icon in list (`◐` Yellow)
- Detail panel: `"Loading..."` message in Yellow

### 11.2 Errors

- Fetch failures: `"✗ {error}"` in `Color::Red`
- User should always see feedback — never silently swallow errors
- Fallback/empty states: descriptive message in `Color::DarkGray` (e.g., `"No model selected"`, `"No matching results"`)

## 12. Mouse Interaction

Mouse capture is on for the whole TUI. All tabs support the same three interactions; keep the behavior consistent when adding panels.

### 12.1 Interaction model

| Input | Effect |
|-------|--------|
| **Left-click a list row** | Focus that panel **and** select the row under the cursor |
| **Left-click a panel** (detail / scrollable / chart) | Focus that panel only — no row selection |
| **Left-click header tab label** | Switch to that tab |
| **Left-click clickable chrome** | Activate it (e.g. Benchmarks source-bar `[name]` → switch source; `[H2H] [Scatter] [Radar]` subtab → switch view) |
| **Scroll wheel** | **Focus-then-scroll**: the panel under the cursor gains focus, then scrolls/navigates with the same action the arrow keys drive |
| **Mouse move / drag** | Ignored (not consumed) |

- **Single-click only.** No double-click or click-to-activate in v1 — keyboard retains activation (`Enter`/`o`/`Space`).
- **Popups take precedence.** While a popup is open, the **wheel scrolls/navigates the popup** (help, benchmarks sort picker / glossary / column picker, and the agents/status tracker modals) by emitting that popup's existing scroll/nav `Message`; clicks are swallowed so they can't leak to the panels behind, and the help popup also closes on click. Routed in `handle_modal_popup_mouse` (`event.rs`). In-popup row-clicking (selecting a specific popup row by click) is out of scope for v1.
- **No new `Message` variants.** Mouse handlers receive `&mut App` and apply focus/selection/scroll directly via existing sub-app methods, returning `None`. This keeps the shared `Message` enum collision-free.

### 12.2 Implementation pattern (geometry cache)

ratatui recomputes and discards layout `Rect`s every frame, so hit-testing a click requires retained geometry:

1. Each sub-app stores `Option<Rect>` fields (`ratatui::layout::Rect`) for its focusable panels / clickable lists, initialized `None`.
2. The tab's `render.rs` writes the **exact rect each widget rendered into** at render time (the loop draws before it handles events, so the cache and any `ListState::offset()` reflect the clicked frame).
3. The tab's `handle_<tab>_mouse(app, ev)` hit-tests with the pure helpers in `src/tui/mouse.rs`:
   - `hit(area: Option<Rect>, &MouseEvent) -> bool` — `Rect::contains` wrapper.
   - `row_at(area, offset, top_skip, item_count, click_row) -> Option<usize>` — maps a click row to a list index. `top_skip` = non-item rows at the top of the stored rect (0 for a bare item region, 1 if it still includes a top border). An in-list header rendered as item 0 is a real item — pass `item_count = visible + 1` and subtract 1 from the result (see the Models model list).

**`ListState::offset()` is only valid after the widget renders into that same state object** — never render into a copy of the list state, or `offset()` goes stale and click-to-select breaks on a scrolled list.

### 12.3 Testing

Mouse logic is verified without a real terminal: render the tab into a `ratatui::backend::TestBackend` `Terminal` (which stores the panel rects and clamps `ListState` offsets exactly as the live loop does), then synthesize `MouseEvent`s and assert the resulting focus/selection. Every tab with a scrollable list includes a **non-zero-scroll-offset** click test (the case that catches the `offset()` bug). The reference is `mouse_tests` in `src/tui/models/render.rs`.

