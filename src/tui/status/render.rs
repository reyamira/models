use crate::formatting::{format_relative_time_from_str, truncate};
use crate::status::{ProviderHealth, StatusSourceMethod, STATUS_REGISTRY};
use crate::tui::app::App;
use crate::tui::ui::{
    caret, centered_rect_fixed, selection_style, status_health_icon, status_health_style,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

pub(super) fn component_status_icon(status: &str) -> &'static str {
    let s = status.to_lowercase();
    if s.contains("operational") {
        "●"
    } else if s.contains("degraded") || s.contains("partial") {
        "◐"
    } else if s.contains("outage") || s.contains("major") || s.contains("down") {
        "✗"
    } else if s.contains("maintenance") {
        "◆"
    } else {
        "?"
    }
}

pub(super) fn component_status_style(status: &str) -> Style {
    let s = status.to_lowercase();
    if s.contains("operational") {
        Style::default().fg(Color::Green)
    } else if s.contains("partial") {
        Style::default().fg(Color::Red)
    } else if s.contains("degraded") {
        Style::default().fg(Color::Yellow)
    } else if s.contains("outage") || s.contains("major") || s.contains("down") {
        Style::default().fg(Color::Red)
    } else if s.contains("maintenance") {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// 6-char left-aligned gutter tag (padded with spaces, DarkGray) + content spans at column 7.
#[allow(dead_code)]
pub(super) fn gutter_line<'a>(tag: &str, spans: Vec<Span<'a>>) -> Line<'a> {
    let padded = format!("{:<6}", tag);
    let mut all = vec![Span::styled(padded, Style::default().fg(Color::DarkGray))];
    all.extend(spans);
    Line::from(all)
}

/// Chinese component name map for DeepSeek and others.
const CHINESE_NAME_MAP: &[(&str, &str)] =
    &[("API 服务", "API Service"), ("网页对话服务", "Web Chat")];

pub(super) fn translate_component_name(name: &str) -> String {
    for &(chinese, english) in CHINESE_NAME_MAP {
        if name == chinese {
            return format!("{} ({})", english, chinese);
        }
    }
    name.to_string()
}

pub(super) fn provider_last_meaningful_update(
    entry: &crate::status::ProviderStatus,
) -> Option<(&'static str, String)> {
    let latest = entry
        .incidents
        .iter()
        .filter_map(|incident| {
            incident
                .updated_at
                .as_deref()
                .or(incident.created_at.as_deref())
        })
        .chain(entry.scheduled_maintenances.iter().filter_map(|maint| {
            maint
                .scheduled_for
                .as_deref()
                .or(maint.scheduled_until.as_deref())
        }))
        .filter_map(|raw| {
            crate::agents::helpers::parse_date(raw).map(|parsed| (parsed.timestamp(), raw))
        })
        .max_by_key(|(timestamp, _)| *timestamp)
        .map(|(_, raw)| raw.to_string());

    if let Some(raw) = latest {
        return Some(("latest event", format_relative_time_from_str(&raw)));
    }

    entry.source_updated_at.as_deref().map(|raw| {
        let label = match entry.source_method {
            Some(StatusSourceMethod::ApiStatusCheck) => "last checked",
            _ => "source updated",
        };
        (label, format_relative_time_from_str(raw))
    })
}

pub(super) fn title_case_status_time_label(label: &str) -> &'static str {
    match label {
        "latest event" => "Latest event",
        "source updated" => "Source updated",
        "last checked" => "Last checked",
        _ => "Source updated",
    }
}

pub(super) fn overall_non_operational_components(
    entry: &crate::status::ProviderStatus,
) -> Vec<&crate::status::ComponentStatus> {
    entry
        .components
        .iter()
        .filter(|component| {
            let status = component.status.to_lowercase();
            !status.contains("operational") && status != "unknown" && !status.is_empty()
        })
        .collect()
}

