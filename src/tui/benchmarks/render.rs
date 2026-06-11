use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use super::app::BenchmarksApp;
use super::compare::{draw_h2h_table_generic, draw_scatter};
use crate::benchmarks::multi::{
    format_metric_value, groups_in_order, metric_indices_in_group,
    short_label as metric_short_label, SortKey, SourceLoad,
};
use crate::benchmarks::schema::{MetricKind, ModelRow, ScoreCell, SourceFile};
use crate::formatting::{format_relative_time_from_str, format_tokens, truncate};
use crate::tui::app::App;
use crate::tui::ui::{
    caret, centered_rect, centered_rect_fixed, focus_border, section_header_line,
    section_header_line_with_suffix,
};
use crate::tui::widgets::scrollable_panel::ScrollablePanel;

/// Em-dash sentinel for missing values.
const EM: &str = "\u{2014}";

/// Color palette for selected models in comparison mode.
pub(crate) fn compare_colors(index: usize) -> Color {
    const PALETTE: [Color; 8] = [
        Color::Red,
        Color::Green,
        Color::Blue,
        Color::Yellow,
        Color::Magenta,
        Color::Cyan,
        Color::LightRed,
        Color::LightGreen,
    ];
    PALETTE[index % PALETTE.len()]
}

pub(in crate::tui) fn draw_benchmarks_main(f: &mut Frame, area: Rect, app: &mut App) {
    // Source bar (1 line) + existing content (remainder).
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    draw_source_bar(f, outer[0], app);
    let area = outer[1];

    let in_compare = app.selections.len() >= 2;

    if in_compare {
        // Compare mode: compact list (30%, min 35 chars) | comparison (remainder), full height
        let list_w = (area.width * 30 / 100).max(35);
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(list_w), Constraint::Min(0)])
            .split(area);

        if app.benchmarks_app.show_creators_in_compare {
            draw_benchmark_creators(f, h_chunks[0], app);
        } else {
            draw_benchmark_list_compact(f, h_chunks[0], app);
        }

        // Comparison panel: sub-tab bar + view
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(h_chunks[1]);

        draw_benchmark_subtab_bar(f, v_chunks[0], &app.benchmarks_app);

        match app.benchmarks_app.bottom_view {
            super::app::BottomView::H2H => {
                draw_h2h_table_generic(f, v_chunks[1], app);
            }
            super::app::BottomView::Scatter => {
                draw_scatter(f, v_chunks[1], app);
            }
            super::app::BottomView::Radar => {
                super::radar::draw_radar(f, v_chunks[1], app);
            }
            super::app::BottomView::Detail => {
                draw_benchmark_detail(f, v_chunks[1], app);
            }
        }
    } else {
        // Browse mode: creators (20%) | list (40%) | detail (40%)
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(40),
                Constraint::Percentage(40),
            ])
            .split(area);

        draw_benchmark_creators(f, h_chunks[0], app);
        draw_benchmark_list(f, h_chunks[1], app);
        draw_benchmark_detail(f, h_chunks[2], app);
    }

    // Detail overlay (drawn last, on top of everything)
    if app.benchmarks_app.show_detail_overlay && app.selections.len() >= 2 {
        draw_detail_overlay(f, area, app);
    }

    // Sort picker popup
    if app.benchmarks_app.show_sort_picker {
        draw_sort_picker(f, area, app);
    }

    // Glossary popup (drawn last so it sits above the sort picker too)
    if app.benchmarks_app.show_glossary {
        draw_glossary(f, area, app);
    }
}

/// Source bar: one bracketed label per compiled-in source (active = Cyan+BOLD,
/// loaded-inactive = DarkGray, loading = label + `◐` Yellow, failed = label +
/// `✗` Red). Right-aligned for the active source: `fetched {relative}` (DarkGray)
/// + ` self-reported` (Yellow) when the source is unverified.
fn draw_source_bar(f: &mut Frame, area: Rect, app: &App) {
    let bench_app = &app.benchmarks_app;
    let active = bench_app.active_source;

    // Left: bracketed source labels.
    let mut left_spans: Vec<Span> = vec![Span::raw(" ")];
    for (idx, state) in app.multi_store.sources.iter().enumerate() {
        let name = state.descriptor.name;
        match &state.load {
            SourceLoad::Loading => {
                left_spans.push(Span::styled(
                    format!("[{}] ", name),
                    Style::default().fg(Color::DarkGray),
                ));
                left_spans.push(Span::styled(
                    "\u{25D0} ",
                    Style::default().fg(Color::Yellow),
                ));
            }
            SourceLoad::Failed => {
                left_spans.push(Span::styled(
                    format!("[{}] ", name),
                    Style::default().fg(Color::DarkGray),
                ));
                left_spans.push(Span::styled("\u{2717} ", Style::default().fg(Color::Red)));
            }
            SourceLoad::Loaded(_) => {
                let style = if idx == active {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                left_spans.push(Span::styled(format!("[{}] ", name), style));
            }
        }
    }
    // Source-switch hint, mirroring the header's `[/] switch tabs` styling.
    left_spans.push(Span::styled(
        "{ } switch source",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(Line::from(left_spans)), area);

    // Right: freshness + self-reported for the active source.
    let mut right_spans: Vec<Span> = Vec::new();
    if let Some(state) = app.multi_store.sources.get(active) {
        if let SourceLoad::Loaded(file) = &state.load {
            right_spans.push(Span::styled(
                format!(
                    "fetched {}",
                    format_relative_time_from_str(&file.source.fetched_at)
                ),
                Style::default().fg(Color::DarkGray),
            ));
            if !file.source.verified {
                right_spans.push(Span::styled(
                    " self-reported",
                    Style::default().fg(Color::Yellow),
                ));
            }
            right_spans.push(Span::raw(" "));
        }
    }
    if !right_spans.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(right_spans)).alignment(ratatui::layout::Alignment::Right),
            area,
        );
    }
}

