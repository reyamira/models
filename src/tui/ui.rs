use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use super::app::{App, Mode, Tab};
use crate::status::ProviderHealth;
use crate::tui::widgets::scroll_offset::ScrollOffset;
use crate::tui::widgets::scrollable_panel::ScrollablePanel;

/// Border style: Cyan when focused, DarkGray when not.
pub(super) fn focus_border(focused: bool) -> Style {
    Style::default().fg(if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    })
}

/// Caret prefix for list items: "> " when focused, "  " when not.
pub(super) fn caret(focused: bool) -> &'static str {
    if focused {
        "> "
    } else {
        "  "
    }
}

/// Selection style: Yellow + BOLD when selected, default otherwise.
pub(super) fn selection_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

/// Build a help-popup line: 16-char padded key in Yellow + description.
fn help_line<'a>(key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {:<14}", key), Style::default().fg(Color::Yellow)),
        Span::raw(desc),
    ])
}

pub(super) fn status_health_style(health: ProviderHealth) -> Style {
    match health {
        ProviderHealth::Operational => Style::default().fg(Color::Green),
        ProviderHealth::Degraded => Style::default().fg(Color::Yellow),
        ProviderHealth::Outage => Style::default().fg(Color::Red),
        ProviderHealth::Maintenance => Style::default().fg(Color::Blue),
        ProviderHealth::Unknown => Style::default().fg(Color::DarkGray),
    }
}

pub(super) fn status_health_icon(health: ProviderHealth) -> &'static str {
    match health {
        ProviderHealth::Operational => "●",
        ProviderHealth::Degraded => "◐",
        ProviderHealth::Outage => "✗",
        ProviderHealth::Maintenance => "◆",
        ProviderHealth::Unknown => "?",
    }
}

/// Compute the visual height of a single line when word-wrapped to `wrap_width`.
///
/// Returns 1 for empty or zero-width lines, otherwise `div_ceil(width, wrap_width)`
/// with +1 buffer for lines that actually wrap (ratatui's word-wrap can overshoot
/// `div_ceil` by one row).
fn visual_line_height(line: &Line<'_>, wrap_width: usize) -> u16 {
    let w = line.width();
    if wrap_width == 0 || w == 0 {
        1
    } else {
        let base = w.div_ceil(wrap_width).max(1) as u16;
        if w > wrap_width {
            base + 1
        } else {
            base
        }
    }
}

