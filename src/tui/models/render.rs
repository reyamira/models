use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::app::{Filters, Focus, ProviderListItem, SortOrder};
use crate::formatting::truncate;
use crate::formatting::EM_DASH;
use crate::provider_category::{provider_category, ProviderCategory};
use crate::tui::app::App;
use crate::tui::ui::{caret, focus_border};
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
    // 3-column layout: providers 20% | models 45% | right panel 35%
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(45),
            Constraint::Percentage(35),
        ])
        .split(area);

    draw_providers(f, chunks[0], app);
    draw_models(f, chunks[1], app);
    draw_right_panel(f, chunks[2], app);
}

fn draw_providers(f: &mut Frame, area: Rect, app: &mut App) {
    let is_focused = app.models_app.focus == Focus::Providers;
    let border_style = focus_border(is_focused);

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Providers ");
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Split inner area into filter row + list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner_area);

    // Filter toggles row
    let cat_active = app.models_app.provider_category_filter != ProviderCategory::All;
    let cat_color = if cat_active {
        app.models_app.provider_category_filter.color()
    } else {
        Color::DarkGray
    };
    let grp_color = if app.models_app.group_by_category {
        Color::Green
    } else {
        Color::DarkGray
    };

    let cat_label = if cat_active {
        app.models_app.provider_category_filter.short_label()
    } else {
        "Cat"
    };

    let filter_line = Line::from(vec![
        Span::styled("[5]", Style::default().fg(cat_color)),
        Span::raw(format!(" {} ", cat_label)),
        Span::styled("[6]", Style::default().fg(grp_color)),
        Span::raw(" Grp"),
    ]);
    f.render_widget(Paragraph::new(filter_line), chunks[0]);

    // Build items list from provider_list_items
    let mut items: Vec<ListItem> = Vec::with_capacity(app.models_app.provider_list_items.len());

    for item in &app.models_app.provider_list_items {
        match item {
            ProviderListItem::All => {
                let count = app.models_app.filtered_model_count();
                let text = format!("All ({})", count);
                items.push(ListItem::new(text).style(Style::default().fg(Color::Green)));
            }
            ProviderListItem::CategoryHeader(cat) => {
                let label = cat.label();
                let color = cat.color();
                // Create a separator line like "── Origin ──────"
                let avail = inner_area.width.saturating_sub(2) as usize; // account for highlight symbol space
                let label_len = label.len() + 4; // "── " + label + " "
                let trailing = if avail > label_len {
                    "\u{2500}".repeat(avail - label_len)
                } else {
                    String::new()
                };
                let text = format!("\u{2500}\u{2500} {} {}", label, trailing);
                items.push(
                    ListItem::new(text)
                        .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
                );
            }
            ProviderListItem::Provider(idx, count) => {
                if let Some((id, _)) = app.providers.get(*idx) {
                    let cat = provider_category(id);
                    let initial = &cat.short_label()[..1];
                    let color = cat.color();
                    let line = Line::from(vec![
                        Span::styled(initial, Style::default().fg(color)),
                        Span::raw(format!(" {} ", id)),
                        Span::styled(format!("({})", count), Style::default().fg(Color::Gray)),
                    ]);
                    items.push(ListItem::new(line));
                }
            }
        }
    }

    let caret = caret(is_focused);
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(caret);

    // Cache the bare list rect (no border, no filter row) for mouse hit-testing.
    app.models_app.provider_list_area = Some(chunks[1]);
    f.render_stateful_widget(list, chunks[1], &mut app.models_app.provider_list_state);
}