fn draw_benchmark_subtab_bar(f: &mut Frame, area: Rect, bench_app: &super::app::BenchmarksApp) {
    use super::app::BottomView;
    let views = [
        ("H2H", BottomView::H2H),
        ("Scatter", BottomView::Scatter),
        ("Radar", BottomView::Radar),
    ];
    let mut spans = Vec::new();
    for (label, view) in &views {
        let style = if bench_app.bottom_view == *view {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!(" [{}] ", label), style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_benchmark_creators(f: &mut Frame, area: Rect, app: &App) {
    use super::app::{
        BenchmarkFocus, CreatorGrouping, CreatorListItem, CreatorRegion, CreatorType,
    };

    let bench_app = &app.benchmarks_app;

    let is_focused = bench_app.focus == BenchmarkFocus::Creators;
    let border_style = focus_border(is_focused);

    let source_indicator = match bench_app.source_filter {
        super::app::SourceFilter::All => String::new(),
        filter => format!(" [{}]", filter.label()),
    };
    let reasoning_indicator = {
        let label = bench_app.reasoning_filter.label();
        if label.is_empty() {
            String::new()
        } else {
            format!(" [{}]", label)
        }
    };
    let creators_title = format!(" Creators{}{} ", source_indicator, reasoning_indicator);

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(creators_title);
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Grouping toggle indicators
    let rgn_active = bench_app.creator_grouping == CreatorGrouping::ByRegion;
    let rgn_color = if rgn_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let typ_active = bench_app.creator_grouping == CreatorGrouping::ByType;
    let typ_color = if typ_active {
        Color::Magenta
    } else {
        Color::DarkGray
    };

    let filter_line = Line::from(vec![
        Span::styled("[1]", Style::default().fg(rgn_color)),
        Span::raw(if rgn_active { "Region " } else { "Rgn " }),
        Span::styled("[2]", Style::default().fg(typ_color)),
        Span::raw("Type"),
    ]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner_area);

    f.render_widget(Paragraph::new(filter_line), chunks[0]);

    // Available width for creator items (inner area minus highlight symbol "  " or "> ")
    let item_width = inner_area.width.saturating_sub(2) as usize;

    let items: Vec<ListItem> = bench_app
        .creator_list_items
        .iter()
        .map(|item| match item {
            CreatorListItem::All => {
                let count = bench_app.filtered_creator_count();
                ListItem::new(Line::from(vec![
                    Span::styled("All", Style::default().fg(Color::Green)),
                    Span::raw(format!(" ({})", count)),
                ]))
            }
            CreatorListItem::GroupHeader(label) => {
                // Match models panel: full-width colored header with trailing ───
                let header_color = match bench_app.creator_grouping {
                    CreatorGrouping::ByRegion => {
                        CreatorRegion::from_label(label).map_or(Color::DarkGray, |r| r.color())
                    }
                    CreatorGrouping::ByType => {
                        CreatorType::from_label(label).map_or(Color::DarkGray, |t| t.color())
                    }
                    _ => Color::DarkGray,
                };
                let label_len = label.len() + 4; // "── " + label + " "
                let trailing = if item_width > label_len {
                    "\u{2500}".repeat(item_width - label_len)
                } else {
                    String::new()
                };
                let text = format!("\u{2500}\u{2500} {} {}", label, trailing);
                ListItem::new(text).style(
                    Style::default()
                        .fg(header_color)
                        .add_modifier(Modifier::BOLD),
                )
            }
            CreatorListItem::Creator(slug) => {
                let (display_name, count) = bench_app.creator_display(slug);
                // When grouped, show a colored tag for the creator's classification
                let tag = match bench_app.creator_grouping {
                    CreatorGrouping::ByRegion => {
                        let r = CreatorRegion::from_creator(slug);
                        Some((r.label(), r.color()))
                    }
                    CreatorGrouping::ByType => {
                        let t = CreatorType::from_creator(slug);
                        Some((t.label(), t.color()))
                    }
                    CreatorGrouping::None => None,
                };
                let count_str = format!("({})", count);
                let tag_len = tag.as_ref().map_or(0, |(l, _)| l.len() + 1);
                let overhead = count_str.len() + 1 + tag_len;
                let max_name = item_width.saturating_sub(overhead);
                let name = truncate(display_name, max_name);
                let mut spans = vec![
                    Span::raw(format!("{} ", name)),
                    Span::styled(count_str, Style::default().fg(Color::Gray)),
                ];
                if let Some((label, color)) = tag {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(label, Style::default().fg(color)));
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();

    let caret = caret(is_focused);
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(caret);

    let mut state = bench_app.creator_list_state;
    f.render_stateful_widget(list, chunks[1], &mut state);
}

/// Loading / failed / empty state lines for the active source, or `None` when a
/// loaded non-empty file is available. Rendered inside the list panel area.
fn source_state_lines(app: &App) -> Option<Vec<Line<'static>>> {
    let store = &app.multi_store;
    let active = app.benchmarks_app.active_source;

    if BenchmarksApp::active_is_failed(store, active) {
        let name = BenchmarksApp::active_descriptor(store, active)
            .map(|d| d.name)
            .unwrap_or("source");
        return Some(vec![Line::from(Span::styled(
            format!("\u{2717} Failed to fetch {name} data"),
            Style::default().fg(Color::Red),
        ))]);
    }

    match BenchmarksApp::active_file(store, active) {
        // Not yet loaded (Loading or absent) -> loading state.
        None => Some(vec![Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::Yellow),
        ))]),
        Some(file) if file.models.is_empty() => Some(vec![Line::from(Span::styled(
            "No models",
            Style::default().fg(Color::DarkGray),
        ))]),
        Some(_) => None,
    }
}

/// Build the value-column string for a model under the active sort key.
/// ReleaseDate -> date string, Metric -> formatted value, Name -> empty.
fn list_value_for(file: &SourceFile, model: &ModelRow, key: SortKey) -> String {
    match key {
        SortKey::Name => String::new(),
        SortKey::ReleaseDate => model.release_date.clone().unwrap_or_else(|| EM.to_string()),
        SortKey::Metric(mi) => {
            BenchmarksApp::formatted_score(file, model, mi).unwrap_or_else(|| EM.to_string())
        }
    }
}

/// Header label for the active sort value column.
///
/// Uses the metric's curated `short_label` via `multi::short_label` (falls back
/// to `truncate(label, 11)` when no short label is set). The sort picker, panel
/// title sort indicator, glossary, and detail panel keep using the full label.
fn list_value_header(file: Option<&SourceFile>, key: SortKey) -> String {
    match key {
        SortKey::Name => String::new(),
        SortKey::ReleaseDate => "Released".to_string(),
        SortKey::Metric(mi) => file
            .and_then(|f| f.metrics.get(mi))
            .map(metric_short_label)
            .unwrap_or_else(|| EM.to_string()),
    }
}

/// Compact list for compare mode: selection marker + name only, full height.
fn draw_benchmark_list_compact(f: &mut Frame, area: Rect, app: &App) {
    use super::app::BenchmarkFocus;

    let is_focused = app.benchmarks_app.focus == BenchmarkFocus::List;
    let border_style = focus_border(is_focused);

    let bench_app = &app.benchmarks_app;
    let sort_dir = if bench_app.sort_descending {
        "\u{2193}"
    } else {
        "\u{2191}"
    };
    let file = app.multi_store.file(bench_app.active_source);
    let sort_indicator = format!(
        " {}{}",
        sort_dir,
        BenchmarksApp::sort_label(file, bench_app.sort_key)
    );

    let source_indicator = match bench_app.source_filter {
        super::app::SourceFilter::All => String::new(),
        filter => format!(" [{}]", filter.label()),
    };

    let reasoning_indicator = {
        let label = bench_app.reasoning_filter.label();
        if label.is_empty() {
            String::new()
        } else {
            format!(" [{}]", label)
        }
    };

    let loading_suffix =
        if BenchmarksApp::active_is_loading(&app.multi_store, bench_app.active_source) {
            " loading..."
        } else {
            ""
        };

    let title = if bench_app.search_query.is_empty() {
        format!(
            " Models ({}){}{}{}{} ",
            bench_app.filtered_indices.len(),
            source_indicator,
            reasoning_indicator,
            sort_indicator,
            loading_suffix
        )
    } else {
        format!(
            " Models ({}) [/{}]{}{}{}{} ",
            bench_app.filtered_indices.len(),
            bench_app.search_query,
            source_indicator,
            reasoning_indicator,
            sort_indicator,
            loading_suffix
        )
    };

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Loading / failed / empty state.
    if let Some(lines) = source_state_lines(app) {
        f.render_widget(Paragraph::new(lines), inner_area);
        return;
    }
    let Some(file) = app.multi_store.file(app.benchmarks_app.active_source) else {
        return;
    };

    let bench_app = &app.benchmarks_app;
    let caret = caret(is_focused);
    let openness = bench_app.creator_openness();

    // Extra columns: marker(2) + caret(2) + reasoning(3) + source(2) + optional region/type
    let show_region = bench_app.creator_grouping == super::app::CreatorGrouping::ByRegion;
    let show_type = bench_app.creator_grouping == super::app::CreatorGrouping::ByType;
    let extra_w: u16 = 2 + 2 + 3 + 2 + if show_region || show_type { 4 } else { 0 };
    let name_width = inner_area.width.saturating_sub(extra_w) as usize;

    let items: Vec<ListItem> = bench_app
        .filtered_indices
        .iter()
        .enumerate()
        .map(|(display_idx, &model_idx)| {
            let model = &file.models[model_idx];
            let is_selected = display_idx == bench_app.selected;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let prefix = if is_selected { caret } else { "  " };
            let mut row_spans: Vec<Span> = Vec::new();

            // Selection marker
            if let Some(sel_pos) = app.selections.iter().position(|&i| i == model_idx) {
                row_spans.push(Span::styled(
                    "\u{25CF} ",
                    Style::default().fg(compare_colors(sel_pos)),
                ));
            } else {
                row_spans.push(Span::raw("  "));
            }

            row_spans.push(Span::styled(prefix, style));

            // Reasoning status indicator
            row_spans.push(reasoning_span(model));

            // Source indicator (Open/Closed) via creator-openness map
            row_spans.push(openness_span(model, openness));

            // Region/Type indicator when grouping is active
            if show_region {
                let region = super::app::CreatorRegion::from_creator(&model.creator);
                row_spans.push(Span::styled(
                    format!("{:<4}", region.short_label()),
                    Style::default().fg(region.color()),
                ));
            } else if show_type {
                let ct = super::app::CreatorType::from_creator(&model.creator);
                row_spans.push(Span::styled(
                    format!("{:<4}", ct.short_label()),
                    Style::default().fg(ct.color()),
                ));
            }

            row_spans.push(Span::styled(
                truncate(&model.display_name, name_width),
                style,
            ));
            ListItem::new(Line::from(row_spans))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("");

    let mut state = bench_app.list_state;
    state.select(Some(bench_app.selected));
    f.render_stateful_widget(list, inner_area, &mut state);
}

/// Reasoning status indicator span (R / NR / AR / blank).
fn reasoning_span(model: &ModelRow) -> Span<'static> {
    use crate::benchmarks::ReasoningStatus;
    let (label, color) = match model.reasoning_status {
        ReasoningStatus::Reasoning => ("R  ", Color::Cyan),
        ReasoningStatus::NonReasoning => ("NR ", Color::DarkGray),
        ReasoningStatus::Adaptive => ("AR ", Color::Yellow),
        ReasoningStatus::None => ("   ", Color::Reset),
    };
    Span::styled(label, Style::default().fg(color))
}

/// Open/closed source indicator span. Openness is resolved per-model first
/// (`ModelRow.open_weights`), falling back to the creator-openness map.
fn openness_span(
    model: &ModelRow,
    openness: &std::collections::HashMap<String, bool>,
) -> Span<'static> {
    let open = model
        .open_weights
        .or_else(|| openness.get(&model.creator).copied());
    let (label, color) = match open {
        Some(true) => ("O ", Color::Green),
        Some(false) => ("C ", Color::Red),
        None => ("  ", Color::Reset),
    };
    Span::styled(label, Style::default().fg(color))
}

fn draw_benchmark_list(f: &mut Frame, area: Rect, app: &App) {
    use super::app::BenchmarkFocus;

    let is_focused = app.benchmarks_app.focus == BenchmarkFocus::List;
    let border_style = focus_border(is_focused);

    let bench_app = &app.benchmarks_app;
    let file_opt = app.multi_store.file(bench_app.active_source);

    let sort_dir = if bench_app.sort_descending {
        "\u{2193}"
    } else {
        "\u{2191}"
    };
    let sort_indicator = format!(
        " {}{}",
        sort_dir,
        BenchmarksApp::sort_label(file_opt, bench_app.sort_key)
    );

    let source_indicator = match bench_app.source_filter {
        super::app::SourceFilter::All => String::new(),
        filter => format!(" [{}]", filter.label()),
    };

    let reasoning_indicator = {
        let label = bench_app.reasoning_filter.label();
        if label.is_empty() {
            String::new()
        } else {
            format!(" [{}]", label)
        }
    };

    let creator_label = bench_app.selected_creator_name().unwrap_or("Benchmarks");
    let loading_suffix =
        if BenchmarksApp::active_is_loading(&app.multi_store, bench_app.active_source) {
            " loading..."
        } else {
            ""
        };

    let title = if bench_app.search_query.is_empty() {
        format!(
            " {} ({}){}{}{}{} ",
            creator_label,
            bench_app.filtered_indices.len(),
            source_indicator,
            reasoning_indicator,
            sort_indicator,
            loading_suffix
        )
    } else {
        format!(
            " {} ({}) [/{}]{}{}{}{} ",
            creator_label,
            bench_app.filtered_indices.len(),
            bench_app.search_query,
            source_indicator,
            reasoning_indicator,
            sort_indicator,
            loading_suffix
        )
    };

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Loading / failed / empty state.
    if let Some(lines) = source_state_lines(app) {
        f.render_widget(Paragraph::new(lines), inner_area);
        return;
    }
    let Some(file) = app.multi_store.file(app.benchmarks_app.active_source) else {
        return;
    };

    let bench_app = &app.benchmarks_app;
    let openness = bench_app.creator_openness();
    let sort_key = bench_app.sort_key;

    // Column widths.
    let caret_w: u16 = 2;
    let reasoning_col_w: u16 = 3;
    let source_col_w: u16 = 2;
    let show_region = bench_app.creator_grouping == super::app::CreatorGrouping::ByRegion;
    let show_type = bench_app.creator_grouping == super::app::CreatorGrouping::ByType;
    let grouping_col_w: u16 = if show_region || show_type { 4 } else { 0 };
    let selection_w: u16 = if !app.selections.is_empty() { 2 } else { 0 };
    // Value column: 11 wide right-aligned (dates / formatted metric values), with
    // a leading space. Name takes the remaining width. Empty for Name sort.
    let value_w: u16 = if matches!(sort_key, SortKey::Name) {
        0
    } else {
        12 // 11-wide value + 1 leading gap
    };
    let name_width = (inner_area.width.saturating_sub(
        value_w + caret_w + selection_w + reasoning_col_w + source_col_w + grouping_col_w,
    ) as usize)
        .max(10);

    let caret = caret(is_focused);

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let active_header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let has_selections = !app.selections.is_empty();
    let mut header_spans: Vec<Span> = Vec::new();
    if has_selections {
        header_spans.push(Span::raw("  ")); // align with selection marker column
    }
    header_spans.push(Span::raw("  ")); // caret
    header_spans.push(Span::styled("   ", header_style)); // reasoning indicator
    header_spans.push(Span::styled("  ", header_style)); // source indicator
    if show_region {
        header_spans.push(Span::styled("Rgn ", header_style));
    } else if show_type {
        header_spans.push(Span::styled("Typ ", header_style));
    }
    header_spans.push(Span::styled(
        format!("{:<width$}", "Name", width = name_width),
        header_style,
    ));
    if value_w > 0 {
        let label = list_value_header(Some(file), sort_key);
        header_spans.push(Span::styled(format!(" {:>11}", label), active_header_style));
    }
    let header = ListItem::new(Line::from(header_spans));

    let mut items: Vec<ListItem> = vec![header];

    for (display_idx, &model_idx) in bench_app.filtered_indices.iter().enumerate() {
        let model = &file.models[model_idx];
        let is_selected = display_idx == bench_app.selected;

        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let prefix = if is_selected { caret } else { "  " };
        let mut row_spans: Vec<Span> = Vec::new();

        // Selection marker
        if let Some(sel_pos) = app.selections.iter().position(|&i| i == model_idx) {
            row_spans.push(Span::styled(
                "\u{25CF} ",
                Style::default().fg(compare_colors(sel_pos)),
            ));
        } else if has_selections {
            row_spans.push(Span::raw("  "));
        }

        row_spans.push(Span::styled(prefix, style));

        // Reasoning status indicator
        row_spans.push(reasoning_span(model));

        // Source indicator (Open/Closed)
        row_spans.push(openness_span(model, openness));

        // Region/Type indicator when grouping is active
        if show_region {
            let region = super::app::CreatorRegion::from_creator(&model.creator);
            row_spans.push(Span::styled(
                format!("{:<4}", region.short_label()),
                Style::default().fg(region.color()),
            ));
        } else if show_type {
            let ct = super::app::CreatorType::from_creator(&model.creator);
            row_spans.push(Span::styled(
                format!("{:<4}", ct.short_label()),
                Style::default().fg(ct.color()),
            ));
        }

        // Name
        row_spans.push(Span::styled(
            format!(
                "{:<width$}",
                truncate(&model.display_name, name_width.saturating_sub(1)),
                width = name_width
            ),
            style,
        ));

        // Active sort value column
        if value_w > 0 {
            let value = list_value_for(file, model, sort_key);
            row_spans.push(Span::styled(format!(" {:>11}", value), style));
        }

        items.push(ListItem::new(Line::from(row_spans)));
    }

    let list = List::new(items);
    let mut state = bench_app.list_state;
    // Offset by 1 for the header row
    state.select(Some(bench_app.selected + 1));
    f.render_stateful_widget(list, inner_area, &mut state);
}

fn draw_benchmark_detail(f: &mut Frame, area: Rect, app: &App) {
    use super::app::BenchmarkFocus;
    let bench_app = &app.benchmarks_app;
    let focused = bench_app.focus == BenchmarkFocus::Details;
    // Title reflects the active comparator mode: ` Details `, ` Details · vs
    // field avg `, etc. Space-padded to match the surrounding chrome.
    let title = format!(" Details{} ", bench_app.comparator.title_suffix());

    // Loading / failed / empty state shown in the detail panel too.
    if let Some(lines) = source_state_lines(app) {
        ScrollablePanel::new(title, lines, &bench_app.detail_scroll, focused).render(f, area);
        return;
    }
    let Some(file) = app.multi_store.file(bench_app.active_source) else {
        return;
    };

    let model = match bench_app.current_model(file) {
        Some(m) => m,
        None => {
            let lines = vec![Line::from(Span::styled(
                "No model selected",
                Style::default().fg(Color::DarkGray),
            ))];
            ScrollablePanel::new(title, lines, &bench_app.detail_scroll, focused).render(f, area);
            return;
        }
    };

    let inner_w = area.width.saturating_sub(2);
    let lines = build_benchmark_detail_lines(inner_w, file, model, bench_app.comparator);
    ScrollablePanel::new(title, lines, &bench_app.detail_scroll, focused).render(f, area);
}

/// Short human-readable scale blurb for a metric kind, used as the parenthetical
/// suffix on uniform-kind section headers and in the glossary meta line.
fn kind_blurb(kind: MetricKind) -> &'static str {
    match kind {
        MetricKind::Percentage => "% score",
        MetricKind::Index => "index",
        MetricKind::Elo => "Elo rating",
        MetricKind::TokensPerSec => "tokens/sec",
        MetricKind::Seconds => "seconds",
        MetricKind::UsdPerMTok => "$ per 1M tokens",
    }
}

/// The shared scale blurb for `group`, when every metric in it has the same
/// `MetricKind`. Mixed-kind groups (e.g. AA "Performance" mixing tokens/sec and
/// seconds) return `None` so no suffix is appended.
fn group_kind_blurb(file: &SourceFile, group: &str) -> Option<&'static str> {
    let mut kinds = file
        .metrics
        .iter()
        .filter(|m| m.group == group)
        .map(|m| m.kind);
    let first = kinds.next()?;
    if kinds.all(|k| k == first) {
        Some(kind_blurb(first))
    } else {
        None
    }
}