/// Sum visual (wrapped) heights for a slice of lines.
///
/// Uses `div_ceil(line.width(), wrap_width)` with a +1 buffer for lines that
/// actually wrap, since ratatui's word-wrap can produce one extra visual row.
#[allow(dead_code)]
pub(in crate::tui) fn visual_line_total(lines: &[Line<'_>], wrap_width: usize) -> u16 {
    lines
        .iter()
        .map(|line| visual_line_height(line, wrap_width))
        .sum()
}

/// Return per-line visual (wrapped) heights for a slice of lines.
///
/// Callers can derive cumulative offsets by scanning the returned `Vec`.
#[allow(dead_code)]
pub(in crate::tui) fn visual_line_heights(lines: &[Line<'_>], wrap_width: usize) -> Vec<u16> {
    lines
        .iter()
        .map(|line| visual_line_height(line, wrap_width))
        .collect()
}

/// Build a dash-padded section header line like `"── Title ──────"`.
///
/// The result is styled DarkGray + BOLD, matching the models detail panel pattern.
#[allow(dead_code)]
pub(in crate::tui) fn section_header_line(title: &str, width: usize) -> Line<'static> {
    let prefix = format!("\u{2500}\u{2500} {} ", title);
    let fill_len = width.saturating_sub(prefix.chars().count());
    let header = format!("{}{}", prefix, "\u{2500}".repeat(fill_len));
    Line::from(Span::styled(
        header,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Like [`section_header_line`], but with an inline `(suffix)` blurb after the
/// title, e.g. `"── Pricing ($ per 1M tokens) ──────"`. The title keeps the
/// DarkGray + BOLD style; the suffix (including its parens and surrounding
/// spaces) is DarkGray **without** bold so it reads as a secondary annotation.
/// The trailing dash fill respects the suffix's visual width.
///
/// Passing an empty `suffix` is equivalent to [`section_header_line`].
#[allow(dead_code)]
pub(in crate::tui) fn section_header_line_with_suffix(
    title: &str,
    suffix: &str,
    width: usize,
) -> Line<'static> {
    if suffix.is_empty() {
        return section_header_line(title, width);
    }
    let prefix = format!("\u{2500}\u{2500} {} ", title);
    let suffix_text = format!("({}) ", suffix);
    let consumed = prefix.chars().count() + suffix_text.chars().count();
    let fill_len = width.saturating_sub(consumed);
    let fill = "\u{2500}".repeat(fill_len);
    Line::from(vec![
        Span::styled(
            prefix,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(suffix_text, Style::default().fg(Color::DarkGray)),
        Span::styled(
            fill,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

/// Build filter toggle spans in `[N] label` format.
///
/// Each tuple is `(key, label, active)`. Active keys render in Green, inactive
/// in DarkGray. Returns a flat `Vec<Span>` ready for `Line::from(...)`.
pub(in crate::tui) fn filter_toggle_spans(toggles: &[(&str, &str, bool)]) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(toggles.len() * 2);
    for (key, label, active) in toggles {
        let color = if *active {
            Color::Green
        } else {
            Color::DarkGray
        };
        spans.push(Span::styled(
            format!("[{}]", key),
            Style::default().fg(color),
        ));
        spans.push(Span::raw(format!(" {} ", label)));
    }
    spans
}

/// Create a centered rect using fixed width and height
pub(super) fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

/// Create a centered rect using percentage of the available area
pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(1), // Footer/search
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);

    match app.current_tab {
        Tab::Models => {
            super::models::render::draw_main(f, chunks[1], app);
        }
        Tab::Agents => {
            super::agents::render::draw_agents_main(f, chunks[1], app);
        }
        Tab::Benchmarks => {
            super::benchmarks::render::draw_benchmarks_main(f, chunks[1], app);
        }
        Tab::Status => {
            super::status::render::draw_status_main(f, chunks[1], app);
        }
    }

    draw_footer(f, chunks[2], app);

    // Draw help popup on top if visible
    if app.show_help {
        draw_help_popup(f, &app.help_scroll, app);
    }

    // Draw picker / add-agent modals on top if visible (agents tab only)
    if app.current_tab == Tab::Agents {
        if let Some(agents_app) = &app.agents_app {
            if agents_app.show_picker {
                super::agents::render::draw_picker_modal(f, app);
            } else if agents_app.show_add_form {
                super::agents::render::draw_add_agent_modal(f, app);
            } else if agents_app.show_update_confirm {
                super::agents::render::draw_update_confirm_modal(f, app);
            }
        }
    }
}

/// Map a click coordinate to a tab, if it lands on a header tab label.
///
/// The header is a single line at the top of the screen (`row == 0`). The x
/// ranges are computed from the same label/separator layout `draw_header`
/// renders, so the two stay in lockstep. Returns `None` for clicks off the
/// labels (separators, the hint text, or any other row).
pub(super) fn tab_at(column: u16, row: u16) -> Option<Tab> {
    if row != 0 {
        return None;
    }
    // Mirror of draw_header: leading space, then "Label" / " | " / "Label" …
    let labels = [
        ("Models", Tab::Models),
        ("Agents", Tab::Agents),
        ("Benchmarks", Tab::Benchmarks),
        ("Status", Tab::Status),
    ];
    let mut x: u16 = 1; // leading Span::raw(" ")
    for (i, (label, tab)) in labels.iter().enumerate() {
        let w = label.len() as u16;
        if column >= x && column < x + w {
            return Some(*tab);
        }
        x += w;
        if i + 1 < labels.len() {
            x += 3; // " | " separator
        }
    }
    None
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let tab_style = |tab: Tab| {
        if app.current_tab == tab {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    };

    let header = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled("Models", tab_style(Tab::Models)),
        Span::raw(" | "),
        Span::styled("Agents", tab_style(Tab::Agents)),
        Span::raw(" | "),
        Span::styled("Benchmarks", tab_style(Tab::Benchmarks)),
        Span::raw(" | "),
        Span::styled("Status", tab_style(Tab::Status)),
        Span::styled("  [/] switch tabs", Style::default().fg(Color::DarkGray)),
    ]));
    f.render_widget(header, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    // If there's a status message, show it instead of normal footer
    if let Some(status) = &app.status_message {
        let content = Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(status, Style::default().fg(Color::Green)),
        ]);
        let paragraph = Paragraph::new(content);
        f.render_widget(paragraph, area);
        return;
    }

    match app.mode {
        Mode::Normal => {
            // Split footer into left and right sections
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0), Constraint::Length(10)])
                .split(area);

            let left_content = match app.current_tab {
                Tab::Models => Line::from(vec![
                    Span::styled(" q ", Style::default().fg(Color::Yellow)),
                    Span::raw("quit  "),
                    Span::styled(" ↑/↓ ", Style::default().fg(Color::Yellow)),
                    Span::raw("nav  "),
                    Span::styled(" Tab ", Style::default().fg(Color::Yellow)),
                    Span::raw("switch  "),
                    Span::styled(" / ", Style::default().fg(Color::Yellow)),
                    Span::raw("search  "),
                    Span::styled(" s/S ", Style::default().fg(Color::Yellow)),
                    Span::raw("sort  "),
                    Span::styled(" 1-6 ", Style::default().fg(Color::Yellow)),
                    Span::raw("filter  "),
                    Span::styled(" r ", Style::default().fg(Color::Yellow)),
                    Span::raw("refresh  "),
                    Span::styled(" c ", Style::default().fg(Color::Yellow)),
                    Span::raw("copy"),
                ]),
                Tab::Agents => Line::from(vec![
                    Span::styled(" q ", Style::default().fg(Color::Yellow)),
                    Span::raw("quit  "),
                    Span::styled(" / ", Style::default().fg(Color::Yellow)),
                    Span::raw("search  "),
                    Span::styled(" s ", Style::default().fg(Color::Yellow)),
                    Span::raw("sort  "),
                    Span::styled(" a ", Style::default().fg(Color::Yellow)),
                    Span::raw("track  "),
                    Span::styled(" A ", Style::default().fg(Color::Yellow)),
                    Span::raw("add  "),
                    Span::styled(" u ", Style::default().fg(Color::Yellow)),
                    Span::raw("update  "),
                    Span::styled(" U ", Style::default().fg(Color::Yellow)),
                    Span::raw("update all  "),
                    Span::styled(" x ", Style::default().fg(Color::Yellow)),
                    Span::raw("cancel  "),
                    Span::styled(" o ", Style::default().fg(Color::Yellow)),
                    Span::raw("docs  "),
                    Span::styled(" r ", Style::default().fg(Color::Yellow)),
                    Span::raw("repo  "),
                    Span::styled(" R ", Style::default().fg(Color::Yellow)),
                    Span::raw("refresh"),
                ]),
                Tab::Benchmarks => {
                    if app.selections.len() >= 2 {
                        use super::benchmarks::{BenchmarkFocus, BottomView};
                        let mut spans = vec![
                            Span::styled(" q ", Style::default().fg(Color::Yellow)),
                            Span::raw("quit  "),
                            Span::styled(" h/l ", Style::default().fg(Color::Yellow)),
                            Span::raw("focus  "),
                            Span::styled(" t ", Style::default().fg(Color::Yellow)),
                            Span::raw(if app.benchmarks_app.show_creators_in_compare {
                                "models  "
                            } else {
                                "creators  "
                            }),
                            Span::styled(" Space ", Style::default().fg(Color::Yellow)),
                            Span::raw("select  "),
                            Span::styled(" v ", Style::default().fg(Color::Yellow)),
                            Span::raw("view  "),
                        ];
                        match app.benchmarks_app.bottom_view {
                            BottomView::H2H => {
                                spans.extend([
                                    Span::styled(" d ", Style::default().fg(Color::Yellow)),
                                    Span::raw("detail  "),
                                ]);
                                if app.benchmarks_app.focus == BenchmarkFocus::Compare {
                                    spans.extend([
                                        Span::styled(" j/k ", Style::default().fg(Color::Yellow)),
                                        Span::raw("scroll  "),
                                    ]);
                                }
                            }
                            BottomView::Scatter => {
                                spans.extend([
                                    Span::styled(" x ", Style::default().fg(Color::Yellow)),
                                    Span::raw("X-axis  "),
                                    Span::styled(" y ", Style::default().fg(Color::Yellow)),
                                    Span::raw("Y-axis  "),
                                ]);
                            }
                            BottomView::Radar => {
                                spans.extend([
                                    Span::styled(" a ", Style::default().fg(Color::Yellow)),
                                    Span::raw("preset  "),
                                ]);
                            }
                            BottomView::Detail => {}
                        }
                        spans.extend([
                            Span::styled(" c ", Style::default().fg(Color::Yellow)),
                            Span::raw("clear  "),
                            Span::styled(" s/S ", Style::default().fg(Color::Yellow)),
                            Span::raw("sort  "),
                            Span::styled(" r ", Style::default().fg(Color::Yellow)),
                            Span::raw("refresh  "),
                            Span::styled(" / ", Style::default().fg(Color::Yellow)),
                            Span::raw("search"),
                        ]);
                        Line::from(spans)
                    } else {
                        let active_file = app.multi_store.file(app.benchmarks_app.active_source);
                        let mut spans = vec![
                            Span::styled(" q ", Style::default().fg(Color::Yellow)),
                            Span::raw("quit  "),
                            Span::styled(" 1-2 ", Style::default().fg(Color::Yellow)),
                            Span::raw("group  "),
                        ];
                        // Reasoning filter hint hidden when the active source
                        // carries no reasoning metadata (key is a no-op then).
                        if super::benchmarks::BenchmarksApp::reasoning_filter_available(active_file)
                        {
                            spans.push(Span::styled(" 3 ", Style::default().fg(Color::Yellow)));
                            spans.push(Span::raw("reasoning  "));
                        }
                        spans.extend([
                            Span::styled(" 4 ", Style::default().fg(Color::Yellow)),
                            Span::raw("weights  "),
                            Span::styled(" s/S ", Style::default().fg(Color::Yellow)),
                            Span::raw("sort  "),
                            Span::styled(" C ", Style::default().fg(Color::Yellow)),
                            Span::raw("columns  "),
                            Span::styled(" a ", Style::default().fg(Color::Yellow)),
                            Span::raw("avg  "),
                            Span::styled(" r ", Style::default().fg(Color::Yellow)),
                            Span::raw("refresh  "),
                            Span::styled(" / ", Style::default().fg(Color::Yellow)),
                            Span::raw("search  "),
                            Span::styled(" i ", Style::default().fg(Color::Yellow)),
                            Span::raw("glossary  "),
                            Span::styled(" Space ", Style::default().fg(Color::Yellow)),
                            Span::raw("select"),
                        ]);
                        Line::from(spans)
                    }
                }
                Tab::Status => {
                    let hints = vec![
                        Span::styled(" q ", Style::default().fg(Color::Yellow)),
                        Span::raw("quit  "),
                        Span::styled(" / ", Style::default().fg(Color::Yellow)),
                        Span::raw("search  "),
                        Span::styled(" Tab ", Style::default().fg(Color::Yellow)),
                        Span::raw("focus  "),
                        Span::styled(" a ", Style::default().fg(Color::Yellow)),
                        Span::raw("track  "),
                        Span::styled(" o ", Style::default().fg(Color::Yellow)),
                        Span::raw("open page  "),
                        Span::styled(" r ", Style::default().fg(Color::Yellow)),
                        Span::raw("refresh"),
                    ];
                    Line::from(hints)
                }
            };

            let right_content = Line::from(vec![
                Span::styled(" ? ", Style::default().fg(Color::Yellow)),
                Span::raw("help "),
            ]);

            f.render_widget(Paragraph::new(left_content), chunks[0]);
            f.render_widget(
                Paragraph::new(right_content).alignment(ratatui::layout::Alignment::Right),
                chunks[1],
            );
        }
        Mode::Search => {
            // Get the correct search query based on current tab
            let search_query = match app.current_tab {
                Tab::Models => &app.models_app.search_query,
                Tab::Agents => app
                    .agents_app
                    .as_ref()
                    .map(|a| &a.search_query)
                    .unwrap_or(&app.models_app.search_query),
                Tab::Benchmarks => &app.benchmarks_app.search_query,
                Tab::Status => app
                    .status_app
                    .as_ref()
                    .map(|a| &a.search_query)
                    .unwrap_or(&app.models_app.search_query),
            };
            let content = Line::from(vec![
                Span::styled(" Search: ", Style::default().fg(Color::Cyan)),
                Span::raw(search_query),
                Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
                Span::raw("  "),
                Span::styled(" Enter/Esc ", Style::default().fg(Color::Yellow)),
                Span::raw("confirm"),
            ]);
            f.render_widget(Paragraph::new(content), area);
        }
    };
}