pub(super) fn overall_attention_components(
    entry: &crate::status::ProviderStatus,
) -> Vec<&crate::status::ComponentStatus> {
    overall_non_operational_components(entry)
        .into_iter()
        .filter(|component| !component.status.to_lowercase().contains("maint"))
        .collect()
}

pub(super) fn overall_attention_entries(
    status_app: &super::app::StatusApp,
) -> Vec<&crate::status::ProviderStatus> {
    let mut entries: Vec<_> = status_app
        .entries
        .iter()
        .filter(|entry| status_app.tracked.contains(&entry.slug))
        .filter(|entry| {
            !entry.active_incidents().is_empty()
                || !overall_attention_components(entry).is_empty()
                || matches!(
                    entry.health,
                    ProviderHealth::Outage | ProviderHealth::Degraded | ProviderHealth::Unknown
                )
        })
        .collect();
    entries.sort_by_key(|a| a.health.sort_rank());
    entries
}

pub(super) fn component_scope_name(component: &crate::status::ComponentStatus) -> String {
    component
        .group_name
        .as_deref()
        .filter(|group| !group.is_empty())
        .unwrap_or(&component.name)
        .to_string()
}

pub(super) fn component_display_name(component: &crate::status::ComponentStatus) -> String {
    let name = translate_component_name(&component.name);
    match component.group_name.as_deref() {
        Some(group) if !group.is_empty() && group != component.name => {
            format!("{group}: {name}")
        }
        _ => name,
    }
}

pub(super) fn component_only_scope_title(components: &[&crate::status::ComponentStatus]) -> String {
    let mut scopes: Vec<String> = Vec::new();
    for component in components {
        let scope = component_scope_name(component);
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }

    match scopes.len() {
        0 => "Component-reported service degradation".to_string(),
        1 => scopes[0].clone(),
        _ => "Multiple affected services".to_string(),
    }
}

pub(super) fn provider_health_label(health: ProviderHealth) -> &'static str {
    match health {
        ProviderHealth::Operational => "operational",
        ProviderHealth::Degraded => "degraded",
        ProviderHealth::Outage => "outage",
        ProviderHealth::Maintenance => "maintenance",
        ProviderHealth::Unknown => "unknown",
    }
}

pub(super) fn sparse_incident_metadata(incident: &crate::status::ActiveIncident) -> bool {
    incident.created_at.is_none()
        && incident.updated_at.is_none()
        && incident.latest_update.is_none()
        && incident.impact.trim().eq_ignore_ascii_case("none")
        && incident.affected_components.is_empty()
}

pub(super) fn incident_status_value(incident: &crate::status::ActiveIncident) -> String {
    if sparse_incident_metadata(incident) && incident.status.eq_ignore_ascii_case("investigating") {
        "reported".to_string()
    } else {
        incident.status.clone()
    }
}

pub(super) fn incident_time_value(
    entry: &crate::status::ProviderStatus,
    incident: &crate::status::ActiveIncident,
) -> Option<(&'static str, String)> {
    if let Some(updated_at) = incident.updated_at.as_deref() {
        return Some(("Updated", format_relative_time_from_str(updated_at)));
    }

    if let Some(update) = incident.latest_update.as_ref() {
        if !update.created_at.trim().is_empty() {
            return Some(("Updated", format_relative_time_from_str(&update.created_at)));
        }
    }

    if let Some(created_at) = incident.created_at.as_deref() {
        return Some(("Reported", format_relative_time_from_str(created_at)));
    }

    provider_last_meaningful_update(entry).map(|(label, value)| {
        let display_label = match label {
            "source updated" => "Source updated",
            "last checked" => "Last checked",
            _ => "Updated",
        };
        (display_label, value)
    })
}