/// Direction phrase when every metric in the group agrees on it.
fn group_direction_blurb(file: &SourceFile, group: &str) -> Option<&'static str> {
    let mut dirs = file
        .metrics
        .iter()
        .filter(|m| m.group == group)
        .map(|m| m.higher_is_better);
    let first = dirs.next()?;
    if dirs.all(|d| d == first) {
        Some(if first {
            "higher is better"
        } else {
            "lower is better"
        })
    } else {
        None
    }
}

/// Combined section-header suffix: scale blurb and/or direction, joined with
/// " · " when both are present. Per-metric direction markers were removed on
/// user feedback (too distracting) — the header carries the explanation.
fn group_header_suffix(file: &SourceFile, group: &str) -> Option<String> {
    match (
        group_kind_blurb(file, group),
        group_direction_blurb(file, group),
    ) {
        (Some(kind), Some(dir)) => Some(format!("{kind} \u{00B7} {dir}")),
        (Some(kind), None) => Some(kind.to_string()),
        (None, Some(dir)) => Some(dir.to_string()),
        (None, None) => None,
    }
}

/// Direction phrase for a single metric (glossary meta line).
fn direction_blurb(higher_is_better: bool) -> &'static str {
    if higher_is_better {
        "higher is better"
    } else {
        "lower is better"
    }
}

