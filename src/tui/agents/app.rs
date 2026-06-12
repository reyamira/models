use std::collections::HashMap;

use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::agents::{detect_installed, AgentEntry, AgentsFile, FetchStatus, GitHubData};
use crate::config::Config;
use crate::tui::mouse::{hit, row_at};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentSortOrder {
    Name,
    #[default]
    Updated,
    Stars,
    Status,
}

impl AgentSortOrder {
    pub fn next(self) -> Self {
        match self {
            AgentSortOrder::Name => AgentSortOrder::Updated,
            AgentSortOrder::Updated => AgentSortOrder::Stars,
            AgentSortOrder::Stars => AgentSortOrder::Status,
            AgentSortOrder::Status => AgentSortOrder::Name,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AgentSortOrder::Name => "name",
            AgentSortOrder::Updated => "updated",
            AgentSortOrder::Stars => "stars",
            AgentSortOrder::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentCategory {
    #[default]
    All,
    Installed,
    Cli,
    Ide,
    OpenSource,
}

impl AgentCategory {
    pub fn label(&self) -> &'static str {
        match self {
            AgentCategory::All => "All",
            AgentCategory::Installed => "Installed",
            AgentCategory::Cli => "CLI Tools",
            AgentCategory::Ide => "IDEs",
            AgentCategory::OpenSource => "Open Source",
        }
    }

    pub fn variants() -> &'static [AgentCategory] {
        &[
            AgentCategory::All,
            AgentCategory::Installed,
            AgentCategory::Cli,
            AgentCategory::Ide,
            AgentCategory::OpenSource,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentFocus {
    #[default]
    List,
    Details,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AgentFilters {
    pub installed_only: bool,
    pub cli_only: bool,
    pub open_source_only: bool,
}

pub struct AgentsApp {
    pub entries: Vec<AgentEntry>,
    pub filtered_entries: Vec<usize>, // indices into entries
    pub selected_category: usize,
    pub selected_agent: usize,
    pub agent_list_state: ListState,
    pub focus: AgentFocus,
    pub filters: AgentFilters,
    pub search_query: String,
    pub sort_order: AgentSortOrder,
    // Picker modal state
    pub show_picker: bool,
    pub picker_selected: usize,
    pub picker_changes: HashMap<String, bool>, // agent_id -> new tracked state
    // Detail panel scroll
    pub detail_scroll: u16,
    // Search match navigation (line indices in detail content)
    pub search_match_lines: Vec<u16>,
    pub search_match_visual_offsets: Vec<u16>, // visual (wrapped) line offsets for scroll
    pub current_match: usize,
    // Loading state for async GitHub fetches
    pub loading_github: bool,
    pub pending_github_fetches: usize,
    /// Panel rects cached at render time for mouse hit-testing (see
    /// `crate::tui::mouse`). `agent_list_area` is the bare list region below the
    /// filter-toggle row (the header is rendered as list item 0, so `row_at`
    /// uses `top_skip = 0` and the handler subtracts 1 for the header).
    /// `detail_area` is the scrollable detail panel's outer rect.
    pub agent_list_area: Option<Rect>,
    pub detail_area: Option<Rect>,
}

impl AgentsApp {
    pub fn new(agents_file: &AgentsFile, config: &Config) -> Self {
        use std::sync::mpsc;
        use std::thread;

        // Collect agents that need version detection (tracked only)
        let agents_to_detect: Vec<_> = agents_file
            .agents
            .iter()
            .filter(|(id, _)| config.is_tracked(id))
            .map(|(id, agent)| (id.clone(), agent.clone()))
            .collect();

        // Run version detection in parallel using threads
        let (tx, rx) = mpsc::channel();
        for (id, agent) in agents_to_detect {
            let tx = tx.clone();
            thread::spawn(move || {
                let installed = detect_installed(&agent);
                let _ = tx.send((id, installed));
            });
        }
        drop(tx); // Close sender so rx.iter() terminates

        // Collect results
        let detected: std::collections::HashMap<String, _> = rx.iter().collect();

        // Build entries with detected versions
        let mut entries: Vec<AgentEntry> = agents_file
            .agents
            .iter()
            .map(|(id, agent)| {
                let tracked = config.is_tracked(id);
                let installed = detected.get(id).cloned().unwrap_or_default();
                AgentEntry {
                    id: id.clone(),
                    agent: agent.clone(),
                    github: GitHubData::default(),
                    installed,
                    tracked,
                    fetch_status: if tracked {
                        FetchStatus::Loading
                    } else {
                        FetchStatus::NotStarted
                    },
                }
            })
            .collect();

        // Add custom agents from config
        for custom in &config.agents.custom {
            let id = custom.name.to_lowercase().replace(' ', "-");
            // Skip if already exists (curated agent takes precedence)
            if entries.iter().any(|e| e.id == id) {
                continue;
            }
            let agent = custom.to_agent();
            let installed = detect_installed(&agent);
            let tracked = config.is_tracked(&id);
            entries.push(AgentEntry {
                id,
                agent,
                github: GitHubData::default(),
                installed,
                tracked,
                fetch_status: if tracked {
                    FetchStatus::Loading
                } else {
                    FetchStatus::NotStarted
                },
            });
        }

        // Sort by name
        entries.sort_by(|a, b| a.agent.name.cmp(&b.agent.name));

        let mut agent_list_state = ListState::default();
        agent_list_state.select(Some(0));

        // Only count tracked agents for pending fetches
        let pending_fetches = entries.iter().filter(|e| e.tracked).count();
        let mut app = Self {
            entries,
            filtered_entries: Vec::new(),
            selected_category: 0,
            selected_agent: 0,
            agent_list_state,
            focus: AgentFocus::default(),
            filters: AgentFilters::default(),
            search_query: String::new(),
            sort_order: AgentSortOrder::default(),
            show_picker: false,
            picker_selected: 0,
            picker_changes: HashMap::new(),
            detail_scroll: 0,
            search_match_lines: Vec::new(),
            search_match_visual_offsets: Vec::new(),
            current_match: 0,
            loading_github: true,
            pending_github_fetches: pending_fetches,
            agent_list_area: None,
            detail_area: None,
        };

        app.update_filtered();
        app
    }

    pub fn update_filtered(&mut self) {
        let category = AgentCategory::variants()[self.selected_category];
        let query_lower = self.search_query.to_lowercase();

        self.filtered_entries = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                // Category filter
                let category_match = match category {
                    AgentCategory::All => true,
                    AgentCategory::Installed => entry.installed.version.is_some(),
                    AgentCategory::Cli => entry.agent.categories.contains(&"cli".to_string()),
                    AgentCategory::Ide => entry.agent.categories.contains(&"ide".to_string()),
                    AgentCategory::OpenSource => entry.agent.open_source,
                };

                // Tracked agents only (primary filter)
                if !entry.tracked {
                    return false;
                }

                // Additional filters
                let filter_match = (!self.filters.installed_only
                    || entry.installed.version.is_some())
                    && (!self.filters.cli_only
                        || entry.agent.categories.contains(&"cli".to_string()))
                    && (!self.filters.open_source_only || entry.agent.open_source);

                // Search filter (includes changelog content)
                let search_match = query_lower.is_empty()
                    || entry.agent.name.to_lowercase().contains(&query_lower)
                    || entry.id.to_lowercase().contains(&query_lower)
                    || entry.github.releases.iter().any(|r| {
                        r.changelog
                            .as_ref()
                            .is_some_and(|c| c.to_lowercase().contains(&query_lower))
                    });

                category_match && filter_match && search_match
            })
            .map(|(i, _)| i)
            .collect();

        self.apply_sort();

        // Reset selection if out of bounds
        if self.selected_agent >= self.filtered_entries.len() {
            self.selected_agent = 0;
        }
        self.agent_list_state.select(Some(self.selected_agent));
    }

    pub fn cycle_sort(&mut self) {
        self.sort_order = self.sort_order.next();
        self.apply_sort();
    }

    pub fn apply_sort(&mut self) {
        let entries = &self.entries;
        self.filtered_entries.sort_by(|&a, &b| {
            let ea = &entries[a];
            let eb = &entries[b];
            match self.sort_order {
                AgentSortOrder::Name => ea.agent.name.cmp(&eb.agent.name),
                AgentSortOrder::Updated => {
                    let da = ea
                        .github
                        .latest_release()
                        .and_then(|r| r.date.as_deref())
                        .and_then(crate::agents::helpers::parse_date);
                    let db = eb
                        .github
                        .latest_release()
                        .and_then(|r| r.date.as_deref())
                        .and_then(crate::agents::helpers::parse_date);
                    match (da, db) {
                        (Some(da), Some(db)) => db.cmp(&da), // Descending (newest first)
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => ea.agent.name.cmp(&eb.agent.name),
                    }
                }
                AgentSortOrder::Stars => {
                    let sa = ea.github.stars.unwrap_or(0);
                    let sb = eb.github.stars.unwrap_or(0);
                    sb.cmp(&sa) // Descending (most stars first)
                }
                AgentSortOrder::Status => {
                    let status_a = if ea.update_available() {
                        0
                    } else if ea.installed.version.is_some() {
                        1
                    } else {
                        2
                    };
                    let status_b = if eb.update_available() {
                        0
                    } else if eb.installed.version.is_some() {
                        1
                    } else {
                        2
                    };
                    status_a.cmp(&status_b)
                }
            }
        });
    }

    pub fn current_entry(&self) -> Option<&AgentEntry> {
        self.filtered_entries
            .get(self.selected_agent)
            .and_then(|&i| self.entries.get(i))
    }

    pub fn next_agent(&mut self) {
        if self.selected_agent < self.filtered_entries.len().saturating_sub(1) {
            self.selected_agent += 1;
            self.agent_list_state.select(Some(self.selected_agent));
            self.detail_scroll = 0;
        }
    }

    pub fn prev_agent(&mut self) {
        if self.selected_agent > 0 {
            self.selected_agent -= 1;
            self.agent_list_state.select(Some(self.selected_agent));
            self.detail_scroll = 0;
        }
    }

    /// Select an agent by its index into `filtered_entries` (used by mouse
    /// clicks). The list state stays 0-based here; `render.rs` applies the
    /// `+1` header offset onto the real state at render time.
    pub fn select_agent_at_index(&mut self, index: usize) {
        if index < self.filtered_entries.len() && index != self.selected_agent {
            self.selected_agent = index;
            self.agent_list_state.select(Some(index));
            self.detail_scroll = 0;
        }
    }

    pub fn select_first_agent(&mut self) {
        if self.selected_agent > 0 {
            self.selected_agent = 0;
            self.agent_list_state.select(Some(0));
            self.detail_scroll = 0;
        }
    }

    pub fn select_last_agent(&mut self) {
        let last = self.filtered_entries.len().saturating_sub(1);
        if self.selected_agent < last {
            self.selected_agent = last;
            self.agent_list_state.select(Some(last));
            self.detail_scroll = 0;
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        let last_index = self.filtered_entries.len().saturating_sub(1);
        self.selected_agent = (self.selected_agent + page_size).min(last_index);
        self.agent_list_state.select(Some(self.selected_agent));
        self.detail_scroll = 0;
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected_agent = self.selected_agent.saturating_sub(page_size);
        self.agent_list_state.select(Some(self.selected_agent));
        self.detail_scroll = 0;
    }

    pub fn switch_focus(&mut self) {
        self.focus = match self.focus {
            AgentFocus::List => AgentFocus::Details,
            AgentFocus::Details => AgentFocus::List,
        };
    }

    pub fn toggle_installed_filter(&mut self) {
        self.filters.installed_only = !self.filters.installed_only;
        self.selected_agent = 0;
        self.update_filtered();
    }

    pub fn toggle_cli_filter(&mut self) {
        self.filters.cli_only = !self.filters.cli_only;
        self.selected_agent = 0;
        self.update_filtered();
    }

    pub fn toggle_open_source_filter(&mut self) {
        self.filters.open_source_only = !self.filters.open_source_only;
        self.selected_agent = 0;
        self.update_filtered();
    }

    // Picker modal methods
    pub fn open_picker(&mut self) {
        self.show_picker = true;
        self.picker_selected = 0;
        self.picker_changes.clear();
        // Initialize with current tracked states
        for entry in &self.entries {
            self.picker_changes.insert(entry.id.clone(), entry.tracked);
        }
    }

    pub fn close_picker(&mut self) {
        self.show_picker = false;
        self.picker_changes.clear();
    }

    pub fn picker_toggle_current(&mut self) {
        if let Some(entry) = self.entries.get(self.picker_selected) {
            let current = self
                .picker_changes
                .get(&entry.id)
                .copied()
                .unwrap_or(entry.tracked);
            self.picker_changes.insert(entry.id.clone(), !current);
        }
    }

    pub fn picker_next(&mut self) {
        if self.picker_selected < self.entries.len().saturating_sub(1) {
            self.picker_selected += 1;
        }
    }

    pub fn picker_prev(&mut self) {
        if self.picker_selected > 0 {
            self.picker_selected -= 1;
        }
    }

    /// Save picker changes and return list of newly tracked agents (id, repo) for fetching
    pub fn picker_save(&mut self, config: &mut Config) -> Result<Vec<(String, String)>, String> {
        let mut newly_tracked = Vec::new();

        for (agent_id, tracked) in &self.picker_changes {
            config.set_tracked(agent_id, *tracked);
            if let Some(entry) = self.entries.iter_mut().find(|e| e.id == *agent_id) {
                // Track if this is a newly tracked agent (was not tracked, now is)
                if *tracked && !entry.tracked {
                    newly_tracked.push((agent_id.clone(), entry.agent.repo.clone()));
                    entry.fetch_status = FetchStatus::Loading;
                }
                entry.tracked = *tracked;
            }
        }

        if let Err(e) = config.save() {
            self.close_picker();
            return Err(format!("Failed to save config: {}", e));
        }

        self.close_picker();
        self.update_filtered(); // Re-filter in case tracked_only is active
        Ok(newly_tracked)
    }

    /// Update match line indices and visual offsets from rendered detail content.
    /// Only resets current_match when the match set actually changes.
    pub fn update_search_matches(&mut self, match_lines: Vec<u16>, visual_offsets: Vec<u16>) {
        if self.search_match_lines != match_lines {
            self.search_match_lines = match_lines;
            self.search_match_visual_offsets = visual_offsets;
            self.current_match = 0;
        }
    }

    /// Jump to next search match, returning the scroll position.
    pub fn next_search_match(&mut self, visible_height: u16) -> Option<u16> {
        if self.search_match_lines.is_empty() || self.search_match_visual_offsets.is_empty() {
            return None;
        }
        self.current_match = (self.current_match + 1) % self.search_match_lines.len();
        let visual = self.search_match_visual_offsets[self.current_match];
        Some(self.scroll_to_current_match(visible_height, visual))
    }

    /// Jump to previous search match, returning the scroll position.
    pub fn prev_search_match(&mut self, visible_height: u16) -> Option<u16> {
        if self.search_match_lines.is_empty() || self.search_match_visual_offsets.is_empty() {
            return None;
        }
        if self.current_match == 0 {
            self.current_match = self.search_match_lines.len() - 1;
        } else {
            self.current_match -= 1;
        }
        let visual = self.search_match_visual_offsets[self.current_match];
        Some(self.scroll_to_current_match(visible_height, visual))
    }

    fn scroll_to_current_match(&self, visible_height: u16, visual_offset: u16) -> u16 {
        // Use the pre-computed visual line offset (accounts for wrapping)
        visual_offset.saturating_sub(visible_height / 2)
    }

    /// Format active filters for display in block title
    pub fn format_active_filters(&self) -> String {
        let mut active = Vec::new();

        // Category (if not "All")
        let category = AgentCategory::variants()[self.selected_category];
        if category != AgentCategory::All {
            active.push(category.label().to_lowercase());
        }

        // Additional filters
        if self.filters.installed_only {
            active.push("installed".to_string());
        }
        if self.filters.cli_only {
            active.push("cli".to_string());
        }
        if self.filters.open_source_only {
            active.push("open".to_string());
        }

        active.join(", ")
    }
}

/// Handle a mouse event while the Agents tab is active.
///
/// All state changes (focus, selection, scroll) are applied directly to `app`,
/// so the function returns `None`; the main loop redraws after every event. The
/// `Option<Message>` return keeps the per-tab handler signature uniform with the
/// other tabs' dispatchers. See `crate::tui::models::handle_models_mouse` for
/// the reference pattern and `crate::tui::mouse` for the hit-test helpers.
pub fn handle_agents_mouse(
    app: &mut crate::tui::app::App,
    ev: crossterm::event::MouseEvent,
) -> Option<crate::tui::app::Message> {
    let agents_app = match app.agents_app {
        Some(ref mut a) => a,
        None => return None,
    };

    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if hit(agents_app.agent_list_area, &ev) {
                agents_app.focus = AgentFocus::List;
                if let Some(area) = agents_app.agent_list_area {
                    // Item 0 is the column header; agents occupy items 1..=N.
                    if let Some(idx) = row_at(
                        area,
                        agents_app.agent_list_state.offset(),
                        0,
                        agents_app.filtered_entries.len() + 1,
                        ev.row,
                    ) {
                        if let Some(agent_idx) = idx.checked_sub(1) {
                            agents_app.select_agent_at_index(agent_idx);
                        }
                    }
                }
            } else if hit(agents_app.detail_area, &ev) {
                agents_app.focus = AgentFocus::Details;
            }
        }
        // Wheel: focus the panel under the cursor, then scroll it (reusing the
        // same per-panel nav the arrow keys drive).
        MouseEventKind::ScrollDown => {
            if hit(agents_app.agent_list_area, &ev) {
                agents_app.focus = AgentFocus::List;
                agents_app.next_agent();
            } else if hit(agents_app.detail_area, &ev) {
                agents_app.focus = AgentFocus::Details;
                agents_app.detail_scroll = agents_app.detail_scroll.saturating_add(1);
            }
        }
        MouseEventKind::ScrollUp => {
            if hit(agents_app.agent_list_area, &ev) {
                agents_app.focus = AgentFocus::List;
                agents_app.prev_agent();
            } else if hit(agents_app.detail_area, &ev) {
                agents_app.focus = AgentFocus::Details;
                agents_app.detail_scroll = agents_app.detail_scroll.saturating_sub(1);
            }
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{Agent, FetchStatus, GitHubData, InstalledInfo, Release};

    fn agent_entry(id: &str, name: &str, release_date: Option<&str>) -> AgentEntry {
        AgentEntry {
            id: id.to_string(),
            agent: Agent {
                name: name.to_string(),
                repo: format!("owner/{id}"),
                categories: vec![],
                installation_method: None,
                pricing: None,
                supported_providers: vec![],
                platform_support: vec![],
                open_source: true,
                cli_binary: None,
                alt_binaries: vec![],
                version_command: vec![],
                version_regex: None,
                config_files: vec![],
                homepage: None,
                docs: None,
            },
            github: GitHubData {
                releases: vec![Release {
                    version: "1.0.0".to_string(),
                    date: release_date.map(str::to_string),
                    changelog: None,
                }],
                ..GitHubData::default()
            },
            installed: InstalledInfo::default(),
            tracked: true,
            fetch_status: FetchStatus::Loaded,
        }
    }

    fn test_app(entries: Vec<AgentEntry>) -> AgentsApp {
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
            sort_order: AgentSortOrder::Updated,
            show_picker: false,
            picker_selected: 0,
            picker_changes: HashMap::new(),
            detail_scroll: 0,
            search_match_lines: Vec::new(),
            search_match_visual_offsets: Vec::new(),
            current_match: 0,
            loading_github: false,
            pending_github_fetches: 0,
            agent_list_area: None,
            detail_area: None,
        }
    }

    #[test]
    fn updated_sort_uses_parsed_timestamps_not_lexical_order() {
        let mut app = test_app(vec![
            agent_entry(
                "offset-older",
                "Offset Older",
                Some("2024-01-01T00:30:00+01:00"),
            ),
            agent_entry("utc-newer", "UTC Newer", Some("2023-12-31T23:45:00Z")),
            agent_entry("no-date", "No Date", None),
        ]);

        app.apply_sort();

        let ordered_ids: Vec<_> = app
            .filtered_entries
            .iter()
            .map(|&idx| app.entries[idx].id.as_str())
            .collect();
        assert_eq!(ordered_ids, vec!["utc-newer", "offset-older", "no-date"]);
    }
}