pub(super) fn incident_impact_style(impact: &str) -> Style {
    let normalized = impact.to_lowercase();
    if normalized.contains("critical") || normalized.contains("major") {
        Style::default().fg(Color::Red)
    } else if normalized.contains("minor") || normalized.contains("partial") {
        Style::default().fg(Color::Yellow)
    } else if normalized.contains("maint") {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

pub(super) fn status_field_label_style() -> Style {
    Style::default().fg(Color::Blue)
}

pub(super) fn status_section_label_style() -> Style {
    Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn push_component_scope_lines(
    lines: &mut Vec<Line<'static>>,
    components: &[&crate::status::ComponentStatus],
    max_items: usize,
) {
    if components.is_empty() {
        return;
    }

    lines.push(Line::from(Span::styled(
        "  Services",
        status_section_label_style(),
    )));

    for component in components.iter().take(max_items) {
        lines.push(Line::from(vec![
            Span::styled("    - ", Style::default().fg(Color::DarkGray)),
            Span::raw(component_display_name(component)),
            Span::styled(" (", Style::default().fg(Color::DarkGray)),
            Span::styled(
                component.status.replace('_', " "),
                component_status_style(&component.status),
            ),
            Span::styled(")", Style::default().fg(Color::DarkGray)),
        ]));
    }

    let remaining = components.len().saturating_sub(max_items);
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("    +{remaining} more affected service(s)"),
            Style::default().fg(Color::DarkGray),
        )));
    }
}

pub(super) fn push_plain_scope_lines(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    items: &[String],
    max_items: usize,
) {
    if items.is_empty() {
        return;
    }

    lines.push(Line::from(Span::styled(
        format!("  {label}"),
        status_section_label_style(),
    )));

    for item in items.iter().take(max_items) {
        lines.push(Line::from(vec![
            Span::styled("    - ", Style::default().fg(Color::DarkGray)),
            Span::raw(item.clone()),
        ]));
    }

    let remaining = items.len().saturating_sub(max_items);
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("    +{remaining} more"),
            Style::default().fg(Color::DarkGray),
        )));
    }
}

pub(super) fn push_wrapped_bullet_lines(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    body_width: usize,
    bullet_indent: &str,
    continuation_indent: &str,
) {
    let available_width = body_width.saturating_sub(continuation_indent.len()).max(12);
    let wrapped = textwrap::wrap(text.trim(), available_width);

    if let Some(first_line) = wrapped.first() {
        lines.push(Line::from(vec![
            Span::styled(
                bullet_indent.to_string(),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(first_line.to_string()),
        ]));
    }

    for line in wrapped.iter().skip(1) {
        lines.push(Line::from(vec![
            Span::raw(continuation_indent.to_string()),
            Span::raw(line.to_string()),
        ]));
    }
}

pub(super) fn status_verdict_copy(health: ProviderHealth) -> &'static str {
    match health {
        ProviderHealth::Operational => "All systems operational",
        ProviderHealth::Degraded => "Some services degraded",
        ProviderHealth::Outage => "Major service disruption",
        ProviderHealth::Maintenance => "Scheduled maintenance in progress",
        ProviderHealth::Unknown => "Status unavailable",
    }
}

/// Map incident stage to a `ProviderHealth` for accent stripe coloring.
pub(super) fn incident_stage_health(stage: &str) -> ProviderHealth {
    let normalized = stage.to_lowercase();
    if normalized.contains("resolved") {
        ProviderHealth::Operational
    } else if normalized.contains("monitoring") {
        ProviderHealth::Degraded
    } else if normalized.contains("maint") {
        ProviderHealth::Maintenance
    } else {
        ProviderHealth::Degraded
    }
}

pub(super) fn incident_stage_style(stage: &str) -> Style {
    let normalized = stage.to_lowercase();
    if normalized.contains("resolved") {
        Style::default().fg(Color::Green)
    } else if normalized.contains("monitoring") {
        Style::default().fg(Color::Cyan)
    } else if normalized.contains("maint") {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::Yellow)
    }
}