/// Registry-driven model detail: identity block + one section per metric group
/// (`groups_in_order`), values formatted via `format_metric_value`, with `±ci`
/// for Elo cells and a dim `· {N} votes` suffix where the cell carries a vote
/// count. Source attribution lives in the source bar's freshness text, not here.
///
/// Returns owned `Line<'static>` so the overlay (compare.rs) can render it.
pub(super) fn build_benchmark_detail_lines(
    inner_width: u16,
    file: &SourceFile,
    model: &ModelRow,
    comparator: super::app::ComparatorMode,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let width = inner_width as usize;
    let cw = ColumnWidths::from_width(inner_width);

    // --- Identity block ---
    lines.push(Line::from(Span::styled(
        model.display_name.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        model.id.clone(),
        Style::default().fg(Color::DarkGray),
    )));

    // Creator + release date row
    let creator_display = if !model.creator_name.is_empty() {
        model.creator_name.clone()
    } else {
        model.creator.clone()
    };
    let date_str = model.release_date.clone().unwrap_or_else(|| EM.to_string());
    push_meta_row(
        &mut lines,
        &cw,
        ("Creator", creator_display.as_str(), Color::White),
        ("Released", date_str.as_str(), Color::White),
    );

    // Reasoning / Effort / Variant (each only when present, except reasoning
    // which always shows the status word or em-dash).
    let (reasoning_label, reasoning_color) = {
        use crate::benchmarks::ReasoningStatus;
        match model.reasoning_status {
            ReasoningStatus::Reasoning => ("Reasoning", Color::Cyan),
            ReasoningStatus::NonReasoning => ("Non-reasoning", Color::DarkGray),
            ReasoningStatus::Adaptive => ("Adaptive", Color::Yellow),
            ReasoningStatus::None => (EM, Color::DarkGray),
        }
    };
    let effort_str = model.effort_level.clone();
    push_meta_row(
        &mut lines,
        &cw,
        ("Reasoning", reasoning_label, reasoning_color),
        (
            "Effort",
            effort_str.as_deref().unwrap_or(EM),
            if effort_str.is_some() {
                Color::White
            } else {
                Color::DarkGray
            },
        ),
    );

    // Open weights + context window (only meaningful values shown plainly).
    // Openness resolves per-model first, then falls back to creator-level
    // openness derived from sibling models — same semantics as openness_span,
    // so the detail panel never disagrees with the list/compare indicators.
    let resolved_open = model.open_weights.or_else(|| {
        let mut known_closed = false;
        for sibling in file.models.iter().filter(|m| m.creator == model.creator) {
            match sibling.open_weights {
                Some(true) => return Some(true),
                Some(false) => known_closed = true,
                None => {}
            }
        }
        known_closed.then_some(false)
    });
    let (open_label, open_color) = match resolved_open {
        Some(true) => ("Open", Color::Green),
        Some(false) => ("Closed", Color::Red),
        None => (EM, Color::DarkGray),
    };
    let ctx_str = model
        .context_window
        .map(format_tokens)
        .unwrap_or_else(|| EM.to_string());
    push_meta_row(
        &mut lines,
        &cw,
        ("Weights", open_label, open_color),
        (
            "Context",
            ctx_str.as_str(),
            if model.context_window.is_some() {
                Color::White
            } else {
                Color::DarkGray
            },
        ),
    );

    // Region / Type — heuristic creator classification (same buckets as the
    // sidebar grouping). Guarded on a known creator so unmatched rows (empty
    // creator) show an honest em-dash instead of a confident "Other"/"Startup".
    {
        use super::app::{CreatorRegion, CreatorType};
        let creator_known = !model.creator.is_empty();
        let region = CreatorRegion::from_creator(&model.creator);
        let ctype = CreatorType::from_creator(&model.creator);
        let (region_label, region_color) = if creator_known {
            (region.label(), region.color())
        } else {
            (EM, Color::DarkGray)
        };
        let (type_label, type_color) = if creator_known {
            (ctype.label(), ctype.color())
        } else {
            (EM, Color::DarkGray)
        };
        push_meta_row(
            &mut lines,
            &cw,
            ("Region", region_label, region_color),
            ("Type", type_label, type_color),
        );
    }

    // Tools / Output — backfilled from a models.dev match (em-dash where the
    // source model didn't match a models.dev entry).
    let (tools_label, tools_color) = match model.supports_tools {
        Some(true) => ("Yes", Color::Green),
        Some(false) => ("No", Color::DarkGray),
        None => (EM, Color::DarkGray),
    };
    let out_str = model
        .max_output
        .map(format_tokens)
        .unwrap_or_else(|| EM.to_string());
    push_meta_row(
        &mut lines,
        &cw,
        ("Tools", tools_label, tools_color),
        (
            "Output",
            out_str.as_str(),
            if model.max_output.is_some() {
                Color::White
            } else {
                Color::DarkGray
            },
        ),
    );

    if let Some(variant) = &model.variant_tag {
        push_meta_row(
            &mut lines,
            &cw,
            ("Variant", variant.as_str(), Color::White),
            ("", "", Color::Reset),
        );
    }

    // --- One section per metric group ---
    // Label column sized to the source's longest metric label (+2-space gap)
    // so values never collide with long labels like "Epoch Capabilities Index";
    // capped so a pathological label can't push values off-panel.
    let label_cap = width.saturating_sub(cw.indent as usize + 12).max(8);
    let metric_label_w = file
        .metrics
        .iter()
        .map(|m| unicode_width::UnicodeWidthStr::width(m.label.as_str()))
        .max()
        .unwrap_or(8)
        .min(label_cap)
        // +4 clear gutter before the score column (user feedback: values sat
        // flush against the longest label).
        + 4;
    // Comparator column position: the widest value cell (value + ±ci + votes)
    // across this model's metric rows, so the comparator forms a true column
    // instead of trailing each row at a different x.
    let metric_row_layout = MetricRowLayout {
        indent: cw.indent,
        label_w: metric_label_w,
        value_w: file
            .metrics
            .iter()
            .map(|m| value_cell_width(m.kind, model.scores.get(&m.id)))
            .max()
            .unwrap_or(1),
    };
    for group in groups_in_order(file) {
        lines.push(Line::from(""));
        // Headers carry the scale and/or direction explanation when the
        // group is uniform, e.g. "── Pricing ($ per 1M tokens · lower is
        // better) ──". Fully mixed groups get the plain header.
        match group_header_suffix(file, group) {
            Some(suffix) => lines.push(section_header_line_with_suffix(group, &suffix, width)),
            None => lines.push(section_header_line(group, width)),
        }
        for mi in metric_indices_in_group(file, group) {
            let metric = &file.metrics[mi];
            let cell = model.scores.get(&metric.id);
            let comparator_cell = comparator_cell_text(comparator, file, mi, model);
            push_metric_row(
                &mut lines,
                &metric_row_layout,
                &metric.label,
                metric.kind,
                cell,
                comparator_cell,
            );
        }
    }

    lines
}

