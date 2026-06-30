use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::app::{AddAgentField, AgentUpdateState};
use crate::agents::{format_stars, FetchStatus};
use crate::formatting::truncate;
use crate::formatting::EM_DASH;
use crate::tui::app::App;
use crate::tui::ui::{
    caret, centered_rect_fixed, filter_toggle_spans, focus_border, selection_style,
};
use crate::tui::widgets::scroll_offset::ScrollOffset;
use crate::tui::widgets::scrollable_panel::ScrollablePanel;

pub(in crate::tui) fn draw_agents_main(f: &mut Frame, area: Rect, app: &mut App) {
    if app.agents_app.is_none() {
        let msg = Paragraph::new("Failed to load agents data")
            .block(Block::default().borders(Borders::ALL).title(" Agents "));
        f.render_widget(msg, area);
        return;
    }

    // Compute list panel width from content
    let max_name_len = app
        .agents_app
        .as_ref()
        .and_then(|a| {
            a.filtered_entries
                .iter()
                .filter_map(|&idx| a.entries.get(idx))
                .map(|e| e.agent.name.len())
                .max()
        })
        .unwrap_or(5)
        .max(5);
    // 2 borders + 2 highlight + 2 (dot+space) + name + 2 gap + 6 type + 4 padding
    let list_width = (max_name_len + 18) as u16;

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(list_width), Constraint::Min(0)])
        .split(area);

    draw_agent_list(f, chunks[0], app);
    draw_agent_detail(f, chunks[1], &mut *app);
}

fn draw_agent_list(f: &mut Frame, area: Rect, app: &mut App) {
    use super::app::AgentFocus;

    let agents_app = match &mut app.agents_app {
        Some(a) => a,
        None => return,
    };

    let is_focused = agents_app.focus == AgentFocus::List;
    let border_style = focus_border(is_focused);

    // Build title with count, filter, and sort indicators
    let sort_indicator = format!(" \u{2193}{}", agents_app.sort_order.label());
    let filter_indicator = agents_app.format_active_filters();

    let title = if filter_indicator.is_empty() {
        format!(
            " Agents ({}){} ",
            agents_app.filtered_entries.len(),
            sort_indicator
        )
    } else {
        format!(
            " Agents ({}) [{}]{} ",
            agents_app.filtered_entries.len(),
            filter_indicator,
            sort_indicator
        )
    };

    // Outer block with title at top
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Split inner area into filter row + list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner_area);

    // Filter toggles row
    let filter_line = Line::from(filter_toggle_spans(&[
        ("1", "Inst", agents_app.filters.installed_only),
        ("2", "CLI", agents_app.filters.cli_only),
        ("3", "OSS", agents_app.filters.open_source_only),
    ]));
    f.render_widget(Paragraph::new(filter_line), chunks[0]);

    // Agent list
    let mut items: Vec<ListItem> = Vec::new();

    // Compute dynamic agent name column width
    let max_name_len = agents_app
        .filtered_entries
        .iter()
        .filter_map(|&idx| agents_app.entries.get(idx))
        .map(|e| e.agent.name.len())
        .max()
        .unwrap_or(5)
        .max(5); // minimum width of 5 for "Agent" header

    // Header row (leading spaces match the "> " / "  " prefix)
    let header = format!(
        "  {:<2} {:<width$}  {:>6}",
        "St",
        "Agent",
        "Type",
        width = max_name_len,
    );
    items.push(
        ListItem::new(header).style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::UNDERLINED),
        ),
    );

    // Agent rows (manual highlight to preserve status dot color). Compare
    // against the 0-based `selected_agent` field, not the list state's
    // `selected()` — the state is bumped to `selected_agent + 1` below (for the
    // header item) so its post-render `offset()` is valid for mouse hit-testing.
    let selected_agent = agents_app.selected_agent;

    for (row_idx, &idx) in agents_app.filtered_entries.iter().enumerate() {
        if let Some(entry) = agents_app.entries.get(idx) {
            let is_selected = row_idx == selected_agent;

            let agent_type = if entry.agent.categories.contains(&"cli".to_string()) {
                "CLI"
            } else if entry.agent.categories.contains(&"ide".to_string()) {
                "IDE"
            } else {
                EM_DASH
            };

            // Status indicator: a Magenta spinner while an in-app update runs,
            // else a colored dot for installed agents, dash for others.
            let updating = matches!(
                agents_app.update_states.get(&entry.id),
                Some(AgentUpdateState::Running)
            );
            let (status_indicator, status_style) = if updating {
                ("\u{25D0}", Style::default().fg(Color::Magenta)) // ◐ magenta = updating
            } else if entry.installed.version.is_some() {
                match &entry.fetch_status {
                    FetchStatus::NotStarted => ("\u{25CB}", Style::default().fg(Color::DarkGray)), // ○ gray
                    FetchStatus::Loading => ("\u{25D0}", Style::default().fg(Color::Yellow)), // ◐ yellow
                    FetchStatus::Loaded => {
                        if entry.update_available() {
                            ("\u{25CF}", Style::default().fg(Color::Blue)) // ● blue = update available
                        } else {
                            ("\u{25CF}", Style::default().fg(Color::Green)) // ● green = up to date
                        }
                    }
                    FetchStatus::Failed(_) => ("\u{2717}", Style::default().fg(Color::Red)), // ✗ red
                }
            } else {
                (EM_DASH, Style::default().fg(Color::DarkGray))
            };

            let (prefix, text_style) = if is_selected {
                (caret(is_focused), selection_style(true))
            } else {
                ("  ", Style::default())
            };

            let row = Line::from(vec![
                Span::styled(prefix, text_style),
                Span::styled(status_indicator, status_style),
                Span::styled(
                    format!(
                        " {:<width$}  {:>6}",
                        truncate(&entry.agent.name, max_name_len),
                        agent_type,
                        width = max_name_len,
                    ),
                    text_style,
                ),
            ]);
            items.push(ListItem::new(row));
        }
    }

    let list = List::new(items);

    // Cache the bare list rect (below the filter row, inside the border) for
    // mouse hit-testing, and render into the REAL state so its post-render
    // `offset()` (viewport-clamped) is valid. The state's selection is bumped
    // to `selected_agent + 1` to account for the header item at index 0.
    agents_app.agent_list_area = Some(chunks[1]);
    agents_app.agent_list_state.select(Some(selected_agent + 1));
    f.render_stateful_widget(list, chunks[1], &mut agents_app.agent_list_state);
}