fn draw_models(f: &mut Frame, area: Rect, app: &mut App) {
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

    // Show provider name in title when a specific provider is selected
    let provider_label = app
        .models_app
        .selected_provider_data(&app.providers)
        .map(|(_, p)| p.name.as_str())
        .unwrap_or("Models");

    let title = if app.models_app.search_query.is_empty() && filter_indicator.is_empty() {
        format!(" {} ({}){} ", provider_label, models.len(), sort_indicator)
    } else if app.models_app.search_query.is_empty() {
        format!(
            " {} ({}){} [{}] ",
            provider_label,
            models.len(),
            sort_indicator,
            filter_indicator
        )
    } else if filter_indicator.is_empty() {
        format!(
            " {} ({}) [/{}]{} ",
            provider_label,
            models.len(),
            app.models_app.search_query,
            sort_indicator
        )
    } else {
        format!(
            " {} ({}) [/{}] [{}]{} ",
            provider_label,
            models.len(),
            app.models_app.search_query,
            filter_indicator,
            sort_indicator
        )
    };

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Fixed column widths: caret(2) + caps(5) + Input(8) Output(8) Context(8) + gaps(3)
    let caret_w: u16 = 2;
    let caps_w: u16 = 5; // "RTFO " — 4 indicator chars + 1 space
    let input_w: u16 = 8;
    let output_w: u16 = 8;
    let ctx_w: u16 = 8;
    let num_gaps: u16 = 3;
    let fixed_w = caret_w + caps_w + input_w + output_w + ctx_w + num_gaps;
    let name_width = (inner_area.width.saturating_sub(fixed_w) as usize).max(10);

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
            format!("{:<width$}", "Model ID", width = name_width),
            if sort_col == "name" {
                active_header_style
            } else {
                header_style
            },
        ),
    ];
    header_spans.push(Span::styled(format!(" {:>8}", "Input"), cost_style));
    header_spans.push(Span::styled(format!(" {:>8}", "Output"), cost_style));
    header_spans.push(Span::styled(
        format!(" {:>8}", "Context"),
        if sort_col == "context" {
            active_header_style
        } else {
            header_style
        },
    ));

    // Build items with header row
    let mut items: Vec<ListItem> = Vec::with_capacity(models.len() + 1);
    items.push(ListItem::new(Line::from(header_spans)));

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
                    truncate(&entry.id, name_width.saturating_sub(1)),
                    width = name_width
                ),
                style,
            ),
        ];
        row_spans.push(Span::styled(format!(" {:>8}", input_cost), style));
        row_spans.push(Span::styled(format!(" {:>8}", output_cost), style));
        row_spans.push(Span::styled(format!(" {:>8}", ctx), style));

        items.push(ListItem::new(Line::from(row_spans)));
    }

    let list = List::new(items);
    // Cache the list rect and render into the real state so its post-render
    // `offset()` (clamped to the viewport) is available for mouse hit-testing.
    app.models_app.model_list_area = Some(inner_area);
    f.render_stateful_widget(list, inner_area, &mut app.models_app.model_list_state);
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
    let (so_val, so_col) = cap_val_opt(model.structured_output, Color::Cyan);
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
    // Reasoning-mode summary — only when the model carries reasoning_options.
    if let Some(summary) = model.reasoning_mode_summary() {
        lines.push(Line::from(vec![
            Span::styled("Mode: ", Style::default().fg(label_color)),
            Span::styled(summary, Style::default().fg(text_color)),
        ]));
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
            Some(v) => {
                let formatted = if v.fract() == 0.0 {
                    format!("${}/M", v as u64)
                } else {
                    format!("${:.2}/M", v)
                };
                (formatted, cost_color)
            }
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
                label: "Reasoning: ",
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
    use crate::tui::models::{handle_models_mouse, Focus};

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

    fn render(app: &mut App, w: u16, h: u16) {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| crate::tui::ui::draw(f, app))
            .expect("draw");
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
    fn click_provider_row_selects_and_focuses() {
        let mut app = test_app();
        render(&mut app, 120, 40);
        let area = app
            .models_app
            .provider_list_area
            .expect("provider rect cached");
        // Row 0 of the list = "All"; row 1 = first real provider (alpha).
        handle_models_mouse(&mut app, click(area.x + 1, area.y + 1));
        assert_eq!(app.models_app.focus, Focus::Providers);
        assert_eq!(app.models_app.selected_provider, 1); // index 0 is "All"
    }

    #[test]
    fn click_model_row_at_top_selects_that_model() {
        let mut app = test_app();
        render(&mut app, 120, 40);
        let area = app.models_app.model_list_area.expect("model rect cached");
        // Item 0 is the column header at area.y; first model is one row below.
        handle_models_mouse(&mut app, click(area.x + 6, area.y + 1));
        assert_eq!(app.models_app.focus, Focus::Models);
        assert_eq!(app.models_app.selected_model, 0);
        // Clicking the header row itself selects nothing new.
        handle_models_mouse(&mut app, click(area.x + 6, area.y + 3));
        assert_eq!(app.models_app.selected_model, 2);
        handle_models_mouse(&mut app, click(area.x + 6, area.y)); // header
        assert_eq!(app.models_app.selected_model, 2); // unchanged
    }

    #[test]
    fn click_model_row_with_nonzero_scroll_offset() {
        // Short viewport forces the list to scroll once selection nears the end.
        let mut app = test_app();
        // Drive selection deep so the model list scrolls (header item 0 leaves view).
        for _ in 0..25 {
            app.models_app.next_model();
        }
        render(&mut app, 120, 20);
        let area = app.models_app.model_list_area.expect("model rect cached");
        let offset = app.models_app.model_list_state.offset();
        assert!(offset > 0, "list should have scrolled (offset={offset})");
        // Click two rows below the top visible row. Top visible list-item index is
        // `offset`; +2 rows → item `offset+2` → model `offset+1`.
        handle_models_mouse(&mut app, click(area.x + 6, area.y + 2));
        let expected_model = offset + 2 - 1; // -1 for the header item at index 0
        assert_eq!(app.models_app.selected_model, expected_model);
    }

    #[test]
    fn scroll_wheel_over_model_list_focuses_and_moves() {
        let mut app = test_app();
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
}
