use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::app::{Filters, Focus, ListMode, ModelGroup, SortOrder};
use crate::formatting::truncate;
use crate::formatting::EM_DASH;
use crate::provider_category::{provider_category, ProviderCategory};
use crate::tui::app::App;
use crate::tui::ui::{caret, centered_rect, focus_border};
use crate::tui::widgets::scrollable_panel::ScrollablePanel;

fn provider_detail_lines(app: &App) -> Vec<Line<'static>> {
    let Some(entry) = app.models_app.current_model() else {
        return vec![Line::from(Span::styled(
            "No model selected",
            Style::default().fg(Color::DarkGray),
        ))];
    };
    let provider = app
        .providers
        .iter()
        .find(|(id, _)| id == &entry.provider_id)
        .map(|(_, p)| p);
    let Some(provider) = provider else {
        return vec![Line::from(Span::styled(
            "Provider not found",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    let cat = provider_category(&entry.provider_id);
    let has_doc = provider.doc.is_some();
    let has_api = provider.api.is_some();

    let mut lines = vec![
        Line::from(vec![Span::styled(
            provider.name.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("Category: ", Style::default().fg(Color::Gray)),
            Span::styled(cat.label(), Style::default().fg(cat.color())),
        ]),
        Line::from(vec![
            Span::styled("Docs: ", Style::default().fg(Color::Gray)),
            Span::raw(provider.doc.clone().unwrap_or_else(|| EM_DASH.into())),
        ]),
        Line::from(vec![
            Span::styled("API:  ", Style::default().fg(Color::Gray)),
            Span::raw(provider.api.clone().unwrap_or_else(|| EM_DASH.into())),
        ]),
        Line::from(vec![
            Span::styled("Env:  ", Style::default().fg(Color::Gray)),
            Span::raw(if provider.env.is_empty() {
                EM_DASH.to_string()
            } else {
                provider.env.join(", ")
            }),
        ]),
    ];

    // Only show keybinding hints for available URLs
    let mut hints: Vec<Span<'static>> = Vec::new();
    if has_doc {
        hints.push(Span::styled("o ", Style::default().fg(Color::Yellow)));
        hints.push(Span::raw("docs"));
    }
    if has_doc && has_api {
        hints.push(Span::raw("  "));
    }
    if has_api {
        hints.push(Span::styled("A ", Style::default().fg(Color::Yellow)));
        hints.push(Span::raw("api"));
    }
    if !hints.is_empty() {
        lines.push(Line::from(hints));
    }

    lines
}

fn draw_right_panel(f: &mut Frame, area: Rect, app: &mut App) {
    // Grouped mode: the provider card is meaningless for a multi-provider
    // group — the detail panel (with its Providers section) takes the full
    // height.
    if app.models_app.list_mode() == ListMode::Grouped {
        app.models_app.provider_card_area = None;
        app.models_app.model_detail_area = Some(area);
        draw_model_detail(f, area, app);
        return;
    }
    let lines = provider_detail_lines(app);

    // Compute visual height: sum of wrapped line heights + 2 for borders.
    // Word-wrapping can use more lines than char-level div_ceil predicts,
    // so we add 1 extra line for each line that wraps as a buffer.
    let border_block = Block::default().borders(Borders::ALL);
    let inner_w = border_block.inner(area).width as usize;
    let visual_lines: u16 = if inner_w == 0 {
        lines.len() as u16
    } else {
        lines
            .iter()
            .map(|line| {
                let w = line.width();
                if w <= inner_w {
                    1u16
                } else {
                    // div_ceil for char-level + 1 for word-wrap slack
                    w.div_ceil(inner_w) as u16 + 1
                }
            })
            .sum()
    };
    let provider_h = visual_lines + 2; // +2 for borders

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(provider_h), Constraint::Min(0)])
        .split(area);

    // Cache rects for mouse hit-testing (provider card focuses Details too).
    app.models_app.provider_card_area = Some(chunks[0]);
    app.models_app.model_detail_area = Some(chunks[1]);

    draw_provider_detail(f, chunks[0], lines);
    draw_model_detail(f, chunks[1], app);
}

pub(in crate::tui) fn draw_main(f: &mut Frame, area: Rect, app: &mut App) {
    // 2-column layout: model list 60% | right panel 40%. The provider
    // sidebar was retired in favor of the `p` provider modal — its filtering
    // job lives there, and its browse job is the grouped list itself.
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // The sidebar rect is gone for good; make sure no stale hit-area lingers.
    app.models_app.provider_list_area = None;

    draw_models(f, chunks[0], app);
    draw_right_panel(f, chunks[1], app);

    if app.models_app.show_glossary {
        draw_glossary(f, area, app);
    }
    if app.models_app.show_provider_picker {
        draw_provider_picker(f, area, app);
    }
}

/// Provider picker modal (`p`): search-first scoping to one provider (or back
/// to All). Search is deliberately scope-local — it matches provider id/name
/// only, never the models a provider carries ("who sells Claude" is answered
/// by the grouped list's Enter drill instead).
fn draw_provider_picker(f: &mut Frame, area: Rect, app: &App) {
    use ratatui::widgets::{List as PickList, ListItem as PickItem, ListState as PickState};

    let rows = app.models_app.picker_rows(&app.providers);
    let height = (rows.len() as u16 + 3).clamp(6, area.height.saturating_sub(4));
    let width = 46u16.min(area.width.saturating_sub(4));
    let popup = crate::tui::ui::centered_rect_fixed(width, height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(format!(" Provider ({}) ", rows.len().saturating_sub(1)))
        .title_bottom(ratatui::text::Line::from(" Enter: browse | Esc: cancel ").centered());
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    // Search line + rows below it.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    let cursor = "_";
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" / ", Style::default().fg(Color::Cyan)),
            Span::raw(app.models_app.picker_query.clone()),
            Span::styled(cursor, Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ])),
        chunks[0],
    );

    let items: Vec<PickItem> = rows
        .iter()
        .map(|row| match row {
            None => {
                let total: usize = app.providers.iter().map(|(_, p)| p.models.len()).sum();
                PickItem::new(Line::from(vec![Span::styled(
                    format!("All models ({})", total),
                    Style::default().fg(Color::Green),
                )]))
            }
            Some(idx) => {
                let (id, p) = &app.providers[*idx];
                let cat = provider_category(id);
                let initial = &cat.short_label()[..1];
                PickItem::new(Line::from(vec![
                    Span::styled(initial.to_string(), Style::default().fg(cat.color())),
                    Span::raw(format!(" {} ", id)),
                    Span::styled(
                        format!("({})", p.models.len()),
                        Style::default().fg(Color::Gray),
                    ),
                ]))
            }
        })
        .collect();

    // Fresh ListState each frame → deterministic offset for popup_row_at.
    let mut state = PickState::default();
    state.select(Some(
        app.models_app
            .picker_selected
            .min(rows.len().saturating_sub(1)),
    ));
    let list = PickList::new(items).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    app.models_app.picker_area.set(Some(chunks[1]));
    f.render_stateful_widget(list, chunks[1], &mut state);
}

/// Glossary popup (`i`) explaining the Models-tab capability and pricing fields.
/// Static content — independent of the selected model.
fn draw_glossary(f: &mut Frame, area: Rect, app: &App) {
    let popup_area = centered_rect(60, 70, area);
    f.render_widget(Clear, popup_area);
    let title = " Models Glossary - i or Esc to close (Up/Down to scroll) ";
    let inner_w = popup_area.width.saturating_sub(2);
    let lines = build_glossary_lines(inner_w);
    ScrollablePanel::new(title, lines, &app.models_app.glossary_scroll, true).render(f, popup_area);
}