pub(super) fn normalized_status_copy(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(super) fn summary_duplicates_issue(summary: &str, issue: &str) -> bool {
    let normalized_summary = normalized_status_copy(summary);
    let normalized_issue = normalized_status_copy(issue);

    !normalized_summary.is_empty() && normalized_summary == normalized_issue
}

pub(super) fn update_duplicates_summary_or_issue(
    update: &str,
    summary: Option<&str>,
    issue: &str,
) -> bool {
    let normalized_update = normalized_status_copy(update);
    if normalized_update.is_empty() {
        return true;
    }

    if summary.is_some_and(|summary| summary_duplicates_issue(summary, update)) {
        return true;
    }

    summary_duplicates_issue(update, issue)
}

pub(super) fn push_overall_caveat(lines: &mut Vec<Line<'static>>, note: &str, body_width: usize) {
    let _ = body_width;
    lines.push(Line::from(vec![
        Span::styled("  Note: ", status_field_label_style()),
        Span::styled(note.to_string(), Style::default().fg(Color::DarkGray)),
    ]));
}

pub(super) fn push_panel_empty_state(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    description: &str,
) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(Color::Green),
    )));
    lines.push(Line::from(Span::styled(
        description.to_string(),
        Style::default().fg(Color::DarkGray),
    )));
}

pub(in crate::tui) fn draw_status_main(f: &mut Frame, area: Rect, app: &mut App) {
    use super::app::StatusFocus;

    let Some(status_app) = app.status_app.as_mut() else {
        let msg = Paragraph::new("Failed to load status data")
            .block(Block::default().borders(Borders::ALL).title(" Status "));
        f.render_widget(msg, area);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(32), Constraint::Min(0)])
        .split(area);

    let list_border = if status_app.focus == StatusFocus::List {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if status_app.loading {
        format!(
            " Providers ({}) refreshing... ",
            status_app.filtered_entries.len()
        )
    } else if status_app.search_query.is_empty() {
        format!(" Providers ({}) ", status_app.filtered_entries.len())
    } else {
        format!(
            " Providers ({}) [/{query}] ",
            status_app.filtered_entries.len(),
            query = status_app.search_query
        )
    };

    let is_list_focused = status_app.focus == StatusFocus::List;

    // Build list items: Overall at index 0, then providers
    let mut items = Vec::new();

    // Overall entry (always first, display index 0)
    let overall_selected = status_app.list_state.selected() == Some(0);
    let (overall_prefix, overall_style) = if overall_selected {
        (
            if is_list_focused { "> " } else { "  " },
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("  ", Style::default())
    };
    items.push(ListItem::new(Line::from(vec![
        Span::styled(overall_prefix, overall_style),
        Span::styled("  Overall", overall_style),
    ])));

    // Provider entries (display index 1+)
    for (row_idx, &idx) in status_app.filtered_entries.iter().enumerate() {
        if let Some(entry) = status_app.entries.get(idx) {
            let display_idx = row_idx + 1; // offset for Overall
            let is_selected = status_app.list_state.selected() == Some(display_idx);
            let (prefix, text_style) = if is_selected {
                (caret(is_list_focused), selection_style(true))
            } else {
                ("  ", Style::default())
            };
            let mut spans = vec![
                Span::styled(prefix, text_style),
                Span::styled(
                    status_health_icon(entry.health),
                    status_health_style(entry.health),
                ),
                Span::raw(" "),
                Span::styled(truncate(&entry.display_name, 20), text_style),
            ];
            let issue_count = entry.issue_count();
            if issue_count > 0 {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    issue_count.to_string(),
                    status_health_style(entry.health),
                ));
            }
            items.push(ListItem::new(Line::from(spans)));
        }
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(list_border)
            .title(title),
    );
    f.render_stateful_widget(list, chunks[0], &mut status_app.list_state);

    // Detail area: dispatch based on selection
    let detail_area = chunks[1];

    if status_app.is_overall_selected() {
        super::overall::draw_overall_dashboard(
            f,
            detail_area,
            status_app,
            status_app.focus == StatusFocus::Details,
        );
    } else if let Some(entry) = status_app.current_entry() {
        let display_name = entry.display_name.clone();
        let health = entry.health;
        let provenance = entry.provenance;
        let error_msg = entry.error_summary();
        let (time_label, time_value) = provider_last_meaningful_update(entry)
            .map(|(label, value)| (title_case_status_time_label(label), value))
            .unwrap_or(("Source updated", "Unknown".to_string()));
        let service_note = entry.detail_state_message(&entry.components_state, "Service details");
        let incident_note = entry.detail_state_message(&entry.incidents_state, "Incident details");
        let maintenance_note =
            entry.detail_state_message(&entry.scheduled_maintenances_state, "Maintenance details");
        let maintenance_problem = entry.scheduled_maintenances_state.is_fetch_failed();
        let caveat = service_note
            .clone()
            .or_else(|| incident_note.clone())
            .or_else(|| entry.user_visible_caveat().map(str::to_string));
        let confirmed_no_components = entry.confirmed_no_components();
        let confirmed_no_incidents = entry.confirmed_no_incidents();
        let active_incidents = super::detail::sorted_active_incidents(entry);
        let components = super::detail::sorted_components(entry, &active_incidents);
        let is_detail_focused = status_app.focus == StatusFocus::Details;

        let status_note = entry.status_note_text().map(str::to_string);

        super::detail::draw_provider_status_detail(
            f,
            detail_area,
            &display_name,
            health,
            provenance,
            &error_msg,
            &status_note,
            time_label,
            &time_value,
            &caveat,
            &service_note,
            &incident_note,
            &maintenance_note,
            confirmed_no_components,
            confirmed_no_incidents,
            maintenance_problem,
            &active_incidents,
            &components,
            &entry.scheduled_maintenances,
            &status_app.detail_scroll,
            is_detail_focused,
            &status_app.services_scroll,
            status_app.detail_panel_focus,
            &status_app.maintenance_scroll,
        );
    } else {
        let detail_border = if status_app.focus == StatusFocus::Details {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let paragraph = Paragraph::new(vec![Line::from(Span::styled(
            "Select a provider to view details",
            Style::default().fg(Color::DarkGray),
        ))])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(detail_border)
                .title(" Status "),
        );
        f.render_widget(paragraph, detail_area);
    }

    // Render picker modal overlay if active
    if app
        .status_app
        .as_ref()
        .map(|a| a.show_picker)
        .unwrap_or(false)
    {
        draw_status_picker_modal(f, app);
    }
}

