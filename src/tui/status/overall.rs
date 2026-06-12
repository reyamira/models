use crate::status::ProviderHealth;
use crate::tui::ui::{status_health_icon, status_health_style};
use crate::tui::widgets::scroll_offset::ScrollOffset;
use crate::tui::widgets::scrollable_panel::ScrollablePanel;
use crate::tui::widgets::soft_card::SoftCard;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use super::render::{
    component_only_scope_title, component_status_style, incident_impact_style,
    incident_stage_style, incident_status_value, incident_time_value, overall_attention_components,
    overall_attention_entries, provider_health_label, provider_last_meaningful_update,
    push_component_scope_lines, push_overall_caveat, push_panel_empty_state,
    push_plain_scope_lines, push_wrapped_bullet_lines, status_field_label_style,
    status_section_label_style, update_duplicates_summary_or_issue,
};

fn format_relative_time_from_instant(instant: std::time::Instant) -> String {
    let elapsed = instant.elapsed();
    let secs = elapsed.as_secs();

    match secs {
        0..=4 => "just now".to_string(),
        5..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

fn overall_freshness_suffix(status_app: &super::app::StatusApp) -> String {
    if status_app.loading {
        return "Refreshing…".to_string();
    }

    let freshness = status_app
        .last_refreshed
        .map(format_relative_time_from_instant)
        .map(|value| format!("Updated {value}"))
        .unwrap_or_else(|| "Waiting for refresh".to_string());

    if status_app.last_error.is_some() {
        format!("{freshness} · Last refresh failed")
    } else {
        freshness
    }
}

fn build_incidents_panel_cards(
    entries: &[&crate::status::ProviderStatus],
    body_width: usize,
) -> Vec<SoftCard> {
    let mut cards = Vec::new();

    for entry in entries.iter() {
        let incidents = entry.active_incidents();
        let non_op_components = overall_attention_components(entry);
        let incident = incidents[0];
        let summary = entry.provider_summary_text();

        let mut card_lines = Vec::new();

        card_lines.push(Line::from(vec![
            Span::styled(
                status_health_icon(entry.health),
                status_health_style(entry.health),
            ),
            Span::raw(" "),
            Span::styled(
                entry.display_name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));

        card_lines.push(Line::from(vec![
            Span::styled("  Issue: ", status_field_label_style()),
            Span::styled(
                incident.name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));

        let mut metadata_spans = vec![Span::raw("  ")];
        metadata_spans.push(Span::styled("Status: ", status_field_label_style()));
        metadata_spans.push(Span::styled(
            incident_status_value(incident),
            incident_stage_style(&incident.status),
        ));

        let impact_lower = incident.impact.to_lowercase();
        if !impact_lower.is_empty() && impact_lower != "none" {
            metadata_spans.push(Span::raw("  "));
            metadata_spans.push(Span::styled("Impact: ", status_field_label_style()));
            metadata_spans.push(Span::styled(
                incident.impact.clone(),
                incident_impact_style(&incident.impact),
            ));
        }

        if let Some((label, value)) = incident_time_value(entry, incident) {
            metadata_spans.push(Span::raw("  "));
            metadata_spans.push(Span::styled(
                format!("{label}: "),
                status_field_label_style(),
            ));
            metadata_spans.push(Span::styled(value, Style::default().fg(Color::Cyan)));
        }
        card_lines.push(Line::from(metadata_spans));

        if incidents.len() > 1 {
            card_lines.push(Line::from(vec![
                Span::styled("  Additional incidents: ", status_field_label_style()),
                Span::styled(
                    format!("{} more", incidents.len() - 1),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }

        if !incident.affected_components.is_empty() {
            push_plain_scope_lines(
                &mut card_lines,
                "Affected",
                &incident.affected_components,
                4,
            );
        } else if !non_op_components.is_empty() {
            push_component_scope_lines(&mut card_lines, &non_op_components, 4);
        }

        if let Some(update) = &incident.latest_update {
            if !update_duplicates_summary_or_issue(&update.body, summary, &incident.name) {
                card_lines.push(Line::from(Span::styled(
                    "  Latest Update",
                    status_section_label_style(),
                )));
                push_wrapped_bullet_lines(
                    &mut card_lines,
                    &update.body,
                    body_width,
                    "    - ",
                    "      ",
                );
            }
        }

        if let Some(note) = entry.user_visible_caveat() {
            push_overall_caveat(&mut card_lines, note, body_width);
        }

        cards.push(SoftCard::new(entry.health, card_lines));
    }

    cards
}

fn build_degradation_panel_cards(
    entries: &[&crate::status::ProviderStatus],
    body_width: usize,
) -> Vec<SoftCard> {
    let mut cards = Vec::new();

    for entry in entries.iter() {
        let non_op_components = overall_attention_components(entry);

        let mut card_lines = Vec::new();

        card_lines.push(Line::from(vec![
            Span::styled(
                status_health_icon(entry.health),
                status_health_style(entry.health),
            ),
            Span::raw(" "),
            Span::styled(
                entry.display_name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));

        if let Some(summary) = entry.provider_summary_text() {
            card_lines.push(Line::from(vec![
                Span::styled("  Summary: ", status_field_label_style()),
                Span::raw(summary.to_string()),
            ]));
        }

        card_lines.push(Line::from(vec![
            Span::styled("  Scope: ", status_field_label_style()),
            Span::styled(
                component_only_scope_title(&non_op_components),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  Status: ", status_field_label_style()),
            Span::styled(
                provider_health_label(entry.health),
                status_health_style(entry.health),
            ),
            Span::raw("  "),
            Span::styled("Updated: ", status_field_label_style()),
            Span::styled(
                provider_last_meaningful_update(entry)
                    .map(|(_, value)| value)
                    .unwrap_or_else(|| "recently updated".to_string()),
                Style::default().fg(Color::Cyan),
            ),
        ]));
        push_component_scope_lines(&mut card_lines, &non_op_components, 4);

        if let Some(note) = entry.user_visible_caveat() {
            push_overall_caveat(&mut card_lines, note, body_width);
        }

        cards.push(SoftCard::new(entry.health, card_lines));
    }

    cards
}

fn build_maintenance_panel_cards(
    items: &[(&str, &crate::status::ScheduledMaintenance)],
) -> Vec<SoftCard> {
    use crate::formatting::format_relative_time_from_str;

    let mut cards = Vec::new();

    for (provider_name, maint) in items.iter() {
        let mut card_lines = Vec::new();

        let maint_active = {
            let s = maint.status.to_lowercase();
            s.contains("progress") || s.contains("active") || s.contains("verifying")
        };
        let maint_icon = if maint_active { "◆" } else { "◇" };
        card_lines.push(Line::from(vec![
            Span::styled(maint_icon, Style::default().fg(Color::Blue)),
            Span::raw(" "),
            Span::styled(
                provider_name.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        card_lines.push(Line::from(vec![
            Span::styled("  Window: ", status_field_label_style()),
            Span::styled(
                maint.name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));

        let mut bits = vec![Span::styled("  Status: ", status_field_label_style())];
        bits.push(Span::styled(
            maint.status.replace('_', " "),
            component_status_style(&maint.status),
        ));
        if let Some(start) = maint.scheduled_for.as_deref() {
            bits.push(Span::raw("  "));
            bits.push(Span::styled("Scheduled: ", status_field_label_style()));
            bits.push(Span::styled(
                format_relative_time_from_str(start),
                Style::default().fg(Color::Cyan),
            ));
        }
        card_lines.push(Line::from(bits));

        if !maint.affected_components.is_empty() {
            push_plain_scope_lines(&mut card_lines, "Affected", &maint.affected_components, 3);
        }

        cards.push(SoftCard::new(ProviderHealth::Maintenance, card_lines));
    }

    cards
}

fn incidents_empty_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_panel_empty_state(
        &mut lines,
        "No active incidents reported right now",
        "Tracked providers are not currently publishing formal incident rows.",
    );
    lines
}

fn degradation_empty_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_panel_empty_state(
        &mut lines,
        "No component-reported degradation right now",
        "Tracked providers are not currently reporting degraded services without incident rows.",
    );
    lines
}

fn render_overall_panel(
    f: &mut Frame,
    area: Rect,
    title: &str,
    cards: Vec<SoftCard>,
    empty_lines: Vec<Line<'static>>,
    scroll: &ScrollOffset,
    focused: bool,
) {
    if cards.is_empty() {
        ScrollablePanel::new(title, empty_lines, scroll, focused).render(f, area);
    } else {
        ScrollablePanel::with_cards(title, cards, scroll, focused).render(f, area);
    }
}

pub(super) fn draw_overall_dashboard(
    f: &mut Frame,
    area: Rect,
    status_app: &super::app::StatusApp,
    is_focused: bool,
) -> super::app::OverallPanelRects {
    let mut rects = super::app::OverallPanelRects::default();
    let (op, _deg, out, other) = status_app.health_counts();
    let total = status_app
        .entries
        .iter()
        .filter(|e| status_app.tracked.contains(&e.slug))
        .count();
    let attention_entries = overall_attention_entries(status_app);
    let incident_entries: Vec<_> = attention_entries
        .iter()
        .copied()
        .filter(|entry| !entry.active_incidents().is_empty())
        .collect();
    let component_entries: Vec<_> = attention_entries
        .iter()
        .copied()
        .filter(|entry| entry.active_incidents().is_empty())
        .collect();
    let all_maint = status_app.all_maintenances();
    let maintenance_visible = !all_maint.is_empty();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);

    {
        let ratio = if total > 0 {
            op as f64 / total as f64
        } else {
            0.0
        };

        let freshness_suffix = overall_freshness_suffix(status_app);
        let title = format!(" Overall Status · {freshness_suffix} ");

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White))
            .title(title);
        let inner = block.inner(rows[0]);
        f.render_widget(block, rows[0]);

        let inner_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(inner);

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
            .ratio(ratio)
            .label(format!("{op}/{total}  {:.0}%", ratio * 100.0));
        f.render_widget(gauge, inner_chunks[0]);

        let mut summary_spans = vec![
            Span::styled("● ", Style::default().fg(Color::Green)),
            Span::raw(format!("{op} operational  ")),
        ];
        if !incident_entries.is_empty() {
            summary_spans.push(Span::styled("◐ ", Style::default().fg(Color::Yellow)));
            summary_spans.push(Span::raw(format!(
                "{} active incident{}  ",
                incident_entries.len(),
                if incident_entries.len() == 1 { "" } else { "s" }
            )));
        }
        if !component_entries.is_empty() {
            summary_spans.push(Span::styled("◐ ", Style::default().fg(Color::Yellow)));
            summary_spans.push(Span::raw(format!(
                "{} service degradation{}  ",
                component_entries.len(),
                if component_entries.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )));
        }
        if out > 0 {
            summary_spans.push(Span::styled("✗ ", Style::default().fg(Color::Red)));
            summary_spans.push(Span::raw(format!("{out} outage  ")));
        }
        if other > 0 {
            summary_spans.push(Span::styled("? ", Style::default().fg(Color::DarkGray)));
            summary_spans.push(Span::raw(format!("{other} other  ")));
        }
        f.render_widget(Paragraph::new(Line::from(summary_spans)), inner_chunks[1]);
    }

    let board_area = rows[1];
    let stacked_layout = board_area.width < 100;

    if stacked_layout {
        let mut constraints = vec![Constraint::Percentage(55), Constraint::Percentage(45)];
        if maintenance_visible {
            constraints = vec![
                Constraint::Percentage(42),
                Constraint::Percentage(34),
                Constraint::Percentage(24),
            ];
        }
        let panels = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(board_area);

        rects.incidents = Some(panels[0]);
        rects.degradation = Some(panels[1]);
        if maintenance_visible {
            rects.maintenance = Some(panels[2]);
        }

        let incident_cards = build_incidents_panel_cards(
            &incident_entries,
            usize::from(panels[0].width.saturating_sub(4)).max(24),
        );
        render_overall_panel(
            f,
            panels[0],
            &format!("Active Incidents ({})", incident_entries.len()),
            incident_cards,
            incidents_empty_lines(),
            &status_app.overall_incidents_scroll,
            is_focused
                && status_app.overall_panel_focus == super::app::OverallPanelFocus::Incidents,
        );

        let degradation_cards = build_degradation_panel_cards(
            &component_entries,
            usize::from(panels[1].width.saturating_sub(4)).max(24),
        );
        render_overall_panel(
            f,
            panels[1],
            &format!("Service Degradation ({})", component_entries.len()),
            degradation_cards,
            degradation_empty_lines(),
            &status_app.overall_degradation_scroll,
            is_focused
                && status_app.overall_panel_focus == super::app::OverallPanelFocus::Degradation,
        );

        if maintenance_visible {
            let maintenance_cards = build_maintenance_panel_cards(&all_maint);
            render_overall_panel(
                f,
                panels[2],
                &format!("Maintenance Outlook ({})", all_maint.len()),
                maintenance_cards,
                Vec::new(),
                &status_app.overall_maintenance_scroll,
                is_focused
                    && status_app.overall_panel_focus == super::app::OverallPanelFocus::Maintenance,
            );
        }
    } else {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(board_area);
        let right_panels = if maintenance_visible {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(columns[1])
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0)])
                .split(columns[1])
        };

        rects.incidents = Some(columns[0]);
        rects.degradation = Some(right_panels[0]);
        if maintenance_visible {
            rects.maintenance = Some(right_panels[1]);
        }

        let incident_cards = build_incidents_panel_cards(
            &incident_entries,
            usize::from(columns[0].width.saturating_sub(4)).max(24),
        );
        render_overall_panel(
            f,
            columns[0],
            &format!("Active Incidents ({})", incident_entries.len()),
            incident_cards,
            incidents_empty_lines(),
            &status_app.overall_incidents_scroll,
            is_focused
                && status_app.overall_panel_focus == super::app::OverallPanelFocus::Incidents,
        );

        let degradation_cards = build_degradation_panel_cards(
            &component_entries,
            usize::from(right_panels[0].width.saturating_sub(4)).max(24),
        );
        render_overall_panel(
            f,
            right_panels[0],
            &format!("Service Degradation ({})", component_entries.len()),
            degradation_cards,
            degradation_empty_lines(),
            &status_app.overall_degradation_scroll,
            is_focused
                && status_app.overall_panel_focus == super::app::OverallPanelFocus::Degradation,
        );

        if maintenance_visible {
            let maintenance_cards = build_maintenance_panel_cards(&all_maint);
            render_overall_panel(
                f,
                right_panels[1],
                &format!("Maintenance Outlook ({})", all_maint.len()),
                maintenance_cards,
                Vec::new(),
                &status_app.overall_maintenance_scroll,
                is_focused
                    && status_app.overall_panel_focus == super::app::OverallPanelFocus::Maintenance,
            );
        }
    }

    rects
}