fn draw_help_popup(f: &mut Frame, scroll: &ScrollOffset, app: &App) {
    let current_tab = app.current_tab;
    let area = centered_rect(50, 70, f.area());

    // Clear the area behind the popup
    f.render_widget(Clear, area);

    let help_section = |title: &'static str| -> Line<'static> {
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let mut help_text = vec![
        // Common: Navigation
        help_section("Navigation"),
        help_line("j/↓", "Move down"),
        help_line("k/↑", "Move up"),
        help_line("g", "First item"),
        help_line("G", "Last item"),
        help_line("Ctrl+d/PgDn", "Page down"),
        help_line("Ctrl+u/PgUp", "Page up"),
        Line::from(""),
        // Common: Panels
        help_section("Panels"),
        help_line("h/←/l/→", "Switch panels"),
        help_line("Tab", "Switch panels"),
        Line::from(""),
        // Common: Search
        help_section("Search"),
        help_line("/", "Start search"),
        help_line("Enter/Esc", "Exit search mode"),
        help_line("Esc", "Clear search (in normal mode)"),
        Line::from(""),
        // Common: Mouse
        help_section("Mouse"),
        help_line("Click row", "Select it / focus its panel"),
        help_line("Click panel", "Focus it"),
        help_line("Click tab", "Switch tab"),
        help_line("Scroll", "Scroll panel under cursor"),
        help_line("In popups", "Scroll; click a row to select"),
        Line::from(""),
    ];

    // Tab-specific sections
    match current_tab {
        Tab::Models => {
            help_text.extend(vec![
                help_section("Filters & Sort"),
                help_line("s", "Cycle sort (name → date → cost → context)"),
                help_line("S", "Toggle sort direction"),
                help_line("1", "Toggle reasoning models filter"),
                help_line("2", "Toggle tools filter"),
                help_line("3", "Toggle open weights filter"),
                help_line("4", "Toggle free models filter"),
                help_line("5", "Cycle provider category filter"),
                help_line("6", "Toggle category grouping"),
                Line::from(""),
                help_section("Actions"),
                help_line("r", "Refresh models.dev data"),
                Line::from(""),
                help_section("Copy & Open"),
                help_line("c", "Copy provider/model"),
                help_line("C", "Copy model only"),
                help_line("o", "Open provider docs in browser"),
                help_line("D", "Copy provider docs URL"),
                help_line("A", "Copy provider API URL"),
                Line::from(""),
            ]);
        }
        Tab::Agents => {
            help_text.extend(vec![
                help_section("Filters & Sort"),
                help_line("s", "Cycle sort (name → updated → stars → status)"),
                help_line("1", "Toggle installed filter"),
                help_line("2", "Toggle CLI filter"),
                help_line("3", "Toggle open source filter"),
                Line::from(""),
                help_section("Actions"),
                help_line("o", "Open docs in browser"),
                help_line("r", "Open GitHub repo in browser"),
                help_line("R", "Refresh GitHub data"),
                help_line("c", "Copy agent name"),
                help_line("a", "Add/remove tracked agents"),
                help_line("A", "Add a new agent (name + repo)"),
                help_line(
                    "u",
                    "Update selected agent (confirm: Enter bg / i interactive)",
                ),
                help_line("U", "Update all agents with an available update"),
                help_line("x", "Cancel the selected agent's running update"),
                Line::from(""),
                help_section("Search Navigation"),
                help_line("n", "Next search match"),
                help_line("N", "Previous search match"),
                Line::from(""),
                help_section("Status Indicators"),
                Line::from(vec![
                    Span::styled(
                        format!("  {:<14}", "○"),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw("Not tracked"),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {:<14}", "◐"), Style::default().fg(Color::Yellow)),
                    Span::raw("Loading GitHub data"),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {:<14}", "●"), Style::default().fg(Color::Green)),
                    Span::raw("Up to date"),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {:<14}", "●"), Style::default().fg(Color::Blue)),
                    Span::raw("Update available"),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {:<14}", "✗"), Style::default().fg(Color::Red)),
                    Span::raw("Fetch failed"),
                ]),
                Line::from(""),
            ]);
        }
        Tab::Benchmarks => {
            help_text.extend(vec![
                help_section("Data Source"),
                help_line("}", "Next data source"),
                help_line("{", "Previous data source"),
                help_line("r", "Refresh active source"),
                Line::from(""),
                help_section("Filters & Grouping"),
                help_line("1", "Cycle region grouping (US/China/Europe/...)"),
                help_line("2", "Cycle type grouping (Startup/Big Tech/Research)"),
            ]);
            // Hidden when the active source has no reasoning metadata (key no-op).
            if super::benchmarks::BenchmarksApp::reasoning_filter_available(
                app.multi_store.file(app.benchmarks_app.active_source),
            ) {
                help_text.push(help_line(
                    "3",
                    "Cycle reasoning filter (All/Reasoning/Non-reasoning)",
                ));
            }
            help_text.extend(vec![
                help_line("4", "Cycle open-weights filter (All/Open/Closed)"),
                Line::from(""),
                help_section("Sort"),
                help_line("s", "Open sort picker"),
                help_line("S", "Toggle sort direction"),
                Line::from(""),
                help_section("Actions"),
                help_line("C", "Choose visible metric columns (browse mode)"),
                help_line("o", "Open source model page in browser"),
                help_line("i", "Toggle benchmark glossary"),
                help_line(
                    "a",
                    "Cycle comparator column (field avg → peers → rank → off)",
                ),
                Line::from(""),
                help_section("Compare"),
                help_line("Space", "Toggle model for comparison (max 8)"),
                help_line("c", "Clear all selections"),
                help_line("v", "Cycle view: H2H → Scatter → Radar"),
                help_line("d", "Show detail overlay (H2H view)"),
                help_line("x", "Cycle scatter X-axis"),
                help_line("y", "Cycle scatter Y-axis"),
                help_line("a", "Cycle radar preset"),
                help_line("j/k", "Scroll H2H table (when Compare focused)"),
                help_line("h/l", "Switch focus: List ↔ Compare"),
                help_line("t", "Toggle left panel: Models ↔ Creators"),
                Line::from(""),
            ]);
        }
        Tab::Status => {
            help_text.extend(vec![
                help_section("Actions"),
                help_line("o", "Open provider status page"),
                help_line("r", "Refresh provider status"),
                help_line("a", "Add/remove tracked providers"),
                Line::from(""),
                help_section("Status view"),
                help_line("Tab/h/l", "Switch list/details focus"),
                help_line("/", "Search providers"),
                Line::from(""),
            ]);
        }
    }

    // Common: Tabs and Other
    help_text.extend(vec![
        help_section("Tabs"),
        help_line("[", "Previous tab"),
        help_line("]", "Next tab"),
        Line::from(""),
        help_section("Other"),
        help_line("q", "Quit"),
        help_line("?", "Toggle this help"),
    ]);

    let title = match current_tab {
        Tab::Models => "Models Help - ? or Esc to close (j/k to scroll)",
        Tab::Agents => "Agents Help - ? or Esc to close (j/k to scroll)",
        Tab::Benchmarks => "Benchmarks Help - ? or Esc to close (j/k to scroll)",
        Tab::Status => "Status Help - ? or Esc to close (j/k to scroll)",
    };

    ScrollablePanel::new(title, help_text, scroll, true)
        .with_wrap(false)
        .render(f, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

    #[test]
    fn visual_line_height_empty() {
        let line = Line::from("");
        assert_eq!(visual_line_height(&line, 40), 1);
    }

    #[test]
    fn visual_line_height_fits() {
        let line = Line::from("short");
        assert_eq!(visual_line_height(&line, 40), 1);
    }

    #[test]
    fn visual_line_height_wraps() {
        // 10 chars in a 4-wide viewport: div_ceil(10, 4) = 3, +1 buffer = 4
        let line = Line::from("abcdefghij");
        assert_eq!(visual_line_height(&line, 4), 4);
    }

    #[test]
    fn visual_line_height_exact_fit() {
        // Exactly fits: no +1 buffer
        let line = Line::from("abcd");
        assert_eq!(visual_line_height(&line, 4), 1);
    }

    #[test]
    fn visual_line_height_zero_wrap() {
        let line = Line::from("hello");
        assert_eq!(visual_line_height(&line, 0), 1);
    }

    #[test]
    fn visual_line_total_sums() {
        let lines = vec![
            Line::from("short"),        // fits in 40 → 1
            Line::from(""),             // empty → 1
            Line::from("a".repeat(80)), // wraps in 40 → div_ceil(80,40)=2 +1 = 3
        ];
        assert_eq!(visual_line_total(&lines, 40), 5);
    }

    #[test]
    fn visual_line_heights_returns_per_line() {
        let lines = vec![Line::from("short"), Line::from("a".repeat(80))];
        let heights = visual_line_heights(&lines, 40);
        assert_eq!(heights, vec![1, 3]);
    }

    #[test]
    fn section_header_line_format() {
        let line = section_header_line("Pricing", 30);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("\u{2500}\u{2500} Pricing "));
        assert_eq!(text.chars().count(), 30);
        // Verify style
        let style = line.spans[0].style;
        assert_eq!(style.fg, Some(Color::DarkGray));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn section_header_line_short_width() {
        // Width shorter than prefix — no trailing dashes, no panic
        let line = section_header_line("Title", 5);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Title"));
    }

    #[test]
    fn section_header_line_with_suffix_format() {
        let line = section_header_line_with_suffix("Pricing", "$ per 1M tokens", 40);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("\u{2500}\u{2500} Pricing ($ per 1M tokens) "));
        assert_eq!(text.chars().count(), 40);
        // Title span is BOLD DarkGray; the suffix span is DarkGray without BOLD.
        let suffix_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("($ per 1M tokens)"))
            .expect("suffix span present");
        assert_eq!(suffix_span.style.fg, Some(Color::DarkGray));
        assert!(!suffix_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn section_header_line_with_suffix_empty_matches_plain() {
        let with = section_header_line_with_suffix("Indexes", "", 30);
        let plain = section_header_line("Indexes", 30);
        let wt: String = with.spans.iter().map(|s| s.content.as_ref()).collect();
        let pt: String = plain.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(wt, pt);
    }

    #[test]
    fn filter_toggle_spans_active_and_inactive() {
        let spans = filter_toggle_spans(&[("1", "reasoning", true), ("2", "tools", false)]);
        assert_eq!(spans.len(), 4);
        // Active key is Green
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        assert_eq!(spans[0].content.as_ref(), "[1]");
        assert_eq!(spans[1].content.as_ref(), " reasoning ");
        // Inactive key is DarkGray
        assert_eq!(spans[2].style.fg, Some(Color::DarkGray));
        assert_eq!(spans[2].content.as_ref(), "[2]");
        assert_eq!(spans[3].content.as_ref(), " tools ");
    }

    #[test]
    fn filter_toggle_spans_empty() {
        let spans = filter_toggle_spans(&[]);
        assert!(spans.is_empty());
    }
}