fn draw_detail_overlay(f: &mut Frame, area: Rect, app: &App) {
    // Centered rect: 60% width, 75% height
    let overlay_area = centered_rect(60, 75, area);

    // Clear background
    f.render_widget(Clear, overlay_area);

    let bench_app = &app.benchmarks_app;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Model Detail (Esc to close) ");

    let Some(file) = app.multi_store.file(bench_app.active_source) else {
        let msg = Paragraph::new("No model selected").block(block);
        f.render_widget(msg, overlay_area);
        return;
    };

    let model = match bench_app.current_model(file) {
        Some(m) => m,
        None => {
            let msg = Paragraph::new("No model selected").block(block);
            f.render_widget(msg, overlay_area);
            return;
        }
    };

    let inner = block.inner(overlay_area);
    f.render_widget(block, overlay_area);
    let lines = build_benchmark_detail_lines(inner.width, file, model, bench_app.comparator);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

struct ColumnWidths {
    indent: u16,
    label: u16,
    value: u16,
    label2: u16,
}

impl ColumnWidths {
    fn from_width(width: u16) -> Self {
        let indent: u16 = 2;
        let usable = width.saturating_sub(indent);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(28),
                Constraint::Percentage(22),
                Constraint::Percentage(28),
                Constraint::Percentage(22),
            ])
            .split(Rect::new(0, 0, usable, 1));
        Self {
            indent,
            label: chunks[0].width.max(8),
            value: chunks[1].width.max(6),
            label2: chunks[2].width.max(8),
        }
    }
}

fn style_for(c: Color) -> Style {
    if c == Color::Reset {
        Style::default()
    } else {
        Style::default().fg(c)
    }
}

/// Push a 2-column label/value metadata row.
fn push_meta_row(
    lines: &mut Vec<Line<'static>>,
    cw: &ColumnWidths,
    left: (&str, &str, Color),
    right: (&str, &str, Color),
) {
    let mut spans = vec![
        Span::styled(
            format!(
                "{:indent$}{:<w$}",
                "",
                left.0,
                indent = cw.indent as usize,
                w = cw.label as usize
            ),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(
            format!("{:<w$}", left.1, w = cw.value as usize),
            style_for(left.2),
        ),
    ];

    if !right.0.is_empty() {
        spans.push(Span::styled(
            format!("{:<w$}", right.0, w = cw.label2 as usize),
            Style::default().fg(Color::Gray),
        ));
        spans.push(Span::styled(right.1.to_string(), style_for(right.2)));
    }

    lines.push(Line::from(spans));
}

/// Build the comparator-cell text for `metric_idx` under `mode`, or `None` when
/// the comparator is `Off` or the value is undefined for this model/metric.
///
/// All computations run over the source's full model list (`multi.rs`):
/// - `FieldAvg` -> `avg {value}` (always defined when the metric has any value)
/// - `PeerAvg`  -> `peers({n}) {value}` (undefined when the model is dateless or
///   the ±6mo peer set is empty -> em-dash cell)
/// - `Rank`     -> `#{rank}/{n}` (undefined when the model lacks the value ->
///   em-dash cell; field/peer averages still render without it)
fn comparator_cell_text(
    mode: super::app::ComparatorMode,
    file: &SourceFile,
    metric_idx: usize,
    model: &ModelRow,
) -> Option<String> {
    use super::app::ComparatorMode;
    use crate::benchmarks::multi::{field_avg, peer_avg, rank};

    let kind = file.metrics.get(metric_idx)?.kind;
    match mode {
        ComparatorMode::Off => None,
        // Field/peer averages render even when the selected model lacks the
        // metric value — the context is still useful.
        ComparatorMode::FieldAvg => match field_avg(file, metric_idx) {
            Some(mean) => Some(format!("avg {}", format_metric_value(kind, mean))),
            None => Some(EM.to_string()),
        },
        ComparatorMode::PeerAvg => match peer_avg(file, metric_idx, model) {
            Some((mean, n)) => Some(format!("peers({n}) {}", format_metric_value(kind, mean))),
            None => Some(EM.to_string()),
        },
        // Rank is undefined when the model has no value for the metric.
        ComparatorMode::Rank => rank(file, metric_idx, model).map(|(r, n)| format!("#{r}/{n}")),
    }
}

/// The rendered text parts of a metric row's value cell: the value (+ ` ±{ci}`
/// for Elo) and the dim ` · {N} votes` suffix. Shared by rendering and by the
/// comparator-column width measurement so the two can never drift apart.
fn value_cell_parts(kind: MetricKind, cell: Option<&ScoreCell>) -> (String, Option<String>) {
    match cell {
        Some(cell) => {
            let mut value = format_metric_value(kind, cell.value);
            if kind == MetricKind::Elo {
                if let Some(ci) = cell.ci {
                    value.push_str(&format!(" \u{00B1}{ci:.0}"));
                }
            }
            let votes = cell
                .votes
                .map(|v| format!(" \u{00B7} {} votes", format_tokens(v)));
            (value, votes)
        }
        None => (EM.to_string(), None),
    }
}

/// Display width of a metric row's full value cell (value + suffixes).
fn value_cell_width(kind: MetricKind, cell: Option<&ScoreCell>) -> usize {
    use unicode_width::UnicodeWidthStr;
    let (value, votes) = value_cell_parts(kind, cell);
    value.width() + votes.as_deref().map_or(0, |v| v.width())
}

/// Column layout for a metric row: indent, label-gutter width, and the value
/// column width (widest value cell — pads the comparator into a true column).
struct MetricRowLayout {
    indent: u16,
    label_w: usize,
    value_w: usize,
}

/// Push a single metric row: label (Gray) + value (White, em-dash DarkGray when
/// missing). Elo cells with a confidence interval append ` ±{ci:.0}`; cells with
/// a vote count append a dim `· {N} votes`. When `comparator_cell` is `Some`,
/// the value cell is padded out to `layout.value_w` so the dim comparator text
/// forms a true aligned column across rows (not a trailing annotation).
fn push_metric_row(
    lines: &mut Vec<Line<'static>>,
    layout: &MetricRowLayout,
    label: &str,
    kind: MetricKind,
    cell: Option<&ScoreCell>,
    comparator_cell: Option<String>,
) {
    use unicode_width::UnicodeWidthStr;

    let shown = truncate(label, layout.label_w.saturating_sub(2).max(6));

    let mut spans = vec![Span::styled(
        format!(
            "{:indent$}{:<w$}",
            "",
            shown,
            indent = layout.indent as usize,
            w = layout.label_w
        ),
        Style::default().fg(Color::Gray),
    )];

    let (value, votes) = value_cell_parts(kind, cell);
    let mut used = value.width();
    let value_color = if cell.is_some() {
        Color::White
    } else {
        Color::DarkGray
    };
    spans.push(Span::styled(value, Style::default().fg(value_color)));
    // Sample size (Arena vote count) as a dim confidence signal.
    if let Some(votes) = votes {
        used += votes.width();
        spans.push(Span::styled(votes, Style::default().fg(Color::DarkGray)));
    }

    // Comparator column (field avg / peers / rank) in dim gray, padded to a
    // consistent x-position (`value_w` = widest value cell + 2-space gap).
    if let Some(text) = comparator_cell {
        let pad = layout.value_w.saturating_sub(used) + 2;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(text, Style::default().fg(Color::DarkGray)));
    }

    lines.push(Line::from(spans));
}