fn draw_status_picker_modal(f: &mut Frame, app: &App) {
    let status_app = match &app.status_app {
        Some(a) => a,
        None => return,
    };

    let num_providers = STATUS_REGISTRY.len();

    let popup_width = std::cmp::min(60, f.area().width.saturating_sub(4));
    let popup_height = std::cmp::min(
        (num_providers + 4) as u16,
        f.area().height.saturating_sub(4),
    );

    let area = centered_rect_fixed(popup_width, popup_height, f.area());

    f.render_widget(Clear, area);

    let items: Vec<ListItem> = STATUS_REGISTRY
        .iter()
        .enumerate()
        .map(|(idx, reg_entry)| {
            let is_tracked = status_app
                .picker_changes
                .get(reg_entry.slug)
                .copied()
                .unwrap_or_else(|| status_app.tracked.contains(reg_entry.slug));

            let checkbox = if is_tracked { "[x]" } else { "[ ]" };

            // Show health icon if tracked and data loaded
            let health_icon = if is_tracked {
                status_app
                    .entries
                    .iter()
                    .find(|e| e.slug == reg_entry.slug)
                    .map(|e| {
                        let icon = status_health_icon(e.health);
                        let style = status_health_style(e.health);
                        Span::styled(format!(" {}", icon), style)
                    })
            } else {
                Some(Span::styled(" ?", Style::default().fg(Color::DarkGray)))
            };

            let line = Line::from(vec![
                Span::raw(format!("{} ", checkbox)),
                Span::styled(
                    format!("{:<30}", truncate(reg_entry.display_name, 30)),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                health_icon.unwrap_or_else(|| Span::raw("")),
            ]);

            if idx == status_app.picker_selected {
                ListItem::new(line).style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Track Providers ")
                .title_bottom(Line::from(" Space: toggle | Enter: save | Esc: cancel ").centered()),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    let mut list_state = ListState::default();
    list_state.select(Some(status_app.picker_selected));

    f.render_stateful_widget(list, area, &mut list_state);
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Instant};

    use ratatui::{backend::TestBackend, Terminal};

    use super::*;
    use crate::{
        benchmarks::BenchmarkStore,
        status::{
            ActiveIncident, ComponentStatus, IncidentUpdate, ProviderStatus, ScheduledMaintenance,
            StatusProvenance, StatusSourceMethod, StatusSupportTier,
        },
        tui::app::{App, Tab},
    };

    fn make_status_app(entry: ProviderStatus) -> App {
        let mut app = App::new(HashMap::new(), None, None, BenchmarkStore::empty());
        app.current_tab = Tab::Status;
        let status_app = app.status_app.as_mut().expect("status app");
        status_app.entries = vec![entry];
        status_app.loading = false;
        status_app.last_refreshed = Some(Instant::now());
        status_app.update_filtered();
        // Select the first provider (display index 1; index 0 = Overall)
        status_app.selected = 1;
        status_app.list_state.select(Some(1));
        app
    }

    fn sample_provider_status() -> ProviderStatus {
        ProviderStatus {
            slug: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            source_slug: "openai".to_string(),
            support_tier: StatusSupportTier::Required,
            health: ProviderHealth::Degraded,
            provenance: StatusProvenance::Fallback,
            load_state: crate::status::StatusLoadState::Loaded,
            source_label: Some("API Status Check".to_string()),
            source_method: Some(StatusSourceMethod::ApiStatusCheck),
            official_url: Some("https://status.openai.com".to_string()),
            fallback_url: Some("https://apistatuscheck.com/openai".to_string()),
            source_updated_at: Some("2026-03-16T23:55:00Z".to_string()),
            provider_summary: Some("Elevated API errors affecting chat completions.".to_string()),
            status_note: Some(
                "Fallback adapter exposes only provider-level summary status.".to_string(),
            ),
            components: vec![
                ComponentStatus {
                    name: "API".to_string(),
                    status: "partial_outage".to_string(),
                    group_name: None,
                    position: None,
                    only_show_if_degraded: false,
                },
                ComponentStatus {
                    name: "Auth".to_string(),
                    status: "operational".to_string(),
                    group_name: None,
                    position: None,
                    only_show_if_degraded: false,
                },
            ],
            components_state: crate::status::StatusDetailState {
                availability: crate::status::StatusDetailAvailability::Available,
                source: crate::status::StatusDetailSource::Inline,
                note: None,
                error: None,
            },
            incidents: vec![ActiveIncident {
                name: "Elevated API errors".to_string(),
                status: "investigating".to_string(),
                impact: "minor".to_string(),
                shortlink: None,
                created_at: Some("2026-03-16T23:40:00Z".to_string()),
                updated_at: Some("2026-03-16T23:58:00Z".to_string()),
                latest_update: Some(IncidentUpdate {
                    status: "investigating".to_string(),
                    body: "We are investigating elevated error rates for API requests.".to_string(),
                    created_at: "2026-03-16T23:58:00Z".to_string(),
                }),
                affected_components: vec!["API".to_string()],
            }],
            incidents_state: crate::status::StatusDetailState {
                availability: crate::status::StatusDetailAvailability::Available,
                source: crate::status::StatusDetailSource::Inline,
                note: None,
                error: None,
            },
            scheduled_maintenances: vec![ScheduledMaintenance {
                name: "Database maintenance".to_string(),
                status: "scheduled".to_string(),
                impact: "none".to_string(),
                shortlink: None,
                scheduled_for: Some("2026-03-17T03:00:00Z".to_string()),
                scheduled_until: Some("2026-03-17T04:00:00Z".to_string()),
                affected_components: vec!["Auth".to_string()],
            }],
            scheduled_maintenances_state: crate::status::StatusDetailState {
                availability: crate::status::StatusDetailAvailability::Available,
                source: crate::status::StatusDetailSource::Inline,
                note: None,
                error: None,
            },
            official_error: None,
            fallback_error: None,
        }
    }

    fn render_status_buffer_with_size(
        app: &mut App,
        width: u16,
        height: u16,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| draw_status_main(frame, frame.area(), app))
            .expect("draw succeeds");
        terminal.backend().buffer().clone()
    }

    fn render_status_text_with_size(app: &mut App, width: u16, height: u16) -> String {
        let buffer = render_status_buffer_with_size(app, width, height);
        let mut lines = Vec::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    fn render_status_text(app: &mut App) -> String {
        render_status_text_with_size(app, 140, 40)
    }

    #[test]
    fn status_detail_reads_like_a_status_page() {
        let mut app = make_status_app(sample_provider_status());

        let rendered = render_status_text(&mut app);

        assert!(rendered.contains("Status"));
        assert!(rendered.contains("1 active incident"));
        assert!(!rendered.contains("Narrative"));
        assert!(!rendered.contains("Status page"));
        assert!(rendered.contains("Current Incidents"));
        assert!(rendered.contains("Services"));
        assert!(rendered.contains("Database maintenance"));
        assert!(!rendered.contains("Tracking:"));
        assert!(!rendered.contains("Agents:"));
        assert!(!rendered.contains("confidence"));
        assert!(!rendered.contains("coverage"));
        assert!(!rendered.contains("freshness"));
        assert!(!rendered.contains("contradiction"));
        assert!(!rendered.contains("R/FB"));
    }

    #[test]
    fn operational_status_hides_affected_right_now_summary() {
        let mut entry = sample_provider_status();
        entry.health = ProviderHealth::Operational;
        entry.provenance = StatusProvenance::Official;
        entry.provider_summary = Some("All systems operational".to_string());
        entry.incidents.clear();
        entry.scheduled_maintenances.clear();
        entry.incidents_state.availability = crate::status::StatusDetailAvailability::NoneReported;
        entry.scheduled_maintenances_state.availability =
            crate::status::StatusDetailAvailability::NoneReported;
        for component in &mut entry.components {
            component.status = "operational".to_string();
        }

        let mut app = make_status_app(entry);
        let rendered = render_status_text(&mut app);

        assert!(rendered.contains("2/2"));
        assert!(rendered.contains("100%"));
        assert!(!rendered.contains("Affected right now:"));
    }

    #[test]
    fn summary_only_status_hides_services_section_and_shows_service_note() {
        let mut entry = sample_provider_status();
        entry.health = ProviderHealth::Operational;
        entry.provenance = StatusProvenance::Official;
        entry.source_method = Some(StatusSourceMethod::ApiStatusCheck);
        entry.provider_summary = Some("All systems operational".to_string());
        entry.components.clear();
        entry.incidents.clear();
        entry.scheduled_maintenances.clear();
        entry.components_state = crate::status::StatusDetailState {
            availability: crate::status::StatusDetailAvailability::Unsupported,
            source: crate::status::StatusDetailSource::SummaryOnly,
            note: Some("Service details unavailable".to_string()),
            error: None,
        };
        entry.incidents_state = crate::status::StatusDetailState {
            availability: crate::status::StatusDetailAvailability::Unsupported,
            source: crate::status::StatusDetailSource::SummaryOnly,
            note: Some("Incident details unavailable".to_string()),
            error: None,
        };
        entry.scheduled_maintenances_state = crate::status::StatusDetailState {
            availability: crate::status::StatusDetailAvailability::Unsupported,
            source: crate::status::StatusDetailSource::SummaryOnly,
            note: Some("Maintenance details unavailable".to_string()),
            error: None,
        };

        let mut app = make_status_app(entry);
        let rendered = render_status_text(&mut app);

        assert!(rendered.contains("Service details unavailable"));
        assert!(rendered.contains("Last checked"));
        assert!(!rendered.contains("Affected right now:"));
    }

    #[test]
    fn incident_driven_status_uses_latest_event_label() {
        let mut app = make_status_app(sample_provider_status());

        let rendered = render_status_text(&mut app);

        assert!(rendered.contains("Latest event"));
        assert!(!rendered.contains("updated 23"));
    }

    #[test]
    fn provider_list_stays_navigation_focused() {
        let mut app = make_status_app(sample_provider_status());

        let rendered = render_status_text(&mut app);

        assert!(rendered.contains("Providers (1)"));
        assert!(rendered.contains("OpenAI 1"));
        assert!(!rendered.contains("R/"));
        assert!(!rendered.contains("/FB"));
        assert!(!rendered.contains("/OFF"));
        assert!(!rendered.contains("/MISS"));
    }

    #[test]
    fn overall_dashboard_prioritizes_attention_details_over_signal_quality() {
        let mut app = make_status_app(sample_provider_status());
        let status_app = app.status_app.as_mut().expect("status app");
        status_app.selected = 0;
        status_app.list_state.select(Some(0));

        let rendered = render_status_text(&mut app);

        assert!(rendered.contains("Overall Status"));
        assert!(rendered.contains("Active Incidents"));
        assert!(rendered.contains("Service Degradation"));
        assert!(rendered.contains("Maintenance Outlook"));
        assert!(rendered.contains("Updated just now"));
        assert!(rendered.contains("Elevated API errors"));
        assert!(rendered.contains("investigating"));
        assert!(rendered.contains("Affected"));
        assert!(rendered.contains("API"));
        assert!(rendered.contains("Update"));
        assert!(!rendered.contains("Signal Quality"));
        assert!(!rendered.contains("Active Issues"));
        assert!(!rendered.contains("need attention •"));
    }

    #[test]
    fn overall_dashboard_uses_stacked_panels_on_narrow_widths() {
        let mut app = make_status_app(sample_provider_status());
        let status_app = app.status_app.as_mut().expect("status app");
        status_app.selected = 0;
        status_app.list_state.select(Some(0));

        let rendered = render_status_text_with_size(&mut app, 90, 40);

        assert!(rendered.contains("Active Incidents"));
        assert!(rendered.contains("Service Degradation"));
        assert!(rendered.contains("Maintenance Outlook"));
    }

    #[test]
    fn overall_incident_card_avoids_repeating_summary_as_issue_and_update() {
        let mut entry = sample_provider_status();
        entry.provider_summary = Some("Elevated API errors".to_string());
        entry.incidents[0].name = "Elevated API errors".to_string();
        entry.incidents[0].latest_update = Some(IncidentUpdate {
            status: "investigating".to_string(),
            body: "Elevated API errors".to_string(),
            created_at: "2026-03-16T23:58:00Z".to_string(),
        });

        let mut app = make_status_app(entry);
        let status_app = app.status_app.as_mut().expect("status app");
        status_app.selected = 0;
        status_app.list_state.select(Some(0));

        let rendered = render_status_text(&mut app);

        assert!(rendered.contains("Elevated API errors"));
        assert!(rendered.contains("Issue: Elevated API errors"));
        assert!(!rendered.contains("Latest Update"));
        assert!(!rendered.contains("  Elevated API errors\n"));
    }

    #[test]
    fn overall_update_renders_as_labeled_block() {
        let mut entry = sample_provider_status();
        entry.provider_summary = Some("Distinct summary".to_string());
        entry.incidents[0].name = "Distinct issue".to_string();
        entry.incidents[0].latest_update = Some(IncidentUpdate {
            status: "investigating".to_string(),
            body: "This is a long update message that should wrap onto another rendered line in the incidents panel for styling verification.".to_string(),
            created_at: "2026-03-16T23:58:00Z".to_string(),
        });

        let mut app = make_status_app(entry);
        let status_app = app.status_app.as_mut().expect("status app");
        status_app.selected = 0;
        status_app.list_state.select(Some(0));

        let rendered = render_status_text_with_size(&mut app, 100, 40);

        assert!(rendered.contains("Latest Update"));
        assert!(rendered.contains("- This is a long update message"));
    }
}