/// Build the (static) glossary content. Sections mirror the detail panel.
fn build_glossary_lines(width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let term = |t: &str| {
        Line::from(Span::styled(
            t.to_string(),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let desc = |d: &str| {
        Line::from(Span::styled(
            d.to_string(),
            Style::default().fg(Color::White),
        ))
    };

    let entry = |lines: &mut Vec<Line<'static>>, t: &str, d: &str| {
        lines.push(term(t));
        lines.push(desc(d));
        lines.push(Line::from(""));
    };

    lines.push(section_header_line(width, "List column (RTFO)"));
    entry(
        &mut lines,
        "R T F O",
        "Compact capability flags in the model list: Reasoning, Tools, Files (attachments), Open weights (O green = open, C red = closed). A dot means the model lacks that capability.",
    );

    lines.push(section_header_line(width, "Capabilities"));
    entry(
        &mut lines,
        "Reasoning",
        "The model performs internal step-by-step reasoning before answering.",
    );
    entry(&mut lines, "Tools", "Supports tool / function calling.");
    entry(
        &mut lines,
        "Files",
        "Accepts file or image attachments (multimodal input).",
    );
    entry(
        &mut lines,
        "Source",
        "Open = open-weights (downloadable). Closed = proprietary, API-only.",
    );
    entry(
        &mut lines,
        "Temp",
        "The sampling temperature parameter can be adjusted.",
    );
    entry(
        &mut lines,
        "Structured",
        "Supports structured / JSON-schema-constrained output. An em-dash (—) means models.dev does not report this capability for the model.",
    );

    lines.push(section_header_line(width, "Reasoning controls"));
    entry(
        &mut lines,
        "Budget",
        "You set an explicit thinking-token budget; the range shows the min–max allowed.",
    );
    entry(
        &mut lines,
        "Effort",
        "You pick a reasoning-effort level (e.g. Low, Medium, High).",
    );
    entry(
        &mut lines,
        "Toggle",
        "Reasoning can only be turned on or off (no fine-grained control).",
    );

    lines.push(section_header_line(width, "Pricing (USD per 1M tokens)"));
    entry(
        &mut lines,
        "Input / Output",
        "Price per million prompt (input) and completion (output) tokens.",
    );
    entry(
        &mut lines,
        "Cache Read / Write",
        "Prices for reading from and writing to the prompt cache (cheaper reuse of repeated context).",
    );
    entry(
        &mut lines,
        "Thinking",
        "Price for reasoning / thinking tokens — often billed higher than output.",
    );
    entry(
        &mut lines,
        "Audio In / Out",
        "Per-token prices for audio input and output (omni / speech models).",
    );
    entry(
        &mut lines,
        "Over {size}",
        "Tiered pricing: the rate that applies once a request exceeds the given context-size threshold.",
    );

    lines.push(section_header_line(width, "Limits"));
    entry(
        &mut lines,
        "Context",
        "Maximum total tokens (input + output) the model accepts per request.",
    );
    entry(
        &mut lines,
        "Input / Output",
        "Maximum input tokens accepted and maximum output tokens generated.",
    );

    // Drop the trailing blank line for a tidy bottom.
    if lines.last().map(|l| l.width() == 0).unwrap_or(false) {
        lines.pop();
    }
    lines
}

fn draw_models(f: &mut Frame, area: Rect, app: &mut App) {
    if app.models_app.list_mode() == ListMode::Grouped {
        draw_grouped_list(f, area, app);
        return;
    }
    let is_focused = app.models_app.focus == Focus::Models;
    let border_style = focus_border(is_focused);

    let models = app.models_app.filtered_models();

    let sort_indicator = match app.models_app.sort_order {
        SortOrder::Default => String::new(),
        _ => {
            let arrow = if app.models_app.sort_ascending {
                "\u{2191}"
            } else {
                "\u{2193}"
            };
            let label = match app.models_app.sort_order {
                SortOrder::ReleaseDate => "date",
                SortOrder::Cost => "cost",
                SortOrder::Context => "ctx",
                SortOrder::Default => unreachable!(),
            };
            format!(" {}{}", arrow, label)
        }
    };

    let filter_indicator = format_filters(
        &app.models_app.filters,
        app.models_app.provider_category_filter,
    );

    // Base label per mode: provider display name when scoped, breadcrumb when
    // drilled into a group, "Models · flat" for the V-toggled fire-hose.
    let breadcrumb;
    let provider_label = match app.models_app.list_mode() {
        ListMode::Offerings => {
            let name = app.models_app.drill_name.as_deref().unwrap_or("?");
            breadcrumb = format!("Models \u{25B8} {}", name);
            breadcrumb.as_str()
        }
        _ => app
            .models_app
            .selected_provider_data(&app.providers)
            .map(|(_, p)| p.name.as_str())
            .unwrap_or("Models \u{00B7} flat"),
    };
    // Offerings mode counts distinct providers (the number the breadcrumb
    // promises); other modes count rows.
    let count = if app.models_app.list_mode() == ListMode::Offerings {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for e in models {
            seen.insert(e.provider_id.as_str());
        }
        let n = seen.len();
        format!("{} provider{}", n, if n == 1 { "" } else { "s" })
    } else {
        models.len().to_string()
    };
    let count = count.as_str();

    let title = if app.models_app.search_query.is_empty() && filter_indicator.is_empty() {
        format!(" {} ({}){} ", provider_label, count, sort_indicator)
    } else if app.models_app.search_query.is_empty() {
        format!(
            " {} ({}){} [{}] ",
            provider_label, count, sort_indicator, filter_indicator
        )
    } else if filter_indicator.is_empty() {
        format!(
            " {} ({}) [/{}]{} ",
            provider_label, count, app.models_app.search_query, sort_indicator
        )
    } else {
        format!(
            " {} ({}) [/{}] [{}]{} ",
            provider_label, count, app.models_app.search_query, filter_indicator, sort_indicator
        )
    };

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Fixed column widths: caret(2) + caps(5), then the Provider column
    // (1-space gap + 14-wide display name) and up to three 9-wide numeric
    // columns (1-space gap + 8-wide value). Columns shed greedily from the
    // right of the keep-priority order — Provider, Input, Output, Context —
    // before the name column starves (mirrors the Benchmarks list): Provider
    // is the last to go because in the "All" view it is the differentiator
    // between otherwise-identical duplicate rows. The old fixed layout
    // silently clipped the Context column at narrow widths, rendering a
    // 1M-context model as "1". The actively-sorted column always survives
    // the drop.
    let caret_w: u16 = 2;
    let caps_w: u16 = 5; // "RTFO " — 4 indicator chars + 1 space
    const NAME_MIN: u16 = 18;
    let fixed_w = caret_w + caps_w;
    let mut budget = inner_area.width.saturating_sub(fixed_w + NAME_MIN);
    let mut num_cols: Vec<ModelListColumn> = Vec::new();
    for col in [
        ModelListColumn::Provider,
        ModelListColumn::Input,
        ModelListColumn::Output,
        ModelListColumn::Context,
    ] {
        if budget >= col.width() {
            budget -= col.width();
            num_cols.push(col);
        }
    }
    let sort_needs = match app.models_app.sort_order {
        SortOrder::Cost => Some(ModelListColumn::Input),
        SortOrder::Context => Some(ModelListColumn::Context),
        SortOrder::Default | SortOrder::ReleaseDate => None,
    };
    if let Some(needed) = sort_needs {
        if !num_cols.is_empty() && !num_cols.contains(&needed) {
            // Numeric columns are never wider than the column they replace
            // (Provider is the widest), so the swap always fits.
            *num_cols.last_mut().unwrap() = needed;
        }
    }
    let cols_w: u16 = num_cols.iter().map(|c| c.width()).sum();
    let name_width = (inner_area.width.saturating_sub(fixed_w + cols_w) as usize).max(10);

    // Provider display names for the Provider column, keyed by provider id.
    let provider_names: std::collections::HashMap<&str, &str> = app
        .providers
        .iter()
        .map(|(id, p)| (id.as_str(), p.name.as_str()))
        .collect();

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let active_header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // Determine which column is actively sorted
    let sort_col = match app.models_app.sort_order {
        SortOrder::Default => "name",
        SortOrder::ReleaseDate => "name",
        SortOrder::Cost => "cost",
        SortOrder::Context => "context",
    };
    let cost_style = if sort_col == "cost" {
        active_header_style
    } else {
        header_style
    };

    // Caret prefix for focused panel
    let caret = caret(is_focused);

    // Build header spans (leading spaces to align with caret)
    let mut header_spans: Vec<Span> = vec![
        Span::raw("  "),
        Span::styled("RTFO ", header_style),
        Span::styled(
            format!("{:<width$}", "Model", width = name_width),
            if sort_col == "name" {
                active_header_style
            } else {
                header_style
            },
        ),
    ];
    for col in &num_cols {
        let (label, style) = match col {
            ModelListColumn::Provider => ("Provider", header_style),
            ModelListColumn::Input => ("Input", cost_style),
            ModelListColumn::Output => ("Output", cost_style),
            ModelListColumn::Context => (
                "Context",
                if sort_col == "context" {
                    active_header_style
                } else {
                    header_style
                },
            ),
        };
        let cell = match col {
            ModelListColumn::Provider => format!(" {:<14}", label),
            _ => format!(" {:>8}", label),
        };
        header_spans.push(Span::styled(cell, style));
    }

    // Sticky header: fixed line above the list (see draw_grouped_list).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner_area);
    f.render_widget(Paragraph::new(Line::from(header_spans)), chunks[0]);

    let mut items: Vec<ListItem> = Vec::with_capacity(models.len());

    // Model rows
    for (display_idx, entry) in models.iter().enumerate() {
        let is_selected = display_idx == app.models_app.selected_model;
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let cost = &entry.model.cost;
        let input_cost = crate::data::Model::cost_short(cost.as_ref().and_then(|c| c.input));
        let output_cost = crate::data::Model::cost_short(cost.as_ref().and_then(|c| c.output));
        let ctx = entry.model.context_str();

        let prefix = if is_selected { caret } else { "  " };
        let m = &entry.model;
        let raw_name = if m.name.is_empty() {
            &entry.id
        } else {
            &m.name
        };
        let display_name = if entry
            .identity
            .is_some_and(super::app::ModelIdentityProvenance::is_inferred)
        {
            format!("≈ {raw_name}")
        } else {
            raw_name.to_string()
        };

        let (r_ch, r_color) = if m.reasoning {
            ("R", Color::Cyan)
        } else {
            ("·", Color::DarkGray)
        };
        let (t_ch, t_color) = if m.tool_call {
            ("T", Color::Yellow)
        } else {
            ("·", Color::DarkGray)
        };
        let (f_ch, f_color) = if m.attachment {
            ("F", Color::Magenta)
        } else {
            ("·", Color::DarkGray)
        };
        let (o_ch, o_color) = if m.open_weights {
            ("O", Color::Green)
        } else {
            ("C", Color::Red)
        };
        let mut row_spans: Vec<Span> = vec![
            Span::styled(prefix, style),
            Span::styled(r_ch, Style::default().fg(r_color)),
            Span::styled(t_ch, Style::default().fg(t_color)),
            Span::styled(f_ch, Style::default().fg(f_color)),
            Span::styled(o_ch, Style::default().fg(o_color)),
            Span::raw(" "),
            Span::styled(
                format!(
                    "{:<width$}",
                    // Display name, not id: the id is the *acting* artifact
                    // (config strings — served by `c`/`C` copy and the detail
                    // panel), the name is the *reading* one. Names are
                    // shorter, curated upstream, and match what search
                    // matches. Id fallback for a hypothetical nameless model.
                    truncate(&display_name, name_width.saturating_sub(1)),
                    width = name_width
                ),
                style,
            ),
        ];
        for col in &num_cols {
            match col {
                ModelListColumn::Provider => {
                    let name = provider_names
                        .get(entry.provider_id.as_str())
                        .copied()
                        .unwrap_or(entry.provider_id.as_str());
                    row_spans.push(Span::styled(format!(" {:<14}", truncate(name, 14)), style));
                }
                ModelListColumn::Input => {
                    row_spans.push(Span::styled(format!(" {:>8}", input_cost), style))
                }
                ModelListColumn::Output => {
                    row_spans.push(Span::styled(format!(" {:>8}", output_cost), style))
                }
                ModelListColumn::Context => {
                    row_spans.push(Span::styled(format!(" {:>8}", ctx), style))
                }
            }
        }

        items.push(ListItem::new(Line::from(row_spans)));
    }

    let list = List::new(items);
    // Cache the bare row rect and render into the real state so its
    // post-render `offset()` (clamped to the viewport) is available for
    // mouse hit-testing.
    app.models_app.model_list_area = Some(chunks[1]);
    f.render_stateful_widget(list, chunks[1], &mut app.models_app.model_list_state);
}

/// Columns right of the model id, in display order (which doubles as the
/// keep-priority order). Which ones render is width-dependent (see the drop
/// policy in `draw_models`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelListColumn {
    Provider,
    Input,
    Output,
    Context,
}

impl ModelListColumn {
    /// Total column width including its 1-space leading gap.
    fn width(self) -> u16 {
        match self {
            // 14-wide left-aligned display name.
            Self::Provider => 15,
            // 8-wide right-aligned value.
            Self::Input | Self::Output | Self::Context => 9,
        }
    }
}

/// Format a min–max cost range compactly: `—` / `$5` / `$5–6`.
fn fmt_cost_range(range: Option<(f64, f64)>) -> String {
    match range {
        None => EM_DASH.to_string(),
        Some((lo, hi)) if lo == hi => crate::data::Model::cost_short(Some(lo)),
        Some((lo, hi)) => format!(
            "{}\u{2013}{}",
            crate::data::Model::cost_short(Some(lo)),
            crate::data::Model::cost_short(Some(hi)).trim_start_matches('$')
        ),
    }
}

/// Format a min–max context range: `—` / `1M` / `131.1k–1M`. Values within
/// 5% of each other collapse to one — providers spell the "same" window as
/// 1,000,000 vs 1,048,576, and a `1M–1.0M` range is noise, not information.
fn fmt_ctx_range(range: Option<(u64, u64)>) -> String {
    match range {
        None => EM_DASH.to_string(),
        Some((lo, hi)) if lo == hi || (hi - lo) * 20 < hi => crate::formatting::format_tokens(hi),
        Some((lo, hi)) => format!(
            "{}\u{2013}{}",
            crate::formatting::format_tokens(lo),
            crate::formatting::format_tokens(hi)
        ),
    }
}

/// Capability char for a grouped row under the majority-plus-dim policy:
/// majority value picks the char; a split (0 < true < total) dims it.
fn group_cap_char(
    tally: (usize, usize),
    active: (&'static str, Color),
    inactive: (&'static str, Color),
) -> (&'static str, Color) {
    let (yes, total) = tally;
    let majority_active = yes * 2 > total;
    let mixed = yes > 0 && yes < total;
    let (ch, color) = if majority_active { active } else { inactive };
    if mixed {
        (ch, Color::DarkGray)
    } else {
        (ch, color)
    }
}

/// The grouped Models view: one row per canonical model or unlinked offering.
/// Columns after the name shed greedily (Lab, Providers, Input, Output,
/// Context) before the name drops below its minimum, mirroring the flat list's
/// drop policy.
fn draw_grouped_list(f: &mut Frame, area: Rect, app: &mut App) {
    let is_focused = app.models_app.focus == Focus::Models;
    let border_style = focus_border(is_focused);

    let sort_indicator = match app.models_app.sort_order {
        SortOrder::Default => String::new(),
        _ => {
            let arrow = if app.models_app.sort_ascending {
                "\u{2191}"
            } else {
                "\u{2193}"
            };
            let label = match app.models_app.sort_order {
                SortOrder::ReleaseDate => "date",
                SortOrder::Cost => "cost",
                SortOrder::Context => "ctx",
                SortOrder::Default => unreachable!(),
            };
            format!(" {}{}", arrow, label)
        }
    };
    let filter_indicator = format_filters(
        &app.models_app.filters,
        app.models_app.provider_category_filter,
    );
    let n = app.models_app.groups.len();
    let title = if app.models_app.search_query.is_empty() && filter_indicator.is_empty() {
        format!(" Models ({}){} ", n, sort_indicator)
    } else if app.models_app.search_query.is_empty() {
        format!(" Models ({}){} [{}] ", n, sort_indicator, filter_indicator)
    } else if filter_indicator.is_empty() {
        format!(
            " Models ({}) [/{}]{} ",
            n, app.models_app.search_query, sort_indicator
        )
    } else {
        format!(
            " Models ({}) [/{}] [{}]{} ",
            n, app.models_app.search_query, filter_indicator, sort_indicator
        )
    };

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Column set: Lab(16 = gap+15), Providers(11 = gap+10), Input(12),
    // Output(12), Context(12); greedy drop from the right, name min 18,
    // sorted column survives.
    #[derive(Clone, Copy, PartialEq)]
    enum GCol {
        Lab,
        Providers,
        Input,
        Output,
        Context,
    }
    let col_w = |c: &GCol| -> u16 {
        match c {
            GCol::Lab => 16,
            GCol::Providers => 11,
            GCol::Input | GCol::Output | GCol::Context => 12,
        }
    };
    let caret_w: u16 = 2;
    let caps_w: u16 = 5;
    const NAME_MIN: u16 = 18;
    let fixed_w = caret_w + caps_w;
    let mut budget = inner_area.width.saturating_sub(fixed_w + NAME_MIN);
    let mut cols: Vec<GCol> = Vec::new();
    for c in [
        GCol::Lab,
        GCol::Providers,
        GCol::Input,
        GCol::Output,
        GCol::Context,
    ] {
        if budget >= col_w(&c) {
            budget -= col_w(&c);
            cols.push(c);
        }
    }
    let sort_needs = match app.models_app.sort_order {
        SortOrder::Cost => Some(GCol::Input),
        SortOrder::Context => Some(GCol::Context),
        SortOrder::Default | SortOrder::ReleaseDate => None,
    };
    if let Some(needed) = sort_needs {
        if !cols.is_empty() && !cols.contains(&needed) {
            *cols.last_mut().unwrap() = needed;
        }
    }
    let cols_w: u16 = cols.iter().map(col_w).sum();
    let name_width = (inner_area.width.saturating_sub(fixed_w + cols_w) as usize).max(10);

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let active_header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let sort_col = match app.models_app.sort_order {
        SortOrder::Default | SortOrder::ReleaseDate => "name",
        SortOrder::Cost => "cost",
        SortOrder::Context => "context",
    };
    let caret = caret(is_focused);

    let mut header_spans: Vec<Span> = vec![
        Span::raw("  "),
        Span::styled("RTFO ", header_style),
        Span::styled(
            format!("{:<width$}", "Model", width = name_width),
            if sort_col == "name" {
                active_header_style
            } else {
                header_style
            },
        ),
    ];
    for c in &cols {
        let (label, style, left) = match c {
            GCol::Lab => ("Lab", header_style, true),
            GCol::Providers => ("Providers", header_style, false),
            GCol::Input | GCol::Output => (
                if matches!(c, GCol::Input) {
                    "Input"
                } else {
                    "Output"
                },
                if sort_col == "cost" {
                    active_header_style
                } else {
                    header_style
                },
                false,
            ),
            GCol::Context => (
                "Context",
                if sort_col == "context" {
                    active_header_style
                } else {
                    header_style
                },
                false,
            ),
        };
        let w = (col_w(c) - 1) as usize;
        let cell = if left {
            format!(" {:<w$}", label, w = w)
        } else {
            format!(" {:>w$}", label, w = w)
        };
        header_spans.push(Span::styled(cell, style));
    }

    // Sticky header: a fixed line above the list, so it survives scrolling
    // (as a list item it scrolled away and never came back after G-then-g).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner_area);
    f.render_widget(Paragraph::new(Line::from(header_spans)), chunks[0]);

    let mut items: Vec<ListItem> = Vec::with_capacity(app.models_app.groups.len());

    for (display_idx, g) in app.models_app.groups.iter().enumerate() {
        let is_selected = display_idx == app.models_app.selected_group;
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let prefix = if is_selected { caret } else { "  " };
        let (r_ch, r_color) = group_cap_char(
            g.reasoning,
            ("R", Color::Cyan),
            ("\u{00B7}", Color::DarkGray),
        );
        let (t_ch, t_color) =
            group_cap_char(g.tools, ("T", Color::Yellow), ("\u{00B7}", Color::DarkGray));
        let (f_ch, f_color) = group_cap_char(
            g.files,
            ("F", Color::Magenta),
            ("\u{00B7}", Color::DarkGray),
        );
        let (o_ch, o_color) = group_cap_char(g.open, ("O", Color::Green), ("C", Color::Red));
        let peer_only = g.member_provenance.iter().all(|provenance| {
            matches!(
                provenance,
                super::app::ModelIdentityProvenance::InferredPeer
            )
        });
        let display_name = if peer_only {
            format!("≈ {}", g.name)
        } else {
            g.name.clone()
        };

        let mut row_spans: Vec<Span> = vec![
            Span::styled(prefix, style),
            Span::styled(r_ch, Style::default().fg(r_color)),
            Span::styled(t_ch, Style::default().fg(t_color)),
            Span::styled(f_ch, Style::default().fg(f_color)),
            Span::styled(o_ch, Style::default().fg(o_color)),
            Span::raw(" "),
            Span::styled(
                format!(
                    "{:<width$}",
                    truncate(&display_name, name_width.saturating_sub(1)),
                    width = name_width
                ),
                style,
            ),
        ];
        for c in &cols {
            let w = (col_w(c) - 1) as usize;
            match c {
                GCol::Lab => {
                    let lab = g
                        .lab
                        .as_deref()
                        .map(crate::labs::lab_display)
                        .unwrap_or_else(|| EM_DASH.to_string());
                    let lab_style = if g.lab.is_some() {
                        style
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    row_spans.push(Span::styled(
                        format!(" {:<w$}", truncate(&lab, w), w = w),
                        lab_style,
                    ));
                }
                GCol::Providers => row_spans.push(Span::styled(
                    format!(" {:>w$}", g.provider_count, w = w),
                    style,
                )),
                GCol::Input => row_spans.push(Span::styled(
                    format!(" {:>w$}", fmt_cost_range(g.input_range), w = w),
                    style,
                )),
                GCol::Output => row_spans.push(Span::styled(
                    format!(" {:>w$}", fmt_cost_range(g.output_range), w = w),
                    style,
                )),
                GCol::Context => row_spans.push(Span::styled(
                    format!(" {:>w$}", fmt_ctx_range(g.context_range), w = w),
                    style,
                )),
            }
        }
        items.push(ListItem::new(Line::from(row_spans)));
    }

    let list = List::new(items);
    // Shared hit-rect with the flat list (bare row region below the sticky
    // header); the mouse handler dispatches on list_mode() for state/offset.
    app.models_app.model_list_area = Some(chunks[1]);
    f.render_stateful_widget(list, chunks[1], &mut app.models_app.group_list_state);
}

fn draw_provider_detail(f: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Provider "))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

fn section_header_line(width: u16, title: &str) -> Line<'static> {
    let w = width as usize;
    let prefix = format!("\u{2500}\u{2500} {} ", title);
    let fill_len = w.saturating_sub(prefix.chars().count());
    let header = format!("{}{}", prefix, "\u{2500}".repeat(fill_len));
    Line::from(Span::styled(
        header,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

/// A label-value pair for `two_pair_line`.
struct LabelValue<'a> {
    label: &'a str,
    value: &'a str,
    color: Color,
}

/// Build a line with two label-value pairs, manually padded to fill the width.
fn two_pair_line(left: LabelValue<'_>, right: LabelValue<'_>, col_w: usize) -> Line<'static> {
    let label_color = Color::Gray;
    let pad1 = col_w.saturating_sub(left.label.len() + left.value.len());
    let pad2 = col_w.saturating_sub(right.label.len() + right.value.len());
    Line::from(vec![
        Span::styled(left.label.to_string(), Style::default().fg(label_color)),
        Span::styled(left.value.to_string(), Style::default().fg(left.color)),
        Span::raw(" ".repeat(pad1)),
        Span::styled(right.label.to_string(), Style::default().fg(label_color)),
        Span::styled(right.value.to_string(), Style::default().fg(right.color)),
        Span::raw(" ".repeat(pad2)),
    ])
}

/// "── Providers (N) ──" section of the grouped detail panel: one line per
/// offering, cheapest input first, capped with an overflow hint.
fn group_providers_section(app: &App, g: &ModelGroup, width: u16) -> Vec<Line<'static>> {
    const MAX_ROWS: usize = 10;
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(""));
    lines.push(section_header_line(
        width,
        &format!("Providers ({})", g.provider_count),
    ));
    // Collect (provider display name, input, output, ctx) per member.
    type OfferingRow = (String, Option<f64>, Option<f64>, Option<u64>, bool);
    let mut rows: Vec<OfferingRow> = g
        .member_indices
        .iter()
        .enumerate()
        .filter_map(|(member_pos, &i)| {
            app.models_app
                .filtered_models()
                .get(i)
                .map(|entry| (member_pos, entry))
        })
        .map(|(member_pos, e)| {
            let name = app
                .providers
                .iter()
                .find(|(id, _)| *id == e.provider_id)
                .map(|(_, p)| p.name.clone())
                .unwrap_or_else(|| e.provider_id.clone());
            (
                name,
                e.model.cost.as_ref().and_then(|c| c.input),
                e.model.cost.as_ref().and_then(|c| c.output),
                e.model.limit.as_ref().and_then(|l| l.context),
                g.member_provenance
                    .get(member_pos)
                    .is_some_and(|provenance| provenance.is_inferred()),
            )
        })
        .collect();
    rows.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    let overflow = rows.len().saturating_sub(MAX_ROWS);
    for (name, input, output, ctx, inferred) in rows.into_iter().take(MAX_ROWS) {
        let cost = format!(
            "{} / {}",
            crate::data::Model::cost_short(input),
            crate::data::Model::cost_short(output)
        );
        let ctx_str = ctx
            .map(crate::formatting::format_tokens)
            .unwrap_or_else(|| EM_DASH.to_string());
        lines.push(Line::from(vec![
            Span::styled(
                if inferred { "≈ " } else { "  " },
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{:<18}", truncate(&name, 18)),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(format!("{:>15}", cost), Style::default().fg(Color::White)),
            Span::styled(format!("{:>9}", ctx_str), Style::default().fg(Color::White)),
        ]));
    }
    if overflow > 0 {
        lines.push(Line::from(Span::styled(
            format!("  \u{2026} {} more \u{2014} Enter to browse", overflow),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn model_detail_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let Some(entry) = app.models_app.current_model() else {
        return vec![Line::from(Span::styled(
            "No model selected",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    let model = &entry.model;
    let is_deprecated = model.status.as_deref() == Some("deprecated");
    let text_color = if is_deprecated {
        Color::DarkGray
    } else {
        Color::White
    };
    let label_color = Color::Gray;
    let em = EM_DASH;
    let col_w = (width as usize) / 2;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── Identity ──────────────────────────────────────────────────────────
    lines.push(Line::from(Span::styled(
        model.name.clone(),
        Style::default().fg(text_color).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        entry.id.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    // Grouped mode: identify the lab right under the id; the Providers
    // section lands after the description (before Capabilities).
    if app.models_app.list_mode() == ListMode::Grouped {
        if let Some(group) = app.models_app.current_group() {
            if let Some(lab) = group.lab.as_deref() {
                lines.push(Line::from(vec![
                    Span::styled("Lab: ", Style::default().fg(label_color)),
                    Span::styled(
                        crate::labs::lab_display(lab),
                        Style::default().fg(text_color),
                    ),
                ]));
            }

            let authoritative = group
                .member_provenance
                .iter()
                .filter(|provenance| provenance.is_authoritative())
                .count();
            let inferred_canonical = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredCanonical
                    )
                })
                .count();
            let inferred_qualified = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredQualifiedCanonical
                    )
                })
                .count();
            let inferred_exact_pair = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredExactPairCanonical
                    )
                })
                .count();
            let inferred_one_sided = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredOneSidedCreatorCanonical
                    )
                })
                .count();
            let inferred_cross = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredCrossAliasCanonical
                    )
                })
                .count();
            let inferred_full_id = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredFullIdCanonical
                    )
                })
                .count();
            let inferred_self_anchor = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredSelfAnchorCanonical
                    )
                })
                .count();
            let inferred_creator_prefixed = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredCreatorPrefixedCanonical
                    )
                })
                .count();
            let inferred_peer = group
                .member_provenance
                .iter()
                .filter(|provenance| {
                    matches!(
                        provenance,
                        super::app::ModelIdentityProvenance::InferredPeer
                    )
                })
                .count();
            let (identity, color) = if inferred_peer > 0 {
                (
                    "inferred peer group (not canonical)".to_string(),
                    Color::DarkGray,
                )
            } else if inferred_canonical
                + inferred_qualified
                + inferred_exact_pair
                + inferred_one_sided
                + inferred_cross
                + inferred_full_id
                + inferred_self_anchor
                + inferred_creator_prefixed
                > 0
            {
                let inferred = inferred_canonical
                    + inferred_qualified
                    + inferred_exact_pair
                    + inferred_one_sided
                    + inferred_cross
                    + inferred_full_id
                    + inferred_self_anchor
                    + inferred_creator_prefixed;
                let mut notes = Vec::new();
                for (count, label) in [
                    (inferred_qualified, "creator-qualified"),
                    (inferred_exact_pair, "exact-pair"),
                    (inferred_one_sided, "one-sided creator"),
                    (inferred_cross, "cross-alias"),
                    (inferred_full_id, "full-id alias"),
                    (inferred_self_anchor, "self-anchor"),
                    (inferred_creator_prefixed, "creator-prefixed id"),
                ] {
                    if count > 0 {
                        notes.push(format!("{count} {label}"));
                    }
                }
                let reconciliation_note = if notes.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", notes.join(", "))
                };
                (
                    format!(
                        "{} models.dev link{} + {} inferred{}",
                        authoritative,
                        if authoritative == 1 { "" } else { "s" },
                        inferred,
                        reconciliation_note
                    ),
                    Color::LightCyan,
                )
            } else if authoritative > 0 {
                (
                    format!(
                        "models.dev authoritative ({} offering{})",
                        authoritative,
                        if authoritative == 1 { "" } else { "s" }
                    ),
                    Color::Gray,
                )
            } else {
                let reason = group.member_provenance.iter().find_map(|provenance| {
                    if let super::app::ModelIdentityProvenance::Unlinked(reason) = provenance {
                        Some(reason.label())
                    } else {
                        None
                    }
                });
                (
                    format!("unlinked — {}", reason.unwrap_or("insufficient evidence")),
                    Color::DarkGray,
                )
            };
            lines.push(Line::from(vec![
                Span::styled("Identity: ", Style::default().fg(label_color)),
                Span::styled(identity, Style::default().fg(color)),
            ]));
        }
    } else if let Some(provenance) = entry.identity {
        let (identity, color) = match provenance {
            super::app::ModelIdentityProvenance::AuthoritativeRef
            | super::app::ModelIdentityProvenance::AuthoritativeDirectId
            | super::app::ModelIdentityProvenance::AuthoritativeScopedId => {
                ("models.dev authoritative".to_string(), Color::Gray)
            }
            super::app::ModelIdentityProvenance::InferredCanonical => {
                ("inferred canonical match".to_string(), Color::LightCyan)
            }
            super::app::ModelIdentityProvenance::InferredQualifiedCanonical => (
                "inferred canonical match (creator-qualified)".to_string(),
                Color::LightCyan,
            ),
            super::app::ModelIdentityProvenance::InferredExactPairCanonical => (
                "inferred canonical match (exact authoritative pair)".to_string(),
                Color::LightCyan,
            ),
            super::app::ModelIdentityProvenance::InferredOneSidedCreatorCanonical => (
                "inferred canonical match (one-sided creator)".to_string(),
                Color::LightCyan,
            ),
            super::app::ModelIdentityProvenance::InferredCrossAliasCanonical => (
                "inferred canonical match (cross-alias)".to_string(),
                Color::LightCyan,
            ),
            super::app::ModelIdentityProvenance::InferredFullIdCanonical => (
                "inferred canonical match (full-id alias)".to_string(),
                Color::LightCyan,
            ),
            super::app::ModelIdentityProvenance::InferredSelfAnchorCanonical => (
                "inferred canonical match (canonical self-anchor)".to_string(),
                Color::LightCyan,
            ),
            super::app::ModelIdentityProvenance::InferredCreatorPrefixedCanonical => (
                "inferred canonical match (creator-prefixed id)".to_string(),
                Color::LightCyan,
            ),
            super::app::ModelIdentityProvenance::InferredPeer => (
                "inferred peer group (not canonical)".to_string(),
                Color::DarkGray,
            ),
            super::app::ModelIdentityProvenance::Unlinked(reason) => {
                (format!("unlinked — {}", reason.label()), Color::DarkGray)
            }
        };
        lines.push(Line::from(vec![
            Span::styled("Identity: ", Style::default().fg(label_color)),
            Span::styled(identity, Style::default().fg(color)),
        ]));
    }
    // Provider is already shown in the Provider card directly above this panel
    // (always the selected model's provider), so it's omitted here to avoid
    // duplication — this row carries Family + optional Status.
    let mut meta_spans = vec![
        Span::styled("Family: ", Style::default().fg(label_color)),
        Span::raw(model.family.clone().unwrap_or_else(|| em.to_string())),
    ];
    if let Some(status) = model.status.as_deref() {
        if status != "active" {
            let status_color = if status == "deprecated" {
                Color::Red
            } else {
                Color::DarkGray
            };
            meta_spans.push(Span::raw("     "));
            meta_spans.push(Span::styled("Status: ", Style::default().fg(label_color)));
            meta_spans.push(Span::styled(
                status.to_string(),
                Style::default().fg(status_color),
            ));
        }
    }
    lines.push(Line::from(meta_spans));

    // Model description (one wrapped line; ~100% coverage in models.dev data).
    // Blank line above separates it from the identity rows.
    if let Some(desc) = model.description.as_deref() {
        if !desc.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                desc.to_string(),
                Style::default().fg(Color::Gray),
            )));
        }
    }

    // Grouped mode: every provider carrying the model, cheapest first (the
    // rest of the detail describes the representative offering).
    if app.models_app.list_mode() == ListMode::Grouped {
        if let Some(g) = app.models_app.current_group() {
            lines.extend(group_providers_section(app, g, width));
        }
    }

    // ── Capabilities ──────────────────────────────────────────────────────
    lines.push(Line::from(""));
    lines.push(section_header_line(width, "Capabilities"));

    let cap_val = |active: bool, color: Color| -> (&'static str, Color) {
        if active {
            ("Yes", color)
        } else {
            ("No", Color::DarkGray)
        }
    };
    // Three-state variant for `Option<bool>` fields (Yes / No / unknown-em-dash).
    let cap_val_opt = |v: Option<bool>, color: Color| -> (&'static str, Color) {
        match v {
            Some(true) => ("Yes", color),
            Some(false) => ("No", Color::DarkGray),
            None => (em, Color::DarkGray),
        }
    };
    let (r_val, r_col) = cap_val(model.reasoning, Color::Cyan);
    let (t_val, t_col) = cap_val(model.tool_call, Color::Yellow);
    let (f_val, f_col) = cap_val(model.attachment, Color::Magenta);
    let (ow_val, ow_col) = if model.open_weights {
        ("Open", Color::Green)
    } else {
        ("Closed", Color::Red)
    };
    let (tmp_val, tmp_col) = cap_val(model.temperature, Color::White);
    // Structured/reasoning-control colors are deliberately distinct from the
    // four RTFO-mirrored fields (Reasoning=Cyan, Tools=Yellow, Files=Magenta,
    // Source=Green/Red) so no single hue stacks up in the grid.
    let (so_val, so_col) = cap_val_opt(model.structured_output, Color::Blue);
    lines.push(two_pair_line(
        LabelValue {
            label: "Reasoning: ",
            value: r_val,
            color: r_col,
        },
        LabelValue {
            label: "Tools: ",
            value: t_val,
            color: t_col,
        },
        col_w,
    ));
    lines.push(two_pair_line(
        LabelValue {
            label: "Source: ",
            value: ow_val,
            color: ow_col,
        },
        LabelValue {
            label: "Files: ",
            value: f_val,
            color: f_col,
        },
        col_w,
    ));
    lines.push(two_pair_line(
        LabelValue {
            label: "Temp: ",
            value: tmp_val,
            color: tmp_col,
        },
        LabelValue {
            label: "Structured: ",
            value: so_val,
            color: so_col,
        },
        col_w,
    ));
    // Reasoning controls — the API knobs for controlling reasoning. Rendered as
    // Label: value pairs in the same 2-column grid as the capabilities above.
    // Each control gets its own non-Cyan hue so the rows read as distinct
    // capabilities rather than a wall of one color. Only present when the model
    // carries reasoning_options.
    let control_color = |label: &str| match label {
        "Budget" => Color::LightGreen,
        "Effort" => Color::LightMagenta,
        "Toggle" => Color::LightBlue,
        _ => Color::Blue,
    };
    let control_cells: Vec<(String, String, Color)> =
        crate::data::reasoning_controls(&model.reasoning_options)
            .into_iter()
            .map(|(label, value)| {
                let color = control_color(&label);
                (format!("{label}: "), value, color)
            })
            .collect();
    for chunk in control_cells.chunks(2) {
        let left = LabelValue {
            label: &chunk[0].0,
            value: &chunk[0].1,
            color: chunk[0].2,
        };
        let right = if chunk.len() > 1 {
            LabelValue {
                label: &chunk[1].0,
                value: &chunk[1].1,
                color: chunk[1].2,
            }
        } else {
            LabelValue {
                label: "",
                value: "",
                color: Color::DarkGray,
            }
        };
        lines.push(two_pair_line(left, right, col_w));
    }

    // ── Pricing ───────────────────────────────────────────────────────────
    lines.push(Line::from(""));
    lines.push(section_header_line(width, "Pricing"));

    let free = model.is_free();
    let cost_color = if free { Color::Green } else { text_color };
    let fmt_cost = |val: Option<f64>| -> (String, Color) {
        match val {
            None => {
                if free {
                    ("Free".to_string(), Color::Green)
                } else {
                    (em.to_string(), Color::DarkGray)
                }
            }
            Some(0.0) => ("$0/M".to_string(), Color::Green),
            Some(v) => (
                format!("{}/M", crate::formatting::format_usd(v)),
                cost_color,
            ),
        }
    };
    let (input_str, input_color) = fmt_cost(model.cost.as_ref().and_then(|c| c.input));
    let (output_str, output_color) = fmt_cost(model.cost.as_ref().and_then(|c| c.output));
    let (cache_read_str, cache_read_color) =
        fmt_cost(model.cost.as_ref().and_then(|c| c.cache_read));
    let (cache_write_str, cache_write_color) =
        fmt_cost(model.cost.as_ref().and_then(|c| c.cache_write));
    lines.push(two_pair_line(
        LabelValue {
            label: "Input: ",
            value: &input_str,
            color: input_color,
        },
        LabelValue {
            label: "Output: ",
            value: &output_str,
            color: output_color,
        },
        col_w,
    ));
    lines.push(two_pair_line(
        LabelValue {
            label: "Cache Read: ",
            value: &cache_read_str,
            color: cache_read_color,
        },
        LabelValue {
            label: "Cache Write: ",
            value: &cache_write_str,
            color: cache_write_color,
        },
        col_w,
    ));

    // Conditional pricing rows — only rendered when the model carries them, so
    // the common case (none of these) leaves Pricing unchanged.
    let cost_ref = model.cost.as_ref();
    let reasoning_cost = cost_ref.and_then(|c| c.reasoning);
    let audio_in = cost_ref.and_then(|c| c.input_audio);
    let audio_out = cost_ref.and_then(|c| c.output_audio);

    if reasoning_cost.is_some() {
        let (rc_str, rc_color) = fmt_cost(reasoning_cost);
        lines.push(two_pair_line(
            LabelValue {
                label: "Thinking: ",
                value: &rc_str,
                color: rc_color,
            },
            LabelValue {
                label: "",
                value: "",
                color: Color::DarkGray,
            },
            col_w,
        ));
    }
    if audio_in.is_some() || audio_out.is_some() {
        let (ai_str, ai_color) = fmt_cost(audio_in);
        let (ao_str, ao_color) = fmt_cost(audio_out);
        lines.push(two_pair_line(
            LabelValue {
                label: "Audio In: ",
                value: &ai_str,
                color: ai_color,
            },
            LabelValue {
                label: "Audio Out: ",
                value: &ao_str,
                color: ao_color,
            },
            col_w,
        ));
    }
    // Tiered pricing (e.g. higher rates above a context threshold): one line per tier.
    if let Some(cost) = cost_ref {
        for t in &cost.tiers {
            let threshold = t
                .tier
                .as_ref()
                .and_then(|ts| ts.size)
                .map(|s| format!("Over {}: ", crate::formatting::format_tokens(s)))
                .unwrap_or_else(|| "Tier: ".to_string());
            let (ti_str, ti_color) = fmt_cost(t.input);
            let (to_str, to_color) = fmt_cost(t.output);
            lines.push(Line::from(vec![
                Span::styled(threshold, Style::default().fg(label_color)),
                Span::styled(ti_str, Style::default().fg(ti_color)),
                Span::styled(" / ", Style::default().fg(Color::DarkGray)),
                Span::styled(to_str, Style::default().fg(to_color)),
            ]));
        }
    }

    // ── Limits ────────────────────────────────────────────────────────────
    lines.push(Line::from(""));
    lines.push(section_header_line(width, "Limits"));

    let ctx_str = model.context_str();
    let inp_lim_str = model.input_limit_str();
    let out_str = model.output_str();
    let (ctx_val, ctx_color) = if ctx_str == "-" {
        (em.to_string(), Color::DarkGray)
    } else {
        (ctx_str, text_color)
    };
    let (inp_lim_val, inp_lim_color) = if inp_lim_str == "-" {
        (em.to_string(), Color::DarkGray)
    } else {
        (inp_lim_str, text_color)
    };
    let (out_val, out_color) = if out_str == "-" {
        (em.to_string(), Color::DarkGray)
    } else {
        (out_str, text_color)
    };
    // Limits uses a 3-pair layout — pack into a single line
    let third_w = (width as usize) / 3;
    let pad_ctx = third_w.saturating_sub("Context: ".len() + ctx_val.len());
    let pad_inp = third_w.saturating_sub("Input: ".len() + inp_lim_val.len());
    lines.push(Line::from(vec![
        Span::styled("Context: ", Style::default().fg(label_color)),
        Span::styled(ctx_val, Style::default().fg(ctx_color)),
        Span::raw(" ".repeat(pad_ctx)),
        Span::styled("Input: ", Style::default().fg(label_color)),
        Span::styled(inp_lim_val, Style::default().fg(inp_lim_color)),
        Span::raw(" ".repeat(pad_inp)),
        Span::styled("Output: ", Style::default().fg(label_color)),
        Span::styled(out_val, Style::default().fg(out_color)),
    ]));

    // ── Modalities ────────────────────────────────────────────────────────
    lines.push(Line::from(""));
    lines.push(section_header_line(width, "Modalities"));

    let (mod_in, mod_out) = match &model.modalities {
        Some(m) => (
            if m.input.is_empty() {
                "text".to_string()
            } else {
                m.input.join(", ")
            },
            if m.output.is_empty() {
                "text".to_string()
            } else {
                m.output.join(", ")
            },
        ),
        None => ("text".to_string(), "text".to_string()),
    };
    lines.push(Line::from(vec![
        Span::styled("Input:  ", Style::default().fg(label_color)),
        Span::styled(mod_in, Style::default().fg(text_color)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Output: ", Style::default().fg(label_color)),
        Span::styled(mod_out, Style::default().fg(text_color)),
    ]));

    // ── Dates ─────────────────────────────────────────────────────────────
    lines.push(Line::from(""));
    lines.push(section_header_line(width, "Dates"));

    let released = model.release_date.as_deref().unwrap_or(em);
    let knowledge = model.knowledge.as_deref().unwrap_or(em);
    let rel_color = if released == em {
        Color::DarkGray
    } else {
        text_color
    };
    let know_color = if knowledge == em {
        Color::DarkGray
    } else {
        text_color
    };
    lines.push(two_pair_line(
        LabelValue {
            label: "Released: ",
            value: released,
            color: rel_color,
        },
        LabelValue {
            label: "Knowledge: ",
            value: knowledge,
            color: know_color,
        },
        col_w,
    ));
    if let Some(updated) = &model.last_updated {
        let upd_color = if is_deprecated {
            Color::DarkGray
        } else {
            text_color
        };
        lines.push(two_pair_line(
            LabelValue {
                label: "Updated: ",
                value: updated,
                color: upd_color,
            },
            LabelValue {
                label: "",
                value: "",
                color: Color::DarkGray,
            },
            col_w,
        ));
    }

    lines
}