fn draw_sort_picker(f: &mut Frame, area: Rect, app: &App) {
    let bench_app = &app.benchmarks_app;
    let Some(file) = app.multi_store.file(bench_app.active_source) else {
        return;
    };
    let options = BenchmarksApp::sort_options(file);
    let selected = bench_app.sort_picker_selected;

    // Fixed-width popup: 30 wide; height clamped to fit all options + border.
    let height = (options.len() as u16 + 2).min(area.height.max(3));
    let width = 30u16.min(area.width);
    let popup_area = centered_rect_fixed(width, height, area);

    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = options
        .iter()
        .map(|opt| {
            let marker = if opt.key == bench_app.sort_key {
                let arrow = if bench_app.sort_descending {
                    "\u{25bc}"
                } else {
                    "\u{25b2}"
                };
                format!(" {arrow}")
            } else {
                String::new()
            };
            ListItem::new(Line::from(format!(" {}{}", opt.label, marker)))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Sort By "),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    f.render_stateful_widget(list, popup_area, &mut list_state);
}

/// Build the glossary content for the active source: every metric in display
/// order (`groups_in_order` → `metric_indices_in_group`), grouped under the same
/// dash-padded section headers used in the detail panel.
///
/// Per metric:
/// 1. label (Gray) + dim direction arrow (DarkGray)
/// 2. meta line (DarkGray): kind blurb, plus `updated {last_updated}` when set
/// 3. description (White), or an em-dash line when `description` is `None`
///
/// A blank line separates entries. `width` is the popup inner width; the
/// `ScrollablePanel` wraps long descriptions, so they need not be pre-wrapped.
pub(super) fn build_glossary_lines(file: &SourceFile, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let w = width as usize;
    let mut first_group = true;

    for group in groups_in_order(file) {
        if !first_group {
            lines.push(Line::from(""));
        }
        first_group = false;
        match group_kind_blurb(file, group) {
            Some(blurb) => lines.push(section_header_line_with_suffix(group, blurb, w)),
            None => lines.push(section_header_line(group, w)),
        }
        lines.push(Line::from(""));

        for mi in metric_indices_in_group(file, group) {
            let metric = &file.metrics[mi];

            // 1. Label.
            lines.push(Line::from(Span::styled(
                metric.label.clone(),
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            )));

            // 2. Meta line: kind blurb · direction (+ updated date when present).
            let mut meta = format!(
                "{} \u{00B7} {}",
                kind_blurb(metric.kind),
                direction_blurb(metric.higher_is_better)
            );
            if let Some(updated) = &metric.last_updated {
                // Sources disagree on `last_updated` shape: epoch/arena emit a
                // plain `YYYY-MM-DD`, llmstats an RFC3339 timestamp. Show only
                // the date portion so the meta line stays uniform and tidy.
                let date = updated.split(['T', ' ']).next().unwrap_or(updated);
                meta.push_str(&format!("  updated {date}"));
            }
            lines.push(Line::from(Span::styled(
                meta,
                Style::default().fg(Color::DarkGray),
            )));

            // 3. Description (White) or em-dash when absent.
            match &metric.description {
                Some(desc) if !desc.is_empty() => {
                    lines.push(Line::from(Span::styled(
                        desc.clone(),
                        Style::default().fg(Color::White),
                    )));
                }
                _ => {
                    lines.push(Line::from(Span::styled(
                        EM.to_string(),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }

            // Blank line between entries.
            lines.push(Line::from(""));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No metrics for this source",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

/// Scrollable glossary popup over the active source's metrics. Centered 60% ×
/// 70%, Cyan border, Clear background. Falls back to a loading/empty message
/// when the active source has no loaded file.
fn draw_glossary(f: &mut Frame, area: Rect, app: &App) {
    let bench_app = &app.benchmarks_app;
    let popup_area = centered_rect(60, 70, area);
    f.render_widget(Clear, popup_area);

    let title = " Benchmark Glossary - i or Esc to close (Up/Down to scroll) ";

    let Some(file) = app.multi_store.file(bench_app.active_source) else {
        let lines = vec![Line::from(Span::styled(
            "No source data loaded",
            Style::default().fg(Color::DarkGray),
        ))];
        ScrollablePanel::new(title, lines, &bench_app.glossary_scroll, true).render(f, popup_area);
        return;
    };

    // Inner width = popup width minus the 2 border columns.
    let inner_w = popup_area.width.saturating_sub(2);
    let lines = build_glossary_lines(file, inner_w);
    ScrollablePanel::new(title, lines, &bench_app.glossary_scroll, true).render(f, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::schema::{MetricDef, ReasoningStatus, ScoreCell, SourceMeta};
    use std::collections::BTreeMap;

    fn meta(verified: bool) -> SourceMeta {
        SourceMeta {
            id: "test".into(),
            name: "Test Source".into(),
            url: "https://example.com".into(),
            fetched_at: "2026-06-10T20:37:40.687663442+00:00".into(),
            verified,
        }
    }

    fn metric(id: &str, label: &str, kind: MetricKind, group: &str) -> MetricDef {
        MetricDef {
            id: id.into(),
            label: label.into(),
            kind,
            group: group.into(),
            higher_is_better: true,
            last_updated: None,
            description: None,
            short_label: None,
        }
    }

    fn cell(value: f64, ci: Option<f64>, date: Option<&str>) -> ScoreCell {
        ScoreCell {
            value,
            ci,
            date: date.map(str::to_string),
            votes: None,
        }
    }

    fn model_with(scores: Vec<(&str, ScoreCell)>) -> ModelRow {
        let mut score_map = BTreeMap::new();
        for (id, c) in scores {
            score_map.insert(id.to_string(), c);
        }
        ModelRow {
            id: "test-model".into(),
            name: "Test Model (Reasoning)".into(),
            display_name: "Test Model".into(),
            creator: "openai".into(),
            creator_name: "OpenAI".into(),
            release_date: Some("2026-01-15".into()),
            reasoning_status: ReasoningStatus::Reasoning,
            effort_level: Some("high".into()),
            variant_tag: None,
            open_weights: Some(false),
            context_window: Some(200_000),
            supports_tools: None,
            max_output: None,
            scores: score_map,
        }
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn detail_lines_identity_and_groups() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                metric(
                    "intelligence_index",
                    "Intelligence",
                    MetricKind::Index,
                    "Indexes",
                ),
                metric("gpqa", "GPQA", MetricKind::Percentage, "Academic"),
            ],
            models: vec![model_with(vec![
                ("intelligence_index", cell(70.0, None, None)),
                ("gpqa", cell(0.914, None, None)),
            ])],
        };
        let lines = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off);
        let text: Vec<String> = lines.iter().map(line_text).collect();
        let joined = text.join("\n");

        // Identity: display name + id.
        assert_eq!(text[0], "Test Model");
        assert_eq!(text[1], "test-model");
        // Group section headers present (first-appearance order).
        assert!(joined.contains("Indexes"));
        assert!(joined.contains("Academic"));
        // Index value formatted as one decimal; percentage *100.
        assert!(joined.contains("70.0"));
        assert!(joined.contains("91.4%"));
        // Source attribution moved to the source bar — no longer in the detail.
        assert!(!joined.contains("Source: Test Source"));
        assert!(!joined.contains("self-reported"));
    }

    #[test]
    fn detail_identity_region_type_tools_output() {
        // creator "openai" classifies as US / Startup; tools + max output are
        // the models.dev-backfilled fields.
        let mut m = model_with(vec![("intelligence_index", cell(70.0, None, None))]);
        m.supports_tools = Some(true);
        m.max_output = Some(64_000);
        let file = SourceFile {
            source: meta(true),
            metrics: vec![metric(
                "intelligence_index",
                "Intelligence",
                MetricKind::Index,
                "Indexes",
            )],
            models: vec![m],
        };
        let joined = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off)
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Region"), "got: {joined}");
        assert!(joined.contains("US"), "got: {joined}");
        assert!(joined.contains("Type"), "got: {joined}");
        assert!(joined.contains("Tools"), "got: {joined}");
        assert!(joined.contains("Yes"), "got: {joined}");
        assert!(joined.contains("Output"), "got: {joined}");
        assert!(joined.contains("64k"), "got: {joined}");
    }

    #[test]
    fn detail_identity_unknown_creator_em_dashes_region_type() {
        // Empty creator -> Region/Type are honest em-dashes (not Other/Startup),
        // and unmatched tools/output em-dash too.
        let mut m = model_with(vec![("intelligence_index", cell(70.0, None, None))]);
        m.creator = String::new();
        m.creator_name = String::new();
        let file = SourceFile {
            source: meta(true),
            metrics: vec![metric(
                "intelligence_index",
                "Intelligence",
                MetricKind::Index,
                "Indexes",
            )],
            models: vec![m],
        };
        let region_line =
            build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off)
                .into_iter()
                .map(|l| line_text(&l))
                .find(|t| t.contains("Region"))
                .expect("Region row present");
        assert!(!region_line.contains("Other"), "got: {region_line}");
        assert!(!region_line.contains("Startup"), "got: {region_line}");
        assert!(region_line.contains(EM), "got: {region_line}");
    }

    #[test]
    fn detail_lines_elo_ci_and_no_source_attribution() {
        let file = SourceFile {
            source: meta(false),
            metrics: vec![metric("elo_text", "Text Elo", MetricKind::Elo, "Arena Elo")],
            models: vec![model_with(vec![(
                "elo_text",
                cell(1432.7, Some(8.0), Some("2026-06-01")),
            )])],
        };
        let lines = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off);
        let joined: String = lines
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");

        // Elo rounded, ± ci appended. Per-cell dates are deliberately NOT
        // rendered (user feedback: noise in the score rows).
        assert!(joined.contains("1433 \u{00B1}8"), "got: {joined}");
        assert!(!joined.contains("(upd"));
        // Source attribution (incl. the self-reported note) moved to the source
        // bar — the detail panel no longer carries it, even for unverified sources.
        assert!(!joined.contains("Source: Test Source"), "got: {joined}");
        assert!(!joined.contains("(self-reported)"), "got: {joined}");
    }

    #[test]
    fn detail_elo_appends_vote_count() {
        // Arena cells carry a vote count -> compact dim sample-size suffix.
        let mut c = cell(1432.7, Some(8.0), None);
        c.votes = Some(24_871);
        let file = SourceFile {
            source: meta(true),
            metrics: vec![metric("elo_text", "Text Elo", MetricKind::Elo, "Arena Elo")],
            models: vec![model_with(vec![("elo_text", c)])],
        };
        let joined = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off)
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        // value ± ci · {votes} votes
        assert!(joined.contains("1433 \u{00B1}8"), "got: {joined}");
        assert!(joined.contains("\u{00B7} 24.9k votes"), "got: {joined}");
    }

    #[test]
    fn detail_lines_missing_metric_is_em_dash() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![metric("gpqa", "GPQA", MetricKind::Percentage, "Academic")],
            models: vec![model_with(vec![])], // no scores
        };
        let lines = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off);
        let joined: String = lines
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains(EM));
    }

    #[test]
    fn list_value_for_each_sort_key() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                metric(
                    "intelligence_index",
                    "Intelligence",
                    MetricKind::Index,
                    "Indexes",
                ),
                metric(
                    "price_input",
                    "Input Price",
                    MetricKind::UsdPerMTok,
                    "Pricing",
                ),
            ],
            models: vec![model_with(vec![
                ("intelligence_index", cell(70.0, None, None)),
                ("price_input", cell(2.5, None, None)),
            ])],
        };
        let m = &file.models[0];
        // Name -> empty
        assert_eq!(list_value_for(&file, m, SortKey::Name), "");
        // ReleaseDate -> date
        assert_eq!(list_value_for(&file, m, SortKey::ReleaseDate), "2026-01-15");
        // Metric(0) Index -> "70.0"
        assert_eq!(list_value_for(&file, m, SortKey::Metric(0)), "70.0");
        // Metric(1) price -> "$2.50"
        assert_eq!(list_value_for(&file, m, SortKey::Metric(1)), "$2.50");
    }

    #[test]
    fn list_value_header_labels() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                // Short label (<= 11) passes through unchanged.
                metric("gpqa", "GPQA", MetricKind::Percentage, "Academic"),
                // Long label (> 11) is truncated to fit the value column.
                metric(
                    "intelligence_index",
                    "Intelligence",
                    MetricKind::Index,
                    "Indexes",
                ),
            ],
            models: vec![],
        };
        assert_eq!(list_value_header(Some(&file), SortKey::Name), "");
        assert_eq!(
            list_value_header(Some(&file), SortKey::ReleaseDate),
            "Released"
        );
        assert_eq!(list_value_header(Some(&file), SortKey::Metric(0)), "GPQA");
        // "Intelligence" (12 chars) truncates to 11 with an ellipsis.
        assert_eq!(
            list_value_header(Some(&file), SortKey::Metric(1)),
            "Intellig..."
        );
    }

    #[test]
    fn fetched_at_timestamp_parses_to_relative() {
        // The committed aa.json carries a nanosecond-precision RFC3339 timestamp;
        // the source bar's freshness must not echo it raw.
        let out = format_relative_time_from_str("2026-06-10T20:37:40.687663442+00:00");
        assert!(out.ends_with("ago"), "expected relative time, got: {out}");
    }

    /// Fully-configurable metric for the polish-feature tests (direction arrows,
    /// section-header suffixes, glossary).
    #[allow(clippy::too_many_arguments)]
    fn metric_full(
        id: &str,
        label: &str,
        kind: MetricKind,
        group: &str,
        higher_is_better: bool,
        last_updated: Option<&str>,
        description: Option<&str>,
    ) -> MetricDef {
        MetricDef {
            id: id.into(),
            label: label.into(),
            kind,
            group: group.into(),
            higher_is_better,
            last_updated: last_updated.map(str::to_string),
            description: description.map(str::to_string),
            short_label: None,
        }
    }

    // --- (1) Direction lives in section headers, not metric rows ---

    #[test]
    fn detail_metric_rows_have_no_direction_markers() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                // higher-is-better -> up arrow
                metric_full(
                    "gpqa",
                    "GPQA",
                    MetricKind::Percentage,
                    "Academic",
                    true,
                    None,
                    None,
                ),
                // lower-is-better -> down arrow
                metric_full(
                    "price_input",
                    "Input Price",
                    MetricKind::UsdPerMTok,
                    "Pricing",
                    false,
                    None,
                    None,
                ),
            ],
            models: vec![model_with(vec![
                ("gpqa", cell(0.9, None, None)),
                ("price_input", cell(2.0, None, None)),
            ])],
        };
        let lines = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off);
        let gpqa_row = lines
            .iter()
            .find(|l| line_text(l).contains("GPQA"))
            .expect("gpqa row");
        let price_row = lines
            .iter()
            .find(|l| line_text(l).contains("Input Price"))
            .expect("price row");
        // Per-metric markers were removed on user feedback (too distracting);
        // direction lives in the section header suffix instead.
        for row in [gpqa_row, price_row] {
            let text = line_text(row);
            assert!(
                !text.contains('\u{25B2}') && !text.contains('\u{25BC}'),
                "metric rows must not carry direction markers, got: {text}"
            );
        }
        let academic_header = lines
            .iter()
            .find(|l| line_text(l).contains("Academic"))
            .expect("Academic header");
        assert!(
            line_text(academic_header).contains("higher is better"),
            "direction in header, got: {}",
            line_text(academic_header)
        );
        let pricing_header = lines
            .iter()
            .find(|l| line_text(l).contains("Pricing"))
            .expect("Pricing header");
        assert!(
            line_text(pricing_header).contains("lower is better"),
            "direction in header, got: {}",
            line_text(pricing_header)
        );
    }

    // --- (2) Section-header scale suffixes ---

    #[test]
    fn detail_uniform_group_gets_kind_suffix() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                metric_full(
                    "price_input",
                    "Input Price",
                    MetricKind::UsdPerMTok,
                    "Pricing",
                    false,
                    None,
                    None,
                ),
                metric_full(
                    "price_output",
                    "Output Price",
                    MetricKind::UsdPerMTok,
                    "Pricing",
                    false,
                    None,
                    None,
                ),
            ],
            models: vec![model_with(vec![])],
        };
        let lines = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off);
        let header = lines
            .iter()
            .find(|l| line_text(l).contains("Pricing"))
            .expect("Pricing header");
        assert!(
            line_text(header).contains("($ per 1M tokens \u{00B7} lower is better)"),
            "uniform UsdPerMTok group gets scale + direction suffix, got: {}",
            line_text(header)
        );
    }

    #[test]
    fn detail_mixed_group_gets_no_suffix() {
        // AA "Performance" mixes tokens/sec and seconds -> no suffix.
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                metric_full(
                    "output_tps",
                    "Output Speed",
                    MetricKind::TokensPerSec,
                    "Performance",
                    true,
                    None,
                    None,
                ),
                metric_full(
                    "ttft",
                    "TTFT",
                    MetricKind::Seconds,
                    "Performance",
                    false,
                    None,
                    None,
                ),
            ],
            models: vec![model_with(vec![])],
        };
        let lines = build_benchmark_detail_lines(80, &file, &file.models[0], ComparatorMode::Off);
        let header = lines
            .iter()
            .find(|l| line_text(l).contains("Performance"))
            .expect("Performance header");
        let text = line_text(header);
        assert!(
            !text.contains("(tokens/sec)") && !text.contains("(seconds)"),
            "mixed-kind group must not get a scale suffix, got: {text}"
        );
    }

    // --- (3) Glossary lines ---

    #[test]
    fn glossary_includes_description_and_meta() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![metric_full(
                "gpqa",
                "GPQA Diamond",
                MetricKind::Percentage,
                "Academic",
                true,
                Some("2026-05-28"),
                Some("Graduate-level science questions; accuracy."),
            )],
            models: vec![],
        };
        let lines = build_glossary_lines(&file, 60);
        let joined: String = lines
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        // Section header for the group.
        assert!(joined.contains("Academic"), "group header present");
        // Label, no direction marker glyphs.
        assert!(joined.contains("GPQA Diamond"));
        assert!(!joined.contains('\u{25B2}') && !joined.contains('\u{25BC}'));
        // Meta line: kind blurb · direction + updated date.
        assert!(joined.contains("% score"), "kind blurb present: {joined}");
        assert!(
            joined.contains("higher is better"),
            "direction in glossary meta: {joined}"
        );
        assert!(
            joined.contains("updated 2026-05-28"),
            "last_updated rendered: {joined}"
        );
        // Description text.
        assert!(joined.contains("Graduate-level science questions; accuracy."));
    }

    #[test]
    fn glossary_none_description_is_em_dash() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![metric_full(
                "elo_text",
                "Text",
                MetricKind::Elo,
                "Arena Elo",
                true,
                None,
                None, // no description
            )],
            models: vec![],
        };
        let lines = build_glossary_lines(&file, 60);
        let joined: String = lines
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Text"), "label present");
        // Kind blurb still shown (no updated date for this metric).
        assert!(joined.contains("Elo rating"));
        assert!(!joined.contains("updated "), "no last_updated -> no date");
        // Missing description renders as an em-dash line.
        assert!(joined.contains(EM), "None description -> em-dash: {joined}");
    }

    #[test]
    fn glossary_meta_trims_rfc3339_last_updated_to_date() {
        // llmstats emits an RFC3339 timestamp for `last_updated`; the meta line
        // must show only the `YYYY-MM-DD` prefix, not the full timestamp.
        let file = SourceFile {
            source: meta(true),
            metrics: vec![metric_full(
                "agents",
                "Agents",
                MetricKind::Index,
                "Categories",
                true,
                Some("2026-06-11T00:59:59.929424Z"),
                Some("Agentic capability."),
            )],
            models: vec![],
        };
        let lines = build_glossary_lines(&file, 60);
        let joined: String = lines
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("updated 2026-06-11"),
            "date prefix shown: {joined}"
        );
        assert!(
            !joined.contains("00:59:59"),
            "timestamp portion stripped: {joined}"
        );
    }

    // --- (4) Comparator column ---

    use super::super::app::ComparatorMode;

    /// A model with explicit id, creator, release date, and a single `score`
    /// value, for the comparator multi-model tests.
    fn cmp_model(id: &str, date: Option<&str>, score: Option<f64>) -> ModelRow {
        let mut scores = BTreeMap::new();
        if let Some(s) = score {
            scores.insert("score".to_string(), cell(s, None, None));
        }
        ModelRow {
            id: id.into(),
            name: id.into(),
            display_name: id.into(),
            creator: "openai".into(),
            creator_name: "OpenAI".into(),
            release_date: date.map(str::to_string),
            reasoning_status: ReasoningStatus::None,
            effort_level: None,
            variant_tag: None,
            open_weights: Some(false),
            context_window: None,
            supports_tools: None,
            max_output: None,
            scores,
        }
    }

    fn cmp_file() -> SourceFile {
        SourceFile {
            source: meta(true),
            metrics: vec![metric("score", "Score", MetricKind::Index, "Indexes")],
            models: vec![
                cmp_model("a", Some("2026-01-01"), Some(60.0)),
                cmp_model("b", Some("2026-03-01"), Some(70.0)),
                cmp_model("c", Some("2026-04-01"), Some(80.0)),
            ],
        }
    }

    /// The metric row for the `score` metric (the row containing its value).
    fn score_row_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(line_text)
            .find(|t| t.contains("Score"))
            .expect("Score metric row present")
    }

    #[test]
    fn comparator_cells_align_into_a_column() {
        // Rows with different value widths ("8.1%" vs "92.6%" vs em-dash) must
        // start their comparator cells at the same x — a true column, not a
        // trailing annotation.
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                metric("m_short", "Short", MetricKind::Percentage, "G"),
                metric("m_long", "Long", MetricKind::Percentage, "G"),
                metric("m_none", "None", MetricKind::Percentage, "G"),
            ],
            models: vec![
                model_with(vec![("m_short", cell(0.081, None, None))]),
                model_with(vec![
                    ("m_short", cell(0.081, None, None)),
                    ("m_long", cell(0.926, None, None)),
                ]),
            ],
        };
        let lines =
            build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::FieldAvg);
        let avg_cols: Vec<usize> = lines
            .iter()
            .map(line_text)
            .filter(|t| t.contains("avg "))
            .map(|t| t.find("avg").unwrap())
            .collect();
        assert!(
            avg_cols.len() >= 2,
            "expected multiple comparator rows: {avg_cols:?}"
        );
        assert!(
            avg_cols.windows(2).all(|w| w[0] == w[1]),
            "comparator cells must share a column: {avg_cols:?}"
        );
    }

    #[test]
    fn comparator_off_renders_no_cell() {
        let file = cmp_file();
        // model "b" = 70; field avg of 60/70/80 = 70.
        let lines = build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::Off);
        let row = score_row_text(&lines);
        assert!(!row.contains("avg"), "Off must render no comparator: {row}");
        assert!(
            !row.contains("peers"),
            "Off must render no comparator: {row}"
        );
        assert!(!row.contains('#'), "Off must render no rank cell: {row}");
    }

    #[test]
    fn comparator_field_avg_cell() {
        let file = cmp_file();
        let lines =
            build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::FieldAvg);
        let row = score_row_text(&lines);
        // mean(60,70,80) = 70.0, Index kind -> "70.0".
        assert!(row.contains("avg 70.0"), "field avg cell: {row}");
    }

    #[test]
    fn comparator_peer_avg_cell() {
        let file = cmp_file();
        // Anchor on "b" (2026-03-01); peers a (60) + c (80) within ±183d -> 70.0.
        let lines =
            build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::PeerAvg);
        let row = score_row_text(&lines);
        assert!(row.contains("peers(2) 70.0"), "peer avg cell: {row}");
    }

    #[test]
    fn comparator_peer_avg_dateless_is_em_dash() {
        // An Arena-like row (no release_date) -> PeerAvg undefined -> em-dash cell.
        let mut file = cmp_file();
        file.models[1].release_date = None;
        let lines =
            build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::PeerAvg);
        let row = score_row_text(&lines);
        assert!(!row.contains("peers("), "dateless peer avg: {row}");
        assert!(row.contains(EM), "dateless peer avg renders em-dash: {row}");
    }

    #[test]
    fn comparator_rank_cell() {
        let file = cmp_file();
        // higher-is-better; b=70 ranks 2nd of 3.
        let lines = build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::Rank);
        let row = score_row_text(&lines);
        assert!(row.contains("#2/3"), "rank cell: {row}");
    }

    #[test]
    fn comparator_rank_missing_value_no_cell() {
        // A model lacking the metric value -> Rank cannot render a cell.
        let mut file = cmp_file();
        file.models[1].scores.clear();
        let lines = build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::Rank);
        let row = score_row_text(&lines);
        assert!(!row.contains('#'), "no rank for missing value: {row}");
    }

    #[test]
    fn comparator_field_avg_renders_when_model_lacks_value() {
        // FieldAvg still shows the field context even when the model has no value.
        let mut file = cmp_file();
        file.models[1].scores.clear();
        let lines =
            build_benchmark_detail_lines(80, &file, &file.models[1], ComparatorMode::FieldAvg);
        let row = score_row_text(&lines);
        // Field avg over the OTHER two models (60, 80) = 70.0.
        assert!(row.contains("avg 70.0"), "field avg w/o own value: {row}");
        // The value cell itself is an em-dash (model lacks the metric).
        assert!(row.contains(EM), "missing value is em-dash: {row}");
    }

    #[test]
    fn comparator_title_suffix_per_mode() {
        assert_eq!(ComparatorMode::Off.title_suffix(), "");
        assert_eq!(
            ComparatorMode::FieldAvg.title_suffix(),
            " \u{00B7} vs field avg"
        );
        assert_eq!(
            ComparatorMode::PeerAvg.title_suffix(),
            " \u{00B7} vs peers (\u{00B1}6mo)"
        );
        assert_eq!(ComparatorMode::Rank.title_suffix(), " \u{00B7} rank");
    }

    #[test]
    fn comparator_mode_cycles() {
        let m = ComparatorMode::default();
        assert_eq!(m, ComparatorMode::FieldAvg);
        let m = m.next();
        assert_eq!(m, ComparatorMode::PeerAvg);
        let m = m.next();
        assert_eq!(m, ComparatorMode::Rank);
        let m = m.next();
        assert_eq!(m, ComparatorMode::Off);
        let m = m.next();
        assert_eq!(m, ComparatorMode::FieldAvg);
    }

    #[test]
    fn glossary_orders_groups_and_metrics() {
        let file = SourceFile {
            source: meta(true),
            metrics: vec![
                metric_full(
                    "idx",
                    "Intelligence Index",
                    MetricKind::Index,
                    "Indexes",
                    true,
                    None,
                    Some("Composite."),
                ),
                metric_full(
                    "gpqa",
                    "GPQA",
                    MetricKind::Percentage,
                    "Academic",
                    true,
                    None,
                    Some("Science."),
                ),
            ],
            models: vec![],
        };
        let lines = build_glossary_lines(&file, 60);
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        let idx_pos = texts.iter().position(|t| t.contains("Indexes")).unwrap();
        let acad_pos = texts.iter().position(|t| t.contains("Academic")).unwrap();
        // First-appearance group order: Indexes before Academic.
        assert!(idx_pos < acad_pos);
        // Indexes group's "index" suffix is present.
        assert!(texts[idx_pos].contains("(index)"));
    }
}
