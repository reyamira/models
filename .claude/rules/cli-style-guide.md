---
description: CLI design style guide — colors, picker infrastructure, table formatting, keybindings, output conventions
globs:
  - src/cli/**
---

# CLI Design Style Guide

This guide defines the visual language and interaction patterns for the Models CLI. All new commands and pickers must follow these conventions. When in doubt, reference `styles.rs` and `picker.rs` as the source of truth for constants.

---

## 1. Color System

### 1.1 Color Constants (`styles.rs`)

```rust
CODE_BG  = Rgb(50, 40, 25)   // inline code background (matches TUI markdown rendering)
INPUT_BG = Rgb(60, 60, 60)   // input badge background
```

### 1.2 TTY Detection

All styling helpers are no-ops when stdout is not a terminal:

```rust
static IS_TTY: OnceLock<bool> = OnceLock::new();
fn is_tty() -> bool {
    *IS_TTY.get_or_init(|| stdout().is_terminal())
}
```

Never apply colors unconditionally — always gate through `is_tty()` or the helper functions below.

### 1.3 Cell Helpers (comfy-table, TTY-gated)

Used for table column headers and highlighted cells:

| Helper | Style |
|--------|-------|
| `header_cell(text)` | Cyan + Bold |
| `bold_cell(text)` | Bold |
| `green_cell(text)` | Green |
| `yellow_cell(text)` | Yellow |
| `dim_cell(text)` | DarkGrey |

### 1.4 Inline Text Helpers (crossterm Stylize, TTY-gated)

Used for prose output and status messages:

| Helper | Style |
|--------|-------|
| `agent_name(text)` | Cyan + Bold |
| `code_ref(text)` | ` text ` Yellow on `CODE_BG` (space-padded) |
| `input_badge(text)` | ` text ` Yellow on `INPUT_BG` (space-padded) |
| `url(text)` | Cyan + Underlined |
| `dim(text)` | DarkGrey |
| `key_value(text)` | Bold |
| `error_prefix()` | `"error:"` Red + Bold |
| `separator(width)` | `─` (U+2500) repeated, DarkGrey |

## 2. Changelog Rendering (termimad)

Changelogs use `changelog_skin()` to build a `MadSkin`:

| Element | Style |
|---------|-------|
| Headers | Magenta + Bold (no underline) |
| Bullets | `•` in Magenta |
| Inline code | Yellow on `Rgb(50, 40, 25)` |
| URLs | Post-processed via `style_urls()` regex → Cyan + Underlined |

This skin is CLI-only. The TUI's regex-based markdown renderer (`src/tui/markdown.rs`) handles inline formatting independently — do not share or merge them.

## 3. Picker Infrastructure

### 3.1 Constants (`picker.rs`)

```rust
VIEWPORT_HEIGHT    = 14        // fixed inline viewport height; preview panes must fit within this
HEADER_STYLE       = Cyan + Bold
ROW_HIGHLIGHT_STYLE = Yellow + Bold
HIGHLIGHT_SYMBOL   = ">> "
ACTIVE_BORDER_STYLE  = Cyan
PREVIEW_BORDER_STYLE = DarkGray
```

### 3.2 Title Format (`picker_title()`)

```
"{name} ({visible} results) | {sort_label} {desc|asc}"
"{name} ({visible} / {total} results) | {sort_label} {desc|asc} | / {query}"
```

The second form is shown when an active filter query reduces the visible count.

### 3.3 Navigation (shared across all pickers)

| Key | Action |
|-----|--------|
| `j` / `Down` | Next |
| `k` / `Up` | Previous |
| `g` | First |
| `G` / `End` | Last |
| `PgDn` / `Ctrl+d` | Page down (10 rows) |
| `PgUp` / `Ctrl+u` | Page up (10 rows) |
| `Enter` | Select / confirm |
| `Esc` / `q` | Cancel |
| `/` | Enter filter mode |

### 3.4 Lifecycle

All three pickers follow the same pattern:

1. Create `PickerTerminal::new()` — enables raw mode
2. Init `TableState::new()`, populate data, call `TableState::select(Some(0))`
3. Event loop: poll stdin → update state → render frame
4. Drop `PickerTerminal` — auto-disables raw mode + clears cursor
5. Return selected value or `None`

**Gotchas:**
- `TableState::select(Some(idx))` must be called before the first render — starts unselected otherwise
- Never use `eprintln!` inside picker code — stdout is in raw mode; stderr corrupts output
- `table.bottom_margin(1)` creates a blank header separator row — remove it for tight inline layouts

## 4. Picker Layouts

### 4.1 Outer Split (all pickers)

```
Constraint::Min(10)    -- table + preview (horizontal split)
Constraint::Length(1)  -- status bar
```

### 4.2 Inner Horizontal Splits

| Picker | Table | Preview |
|--------|-------|---------|
| Models | 55% | 45% |
| Benchmarks | 55% | 45% |
| Agents releases | 38% | 62% |
| Agents sources | 50% | 50% |

### 4.3 Table Columns

**Models (6 cols):** Name(28%), Provider(15%), SortValue(12%), Cost(15%), Capabilities(18%), Release(12%)

**Benchmarks (5 cols):** Name(40%), Creator(22%), Release(20%), R(Length 3), S(Length 3)

**Agents releases — with tool filter (4 cols):** Tool(28%), Version(22%), Released(18%), Ago(16%)

**Agents releases — without tool filter (3 cols):** Version(34%), Released(24%), Ago(20%)

**Agents sources (4 cols):** Track(Length 5), ID(24%), Name(44%), CLI(27%)

## 5. Capability Indicators

Same color mapping as TUI (see `tui-style-guide.md` §2.2):

| Indicator | Color | Meaning |
|-----------|-------|---------|
| `R` | Cyan | Reasoning |
| `A` | Cyan | Adaptive |
| `NR` | DarkGray | Non-reasoning |
| `O` | Green | Open weights |
| `C` | Red | Closed weights |
| `—` | DarkGray | Missing / unknown |

## 6. Score & Data Formatting

| Data | Format | Notes |
|------|--------|-------|
| Benchmark scores | `{:.2}` | 2 decimal places (TUI uses `{:.1}`) |
| Missing values | `—` (U+2014) | DarkGray |
| Prices | `ApiModel::cost_short()` | Shared with TUI list format |
| Star counts | `format_stars()` | e.g., `12.3k`, `1.2m` |

## 7. Picker-Specific Keys

| Picker | Key | Action |
|--------|-----|--------|
| Models | `s` | Cycle sort field |
| Models | `S` | Toggle sort direction |
| Models | `c` | Copy model ID to clipboard |
| Benchmarks | `s` | Cycle sort field |
| Benchmarks | `S` | Toggle sort direction |
| Agents sources | `Space` | Toggle tracked checkbox |

## 8. Status Bar

**Normal mode:**
```
"Enter {action}   / filter   s sort   S reverse   q quit   ↑↓/j/k move"
```

**Filter mode:**
```
"Filter: {query}_  Enter apply  Esc clear  Backspace delete"
```

**Copy feedback:**
```
"Copied to clipboard!"   // Green, shown for 1500ms
```

## 9. Table Output (Non-Interactive)

For non-picker output (e.g., `agents status`, `agents list-sources`):

- Preset: `UTF8_FULL_CONDENSED`
- Headers: `header_cell()` (Cyan + Bold)
- Used when non-TTY or `--json` is not specified
- JSON output: `serde_json::to_string_pretty()` via `--json` flag
- **Width**: call `styles::arrange_table(&mut table)` after `load_preset` on
  every comfy-table — on a TTY it sets `ContentArrangement::Dynamic` so wide
  tables wrap to the terminal width instead of overflowing. **Piped/non-TTY
  output is deliberately left content-sized (byte-identical to previous
  releases)** so text parsers are unaffected; `--json` remains the machine path

## 10. Resolve Pattern

Model lookup priority (most specific to least):

1. Exact `display_id` match
2. Exact `id` match
3. Exact `name` match
4. Partial matches

**Ambiguous:** `"Model query '{q}' was ambiguous; try provider/model. Matches: {list}"`

**Not found:** `"Model '{q}' not found"`

## 11. Copy-to-Clipboard

On Linux/Wayland the `arboard::Clipboard` object must stay alive or the clipboard contents are lost immediately. Spawn a background thread:

```rust
std::thread::spawn(move || {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(&text);
        std::thread::sleep(Duration::from_secs(2));
    }
});
```

This pattern is used in both the TUI (`copy_to_clipboard()`) and the CLI models picker (`KeyCode::Char('c')`). Never hold the `Clipboard` object on the main thread.

## 12. Default Sort Orders

| Picker | Default Sort | Direction |
|--------|-------------|-----------|
| Models | Release date | Descending |
| Benchmarks | Release date | Descending |
