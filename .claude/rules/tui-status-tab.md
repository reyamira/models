---
description: Status tab design conventions — overall dashboard, provider detail, gauges, SoftCards, component status mapping, maintenance icons
globs:
  - src/tui/status/**
---

# Status Tab Design Conventions

Tab-specific patterns only. For shared colors, borders, focus, search, footer, and scrollbars see `tui-style-guide.md`.

---

## 1. Top-Level Layout

```
Length(32)  -- Provider list (fixed width)
Min(0)      -- Detail area (overall dashboard or provider detail)
```

---

## 2. Provider List

**"Overall" entry**: always index 0. Format: `"  Overall"` (2 spaces + text). No health icon. Selected style: Yellow + BOLD with caret prefix.

**Provider entries** (index 1+):

```
{caret} {icon} {name:<20} {issue_count}
```

- Caret: `"> "` (focused+selected), `"  "` (otherwise)
- Icon: health icon in health color (from `status_health_icon()` / `status_health_style()`)
- Name: truncated to 20 chars, Yellow+BOLD when selected
- `issue_count`: omitted when 0; shown in health color when > 0. Excludes maintenance — `issue_count()` counts only non-maintenance problems.

**List title format**:
- Normal: `" Providers ({count}) "`
- Loading: `" Providers ({count}) refreshing... "`
- Search active: `" Providers ({count}) [/{query}] "`

---

## 3. Component Status Mapping

`component_status_icon()` and `component_status_style()` use `contains()` matching on the lowercased status string:

| Status string matches | Icon | Color |
|-----------------------|------|-------|
| `"operational"` | `●` | `Color::Green` |
| `"partial"` | `◐` | `Color::Red` |
| `"degraded"` | `◐` | `Color::Yellow` |
| `"outage"` / `"major"` / `"down"` | `✗` | `Color::Red` |
| `"maintenance"` | `◆` | `Color::Blue` |
| anything else | `?` | `Color::DarkGray` |

Note: `"partial"` is checked before `"degraded"` — `partial_outage` maps to Red `◐`, `degraded_performance` maps to Yellow `◐`.

---

## 4. Maintenance Icons

Both icons use `Color::Blue`:

| Icon | Condition |
|------|-----------|
| `◇` | Scheduled — status does **not** contain `"progress"`, `"active"`, or `"verifying"` |
| `◆` | Active/in-progress — status **does** contain any of those substrings |

---

## 5. Incident Stage Colors

`incident_stage_style()` uses `contains()` matching on the lowercased incident status:

| Stage matches | Color |
|---------------|-------|
| `"resolved"` | `Color::Green` |
| `"monitoring"` | `Color::Cyan` |
| `"maint"` | `Color::Blue` |
| anything else | `Color::Yellow` |

`incident_stage_health()` maps the same stages to `ProviderHealth` for `SoftCard` accent stripe color.

---

## 6. Incident Impact Colors

`incident_impact_style()` uses `contains()` on lowercased impact:

| Impact matches | Color |
|----------------|-------|
| `"critical"` / `"major"` | `Color::Red` |
| `"minor"` / `"partial"` | `Color::Yellow` |
| `"maint"` | `Color::Blue` |
| anything else / empty | `Color::DarkGray` |

---

## 7. Status Field and Section Label Styles

Two reusable helpers used consistently across all card builders:

- `status_field_label_style()` → `Color::Blue` (for inline field labels like `"Issue: "`, `"Status: "`, `"Updated: "`)
- `status_section_label_style()` → `Color::Blue` + `Modifier::BOLD` (for sub-section headers like `"  Services"`, `"  Latest Update"`)

---

## 8. Overall Dashboard (when "Overall" is selected)

**Header area** (`Constraint::Length(4)`): gauge row + freshness/legend row.

**Gauge**:
- Foreground: health color (from `status_health_style()`), background: `Color::DarkGray`
- Label when components present: `"{op}/{total}  {ratio:.0}%"`
- Label when no components: `"{icon} {verdict_copy}"` — e.g. `"● All systems operational"`

**Icon+count legend** (below gauge, 1 line):
- `"● "` (Green) + `"{n} operational  "` — omitted when count is 0
- `"◐ "` (Yellow) + `"{n} degraded  "` — omitted when count is 0
- `"◐ "` (Red) + `"{n} partial  "` — omitted when count is 0
- `"✗ "` (Red) + `"{n} outage  "` — omitted when count is 0
- `"? "` (DarkGray) + `"{n} unknown  "` — omitted when count is 0

**Panel layout** adapts to terminal width:
- Narrow (< 100 cols): 3 vertical panels at `[42%, 34%, 24%]`
- Wide (≥ 100 cols): horizontal `60% | 40%`, right side split `60% / 40%`

**Three SoftCard panels** (Incidents / Degradation / Maintenance):
- Focused panel: Cyan border; unfocused: DarkGray
- `OverallPanelFocus`: Incidents / Degradation / Maintenance — `h`/`l` cycles
- Panel title includes count: `" Incidents (2) "`, `" Degradation (1) "`, `" Maintenance (1) "`

---

## 9. SoftCard Layout

`SoftCard` widget renders:
1. Left accent stripe (2 cols) in health color
2. Content lines with 1-space left margin inside the stripe

**Health color mapping for accent stripe**:
- `Operational` → `Color::Green`
- `Degraded` → `Color::Yellow`
- `Outage` → `Color::Red`
- `Maintenance` → `Color::Blue`
- `Unknown` → `Color::DarkGray`

Cards are passed to `ScrollablePanel::with_cards()`.

---

## 10. Incident Cards (Overall dashboard)

Per incident card content order:
1. `"{icon} {provider_name}"` (icon in health color, name BOLD)
2. `"  Issue: "` (Blue) + `"{incident.name}"` (BOLD)
3. `"  Status: "` (Blue) + stage value (stage color) + optional `"  Impact: "` (Blue) + impact (impact color) + optional time field
4. Additional incidents line (if > 1): `"  Additional incidents: "` (Blue) + `"{n} more"` (DarkGray)
5. Affected components or services (indented, max 4 items + overflow line)
6. Latest update body (if not duplicate of summary/issue name)
7. User-visible caveat note (if present): `"  Note: "` (Blue) + note text (DarkGray)

---

## 11. Provider Detail View (when a specific provider is selected)

**Status header block** (dynamic height):
- Border: `Color::White` (not Cyan — this block is always "special")
- Title: `" {display_name} · {time_label}: {time_value} "`
- Optional lines (each `Constraint::Length(1)`): status_note (DarkGray), caveat/unavailable (Yellow), error (Red)
- Contains: gauge (1 line) + legend (1 line) + optional annotation lines

**Services panel**: `Constraint::Length(min(component_count + 2, 12))`
- Title: `Line` with icon+count summary — `"Services ({total})  ● {op}  ◐ {deg}  ◐ {partial}  ✗ {out}  ◆ {maint}"`  — each icon in its color, counts in default. Zero-count categories omitted.
- Components sorted by severity (✗ → ◐ → ◆ → ●), then by `position`, then alphabetically
- `only_show_if_degraded` components hidden when operational
- Panel focus: `DetailPanelFocus::Services`

**Bottom area** (`Constraint::Min(0)`): incidents + maintenance split

- Width ≥ 60: horizontal `60% | 40%`
- Width < 60: vertical `60% | 40%`
- When no maintenance: incidents take full area
- Focused panel (Services/Incidents/Maintenance): Cyan border; unfocused: DarkGray
- `DetailPanelFocus` cycles via `h`/`l`

---

## 12. Services Panel Grouping

Components are grouped by `group_name` in the detail services panel:

- Group header: worst-status icon (in status color) + `" {group_name}"` (BOLD) + summary (DarkGray)
- Component rows: `"  {icon} {name}"` (icon in status color, name in default) + status string (DarkGray)
- Groups with degraded/outage components sort before operational groups

Chinese component names are translated via a hardcoded map (e.g., `"API 服务"` → `"API Service (API 服务)"`).

---

## 13. Provider Tracker Modal

Popup for tracking/untracking providers (opened with `a`):

- **Size**: `centered_rect_fixed(min(60, screen_width - 4), dynamic_height)`
- **Border**: `Color::Cyan`
- **Title**: `" Track Providers "`
- **Bottom title**: `" Space: toggle | Enter: save | Esc: cancel "` (centered)

**Item format**:

```
{checkbox} {name:<30}  {icon}
```

| Part | Width | Style |
|------|-------|-------|
| `[x]` / `[ ]` | 4 | default |
| name | 30, left-aligned, BOLD | BOLD |
| health icon | 1 | health color |

Selected row: Yellow + BOLD.

**Key interception**: modal must intercept `q` before global handler.

---

## 14. Refresh Timing Format

Relative time shown in overall dashboard freshness suffix and provider detail time values:

| Elapsed | Format |
|---------|--------|
| 0–4 seconds | `"just now"` |
| 5–59 seconds | `"{n}s ago"` |
| 60–3599 seconds | `"{n}m ago"` |
| 3600–86399 seconds | `"{n}h ago"` |
| ≥ 86400 seconds | `"{n}d ago"` |

---

## 15. Empty States

All empty state messages use `Color::DarkGray`:

- `"No active incidents reported right now"` + descriptive subtitle
- `"No component-reported degradation right now"` + descriptive subtitle
- `"No scheduled maintenance"` (maintenance panel)
- `"Select a provider to view details"` (detail area when nothing selected)

Title line of empty state: `Color::Green` (positive confirmation tone). Subtitle: `Color::DarkGray`.

---

## 16. Mouse

`handle_status_mouse` (in `status/app.rs`); see style guide §12 for the shared pattern.

- **Cached rects** (`StatusApp`): `provider_list_area` (the block's bare inner rect, `top_skip = 0`; item 0 = "Overall", so the hit index maps straight to the display index), plus six sub-panel rects — `overall_{incidents,degradation,maintenance}_area` (Overall dashboard) and `detail_{services,incidents,maintenance}_area` (provider detail). `draw_overall_dashboard` / `draw_provider_status_detail` **return** their rects; `draw_status_main` writes them onto `StatusApp` after the detail-branch borrow ends. **All six are reset to `None` at the top of every frame** so a vanished panel can't keep a stale rect and only the active view's set is live.
- **Hit-test branches on `is_overall_selected()`** — only the matching rect set is consulted.
- **Click:** provider row → focus List + `select_at_index` (0 = Overall is valid); a dashboard sub-panel → set `OverallPanelFocus`; a provider-detail sub-panel → set `DetailPanelFocus`. Sub-panels use `hit()` only (no per-row selection in v1).
- **Wheel (focus-then-scroll):** the panel/sub-panel under the cursor gains focus, then its matching scroll (`detail_scroll`, `overall_*_scroll`, `services_scroll`, `maintenance_scroll`).
- The provider list already rendered into the real `&mut list_state`, so `offset()` was valid — no copy bug to fix here.