fn draw_agent_detail(f: &mut Frame, area: Rect, app: &mut App) {
    use super::app::AgentFocus;

    // Extract what we need from agents_app before building lines
    let (is_focused, search_query) = match &app.agents_app {
        Some(a) => (a.focus == AgentFocus::Details, a.search_query.clone()),
        None => return,
    };

    let mut match_line_indices: Vec<u16> = Vec::new();

    let lines: Vec<Line> = if let Some(entry) =
        app.agents_app.as_ref().and_then(|a| a.current_entry())
    {
        let mut detail_lines = Vec::new();

        // Header: Name + Version
        let name = entry.agent.name.clone();
        let version_str = entry.github.latest_version().unwrap_or(EM_DASH).to_string();
        detail_lines.push(Line::from(vec![
            Span::styled(
                name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("v{}", version_str),
                Style::default().fg(Color::Cyan),
            ),
        ]));

        // Repo + Stars
        let repo = entry.agent.repo.clone();
        let stars_str = entry.github.stars.map(format_stars).unwrap_or_default();
        detail_lines.push(Line::from(vec![
            Span::styled(repo, Style::default().fg(Color::Gray)),
            Span::raw("  "),
            Span::styled(
                format!("★ {}", stars_str),
                Style::default().fg(Color::Yellow),
            ),
        ]));

        detail_lines.push(Line::from(""));

        // Installed status
        let installed_str = entry
            .installed
            .version
            .as_deref()
            .unwrap_or("Not installed");
        let status = if entry.update_available() {
            Span::styled(" (update available)", Style::default().fg(Color::Yellow))
        } else if entry.installed.version.is_some() {
            Span::styled(" (up to date)", Style::default().fg(Color::Green))
        } else {
            Span::raw("")
        };

        detail_lines.push(Line::from(vec![
            Span::styled("Installed: ", Style::default().fg(Color::Gray)),
            Span::raw(installed_str),
            status,
        ]));

        let latest_release_date = entry
            .github
            .latest_release_date()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "\u{2014}".to_string());
        let updated_str = entry
            .latest_release_relative_time()
            .unwrap_or_else(|| "\u{2014}".to_string());
        detail_lines.push(Line::from(vec![
            Span::styled("Latest release: ", Style::default().fg(Color::Gray)),
            Span::raw(latest_release_date),
            Span::styled(
                format!(" ({})", updated_str),
                Style::default().fg(Color::Gray),
            ),
        ]));

        detail_lines.push(Line::from(vec![
            Span::styled("Release cadence: ", Style::default().fg(Color::Gray)),
            Span::raw(entry.release_frequency()),
        ]));

        // Service health from status data
        if crate::agents::health::service_mapping_for_agent(&entry.id).is_some() {
            let status_entries = app
                .status_app
                .as_ref()
                .map(|s| s.entries.as_slice())
                .unwrap_or(&[]);
            let health_spans = match crate::agents::health::resolve_agent_service_health(
                &entry.id,
                status_entries,
            ) {
                Some(resolved) => {
                    let icon = crate::tui::ui::status_health_icon(resolved.health);
                    let style = crate::tui::ui::status_health_style(resolved.health);
                    let attribution = match resolved.component_name {
                        Some(comp) => format!("({} \u{2014} {})", resolved.provider_name, comp),
                        None => format!("({})", resolved.provider_name),
                    };
                    vec![
                        Span::styled("Service: ", Style::default().fg(Color::Gray)),
                        Span::styled(format!("{} {}", icon, resolved.health.label()), style),
                        Span::styled(
                            format!("  {}", attribution),
                            Style::default().fg(Color::Gray),
                        ),
                    ]
                }
                None => {
                    vec![
                        Span::styled("Service: ", Style::default().fg(Color::Gray)),
                        Span::styled("? Loading...", Style::default().fg(Color::DarkGray)),
                    ]
                }
            };
            detail_lines.push(Line::from(health_spans));
        }

        // Show status indicator based on fetch_status
        match &entry.fetch_status {
            FetchStatus::Loading => {
                detail_lines.push(Line::from(Span::styled(
                    "Loading GitHub data...",
                    Style::default().fg(Color::Yellow),
                )));
            }
            FetchStatus::Failed(error) => {
                detail_lines.push(Line::from(vec![
                    Span::styled("\u{2717} ", Style::default().fg(Color::Red)), // ✗
                    Span::styled(
                        format!("Failed to fetch: {}", error),
                        Style::default().fg(Color::Red),
                    ),
                ]));
            }
            FetchStatus::NotStarted => {
                if entry.tracked {
                    detail_lines.push(Line::from(Span::styled(
                        "Waiting to fetch GitHub data...",
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
            FetchStatus::Loaded => {
                // No indicator needed when data is loaded
            }
        }

        // In-app update progress / result (this session only).
        if let Some(agents_app) = app.agents_app.as_ref() {
            let state = agents_app.update_states.get(&entry.id);
            let log = agents_app.update_logs.get(&entry.id);
            let has_log = log.map(|l| !l.is_empty()).unwrap_or(false);
            if state.is_some() || has_log {
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(Span::styled(
                    "Update:",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                detail_lines.push(Line::from(Span::styled(
                    "───────────────────────────────────",
                    Style::default().fg(Color::Gray),
                )));
                let state_span = match state {
                    Some(AgentUpdateState::Running) => {
                        Span::styled("\u{25D0} Updating…", Style::default().fg(Color::Magenta))
                    }
                    Some(AgentUpdateState::Succeeded) => {
                        Span::styled("\u{2713} Updated", Style::default().fg(Color::Green))
                    }
                    Some(AgentUpdateState::Failed) => {
                        Span::styled("\u{2717} Update failed", Style::default().fg(Color::Red))
                    }
                    None => Span::raw(""),
                };
                detail_lines.push(Line::from(state_span));
                // On failure (incl. timeout / no-TTY prompt), surface the command
                // so the user can run it in their own shell where they can answer
                // any prompts.
                if state == Some(&AgentUpdateState::Failed) {
                    if let Some(cmd) = entry.update_command() {
                        detail_lines.push(Line::from(vec![
                            Span::styled("Run manually: ", Style::default().fg(Color::Gray)),
                            Span::styled(
                                format!("$ {}", cmd.join(" ")),
                                Style::default().fg(Color::Yellow),
                            ),
                        ]));
                    }
                }
                if let Some(log) = log {
                    // Show the trailing slice (the buffer itself is already capped).
                    let start = log.len().saturating_sub(12);
                    for line in &log[start..] {
                        detail_lines.push(Line::from(Span::styled(
                            line.clone(),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
            }
        }

        detail_lines.push(Line::from(""));

        // Release history
        if entry.github.releases.is_empty() {
            detail_lines.push(Line::from(Span::styled(
                "No releases available",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            detail_lines.push(Line::from(Span::styled(
                "Release History:",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            detail_lines.push(Line::from(Span::styled(
                "───────────────────────────────────",
                Style::default().fg(Color::Gray),
            )));

            let installed_version = entry.installed.version.as_deref();
            let new_releases = entry.new_releases();

            for release in &entry.github.releases {
                let is_installed = installed_version == Some(release.version.as_str());
                let is_new = new_releases.iter().any(|r| r.version == release.version);

                // Version header with markers
                let mut version_spans = vec![Span::styled(
                    format!("v{}", release.version),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )];

                if let Some(date) = &release.date {
                    let display_date = crate::agents::helpers::parse_date(date)
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| date.clone());
                    version_spans.push(Span::styled(
                        format!("  {}", display_date),
                        Style::default().fg(Color::Gray),
                    ));
                }

                if is_installed {
                    version_spans.push(Span::styled(
                        "  ← INSTALLED",
                        Style::default().fg(Color::Green),
                    ));
                } else if is_new {
                    version_spans.push(Span::styled("  ← NEW", Style::default().fg(Color::Yellow)));
                }

                detail_lines.push(Line::from(version_spans));

                // Changelog for this release
                if let Some(changelog) = &release.changelog {
                    if search_query.is_empty() {
                        detail_lines.extend(crate::tui::markdown::changelog_to_lines(changelog));
                    } else {
                        let changelog_lines = crate::tui::markdown::changelog_to_lines_highlighted(
                            changelog,
                            &search_query,
                        );
                        for cl in changelog_lines {
                            if crate::tui::markdown::line_contains_match(&cl, &search_query) {
                                match_line_indices.push(detail_lines.len() as u16);
                            }
                            detail_lines.push(cl);
                        }
                    }
                }

                detail_lines.push(Line::from("")); // Space between releases
            }
        }

        // Keybinding hints at the bottom
        detail_lines.push(Line::from(""));
        let mut hints = vec![
            Span::styled(" o ", Style::default().fg(Color::Yellow)),
            Span::raw("open docs  "),
            Span::styled(" r ", Style::default().fg(Color::Yellow)),
            Span::raw("open repo  "),
            Span::styled(" c ", Style::default().fg(Color::Yellow)),
            Span::raw("copy name"),
        ];
        if !search_query.is_empty() {
            hints.push(Span::raw("  "));
            hints.push(Span::styled(" n/N ", Style::default().fg(Color::Yellow)));
            hints.push(Span::raw("next/prev match"));
        }
        detail_lines.push(Line::from(hints));

        detail_lines
    } else {
        vec![Line::from(Span::styled(
            "Select an agent to view details",
            Style::default().fg(Color::DarkGray),
        ))]
    };

    // Build detail title with match count
    let match_count = match_line_indices.len();
    let current_match_display = app
        .agents_app
        .as_ref()
        .map(|a| a.current_match)
        .unwrap_or(0);
    let detail_title = if !search_query.is_empty() && match_count > 0 {
        format!(
            "Details [/{} {}/{}]",
            search_query,
            current_match_display + 1,
            match_count
        )
    } else if !search_query.is_empty() {
        format!("Details [/{}]", search_query)
    } else {
        "Details".to_string()
    };

    let scroll_pos = app
        .agents_app
        .as_ref()
        .map(|a| a.detail_scroll)
        .unwrap_or(0);

    let scroll_offset = ScrollOffset::new(scroll_pos);
    let panel = ScrollablePanel::new(detail_title, lines, &scroll_offset, is_focused);
    let state = panel.render(f, area);

    // Compute visual offsets for match lines from the panel state
    let match_visual_offsets: Vec<u16> = match_line_indices
        .iter()
        .map(|&idx| state.visual_offsets.get(idx as usize).copied().unwrap_or(0))
        .collect();

    // Update match state and detail height (after lines are consumed)
    app.last_detail_height = state.visible_height;
    if let Some(ref mut agents_app) = app.agents_app {
        agents_app.detail_scroll = scroll_offset.get();
        agents_app.update_search_matches(match_line_indices, match_visual_offsets);
        // Cache the detail panel's outer rect for mouse hit-testing.
        agents_app.detail_area = Some(area);
    }
}

pub(in crate::tui) fn draw_picker_modal(f: &mut Frame, app: &App) {
    let agents_app = match &app.agents_app {
        Some(a) => a,
        None => return,
    };

    let num_agents = agents_app.entries.len();

    // Calculate popup dimensions
    // Width: 60 chars or screen width - 4, whichever is smaller
    let popup_width = std::cmp::min(60, f.area().width.saturating_sub(4));
    // Height: num agents + 4 (for borders and title/footer)
    let popup_height = std::cmp::min((num_agents + 4) as u16, f.area().height.saturating_sub(4));

    // Center the popup
    let area = centered_rect_fixed(popup_width, popup_height, f.area());

    // Cache the inner list rect for click hit-testing.
    agents_app
        .picker_area
        .set(Some(Block::default().borders(Borders::ALL).inner(area)));

    // Clear the background
    f.render_widget(Clear, area);

    // Build list items with checkboxes
    let items: Vec<ListItem> = agents_app
        .entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            // Get tracked state from picker_changes, fallback to entry.tracked
            let is_tracked = agents_app
                .picker_changes
                .get(&entry.id)
                .copied()
                .unwrap_or(entry.tracked);

            let checkbox = if is_tracked { "[x]" } else { "[ ]" };

            // Get first category or empty
            let category = entry
                .agent
                .categories
                .first()
                .map(|c| c.as_str())
                .unwrap_or("");

            // Installed status
            let installed_status = if entry.installed.version.is_some() {
                "installed"
            } else {
                ""
            };

            // Build the line with styled spans
            let line = Line::from(vec![
                Span::raw(format!("{} ", checkbox)),
                Span::styled(
                    format!("{:<20}", truncate(&entry.agent.name, 20)),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {:<10}", truncate(category, 10)),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!(" {}", installed_status),
                    Style::default().fg(Color::Green),
                ),
            ]);

            // Highlight selected row
            if idx == agents_app.picker_selected {
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
                .title(" Add/Remove Tracked Agents ")
                .title_bottom(Line::from(" Space: toggle | Enter: save | Esc: cancel ").centered()),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    // Create a ListState for proper scrolling
    let mut list_state = ListState::default();
    list_state.select(Some(agents_app.picker_selected));

    f.render_stateful_widget(list, area, &mut list_state);
}

/// Render the "Add Agent" modal — a minimal two-field form (name + `owner/repo`)
/// that writes a `CustomAgent` to config. The active field is Cyan+BOLD with a
/// blinking cursor; an inline validation error (if any) shows in Red.
pub(in crate::tui) fn draw_add_agent_modal(f: &mut Frame, app: &App) {
    let agents_app = match &app.agents_app {
        Some(a) => a,
        None => return,
    };
    let form = &agents_app.add_form;

    let popup_width = std::cmp::min(54, f.area().width.saturating_sub(4));
    let popup_height = std::cmp::min(11, f.area().height.saturating_sub(4));
    let area = centered_rect_fixed(popup_width, popup_height, f.area());

    f.render_widget(Clear, area);

    let label_style = Style::default().fg(Color::Gray);
    let active_label = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let cursor_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::SLOW_BLINK);

    let field_line = |label: &str, value: &str, active: bool, placeholder: &str| -> Line<'static> {
        let mut spans = vec![Span::styled(
            format!("  {:<7}", label),
            if active { active_label } else { label_style },
        )];
        spans.push(Span::raw(value.to_string()));
        if active {
            spans.push(Span::styled("_", cursor_style));
        } else if value.is_empty() && !placeholder.is_empty() {
            spans.push(Span::styled(
                placeholder.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        Line::from(spans)
    };

    let mut lines = vec![
        Line::from(""),
        field_line("Name:", &form.name, form.field == AddAgentField::Name, ""),
        Line::from(""),
        field_line(
            "Repo:",
            &form.repo,
            form.field == AddAgentField::Repo,
            "owner/name",
        ),
        Line::from(""),
    ];
    if let Some(err) = &form.error {
        lines.push(Line::from(Span::styled(
            format!("  {}", err),
            Style::default().fg(Color::Red),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Add Agent ")
        .title_bottom(Line::from(" Tab: next field | Enter: save | Esc: cancel ").centered());

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Render the update-confirm modal — lists the exact verified command(s) that
/// will run before anything executes. The user runs them with `Enter`.
pub(in crate::tui) fn draw_update_confirm_modal(f: &mut Frame, app: &App) {
    let agents_app = match &app.agents_app {
        Some(a) => a,
        None => return,
    };
    let targets = &agents_app.update_targets;
    if targets.is_empty() {
        return;
    }

    let popup_width = std::cmp::min(66, f.area().width.saturating_sub(4));
    // header + blank + one row per target + blank + note, plus 2 borders.
    let body = targets.len() + 4;
    let popup_height = std::cmp::min((body + 2) as u16, f.area().height.saturating_sub(4));
    let area = centered_rect_fixed(popup_width, popup_height, f.area());
    f.render_widget(Clear, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        if targets.len() == 1 {
            "  Run this update command?".to_string()
        } else {
            format!("  Run these {} update commands?", targets.len())
        },
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(""));
    for t in targets {
        let mut spans = vec![
            Span::styled(
                format!("  {:<14}", truncate(&t.name, 14)),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(
                format!("$ {}", t.command.join(" ")),
                Style::default().fg(Color::Yellow),
            ),
        ];
        // Show the detected install method so the user can see what's targeted.
        if let Some(method) = &t.method {
            spans.push(Span::styled(
                format!("  (via {})", method),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Runs in the background; output appears in the detail panel.",
        Style::default().fg(Color::DarkGray),
    )));

    let (title, bottom) = if targets.len() == 1 {
        // Single agent → offer the interactive (suspend-and-run) path too.
        (
            " Update Agent ",
            " Enter: background | i: interactive | Esc: cancel ",
        )
    } else {
        (" Update Agents ", " Enter: run | Esc: cancel ")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title)
        .title_bottom(Line::from(bottom).centered());
    f.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod mouse_tests {
    //! End-to-end checks for Agents-tab mouse handling: build an `App` with a
    //! populated `AgentsApp`, render into a `TestBackend` (which caches the panel
    //! rects + clamps the list offset through the filter-row + header offsets),
    //! then synthesize clicks/scroll and assert the resulting selection/focus.
    //! Mirrors `crate::tui::models::render::mouse_tests`.

    use std::collections::HashMap;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::widgets::ListState;
    use ratatui::{backend::TestBackend, Terminal};

    use crate::agents::{Agent, AgentEntry, FetchStatus, GitHubData, InstalledInfo, Release};
    use crate::data::ProvidersMap;
    use crate::tui::agents::app::{
        handle_agents_mouse, AddAgentForm, AgentFilters, AgentFocus, AgentSortOrder,
        AgentUpdateState, AgentsApp, UpdateTarget,
    };
    use crate::tui::app::{App, Tab};

    fn agent_entry(id: &str, name: &str) -> AgentEntry {
        AgentEntry {
            id: id.to_string(),
            agent: Agent {
                name: name.to_string(),
                repo: format!("owner/{id}"),
                categories: vec!["cli".to_string()],
                installation_method: None,
                pricing: None,
                supported_providers: vec![],
                platform_support: vec![],
                open_source: true,
                cli_binary: None,
                alt_binaries: vec![],
                version_command: vec![],
                update_command: vec![],
                version_regex: None,
                config_files: vec![],
                homepage: None,
                docs: None,
            },
            github: GitHubData {
                releases: vec![Release {
                    version: "1.0.0".to_string(),
                    date: Some("2024-01-01".to_string()),
                    changelog: None,
                }],
                ..GitHubData::default()
            },
            installed: InstalledInfo::default(),
            tracked: true,
            fetch_status: FetchStatus::Loaded,
        }
    }

    /// Build an `AgentsApp` with `n` tracked entries (`a00`..), bypassing the
    /// thread-spawning `AgentsApp::new`.
    fn agents_app(n: usize) -> AgentsApp {
        let entries: Vec<AgentEntry> = (0..n)
            .map(|i| agent_entry(&format!("a{i:02}"), &format!("Agent {i:02}")))
            .collect();
        let mut agent_list_state = ListState::default();
        agent_list_state.select(Some(0));
        AgentsApp {
            filtered_entries: (0..entries.len()).collect(),
            entries,
            selected_category: 0,
            selected_agent: 0,
            agent_list_state,
            focus: AgentFocus::List,
            filters: AgentFilters::default(),
            search_query: String::new(),
            sort_order: AgentSortOrder::Name,
            show_picker: false,
            picker_selected: 0,
            picker_changes: HashMap::new(),
            show_add_form: false,
            add_form: AddAgentForm::default(),
            show_update_confirm: false,
            update_targets: Vec::new(),
            update_states: HashMap::new(),
            update_logs: HashMap::new(),
            detail_scroll: 0,
            search_match_lines: Vec::new(),
            search_match_visual_offsets: Vec::new(),
            current_match: 0,
            loading_github: false,
            pending_github_fetches: 0,
            agent_list_area: None,
            detail_area: None,
            picker_area: std::cell::Cell::new(None),
        }
    }

    fn test_app(n: usize) -> App {
        // No providers needed; the Agents tab is independent of models data.
        let map: ProvidersMap = serde_json::from_str("{}").expect("empty providers json");
        let mut app = App::new(map, None, None);
        app.current_tab = Tab::Agents;
        app.agents_app = Some(agents_app(n));
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
    fn add_agent_modal_renders_form_with_typed_input() {
        let mut app = test_app(4);
        app.update(crate::tui::app::Message::OpenAddAgent);
        for c in "Amp".chars() {
            app.update(crate::tui::app::Message::AddAgentInput(c));
        }
        app.update(crate::tui::app::Message::AddAgentToggleField);
        for c in "sourcegraph/amp".chars() {
            app.update(crate::tui::app::Message::AddAgentInput(c));
        }
        // Render the full UI with the modal open; assert it draws expected text.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| crate::tui::ui::draw(f, &mut app))
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Add Agent"), "modal title should render");
        assert!(text.contains("Amp"), "typed name should render");
        assert!(text.contains("sourcegraph/amp"), "typed repo should render");
    }

    #[test]
    fn update_confirm_modal_renders_command() {
        let mut app = test_app(4);
        {
            let a = app.agents_app.as_mut().unwrap();
            a.show_update_confirm = true;
            a.update_targets = vec![UpdateTarget {
                id: "claude-code".to_string(),
                name: "Claude Code".to_string(),
                command: vec!["claude".to_string(), "update".to_string()],
                method: None,
            }];
        }
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| crate::tui::ui::draw(f, &mut app))
            .expect("draw");
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Update Agent"), "modal title should render");
        assert!(text.contains("claude update"), "command should render");
    }

    #[test]
    fn detail_panel_renders_update_progress() {
        let mut app = test_app(4);
        let id = app
            .agents_app
            .as_ref()
            .unwrap()
            .current_entry()
            .unwrap()
            .id
            .clone();
        {
            let a = app.agents_app.as_mut().unwrap();
            a.update_states
                .insert(id.clone(), AgentUpdateState::Running);
            a.update_logs
                .insert(id.clone(), vec!["downloading archive".to_string()]);
        }
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| crate::tui::ui::draw(f, &mut app))
            .expect("draw");
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Updating"), "update state should render");
        assert!(
            text.contains("downloading archive"),
            "update output should render"
        );
    }

    #[test]
    fn detail_panel_shows_run_manually_on_failed_update() {
        let mut app = test_app(4);
        let id = app
            .agents_app
            .as_ref()
            .unwrap()
            .current_entry()
            .unwrap()
            .id
            .clone();
        {
            let a = app.agents_app.as_mut().unwrap();
            for e in &mut a.entries {
                e.agent.update_command = vec!["mytool".to_string(), "update".to_string()];
            }
            a.update_states.insert(id.clone(), AgentUpdateState::Failed);
            a.update_logs
                .insert(id.clone(), vec!["error: boom".to_string()]);
        }
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| crate::tui::ui::draw(f, &mut app))
            .expect("draw");
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            text.contains("Run manually"),
            "manual-run hint should render"
        );
        assert!(text.contains("mytool update"), "command should render");
    }

    #[test]
    fn click_agent_row_at_top_selects_that_agent() {
        let mut app = test_app(8);
        render(&mut app, 120, 40);
        let area = app
            .agents_app
            .as_ref()
            .unwrap()
            .agent_list_area
            .expect("agent list rect cached");
        // Item 0 is the header at area.y; first agent is one row below.
        handle_agents_mouse(&mut app, click(area.x + 4, area.y + 1));
        let a = app.agents_app.as_ref().unwrap();
        assert_eq!(a.focus, AgentFocus::List);
        assert_eq!(a.selected_agent, 0);
        // Click two rows below the header → agent index 2.
        handle_agents_mouse(&mut app, click(area.x + 4, area.y + 3));
        assert_eq!(app.agents_app.as_ref().unwrap().selected_agent, 2);
        // Clicking the header row itself changes nothing.
        handle_agents_mouse(&mut app, click(area.x + 4, area.y));
        assert_eq!(app.agents_app.as_ref().unwrap().selected_agent, 2);
    }

    #[test]
    fn click_agent_row_with_nonzero_scroll_offset() {
        // Short viewport forces the list to scroll once selection nears the end.
        let mut app = test_app(30);
        // Drive selection deep so the list scrolls (header item 0 leaves view).
        if let Some(ref mut a) = app.agents_app {
            for _ in 0..25 {
                a.next_agent();
            }
        }
        render(&mut app, 120, 18);
        let (area, offset) = {
            let a = app.agents_app.as_ref().unwrap();
            (
                a.agent_list_area.expect("agent list rect cached"),
                a.agent_list_state.offset(),
            )
        };
        assert!(offset > 0, "list should have scrolled (offset={offset})");
        // Click two rows below the top visible row. Top visible list-item index
        // is `offset`; +2 rows → item `offset+2` → agent `offset+1`.
        handle_agents_mouse(&mut app, click(area.x + 4, area.y + 2));
        let expected = offset + 2 - 1; // -1 for the header item at index 0
        assert_eq!(app.agents_app.as_ref().unwrap().selected_agent, expected);
    }

    #[test]
    fn scroll_wheel_over_agent_list_focuses_and_moves() {
        let mut app = test_app(8);
        render(&mut app, 120, 40);
        let area = app
            .agents_app
            .as_ref()
            .unwrap()
            .agent_list_area
            .expect("agent list rect cached");
        assert_eq!(app.agents_app.as_ref().unwrap().selected_agent, 0);
        handle_agents_mouse(&mut app, scroll(area.x + 4, area.y + 5, true));
        {
            let a = app.agents_app.as_ref().unwrap();
            assert_eq!(a.focus, AgentFocus::List);
            assert_eq!(a.selected_agent, 1);
        }
        handle_agents_mouse(&mut app, scroll(area.x + 4, area.y + 5, false));
        assert_eq!(app.agents_app.as_ref().unwrap().selected_agent, 0);
    }

    #[test]
    fn click_detail_panel_focuses_details_only() {
        let mut app = test_app(8);
        render(&mut app, 120, 40);
        let (area, before) = {
            let a = app.agents_app.as_ref().unwrap();
            (a.detail_area.expect("detail rect cached"), a.selected_agent)
        };
        handle_agents_mouse(&mut app, click(area.x + 2, area.y + 2));
        let a = app.agents_app.as_ref().unwrap();
        assert_eq!(a.focus, AgentFocus::Details);
        assert_eq!(a.selected_agent, before); // no row selection
    }

    #[test]
    fn scroll_wheel_over_detail_scrolls() {
        let mut app = test_app(8);
        render(&mut app, 120, 40);
        let area = app
            .agents_app
            .as_ref()
            .unwrap()
            .detail_area
            .expect("detail rect cached");
        handle_agents_mouse(&mut app, scroll(area.x + 2, area.y + 2, true));
        {
            let a = app.agents_app.as_ref().unwrap();
            assert_eq!(a.focus, AgentFocus::Details);
            assert_eq!(a.detail_scroll, 1);
        }
        handle_agents_mouse(&mut app, scroll(area.x + 2, area.y + 2, false));
        assert_eq!(app.agents_app.as_ref().unwrap().detail_scroll, 0);
    }
}