fn draw_model_detail(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.models_app.focus == Focus::Details;
    // Inner width for line building (area width minus 2 for borders)
    let inner_w = area.width.saturating_sub(2);
    let lines = model_detail_lines(app, inner_w);
    ScrollablePanel::new("Details", lines, &app.models_app.detail_scroll, focused).render(f, area);
}

/// Unicode-safe truncation with ellipsis for table cells.
pub(super) fn format_filters(filters: &Filters, category: ProviderCategory) -> String {
    let mut active = Vec::new();
    if filters.reasoning {
        active.push("reasoning");
    }
    if filters.tools {
        active.push("tools");
    }
    if filters.open_weights {
        active.push("open");
    }
    if filters.free {
        active.push("free");
    }
    if category != ProviderCategory::All {
        active.push(category.label());
    }
    active.join(", ")
}

#[cfg(test)]
mod mouse_tests {
    //! End-to-end checks for Models-tab mouse handling: render into a
    //! `TestBackend` (which stores the panel rects + clamps list offsets exactly
    //! as the real loop does), then synthesize clicks/scroll and assert the
    //! resulting selection/focus. This is the integration template the
    //! Benchmarks/Agents/Status tabs follow for their own handlers.

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};

    use crate::data::ProvidersMap;
    use crate::tui::app::{App, Tab};
    use crate::tui::models::{handle_models_mouse, Focus, ModelIdentityProvenance};

    /// Two providers; `alpha` has 30 dateless models `m00`..`m29` (so they sort
    /// by id ascending), `beta` has one.
    fn test_app() -> App {
        let mut models = String::new();
        for i in 0..30 {
            models.push_str(&format!(
                r#""m{i:02}": {{ "id": "m{i:02}", "name": "Model {i:02}" }}{}"#,
                if i < 29 { "," } else { "" }
            ));
        }
        let json = format!(
            r#"{{
                "alpha": {{ "id": "alpha", "name": "Alpha", "models": {{ {models} }} }},
                "beta":  {{ "id": "beta",  "name": "Beta",  "models": {{ "b0": {{ "id": "b0", "name": "B0" }} }} }}
            }}"#
        );
        let map: ProvidersMap = serde_json::from_str(&json).expect("valid providers json");
        let mut app = App::new(map, None, None);
        app.current_tab = Tab::Models;
        app
    }

    fn canonical_group_app() -> App {
        let json = r#"{
            "anthropic": { "id": "anthropic", "name": "Anthropic", "models": {
                "claude-opus-5": { "id": "claude-opus-5", "name": "Claude Opus 5" }
            }},
            "amazon-bedrock": { "id": "amazon-bedrock", "name": "Amazon Bedrock", "models": {
                "eu.anthropic.claude-opus-5": { "id": "eu.anthropic.claude-opus-5", "name": "Claude Opus 5 (EU)" }
            }},
            "openrouter": { "id": "openrouter", "name": "OpenRouter", "models": {
                "anthropic/claude-opus-5-fast": { "id": "anthropic/claude-opus-5-fast", "name": "Claude Opus 5 (Fast)" }
            }},
            "venice": { "id": "venice", "name": "Venice", "models": {
                "claude-opus-5-fast": { "id": "claude-opus-5-fast", "name": "Claude Opus 5 Fast" }
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);
        let lab_catalog = crate::labs::LabCatalog::from_test_entries_with_refs(
            &[(
                "anthropic/claude-opus-5",
                "Claude Opus 5",
                Some("claude-opus"),
            )],
            &[
                (
                    "amazon-bedrock/eu.anthropic.claude-opus-5",
                    "anthropic/claude-opus-5",
                ),
                (
                    "openrouter/anthropic/claude-opus-5-fast",
                    "anthropic/claude-opus-5",
                ),
                ("venice/claude-opus-5-fast", "anthropic/claude-opus-5"),
            ],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());
        app.current_tab = Tab::Models;
        app
    }

    /// Switch the app into the flat All view (the `V` toggle) — the mode the
    /// per-offering row/column/dimming tests exercise.
    fn flat(app: &mut App) {
        app.models_app.flat_view = true;
        let providers = app.providers.clone();
        app.models_app.update_filtered_models(&providers);
    }

    fn render(app: &mut App, w: u16, h: u16) {
        render_to_text(app, w, h);
    }

    fn render_to_text(app: &mut App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| crate::tui::ui::draw(f, app))
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                out.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
            out.push('\n');
        }
        out
    }

    fn click(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn scroll(col: u16, row: u16, down: bool) -> MouseEvent {
        MouseEvent {
            kind: if down {
                MouseEventKind::ScrollDown
            } else {
                MouseEventKind::ScrollUp
            },
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn grouped_is_default_and_click_selects_group() {
        let mut app = test_app();
        assert_eq!(
            app.models_app.list_mode(),
            crate::tui::models::ListMode::Grouped
        );
        render(&mut app, 120, 40);
        let area = app.models_app.model_list_area.expect("list rect cached");
        // Rows map 1:1 from the rect top (sticky header sits above it).
        handle_models_mouse(&mut app, click(area.x + 6, area.y + 2));
        assert_eq!(app.models_app.focus, Focus::Models);
        assert_eq!(app.models_app.selected_group, 2);
    }

    #[test]
    fn canonical_base_model_identity_groups_variants_and_drills_all_members() {
        let mut app = canonical_group_app();
        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.name, "Claude Opus 5");
        assert_eq!(group.lab.as_deref(), Some("anthropic"));
        assert_eq!(group.provider_count, 4);
        assert_eq!(group.offering_count, 4);

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        assert_eq!(
            app.models_app.drill_key.as_deref(),
            Some("model:anthropic/claude-opus-5")
        );
        assert_eq!(app.models_app.drill_name.as_deref(), Some("Claude Opus 5"));
        assert_eq!(app.models_app.filtered_models().len(), 4);
    }

    #[test]
    fn inferred_grok_joins_authoritative_group_with_provenance() {
        let json = r#"{
            "xai": {"id":"xai","name":"xAI","models":{
                "grok-4.5":{"id":"grok-4.5","name":"Grok 4.5","family":"grok"}
            }},
            "kenari": {"id":"kenari","name":"Kenari","models":{
                "grok-4-5":{"id":"grok-4-5","name":"Grok 4.5","family":"grok"}
            }},
            "llmgateway": {"id":"llmgateway","name":"LLM Gateway","models":{
                "grok-4-5":{"id":"grok-4-5","name":"Grok 4.5","family":"grok"}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let catalog_providers = map.clone();
        let mut app = App::new(map, None, None);
        let lab_catalog = crate::labs::LabCatalog::from_test_catalog_with_refs(
            &[("xai/grok-4.5", "Grok 4.5", Some("grok"))],
            &catalog_providers,
            &[("kenari/grok-4-5", "xai/grok-4.5")],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.name, "Grok 4.5");
        assert_eq!(group.provider_count, 3);
        assert_eq!(
            group
                .member_provenance
                .iter()
                .filter(|provenance| provenance.is_authoritative())
                .count(),
            2
        );
        assert_eq!(
            group
                .member_provenance
                .iter()
                .filter(|provenance| provenance.is_inferred())
                .count(),
            1
        );
        let text = render_to_text(&mut app, 150, 45);
        assert!(text.contains("2 models.dev links + 1 inferred"));
        assert!(text.contains("≈ LLM Gateway"));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        let llm_gateway = app
            .models_app
            .filtered_models()
            .iter()
            .position(|entry| entry.provider_id == "llmgateway")
            .expect("inferred offering in drill");
        app.models_app.select_model_at_index(llm_gateway);
        let text = render_to_text(&mut app, 150, 45);
        assert!(text.contains("≈ Grok 4.5"));
        assert!(text.contains("Identity: inferred canonical match"));
    }

    #[test]
    fn creator_qualified_fable_joins_canonical_group_with_distinct_provenance() {
        let json = r#"{
            "anthropic": {"id":"anthropic","name":"Anthropic","models":{
                "claude-fable-5":{"id":"claude-fable-5","name":"Claude Fable 5","modalities":{"output":["text"]}}
            }},
            "digitalocean": {"id":"digitalocean","name":"DigitalOcean","models":{
                "anthropic-claude-fable-5":{"id":"anthropic-claude-fable-5","name":"Anthropic Claude Fable 5","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let catalog_providers = map.clone();
        let mut app = App::new(map, None, None);
        let lab_catalog = crate::labs::LabCatalog::from_test_catalog_with_refs(
            &[(
                "anthropic/claude-fable-5",
                "Claude Fable 5",
                Some("claude-fable"),
            )],
            &catalog_providers,
            &[("anthropic/claude-fable-5", "anthropic/claude-fable-5")],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.provider_count, 2);
        assert!(group.member_provenance.iter().any(|provenance| matches!(
            provenance,
            ModelIdentityProvenance::InferredQualifiedCanonical
        )));
        let text = render_to_text(&mut app, 200, 45);
        assert!(text.contains("1 models.dev link + 1 inferred"));
        assert!(text.contains("1 creator-qualified"));
        assert!(text.contains("≈ DigitalOcean"));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        let digitalocean = app
            .models_app
            .filtered_models()
            .iter()
            .position(|entry| entry.provider_id == "digitalocean")
            .expect("creator-qualified offering in drill");
        app.models_app.select_model_at_index(digitalocean);
        let text = render_to_text(&mut app, 150, 45);
        assert!(text.contains("Identity: inferred canonical match (creator-qualified)"));
    }

    #[test]
    fn cross_alias_nemotron_joins_canonical_group_with_distinct_provenance() {
        let json = r#"{
            "wandb":{"id":"wandb","name":"W&B","models":{
                "nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B":{"id":"nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B","name":"Nemotron 3 Ultra","modalities":{"output":["text"]}}
            }},
            "llmgateway":{"id":"llmgateway","name":"LLM Gateway","models":{
                "nemotron-3-ultra-550b":{"id":"nemotron-3-ultra-550b","name":"Nemotron 3 Ultra 550B A55B","modalities":{"output":["text"]}}
            }},
            "digitalocean":{"id":"digitalocean","name":"DigitalOcean","models":{
                "nemotron-3-ultra-550b":{"id":"nemotron-3-ultra-550b","name":"Nemotron 3 Ultra","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let catalog_providers = map.clone();
        let mut app = App::new(map, None, None);
        let target = "nvidia/nemotron-3-ultra-550b-a55b";
        let lab_catalog = crate::labs::LabCatalog::from_test_catalog_with_refs(
            &[(target, "Nemotron 3 Ultra 550B A55B", Some("nemotron"))],
            &catalog_providers,
            &[
                ("wandb/nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B", target),
                ("llmgateway/nemotron-3-ultra-550b", target),
            ],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.name, "Nemotron 3 Ultra 550B A55B");
        assert_eq!(group.provider_count, 3);
        assert!(group.member_provenance.iter().any(|provenance| matches!(
            provenance,
            ModelIdentityProvenance::InferredCrossAliasCanonical
        )));
        let text = render_to_text(&mut app, 180, 45);
        assert!(text.contains("2 models.dev links + 1 inferred (1 cross-alias)"));
        assert!(text.contains("≈ DigitalOcean"));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        let digitalocean = app
            .models_app
            .filtered_models()
            .iter()
            .position(|entry| entry.provider_id == "digitalocean")
            .expect("cross-alias offering in drill");
        app.models_app.select_model_at_index(digitalocean);
        let text = render_to_text(&mut app, 160, 45);
        assert!(text.contains("Identity: inferred canonical match (cross-alias)"));
    }

    #[test]
    fn full_id_alias_joins_canonical_group_with_distinct_provenance() {
        let json = r#"{
            "google":{"id":"google","name":"Google","models":{
                "gemini-3.1-flash-image-preview":{"id":"gemini-3.1-flash-image-preview","name":"Nano Banana 2","modalities":{"output":["image"]}}
            }},
            "candidate":{"id":"candidate","name":"Candidate","models":{
                "gemini-3.1-flash-image-preview":{"id":"gemini-3.1-flash-image-preview","name":"Gemini 3.1 Flash Image Preview","modalities":{"output":["image"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let catalog_providers = map.clone();
        let mut app = App::new(map, None, None);
        let target = "google/gemini-3.1-flash-image-preview";
        let lab_catalog = crate::labs::LabCatalog::from_test_catalog_with_refs(
            &[(target, "Nano Banana 2", None)],
            &catalog_providers,
            &[],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.name, "Nano Banana 2");
        assert_eq!(group.provider_count, 2);
        assert!(group.member_provenance.iter().any(|provenance| matches!(
            provenance,
            ModelIdentityProvenance::InferredFullIdCanonical
        )));
        let text = render_to_text(&mut app, 180, 45);
        assert!(text.contains("1 models.dev link + 1 inferred (1 full-id alias)"));
        assert!(text.contains("≈ Candidate"));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        let candidate = app
            .models_app
            .filtered_models()
            .iter()
            .position(|entry| entry.provider_id == "candidate")
            .expect("full-id offering in drill");
        app.models_app.select_model_at_index(candidate);
        let text = render_to_text(&mut app, 160, 45);
        assert!(text.contains("Identity: inferred canonical match (full-id alias)"));
    }

    #[test]
    fn self_anchor_member_renders_as_inferred() {
        let json = r#"{
            "sap-ai-core":{"id":"sap-ai-core","name":"SAP AI Core","models":{
                "anthropic--claude-3.7-sonnet":{"id":"anthropic--claude-3.7-sonnet","name":"Anthropic Claude 3.7 Sonnet","modalities":{"output":["text"]}}
            }},
            "abacus":{"id":"abacus","name":"Abacus","models":{
                "claude-3-7-sonnet-20250219":{"id":"claude-3-7-sonnet-20250219","name":"Claude Sonnet 3.7","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let catalog_providers = map.clone();
        let mut app = App::new(map, None, None);
        let target = "anthropic/claude-3-7-sonnet-20250219";
        let lab_catalog = crate::labs::LabCatalog::from_test_catalog_with_refs(
            &[(target, "Claude Sonnet 3.7", Some("claude-sonnet"))],
            &catalog_providers,
            &[("sap-ai-core/anthropic--claude-3.7-sonnet", target)],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.name, "Claude Sonnet 3.7");
        assert_eq!(group.provider_count, 2);
        assert!(group.member_provenance.iter().any(|provenance| matches!(
            provenance,
            ModelIdentityProvenance::InferredSelfAnchorCanonical
        )));
        let text = render_to_text(&mut app, 180, 45);
        assert!(text.contains("1 models.dev link + 1 inferred (1 self-anchor)"));
        assert!(text.contains("≈ Abacus"));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        let member = app
            .models_app
            .filtered_models()
            .iter()
            .position(|entry| entry.provider_id == "abacus")
            .expect("self-anchored offering in drill");
        app.models_app.select_model_at_index(member);
        let text = render_to_text(&mut app, 160, 45);
        assert!(text.contains("Identity: inferred canonical match (canonical self-anchor)"));
    }

    #[test]
    fn creator_prefixed_member_renders_as_inferred() {
        let json = r#"{
            "alibaba":{"id":"alibaba","name":"Alibaba","models":{
                "qwen3-32b":{"id":"qwen3-32b","name":"Qwen3 32B","modalities":{"output":["text"]}}
            }},
            "digitalocean":{"id":"digitalocean","name":"DigitalOcean","models":{
                "alibaba-qwen3-32b":{"id":"alibaba-qwen3-32b","name":"Qwen3-32B","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let catalog_providers = map.clone();
        let mut app = App::new(map, None, None);
        let lab_catalog = crate::labs::LabCatalog::from_test_catalog_with_refs(
            &[("alibaba/qwen3-32b", "Qwen3 32B", None)],
            &catalog_providers,
            &[],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.name, "Qwen3 32B");
        assert_eq!(group.provider_count, 2);
        assert!(group.member_provenance.iter().any(|provenance| matches!(
            provenance,
            ModelIdentityProvenance::InferredCreatorPrefixedCanonical
        )));
        let text = render_to_text(&mut app, 180, 45);
        assert!(text.contains("1 models.dev link + 1 inferred (1 creator-prefixed id)"));
        assert!(text.contains("≈ DigitalOcean"));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        let member = app
            .models_app
            .filtered_models()
            .iter()
            .position(|entry| entry.provider_id == "digitalocean")
            .expect("creator-prefixed offering in drill");
        app.models_app.select_model_at_index(member);
        let text = render_to_text(&mut app, 160, 45);
        assert!(text.contains("Identity: inferred canonical match (creator-prefixed id)"));
    }

    #[test]
    fn compatible_unlinked_offerings_form_peer_group_and_drill_together() {
        let json = r#"{
            "alpha": {"id":"alpha","name":"Alpha","models":{
                "orphan-2.0":{"id":"orphan-2.0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }},
            "beta": {"id":"beta","name":"Beta","models":{
                "orphan-2-0":{"id":"orphan-2-0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.provider_count, 2);
        assert!(group.member_provenance.iter().all(|provenance| matches!(
            provenance,
            crate::tui::models::app::ModelIdentityProvenance::InferredPeer
        )));
        let text = render_to_text(&mut app, 140, 40);
        assert!(text.contains("≈ Orphan 2.0"));
        assert!(text.contains("inferred peer group (not canonical)"));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        assert_eq!(app.models_app.filtered_models().len(), 2);
        let text = render_to_text(&mut app, 140, 40);
        assert!(text.contains("≈ Orphan 2.0"));
        assert!(text.contains("Identity: inferred peer group (not canonical)"));
    }

    #[test]
    fn flattened_creator_namespace_forms_conservative_peer_groups() {
        let json = r#"{
            "openrouter": {"id":"openrouter","name":"OpenRouter","models":{
                "aion-labs/aion-3.0":{"id":"aion-labs/aion-3.0","name":"Aion-3.0","modalities":{"output":["text"]}},
                "aion-labs/aion-3.0-mini":{"id":"aion-labs/aion-3.0-mini","name":"Aion-3.0-Mini","modalities":{"output":["text"]}}
            }},
            "venice": {"id":"venice","name":"Venice AI","models":{
                "aion-labs-aion-3-0":{"id":"aion-labs-aion-3-0","name":"Aion 3.0","modalities":{"output":["text"]}},
                "aion-labs-aion-3-0-mini":{"id":"aion-labs-aion-3-0-mini","name":"Aion 3.0 Mini","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);

        assert_eq!(app.models_app.groups.len(), 2);
        assert!(app.models_app.groups.iter().all(|group| {
            group.provider_count == 2
                && group.offering_count == 2
                && group
                    .member_provenance
                    .iter()
                    .all(|provenance| matches!(provenance, ModelIdentityProvenance::InferredPeer))
        }));
        let text = render_to_text(&mut app, 140, 40);
        assert!(text.contains("≈ Aion 3.0"));
        assert!(text.contains("≈ Aion 3.0 Mini"));
    }

    #[test]
    fn full_id_fallback_does_not_erase_creator_tokens() {
        let json = r#"{
            "alpha": {"id":"alpha","name":"Alpha","models":{
                "creator-a/orphan-2.0":{"id":"creator-a/orphan-2.0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }},
            "beta": {"id":"beta","name":"Beta","models":{
                "creator-b-orphan-2-0":{"id":"creator-b-orphan-2-0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let app = App::new(map, None, None);

        assert_eq!(app.models_app.groups.len(), 2);
        assert!(app
            .models_app
            .groups
            .iter()
            .all(|group| group.provider_count == 1));
    }

    #[test]
    fn compact_separator_fallback_preserves_matching_creator_tokens() {
        let json = r#"{
            "frogbot": {"id":"frogbot","name":"Frogbot","models":{
                "zai-glm-5-1":{"id":"zai-glm-5-1","name":"Z.AI GLM-5.1","modalities":{"output":["text"]}}
            }},
            "kilo": {"id":"kilo","name":"Kilo Gateway","models":{
                "z-ai/glm-5.1":{"id":"z-ai/glm-5.1","name":"Z.ai: GLM 5.1","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let app = App::new(map, None, None);

        assert_eq!(app.models_app.groups.len(), 1);
        let group = &app.models_app.groups[0];
        assert_eq!(group.provider_count, 2);
        assert_eq!(group.offering_count, 2);
        assert!(group
            .member_provenance
            .iter()
            .all(|provenance| matches!(provenance, ModelIdentityProvenance::InferredPeer)));
    }

    #[test]
    fn peer_grouping_rejects_creator_and_output_conflicts() {
        let json = r#"{
            "creator-a": {"id":"creator-a","name":"Creator A","models":{
                "openai/orphan-2.0":{"id":"openai/orphan-2.0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }},
            "creator-b": {"id":"creator-b","name":"Creator B","models":{
                "anthropic/orphan-2-0":{"id":"anthropic/orphan-2-0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }},
            "modality-a": {"id":"modality-a","name":"Modality A","models":{
                "media-1.0":{"id":"media-1.0","name":"Media 1.0","modalities":{"output":["text"]}}
            }},
            "modality-b": {"id":"modality-b","name":"Modality B","models":{
                "media-1-0":{"id":"media-1-0","name":"Media 1.0","modalities":{"output":["image"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);
        let lab_catalog = crate::labs::LabCatalog::from_test_entries_with_refs(
            &[
                ("openai/dummy", "OpenAI Dummy", None),
                ("anthropic/dummy", "Anthropic Dummy", None),
            ],
            &[],
        );
        app.models_app
            .set_lab_catalog(lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.groups.len(), 4);
        assert!(app
            .models_app
            .groups
            .iter()
            .all(|group| group.provider_count == 1));
    }

    #[test]
    fn peer_conflict_cannot_be_filtered_away() {
        let json = r#"{
            "alpha": {"id":"alpha","name":"Alpha","models":{
                "orphan-2.0":{"id":"orphan-2.0","name":"Orphan 2.0","reasoning":true,"modalities":{"output":["text"]}}
            }},
            "beta": {"id":"beta","name":"Beta","models":{
                "orphan-2-0":{"id":"orphan-2-0","name":"Orphan 2.0","reasoning":true,"modalities":{"output":["text"]}}
            }},
            "gamma": {"id":"gamma","name":"Gamma","models":{
                "orphan.2.0":{"id":"orphan.2.0","name":"Orphan 2.0","reasoning":false,"modalities":{"output":["image"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);

        assert_eq!(app.models_app.groups.len(), 3);
        app.models_app.filters.reasoning = true;
        app.models_app
            .update_filtered_models(&app.providers.clone());

        assert_eq!(app.models_app.filtered_models().len(), 2);
        assert_eq!(app.models_app.groups.len(), 2);
        assert!(app
            .models_app
            .filtered_models()
            .iter()
            .all(|entry| matches!(entry.identity, Some(ModelIdentityProvenance::Unlinked(_)))));
    }

    #[test]
    fn provider_scope_projects_snapshot_peer_provenance() {
        let json = r#"{
            "alpha": {"id":"alpha","name":"Alpha","models":{
                "orphan-2.0":{"id":"orphan-2.0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }},
            "beta": {"id":"beta","name":"Beta","models":{
                "orphan-2-0":{"id":"orphan-2-0","name":"Orphan 2.0","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);
        let alpha_picker_index = app
            .models_app
            .provider_list_items
            .iter()
            .position(|item| match item {
                crate::tui::models::app::ProviderListItem::Provider(index, _) => {
                    app.providers[*index].0 == "alpha"
                }
                _ => false,
            })
            .expect("alpha provider picker row");

        app.models_app
            .select_provider_at_index(alpha_picker_index, &app.providers.clone());

        assert_eq!(app.models_app.filtered_models().len(), 1);
        assert!(matches!(
            app.models_app.filtered_models()[0].identity,
            Some(ModelIdentityProvenance::InferredPeer)
        ));
        assert!(app.models_app.groups.is_empty());
    }

    /// The relaxed peer lane: an unlinked offering whose exact leaf id
    /// matches an existing peer bucket joins it when its name differs only by
    /// creator attribution, its own id-namespace spelling, or tokens the
    /// shared leaf id already carries. Semantic tokens from nowhere refuse.
    #[test]
    fn relaxed_peer_lane_joins_attributed_spellings_and_refuses_semantics() {
        let json = r#"{
            "alpha": {"id":"alpha","name":"Alpha","models":{
                "meta-llama/llama-9-3b-instruct":{"id":"meta-llama/llama-9-3b-instruct","name":"Llama 9 3B Instruct","modalities":{"output":["text"]}}
            }},
            "beta": {"id":"beta","name":"Beta","models":{
                "meta/llama-9-3b-instruct":{"id":"meta/llama-9-3b-instruct","name":"Llama 9 3B Instruct","modalities":{"output":["text"]}}
            }},
            "gamma": {"id":"gamma","name":"Gamma","models":{
                "meta-llama/llama-9-3b-instruct":{"id":"meta-llama/llama-9-3b-instruct","name":"Meta: Llama 9 3B Instruct","modalities":{"output":["text"]}}
            }},
            "zeta": {"id":"zeta","name":"Zeta","models":{
                "TEE/llama-9-3b-instruct":{"id":"TEE/llama-9-3b-instruct","name":"Llama 9 3B Instruct TEE","modalities":{"output":["text"]}}
            }},
            "delta": {"id":"delta","name":"Delta","models":{
                "meta-llama/llama-9-3b-instruct":{"id":"meta-llama/llama-9-3b-instruct","name":"Llama 9 3B Instruct Preview","modalities":{"output":["text"]}}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);
        let catalog = crate::labs::LabCatalog::from_test_entries_with_refs(
            &[("meta/llama-0-0b", "Llama 0 0B", Some("llama"))],
            &[],
        );
        app.models_app
            .set_lab_catalog(catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        // alpha+beta form the exact bucket; gamma (creator-attributed name)
        // and zeta (own TEE namespace echoed in the name) relax into it;
        // delta's "Preview" is semantic and stays a singleton.
        assert_eq!(app.models_app.groups.len(), 2);
        let peer_group = app
            .models_app
            .groups
            .iter()
            .find(|group| group.key.starts_with("peer:"))
            .expect("relaxed peer group");
        assert_eq!(peer_group.provider_count, 4);
        assert_eq!(peer_group.offering_count, 4);
        let delta = app
            .models_app
            .filtered_models()
            .iter()
            .find(|entry| entry.provider_id == "delta")
            .expect("delta entry");
        assert!(matches!(
            delta.identity,
            Some(ModelIdentityProvenance::Unlinked(_))
        ));
    }

    /// Two neutral-compatible buckets sharing one leaf id are ambiguity —
    /// the joiner must stay out rather than pick one.
    #[test]
    fn relaxed_peer_lane_fails_closed_on_ambiguous_buckets() {
        let json = r#"{
            "p1": {"id":"p1","name":"P1","models":{
                "acme/z-a-b":{"id":"acme/z-a-b","name":"Z A"}
            }},
            "p2": {"id":"p2","name":"P2","models":{
                "acme/z-a-b":{"id":"acme/z-a-b","name":"Z A"}
            }},
            "p3": {"id":"p3","name":"P3","models":{
                "acme/z-a-b":{"id":"acme/z-a-b","name":"Z B"}
            }},
            "p4": {"id":"p4","name":"P4","models":{
                "acme/z-a-b":{"id":"acme/z-a-b","name":"Z B"}
            }},
            "p5": {"id":"p5","name":"P5","models":{
                "acme/z-a-b":{"id":"acme/z-a-b","name":"Z"}
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);
        let catalog = crate::labs::LabCatalog::from_test_entries_with_refs(
            &[("acme/other-model", "Other Model", None)],
            &[],
        );
        app.models_app
            .set_lab_catalog(catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        // Two 2-provider peer groups plus the refused "Z" singleton.
        assert_eq!(app.models_app.groups.len(), 3);
        let p5 = app
            .models_app
            .filtered_models()
            .iter()
            .find(|entry| entry.provider_id == "p5")
            .expect("p5 entry");
        assert!(matches!(
            p5.identity,
            Some(ModelIdentityProvenance::Unlinked(_))
        ));
    }

    /// Read-only live distribution receipt for the same snapshot the TUI
    /// consumes. Kept separate from ordinary tests so CI never requires the
    /// network; `mise run audit-model-identity` invokes it explicitly.
    #[test]
    #[ignore = "live models.dev grouping distribution audit"]
    fn live_grouping_distribution_and_grok_consolidation() {
        let catalog = crate::api::fetch_catalog().expect("fetch live catalog");
        let mut app = App::new(catalog.providers, None, None);
        app.models_app
            .set_lab_catalog(catalog.lab_catalog, &app.providers.clone());
        app.models_app
            .update_filtered_models(&app.providers.clone());

        let mut authoritative = 0usize;
        let mut inferred_canonical = 0usize;
        let mut inferred_qualified = 0usize;
        let mut inferred_exact_pair = 0usize;
        let mut inferred_one_sided = 0usize;
        let mut inferred_cross = 0usize;
        let mut inferred_full_id = 0usize;
        let mut inferred_self_anchor = 0usize;
        let mut inferred_creator_prefixed = 0usize;
        let mut creator_prefixed_leaf_key = 0usize;
        let mut creator_prefixed_full_key = 0usize;
        let mut inferred_peer = 0usize;
        let mut unlinked = 0usize;
        let mut canonical_groups = 0usize;
        let mut peer_groups = 0usize;
        let mut unlinked_groups = 0usize;
        for group in &app.models_app.groups {
            let mut has_canonical = false;
            let mut has_peer = false;
            for provenance in &group.member_provenance {
                match provenance {
                    ModelIdentityProvenance::AuthoritativeRef
                    | ModelIdentityProvenance::AuthoritativeDirectId
                    | ModelIdentityProvenance::AuthoritativeScopedId => {
                        authoritative += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredCanonical => {
                        inferred_canonical += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredQualifiedCanonical => {
                        inferred_qualified += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredExactPairCanonical => {
                        inferred_exact_pair += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredOneSidedCreatorCanonical => {
                        inferred_one_sided += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredCrossAliasCanonical => {
                        inferred_cross += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredFullIdCanonical => {
                        inferred_full_id += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredSelfAnchorCanonical => {
                        inferred_self_anchor += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredCreatorPrefixedCanonical => {
                        inferred_creator_prefixed += 1;
                        has_canonical = true;
                    }
                    ModelIdentityProvenance::InferredPeer => {
                        inferred_peer += 1;
                        has_peer = true;
                    }
                    ModelIdentityProvenance::Unlinked(_) => unlinked += 1,
                }
            }
            if has_canonical {
                canonical_groups += 1;
            } else if has_peer {
                peer_groups += 1;
            } else {
                unlinked_groups += 1;
            }
        }

        println!(
            "live grouping: {} rows = {canonical_groups} canonical + {peer_groups} peer + {unlinked_groups} unlinked; offerings = {authoritative} authoritative + {inferred_canonical} anchored + {inferred_qualified} dual creator + {inferred_exact_pair} exact-pair + {inferred_one_sided} one-sided creator + {inferred_cross} cross-alias + {inferred_full_id} full-id + {inferred_self_anchor} self-anchor + {inferred_creator_prefixed} creator-prefixed + {inferred_peer} peer + {unlinked} unlinked",
            app.models_app.groups.len()
        );

        let mut nemotron_cross = false;
        for entry in app.models_app.filtered_models() {
            let Some(evidence) = app.models_app.reconciliation_evidence(entry) else {
                continue;
            };
            if entry.provider_id == "digitalocean"
                && entry.id == "nemotron-3-ultra-550b"
                && evidence.id == "nvidia/nemotron-3-ultra-550b-a55b"
                && matches!(
                    evidence.kind,
                    crate::labs::CanonicalResolutionKind::InferredCrossAliasCanonical
                )
            {
                nemotron_cross = true;
            }
            match evidence.creator_prefixed_key {
                Some("leaf") => creator_prefixed_leaf_key += 1,
                Some(_) => creator_prefixed_full_key += 1,
                None => {}
            }
            println!(
                "reconciled {:?}{}: {}/{} ({}) -> {} / {} / {} [{} pair, {} name, {} leaf-id, {} full-id witnesses]",
                evidence.kind,
                evidence
                    .creator_prefixed_key
                    .map(|branch| format!(" [{branch} key]"))
                    .unwrap_or_default(),
                entry.provider_id,
                entry.id,
                entry.model.name,
                evidence.id,
                evidence.name,
                evidence.lab,
                evidence.pair_witnesses,
                evidence.name_witnesses,
                evidence.id_witnesses,
                evidence.full_id_witnesses
            );
        }
        // The creator-prefixed lane's full-id key branch had no live firing
        // when it shipped; its own counter makes a first one visible.
        println!(
            "active reconciliation: {inferred_exact_pair} exact-pair + {inferred_one_sided} one-sided creator + {inferred_cross} cross-alias + {inferred_full_id} full-id + {inferred_creator_prefixed} creator-prefixed ({creator_prefixed_leaf_key} leaf key, {creator_prefixed_full_key} full key)"
        );
        assert!(
            inferred_exact_pair + inferred_one_sided + inferred_cross + inferred_full_id > 0,
            "live audit must exercise active reconciliation"
        );
        assert!(
            nemotron_cross,
            "DigitalOcean Nemotron must join its canonical group through cross-alias evidence"
        );

        let nemotron_groups: Vec<_> = app
            .models_app
            .groups
            .iter()
            .filter(|group| group.name == "Nemotron 3 Ultra 550B A55B")
            .collect();
        assert_eq!(
            nemotron_groups.len(),
            1,
            "Nemotron 3 Ultra must have one canonical grouped row"
        );
        let nemotron_group = nemotron_groups[0];
        let has_digitalocean_cross_alias = nemotron_group
            .member_indices
            .iter()
            .zip(&nemotron_group.member_provenance)
            .any(|(&index, provenance)| {
                let entry = &app.models_app.filtered_models()[index];
                entry.provider_id == "digitalocean"
                    && entry.id == "nemotron-3-ultra-550b"
                    && matches!(
                        provenance,
                        ModelIdentityProvenance::InferredCrossAliasCanonical
                    )
            });
        println!(
            "Nemotron 3 Ultra group: {} providers, {} offerings, provenance {:?}",
            nemotron_group.provider_count,
            nemotron_group.offering_count,
            nemotron_group.member_provenance
        );
        assert!(
            has_digitalocean_cross_alias,
            "the canonical Nemotron group must contain the DigitalOcean cross-alias offering"
        );

        for group in &app.models_app.groups {
            let qualified_members: Vec<_> = group
                .member_indices
                .iter()
                .zip(&group.member_provenance)
                .filter_map(|(&index, provenance)| {
                    matches!(
                        provenance,
                        ModelIdentityProvenance::InferredQualifiedCanonical
                    )
                    .then(|| {
                        let entry = &app.models_app.filtered_models()[index];
                        format!("{}/{}", entry.provider_id, entry.id)
                    })
                })
                .collect();
            if !qualified_members.is_empty() {
                println!(
                    "creator-qualified canonical: {} <- {}",
                    group.name,
                    qualified_members.join(", ")
                );
            }
        }

        for group in app
            .models_app
            .groups
            .iter()
            .filter(|group| group.key.starts_with("peer:compact:"))
        {
            println!(
                "compact-id peer: {} = {} providers / {} offerings",
                group.name, group.provider_count, group.offering_count
            );
        }

        let grok_groups: Vec<_> = app
            .models_app
            .groups
            .iter()
            .filter(|group| group.name == "Grok 4.5")
            .collect();
        for group in &grok_groups {
            println!(
                "Grok 4.5 group: {} providers, {} offerings, provenance {:?}",
                group.provider_count, group.offering_count, group.member_provenance
            );
        }
        assert!(
            grok_groups.iter().any(|group| {
                group.provider_count >= 13
                    && group.member_provenance.iter().any(|provenance| {
                        matches!(provenance, ModelIdentityProvenance::InferredCanonical)
                    })
            }),
            "Grok 4.5 must include the inferred LLM Gateway offering"
        );

        let fable_groups: Vec<_> = app
            .models_app
            .groups
            .iter()
            .filter(|group| group.name == "Claude Fable 5")
            .collect();
        for group in &fable_groups {
            println!(
                "Claude Fable 5 group: {} providers, {} offerings, provenance {:?}",
                group.provider_count, group.offering_count, group.member_provenance
            );
        }
        assert!(
            fable_groups.iter().any(|group| {
                group.provider_count >= 22
                    && group.member_provenance.iter().any(|provenance| {
                        matches!(
                            provenance,
                            ModelIdentityProvenance::InferredQualifiedCanonical
                        )
                    })
            }),
            "Claude Fable 5 must include creator-qualified DigitalOcean"
        );

        let aion_groups: Vec<_> = app
            .models_app
            .groups
            .iter()
            .filter(|group| group.name == "Aion 3.0" || group.name == "Aion 3.0 Mini")
            .collect();
        for group in &aion_groups {
            println!(
                "{} group: {} providers, {} offerings, provenance {:?}",
                group.name, group.provider_count, group.offering_count, group.member_provenance
            );
        }
        assert_eq!(
            aion_groups.len(),
            2,
            "Aion 3.0 and Mini should each have one grouped row"
        );
        assert!(
            aion_groups
                .iter()
                .all(|group| group.provider_count == 2 && group.offering_count == 2),
            "each Aion row should contain OpenRouter and Venice"
        );
    }

    #[test]
    fn unresolved_provider_spellings_remain_independent_offerings() {
        let json = r#"{
            "alpha": { "id": "alpha", "name": "Alpha", "models": {
                "a": { "id": "a", "name": "Qwen3.7 Max" }
            }},
            "beta": { "id": "beta", "name": "Beta", "models": {
                "b": { "id": "b", "name": "Qwen3.7 Max" }
            }},
            "gamma": { "id": "gamma", "name": "Gamma", "models": {
                "c": { "id": "c", "name": "Qwen 3.7-Max" }
            }},
            "delta": { "id": "delta", "name": "Delta", "models": {
                "d": { "id": "d", "name": "Qwen: Qwen3.7 Max" }
            }}
        }"#;
        let map: ProvidersMap = serde_json::from_str(json).expect("valid providers json");
        let mut app = App::new(map, None, None);
        assert_eq!(app.models_app.groups.len(), 4);
        assert!(app
            .models_app
            .groups
            .iter()
            .all(|group| group.provider_count == 1 && group.offering_count == 1));

        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        assert_eq!(app.models_app.filtered_models().len(), 1);
    }

    #[test]
    fn enter_drills_into_group_and_esc_pops_back() {
        let mut app = test_app();
        // All 31 models have unique names -> 31 groups; drill into the first.
        let name = app.models_app.groups[0].name.clone();
        let providers = app.providers.clone();
        app.models_app.enter_selection(&providers);
        assert_eq!(
            app.models_app.list_mode(),
            crate::tui::models::ListMode::Offerings
        );
        assert_eq!(app.models_app.drill_name.as_deref(), Some(name.as_str()));
        assert_eq!(app.models_app.filtered_models().len(), 1);
        assert!(app.models_app.escape_back(&providers));
        assert_eq!(
            app.models_app.list_mode(),
            crate::tui::models::ListMode::Grouped
        );
        // Top-level Esc is not consumed (falls through to search clearing).
        assert!(!app.models_app.escape_back(&providers));

        // Selection survives a drill round-trip.
        app.models_app.select_model_at_index(7);
        assert_eq!(app.models_app.selected_group, 7);
        let name7 = app.models_app.groups[7].name.clone();
        app.models_app.enter_selection(&providers);
        assert_eq!(app.models_app.drill_name.as_deref(), Some(name7.as_str()));
        assert!(app.models_app.escape_back(&providers));
        assert_eq!(app.models_app.selected_group, 7, "selection preserved");
    }

    #[test]
    fn provider_picker_filters_and_applies_scope() {
        let mut app = test_app();
        app.models_app.open_provider_picker();
        for c in "bet".chars() {
            app.models_app.picker_query.push(c);
        }
        let providers = app.providers.clone();
        let rows = app.models_app.picker_rows(&providers);
        // "All" + the one matching provider (beta).
        assert_eq!(rows.len(), 2);
        app.models_app.picker_selected = 1;
        app.models_app.apply_picker_selection(&providers);
        assert!(!app.models_app.show_provider_picker);
        assert!(!app.models_app.is_all_selected());
        assert_eq!(
            app.models_app.list_mode(),
            crate::tui::models::ListMode::Flat
        );
        assert_eq!(app.models_app.filtered_models().len(), 1); // beta's b0
                                                               // Esc pops the provider scope back to All-grouped.
        assert!(app.models_app.escape_back(&providers));
        assert!(app.models_app.is_all_selected());
    }

    /// The reported bug: as a list item, the column header scrolled away and
    /// never returned after G-then-g (ratatui only scrolls the *selected*
    /// item into view, and item 0 was never selectable). Sticky header fix:
    /// it must be present in every frame regardless of scroll position.
    #[test]
    fn header_stays_visible_after_jump_to_bottom_and_back() {
        let mut app = test_app();
        // Short viewport so the 31-group list actually scrolls.
        app.models_app.select_last_model();
        let text = render_to_text(&mut app, 120, 12);
        assert!(
            text.lines().any(|l| l.contains("RTFO")),
            "header visible while scrolled to the bottom"
        );
        app.models_app.select_first_model();
        let text = render_to_text(&mut app, 120, 12);
        assert!(
            text.lines().any(|l| l.contains("RTFO")),
            "header visible after jumping back to the top"
        );
    }

    #[test]
    fn grouped_header_shows_group_columns() {
        let mut app = test_app();
        let header = list_header_row(&render_to_text(&mut app, 175, 45));
        assert!(header.contains("Model"));
        assert!(header.contains("Lab"));
        assert!(header.contains("Providers"));
        assert!(header.contains("Input"));
        assert!(header.contains("Context"));
    }

    #[test]
    fn click_model_row_at_top_selects_that_model() {
        let mut app = test_app();
        flat(&mut app);
        render(&mut app, 120, 40);
        let area = app.models_app.model_list_area.expect("model rect cached");
        // The cached rect is the bare row region — the sticky column header
        // sits ABOVE it, so rows map 1:1 from the rect's top.
        handle_models_mouse(&mut app, click(area.x + 6, area.y));
        assert_eq!(app.models_app.focus, Focus::Models);
        assert_eq!(app.models_app.selected_model, 0);
        handle_models_mouse(&mut app, click(area.x + 6, area.y + 2));
        assert_eq!(app.models_app.selected_model, 2);
        // Clicking the sticky header (one row above the rect) selects nothing.
        handle_models_mouse(&mut app, click(area.x + 6, area.y - 1));
        assert_eq!(app.models_app.selected_model, 2); // unchanged
    }

    #[test]
    fn click_model_row_with_nonzero_scroll_offset() {
        // Short viewport forces the list to scroll once selection nears the end.
        let mut app = test_app();
        flat(&mut app);
        // Drive selection deep so the model list scrolls.
        for _ in 0..25 {
            app.models_app.next_model();
        }
        render(&mut app, 120, 20);
        let area = app.models_app.model_list_area.expect("model rect cached");
        let offset = app.models_app.model_list_state.offset();
        assert!(offset > 0, "list should have scrolled (offset={offset})");
        // Click two rows below the top visible row: model `offset + 2`
        // (rows map 1:1 — the sticky header sits above the cached rect).
        handle_models_mouse(&mut app, click(area.x + 6, area.y + 2));
        assert_eq!(app.models_app.selected_model, offset + 2);
    }

    #[test]
    fn scroll_wheel_over_model_list_focuses_and_moves() {
        let mut app = test_app();
        flat(&mut app);
        render(&mut app, 120, 40);
        let area = app.models_app.model_list_area.expect("model rect cached");
        assert_eq!(app.models_app.selected_model, 0);
        handle_models_mouse(&mut app, scroll(area.x + 6, area.y + 5, true));
        assert_eq!(app.models_app.focus, Focus::Models);
        assert_eq!(app.models_app.selected_model, 1); // moved down one
        handle_models_mouse(&mut app, scroll(area.x + 6, area.y + 5, false));
        assert_eq!(app.models_app.selected_model, 0); // moved back up
    }

    #[test]
    fn click_detail_panel_focuses_details_only() {
        let mut app = test_app();
        render(&mut app, 120, 40);
        let area = app
            .models_app
            .model_detail_area
            .expect("detail rect cached");
        let before = app.models_app.selected_model;
        handle_models_mouse(&mut app, click(area.x + 2, area.y + 2));
        assert_eq!(app.models_app.focus, Focus::Details);
        assert_eq!(app.models_app.selected_model, before); // no row selection
    }

    #[test]
    fn header_click_switches_tab() {
        let mut app = test_app();
        render(&mut app, 120, 40);
        // "Agents" label sits at x 10..16 on the header row (row 0).
        assert!(matches!(crate::tui::ui::tab_at(11, 0), Some(Tab::Agents)));
    }

    /// Extract the model-list header row (the one carrying "RTFO").
    fn list_header_row(text: &str) -> String {
        text.lines()
            .find(|l| l.contains("RTFO"))
            .expect("model list header row")
            .to_string()
    }

    #[test]
    fn wide_render_keeps_all_columns() {
        let mut app = test_app();
        flat(&mut app);
        let header = list_header_row(&render_to_text(&mut app, 175, 45));
        assert!(header.contains("Provider"));
        assert!(header.contains("Input"));
        assert!(header.contains("Output"));
        assert!(header.contains("Context"));
    }

    #[test]
    fn narrow_render_keeps_provider_sheds_numerics() {
        let mut app = test_app();
        flat(&mut app);
        // 100 total cols -> the list is 60% (inner ~58): after the 18-char
        // name minimum, Provider (highest keep-priority — it disambiguates
        // duplicate rows) plus Input/Output fit; Context sheds cleanly.
        let header = list_header_row(&render_to_text(&mut app, 100, 40));
        assert!(header.contains("Provider"));
        assert!(header.contains("Input"));
        assert!(header.contains("Output"));
        assert!(
            !header.contains("Contex"),
            "Context must drop cleanly, not clip: {header:?}"
        );
    }

    #[test]
    fn narrow_render_sort_column_survives_drop() {
        let mut app = test_app();
        flat(&mut app);
        app.models_app.sort_order = crate::tui::models::SortOrder::Context;
        app.models_app
            .update_filtered_models(&app.providers.clone());
        let header = list_header_row(&render_to_text(&mut app, 100, 40));
        // Context replaces the last kept column (Output) instead of dropping.
        assert!(header.contains("Context"), "sort column must survive");
        assert!(header.contains("Provider"));
        assert!(!header.contains("Output"));
    }

    #[test]
    fn very_narrow_render_keeps_name_only() {
        let mut app = test_app();
        flat(&mut app);
        // Inner list width < NAME_MIN + the narrowest column: everything
        // sheds; the name takes the full width, nothing clips.
        let header = list_header_row(&render_to_text(&mut app, 48, 40));
        assert!(header.contains("Model"));
        assert!(!header.contains("Provider"));
        assert!(!header.contains("Input"));
        assert!(!header.contains("Output"));
        assert!(!header.contains("Context"));
    }

    /// Read-only live near-miss report over the residual rows (peer groups +
    /// unlinked singletons) left behind by every active identity lane. Fuzzy
    /// similarity here only PROPOSES candidates for human review or upstream
    /// models.dev `base_model` contributions — nothing in this report merges
    /// rows, and the resolver never consumes its output.
    ///
    /// Sections:
    ///   1. same-name seam — residual rows sharing a normalized display name
    ///      across different groups (their ids disagree beyond every lane)
    ///   2. same-id seam — residual rows sharing a normalized leaf id across
    ///      different groups (their names disagree)
    ///   3. canonical near-misses — the best-scoring fuzzy candidate per
    ///      residual group against the canonical registry
    ///   4. residual near-misses — high-similarity cross-provider residual
    ///      pairs that no lane grouped
    #[test]
    #[ignore = "live models.dev residual near-miss candidate report"]
    fn live_residual_near_miss_report() {
        use std::collections::{BTreeMap, BTreeSet, HashMap};

        use crate::labs::{identity_fingerprint, model_id_fingerprint, outputs_are_disjoint};
        use crate::tui::models::app::ModelEntry;

        const BUCKET_CAP: usize = 40;
        const CANDIDATE_CAP: usize = 60;
        const CANDIDATE_SCORE_FLOOR: f64 = 0.55;
        const PAIR_SCORE_FLOOR: f64 = 0.62;

        fn toks(fp: &str) -> BTreeSet<String> {
            fp.split('/')
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect()
        }

        fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
            let union = a.union(b).count();
            if union == 0 {
                return 0.0;
            }
            a.intersection(b).count() as f64 / union as f64
        }

        fn only_in(a: &BTreeSet<String>, b: &BTreeSet<String>) -> String {
            let extra: Vec<&str> = a.difference(b).map(String::as_str).collect();
            if extra.is_empty() {
                "-".into()
            } else {
                extra.join("+")
            }
        }

        fn jaro_winkler(a: &str, b: &str) -> f64 {
            let a: Vec<char> = a.chars().collect();
            let b: Vec<char> = b.chars().collect();
            if a.is_empty() && b.is_empty() {
                return 1.0;
            }
            if a.is_empty() || b.is_empty() {
                return 0.0;
            }
            let window = (a.len().max(b.len()) / 2).saturating_sub(1);
            let mut b_used = vec![false; b.len()];
            let mut a_matches = Vec::new();
            for (i, ca) in a.iter().enumerate() {
                let lo = i.saturating_sub(window);
                let hi = (i + window + 1).min(b.len());
                for (j, used) in b_used.iter_mut().enumerate().take(hi).skip(lo) {
                    if !*used && b[j] == *ca {
                        *used = true;
                        a_matches.push((j, *ca));
                        break;
                    }
                }
            }
            if a_matches.is_empty() {
                return 0.0;
            }
            let m = a_matches.len() as f64;
            let mut b_matches: Vec<(usize, char)> = a_matches.clone();
            b_matches.sort_by_key(|(j, _)| *j);
            let transpositions = a_matches
                .iter()
                .zip(&b_matches)
                .filter(|((_, ca), (_, cb))| ca != cb)
                .count() as f64
                / 2.0;
            let jaro = (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions) / m) / 3.0;
            let prefix = a
                .iter()
                .zip(&b)
                .take(4)
                .take_while(|(ca, cb)| ca == cb)
                .count() as f64;
            jaro + prefix * 0.1 * (1.0 - jaro)
        }

        #[derive(serde::Deserialize)]
        struct ReadCanonical {
            #[serde(default)]
            name: String,
            #[serde(default)]
            modalities: Option<crate::data::Modalities>,
        }
        #[derive(serde::Deserialize)]
        struct ReadCatalog {
            providers: ProvidersMap,
            models: HashMap<String, ReadCanonical>,
        }
        #[derive(serde::Deserialize)]
        struct LabsView {
            providers: ProvidersMap,
            models: HashMap<String, crate::labs::CanonicalModel>,
        }

        // One body, deserialized twice, so every view shares one coherent
        // snapshot (catalog.json is a moving target).
        let body = reqwest::blocking::get("https://models.dev/catalog.json")
            .expect("fetch live catalog")
            .text()
            .expect("read live catalog body");
        let read: ReadCatalog = serde_json::from_str(&body).expect("parse read view");
        let labs_view: LabsView = serde_json::from_str(&body).expect("parse labs view");
        let probe = crate::labs::LabCatalog::from_catalog(&labs_view.models, &labs_view.providers);
        let app_catalog =
            crate::labs::LabCatalog::from_catalog(&labs_view.models, &labs_view.providers);

        let mut app = App::new(read.providers.clone(), None, None);
        let providers = app.providers.clone();
        app.models_app.set_lab_catalog(app_catalog, &providers);
        app.models_app.update_filtered_models(&providers);

        let entries = app.models_app.filtered_models();
        let mut group_key_of: HashMap<usize, &str> = HashMap::new();
        for group in &app.models_app.groups {
            for &idx in &group.member_indices {
                group_key_of.insert(idx, group.key.as_str());
            }
        }
        let residual: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    entry.identity,
                    Some(
                        ModelIdentityProvenance::InferredPeer
                            | ModelIdentityProvenance::Unlinked(_)
                    )
                )
            })
            .map(|(idx, _)| idx)
            .collect();
        assert!(!residual.is_empty(), "audit needs residual rows to profile");

        let rejection = |entry: &ModelEntry| match &entry.identity {
            Some(ModelIdentityProvenance::Unlinked(reason)) => reason.label(),
            Some(ModelIdentityProvenance::InferredPeer) => "peer-grouped",
            _ => "?",
        };

        struct Canon {
            id: String,
            name: String,
            lab: String,
            name_toks: BTreeSet<String>,
            leaf_toks: BTreeSet<String>,
            outputs: Vec<String>,
        }
        let canon: Vec<Canon> = read
            .models
            .iter()
            .filter(|(_, m)| !m.name.is_empty())
            .map(|(cid, m)| Canon {
                id: cid.clone(),
                name: m.name.clone(),
                lab: cid.split('/').next().unwrap_or_default().to_string(),
                name_toks: toks(&identity_fingerprint(&m.name)),
                leaf_toks: toks(&model_id_fingerprint(cid)),
                outputs: m
                    .modalities
                    .as_ref()
                    .map(|modalities| modalities.output.clone())
                    .unwrap_or_default(),
            })
            .collect();
        let mut canon_by_name_fp: HashMap<String, Vec<usize>> = HashMap::new();
        let mut canon_by_leaf_fp: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, c) in canon.iter().enumerate() {
            canon_by_name_fp
                .entry(identity_fingerprint(&c.name))
                .or_default()
                .push(idx);
            canon_by_leaf_fp
                .entry(model_id_fingerprint(&c.id))
                .or_default()
                .push(idx);
        }

        let entry_outputs = |idx: usize| -> &[String] {
            entries[idx]
                .model
                .modalities
                .as_ref()
                .map(|modalities| modalities.output.as_slice())
                .unwrap_or_default()
        };
        let bucket_blockers = |idxs: &[usize]| -> String {
            let creators: BTreeSet<&str> = idxs
                .iter()
                .filter_map(|&i| probe.independent_lab(&entries[i].provider_id, &entries[i].id))
                .collect();
            let outputs_conflict = idxs.iter().enumerate().any(|(pos, &left)| {
                idxs.iter()
                    .skip(pos + 1)
                    .any(|&right| outputs_are_disjoint(entry_outputs(left), entry_outputs(right)))
            });
            format!(
                "creators={{{}}} outputs-conflict={}",
                creators.into_iter().collect::<Vec<_>>().join(","),
                if outputs_conflict { "YES" } else { "no" }
            )
        };

        // ---- Section 1: same normalized name, different groups -------------
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &idx in &residual {
            let fp = identity_fingerprint(&entries[idx].model.name);
            if !fp.is_empty() {
                by_name.entry(fp).or_default().push(idx);
            }
        }
        let mut same_name: Vec<(&String, &Vec<usize>)> = by_name
            .iter()
            .filter(|(_, idxs)| {
                let groups: BTreeSet<&str> = idxs.iter().map(|i| group_key_of[i]).collect();
                let provs: BTreeSet<&str> = idxs
                    .iter()
                    .map(|&i| entries[i].provider_id.as_str())
                    .collect();
                groups.len() >= 2 && provs.len() >= 2
            })
            .collect();
        same_name.sort_by_key(|(fp, idxs)| (std::cmp::Reverse(idxs.len()), (*fp).clone()));
        let same_name_offerings: usize = same_name.iter().map(|(_, idxs)| idxs.len()).sum();
        println!(
            "== seam 1: same name / different ids — {} buckets, {} offerings ==",
            same_name.len(),
            same_name_offerings
        );
        for (fp, idxs) in same_name.iter().take(BUCKET_CAP) {
            let display = &entries[idxs[0]].model.name;
            let canon_matches: Vec<&str> = canon_by_name_fp
                .get(*fp)
                .map(|c| c.iter().map(|&ci| canon[ci].id.as_str()).collect())
                .unwrap_or_default();
            let common: BTreeSet<String> = idxs
                .iter()
                .map(|&i| toks(&model_id_fingerprint(&entries[i].id)))
                .reduce(|acc, t| acc.intersection(&t).cloned().collect())
                .unwrap_or_default();
            println!(
                "[name] \"{display}\" — {} offerings / {} groups; canonical name-match: {}; {}",
                idxs.len(),
                idxs.iter()
                    .map(|i| group_key_of[i])
                    .collect::<BTreeSet<_>>()
                    .len(),
                if canon_matches.is_empty() {
                    "none".to_string()
                } else {
                    canon_matches.join(", ")
                },
                bucket_blockers(idxs)
            );
            for &idx in idxs.iter() {
                let entry = &entries[idx];
                let leaf = toks(&model_id_fingerprint(&entry.id));
                println!(
                    "    {}/{}  distinct-id-tokens=[{}]  ({})",
                    entry.provider_id,
                    entry.id,
                    only_in(&leaf, &common),
                    rejection(entry)
                );
            }
        }
        if same_name.len() > BUCKET_CAP {
            println!(
                "(+{} more same-name buckets suppressed)",
                same_name.len() - BUCKET_CAP
            );
        }

        // ---- Section 2: same normalized leaf id, different groups ----------
        let mut by_leaf: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &idx in &residual {
            let fp = model_id_fingerprint(&entries[idx].id);
            if !fp.is_empty() {
                by_leaf.entry(fp).or_default().push(idx);
            }
        }
        let mut same_id: Vec<(&String, &Vec<usize>)> = by_leaf
            .iter()
            .filter(|(_, idxs)| {
                let groups: BTreeSet<&str> = idxs.iter().map(|i| group_key_of[i]).collect();
                let provs: BTreeSet<&str> = idxs
                    .iter()
                    .map(|&i| entries[i].provider_id.as_str())
                    .collect();
                groups.len() >= 2 && provs.len() >= 2
            })
            .collect();
        same_id.sort_by_key(|(fp, idxs)| (std::cmp::Reverse(idxs.len()), (*fp).clone()));
        let same_id_offerings: usize = same_id.iter().map(|(_, idxs)| idxs.len()).sum();
        println!(
            "== seam 2: same leaf id / different names — {} buckets, {} offerings ==",
            same_id.len(),
            same_id_offerings
        );
        for (fp, idxs) in same_id.iter().take(BUCKET_CAP) {
            let canon_matches: Vec<&str> = canon_by_leaf_fp
                .get(*fp)
                .map(|c| c.iter().map(|&ci| canon[ci].id.as_str()).collect())
                .unwrap_or_default();
            let common: BTreeSet<String> = idxs
                .iter()
                .map(|&i| toks(&identity_fingerprint(&entries[i].model.name)))
                .reduce(|acc, t| acc.intersection(&t).cloned().collect())
                .unwrap_or_default();
            println!(
                "[id] {fp} — {} offerings / {} groups; canonical id-match: {}; {}",
                idxs.len(),
                idxs.iter()
                    .map(|i| group_key_of[i])
                    .collect::<BTreeSet<_>>()
                    .len(),
                if canon_matches.is_empty() {
                    "none".to_string()
                } else {
                    canon_matches.join(", ")
                },
                bucket_blockers(idxs)
            );
            for &idx in idxs.iter() {
                let entry = &entries[idx];
                let name = toks(&identity_fingerprint(&entry.model.name));
                println!(
                    "    {}/{} \"{}\"  distinct-name-tokens=[{}]  ({})",
                    entry.provider_id,
                    entry.id,
                    entry.model.name,
                    only_in(&name, &common),
                    rejection(entry)
                );
            }
        }
        if same_id.len() > BUCKET_CAP {
            println!(
                "(+{} more same-id buckets suppressed)",
                same_id.len() - BUCKET_CAP
            );
        }

        // ---- Residual group representatives for the fuzzy sections ---------
        struct Rep {
            idx: usize,
            name_fp: String,
            leaf_fp: String,
            name_toks: BTreeSet<String>,
            leaf_toks: BTreeSet<String>,
            providers: BTreeSet<String>,
        }
        let mut rep_members: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &idx in &residual {
            rep_members.entry(group_key_of[&idx]).or_default().push(idx);
        }
        let reps: Vec<Rep> = rep_members
            .values()
            .map(|idxs| {
                let idx = idxs[0];
                let name_fp = identity_fingerprint(&entries[idx].model.name);
                let leaf_fp = model_id_fingerprint(&entries[idx].id);
                Rep {
                    idx,
                    name_toks: toks(&name_fp),
                    leaf_toks: toks(&leaf_fp),
                    name_fp,
                    leaf_fp,
                    providers: idxs
                        .iter()
                        .map(|&i| entries[i].provider_id.clone())
                        .collect(),
                }
            })
            .collect();

        // ---- Section 3: best fuzzy canonical candidate per residual group --
        struct CanonCandidate {
            score: f64,
            rep_idx: usize,
            canon_idx: usize,
            name_extra_res: String,
            name_extra_canon: String,
            id_extra_res: String,
            id_extra_canon: String,
            blockers: String,
        }
        let mut canon_candidates: Vec<CanonCandidate> = Vec::new();
        for rep in &reps {
            let entry = &entries[rep.idx];
            let name_str = rep.name_fp.replace('/', " ");
            let mut best: Option<CanonCandidate> = None;
            for (canon_idx, c) in canon.iter().enumerate() {
                let jn = jaccard(&rep.name_toks, &c.name_toks);
                if jn < 0.34 {
                    continue;
                }
                let ji = jaccard(&rep.leaf_toks, &c.leaf_toks);
                let jw = jaro_winkler(&name_str, &identity_fingerprint(&c.name).replace('/', " "));
                let score = 0.45 * jn + 0.35 * ji + 0.20 * jw;
                if score < CANDIDATE_SCORE_FLOOR && (jn - 1.0).abs() > f64::EPSILON {
                    continue;
                }
                if best.as_ref().is_some_and(|b| b.score >= score) {
                    continue;
                }
                let creator_conflict = probe
                    .independent_lab(&entry.provider_id, &entry.id)
                    .is_some_and(|lab| lab != c.lab);
                let output_conflict = outputs_are_disjoint(entry_outputs(rep.idx), &c.outputs);
                let mut blockers = Vec::new();
                if creator_conflict {
                    blockers.push("CREATOR");
                }
                if output_conflict {
                    blockers.push("OUTPUTS");
                }
                best = Some(CanonCandidate {
                    score,
                    rep_idx: rep.idx,
                    canon_idx,
                    name_extra_res: only_in(&rep.name_toks, &c.name_toks),
                    name_extra_canon: only_in(&c.name_toks, &rep.name_toks),
                    id_extra_res: only_in(&rep.leaf_toks, &c.leaf_toks),
                    id_extra_canon: only_in(&c.leaf_toks, &rep.leaf_toks),
                    blockers: if blockers.is_empty() {
                        "-".into()
                    } else {
                        blockers.join("+")
                    },
                });
            }
            if let Some(candidate) = best {
                canon_candidates.push(candidate);
            }
        }
        canon_candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        println!(
            "== fuzzy 3: residual -> canonical candidates — {} scored ==",
            canon_candidates.len()
        );
        for candidate in canon_candidates.iter().take(CANDIDATE_CAP) {
            let entry = &entries[candidate.rep_idx];
            let c = &canon[candidate.canon_idx];
            println!(
                "{:.3}  {}/{} \"{}\" -> {} \"{}\"  name±[res:{} canon:{}] id±[res:{} canon:{}] blockers[{}] ({})",
                candidate.score,
                entry.provider_id,
                entry.id,
                entry.model.name,
                c.id,
                c.name,
                candidate.name_extra_res,
                candidate.name_extra_canon,
                candidate.id_extra_res,
                candidate.id_extra_canon,
                candidate.blockers,
                rejection(entry)
            );
        }
        if canon_candidates.len() > CANDIDATE_CAP {
            println!(
                "(+{} more canonical candidates suppressed)",
                canon_candidates.len() - CANDIDATE_CAP
            );
        }

        // ---- Section 4: high-similarity residual pairs no lane grouped -----
        let mut token_index: HashMap<&str, Vec<usize>> = HashMap::new();
        for (rep_pos, rep) in reps.iter().enumerate() {
            for token in &rep.name_toks {
                token_index.entry(token.as_str()).or_default().push(rep_pos);
            }
        }
        let mut pair_candidates: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        for positions in token_index.values() {
            for (left_pos, &left) in positions.iter().enumerate() {
                for &right in positions.iter().skip(left_pos + 1) {
                    let (a, b) = (&reps[left.min(right)], &reps[right.max(left)]);
                    let key = (left.min(right), right.max(left));
                    if pair_candidates.contains_key(&key) {
                        continue;
                    }
                    // Same-name and same-id buckets are seams 1/2; a pair
                    // sharing one single-provider origin can never peer-group.
                    if a.name_fp == b.name_fp
                        || a.leaf_fp == b.leaf_fp
                        || (a.providers.len() == 1 && a.providers == b.providers)
                    {
                        continue;
                    }
                    let jn = jaccard(&a.name_toks, &b.name_toks);
                    if jn < 0.5 {
                        continue;
                    }
                    let ji = jaccard(&a.leaf_toks, &b.leaf_toks);
                    let jw =
                        jaro_winkler(&a.name_fp.replace('/', " "), &b.name_fp.replace('/', " "));
                    let score = 0.45 * jn + 0.35 * ji + 0.20 * jw;
                    if score >= PAIR_SCORE_FLOOR {
                        pair_candidates.insert(key, score);
                    }
                }
            }
        }
        let mut pairs: Vec<((usize, usize), f64)> = pair_candidates.into_iter().collect();
        pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        println!(
            "== fuzzy 4: residual pair candidates — {} scored ==",
            pairs.len()
        );
        for ((left, right), score) in pairs.iter().take(CANDIDATE_CAP) {
            let a = &entries[reps[*left].idx];
            let b = &entries[reps[*right].idx];
            println!(
                "{score:.3}  {}/{} \"{}\"  <->  {}/{} \"{}\"  name±[l:{} r:{}] id±[l:{} r:{}]",
                a.provider_id,
                a.id,
                a.model.name,
                b.provider_id,
                b.id,
                b.model.name,
                only_in(&reps[*left].name_toks, &reps[*right].name_toks),
                only_in(&reps[*right].name_toks, &reps[*left].name_toks),
                only_in(&reps[*left].leaf_toks, &reps[*right].leaf_toks),
                only_in(&reps[*right].leaf_toks, &reps[*left].leaf_toks),
            );
        }
        if pairs.len() > CANDIDATE_CAP {
            println!(
                "(+{} more pair candidates suppressed)",
                pairs.len() - CANDIDATE_CAP
            );
        }

        println!(
            "near-miss summary: {} residual offerings in {} groups; seam1 {} buckets/{} offerings; seam2 {} buckets/{} offerings; {} canonical candidates; {} pair candidates",
            residual.len(),
            reps.len(),
            same_name.len(),
            same_name_offerings,
            same_id.len(),
            same_id_offerings,
            canon_candidates.len(),
            pairs.len()
        );
    }
}
