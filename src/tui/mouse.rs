//! Shared, terminal-free helpers for mouse hit-testing.
//!
//! Mouse handling is otherwise untestable without a real terminal, so the
//! coordinate math lives here as pure functions with unit tests. Per-tab mouse
//! handlers (`handle_*_mouse`) and the dispatcher in `event.rs` build on these.
//!
//! ## Geometry-cache contract
//!
//! ratatui computes layout `Rect`s fresh inside each render pass and discards
//! them. To hit-test a click we cache the relevant `Rect`s on each sub-app
//! (`ModelsApp`, `BenchmarksApp`, …) at the *end* of that tab's render, then
//! read them back in the tab's mouse handler on the next event. The main loop
//! always draws before it handles events (`terminal.draw(...)` precedes
//! `handle_events(...)`), so cached rects — and any `ListState::offset()` the
//! render mutated — reflect exactly the frame the user clicked on.

use crossterm::event::MouseEvent;
use ratatui::layout::{Position, Rect};

/// True when the event's cursor position falls inside `area`.
///
/// `None` (an un-rendered panel) never contains anything.
pub fn hit(area: Option<Rect>, ev: &MouseEvent) -> bool {
    area.is_some_and(|r| r.contains(Position::new(ev.column, ev.row)))
}

/// Map an absolute terminal `click_row` to an index into a list's items.
///
/// - `area` is the `Rect` the list widget was rendered into.
/// - `offset` is the list's current scroll offset (`ListState::offset()` read
///   *after* render, so it reflects the viewport clamp ratatui applied).
/// - `top_skip` is the number of rows at the top of `area` that are **not**
///   list items — e.g. `1` when the list draws its own top border inside
///   `area`, `0` when `area` is already the bare item region. An in-list header
///   rendered as item index 0 (as in the Models model list) is **not** counted
///   here — it is a real item; the caller maps item 0 → header and subtracts.
/// - `item_count` is the total number of items (including any header item).
///
/// Returns the item index under the cursor, or `None` when the click is above
/// the first item row, below the last visible row, or past the final item.
pub fn row_at(
    area: Rect,
    offset: usize,
    top_skip: u16,
    item_count: usize,
    click_row: u16,
) -> Option<usize> {
    let first = area.y.saturating_add(top_skip);
    let bottom = area.y.saturating_add(area.height);
    if click_row < first || click_row >= bottom {
        return None;
    }
    let visible_row = (click_row - first) as usize;
    let idx = offset.saturating_add(visible_row);
    (idx < item_count).then_some(idx)
}

/// Map a click to a row index in a **popup list** that renders a fresh
/// `ListState` every frame (seeded with `selected`, starting from offset 0 —
/// true for all the picker popups). `inner` is the popup's inner list rect
/// (borders already excluded). The render offset is therefore deterministic:
/// ratatui scrolls a from-zero state just far enough to keep `selected` visible,
/// i.e. `offset = max(0, selected - (visible_rows - 1))`. Returns the clicked
/// item index, or `None` when the click is outside the item rows / past the
/// last item.
pub fn popup_row_at(
    inner: Rect,
    selected: usize,
    item_count: usize,
    click_row: u16,
) -> Option<usize> {
    let visible = inner.height as usize;
    let offset = selected.saturating_sub(visible.saturating_sub(1));
    row_at(inner, offset, 0, item_count, click_row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseEventKind};

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn ev_at(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn hit_none_is_false() {
        assert!(!hit(None, &ev_at(5, 5)));
    }

    #[test]
    fn hit_inside_and_outside() {
        let area = Some(rect(2, 3, 10, 4)); // x:2..12, y:3..7
        assert!(hit(area, &ev_at(2, 3))); // top-left corner inclusive
        assert!(hit(area, &ev_at(11, 6))); // bottom-right corner inclusive
        assert!(!hit(area, &ev_at(12, 6))); // x past right edge (exclusive)
        assert!(!hit(area, &ev_at(5, 7))); // y past bottom edge (exclusive)
        assert!(!hit(area, &ev_at(1, 5))); // left of area
    }

    #[test]
    fn row_at_bare_list_no_skip() {
        // area y:5..10, no border, offset 0
        let a = rect(0, 5, 20, 5);
        assert_eq!(row_at(a, 0, 0, 100, 5), Some(0)); // first row
        assert_eq!(row_at(a, 0, 0, 100, 6), Some(1));
        assert_eq!(row_at(a, 0, 0, 100, 9), Some(4)); // last visible row
    }

    #[test]
    fn row_at_with_top_border_skip() {
        // area includes a top border row at y=5; items start at y=6
        let a = rect(0, 5, 20, 6);
        assert_eq!(row_at(a, 0, 1, 100, 5), None); // border row → None
        assert_eq!(row_at(a, 0, 1, 100, 6), Some(0)); // first item
        assert_eq!(row_at(a, 0, 1, 100, 7), Some(1));
    }

    #[test]
    fn row_at_applies_scroll_offset() {
        // scrolled down: first visible item is index 12
        let a = rect(0, 5, 20, 5);
        assert_eq!(row_at(a, 12, 0, 100, 5), Some(12));
        assert_eq!(row_at(a, 12, 0, 100, 7), Some(14));
    }

    #[test]
    fn row_at_above_first_is_none() {
        let a = rect(0, 5, 20, 5);
        assert_eq!(row_at(a, 0, 0, 100, 4), None); // row above area
    }

    #[test]
    fn row_at_below_last_visible_is_none() {
        let a = rect(0, 5, 20, 5); // rows 5..10
        assert_eq!(row_at(a, 0, 0, 100, 10), None); // first row past bottom
        assert_eq!(row_at(a, 0, 0, 100, 99), None);
    }

    #[test]
    fn row_at_past_last_item_is_none() {
        // only 3 items but viewport is taller — clicking empty space → None
        let a = rect(0, 5, 20, 10);
        assert_eq!(row_at(a, 0, 0, 3, 7), Some(2)); // last real item
        assert_eq!(row_at(a, 0, 0, 3, 8), None); // empty row below items
    }

    #[test]
    fn popup_row_at_no_scroll_when_fits() {
        // inner rect y:5..10 (5 rows), 4 items, selection irrelevant (all fit)
        let inner = rect(0, 5, 20, 5);
        assert_eq!(popup_row_at(inner, 0, 4, 5), Some(0)); // first row
        assert_eq!(popup_row_at(inner, 3, 4, 7), Some(2)); // third row
        assert_eq!(popup_row_at(inner, 0, 4, 9), None); // empty row past items
        assert_eq!(popup_row_at(inner, 0, 4, 4), None); // above the inner rect
    }

    #[test]
    fn popup_row_at_accounts_for_scroll() {
        // inner viewport only 5 rows tall, 30 items, selected near the end →
        // offset = 20 - (5 - 1) = 16, so the top visible row is item 16.
        let inner = rect(0, 5, 20, 5);
        assert_eq!(popup_row_at(inner, 20, 30, 5), Some(16)); // top visible row
        assert_eq!(popup_row_at(inner, 20, 30, 9), Some(20)); // bottom (selected)
    }
}
