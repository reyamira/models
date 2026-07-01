use std::collections::HashMap;

use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::agents::{
    detect_installed, AgentEntry, AgentsFile, FetchStatus, GitHubData, InstalledInfo,
};
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

/// Which field of the "Add Agent" form currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddAgentField {
    #[default]
    Name,
    Repo,
}

impl AddAgentField {
    fn toggled(self) -> Self {
        match self {
            AddAgentField::Name => AddAgentField::Repo,
            AddAgentField::Repo => AddAgentField::Name,
        }
    }
}

/// State for the in-app "Add Agent" modal — a minimal two-field form (name +
/// `owner/repo`) that writes a `CustomAgent` to config without the user editing
/// `config.toml` by hand.
#[derive(Debug, Clone, Default)]
pub struct AddAgentForm {
    pub name: String,
    pub repo: String,
    pub field: AddAgentField,
    /// Inline validation error shown beneath the fields (cleared on next input).
    pub error: Option<String>,
}

impl AddAgentForm {
    fn active_mut(&mut self) -> &mut String {
        match self.field {
            AddAgentField::Name => &mut self.name,
            AddAgentField::Repo => &mut self.repo,
        }
    }
}

/// Lifecycle of an in-app agent self-update (subprocess run in the background).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentUpdateState {
    Running,
    Succeeded,
    Failed,
}

/// One agent queued for update, shown in the confirm modal before anything runs.
#[derive(Debug, Clone)]
pub struct UpdateTarget {
    pub id: String,
    pub name: String,
    /// Verified argv (no shell), e.g. `["claude", "update"]` — already made
    /// install-aware via `AgentEntry::resolved_update_command`.
    pub command: Vec<String>,
    /// Detected install method label (e.g. "Homebrew"), shown in the confirm
    /// modal so the user can see what the command targets. `None` if unknown.
    pub method: Option<String>,
    /// The command needs an interactive terminal (sudo / AUR-helper prompts).
    /// Background execution can't answer a password prompt, so these are excluded
    /// from `U` (update-all) and route `u` to the interactive (suspend-and-run) path.
    pub needs_terminal: bool,
}

/// Whether an update command requires an interactive terminal — a system package
/// manager or `sudo`, which prompt for a password / AUR review that a backgrounded
/// child (detached from the TTY) can't answer.
pub fn command_needs_terminal(command: &[String]) -> bool {
    matches!(
        command.first().map(String::as_str),
        Some("sudo" | "paru" | "yay" | "pacman" | "apt" | "dnf")
    )
}

/// Cap on retained per-agent update output lines (oldest dropped) so a chatty
/// updater can't grow the buffer unbounded.
const UPDATE_LOG_CAP: usize = 200;

/// True when `repo` is a plausible `owner/name` GitHub slug: exactly one `/`,
/// non-empty halves, and only characters GitHub allows in owner/repo names.
fn is_valid_repo_slug(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let valid_part = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    valid_part(owner) && valid_part(name)
}

/// Candidate CLI binary names to probe when adding a custom agent, in priority
/// order: the repo's last path segment, then the display name (kebab and
/// joined). Lowercased, de-duplicated, empties dropped. The repo segment comes
/// first because it's usually the actual binary name (e.g. `sourcegraph/amp` →
/// `amp`).
fn binary_candidates(name: &str, repo: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    if let Some(seg) = repo.rsplit('/').next() {
        push(seg.to_lowercase());
    }
    push(name.to_lowercase().replace(' ', "-"));
    push(name.to_lowercase().replace(' ', ""));
    out
}

/// Probe inferred binaries for a custom agent. Returns the first candidate whose
/// `<bin> --version` yields a version, with its detected install info (version +
/// path). `None` if nothing resolves — the agent stays detection-less.
fn detect_custom_agent(name: &str, repo: &str) -> Option<(String, InstalledInfo)> {
    for cand in binary_candidates(name, repo) {
        // Reuse CustomAgent::to_agent so the probe Agent matches the real shape.
        let probe = crate::config::CustomAgent {
            name: name.to_string(),
            repo: repo.to_string(),
            agent_type: Some("cli".to_string()),
            binary: Some(cand.clone()),
            version_command: Some(vec!["--version".to_string()]),
        }
        .to_agent();
        let info = detect_installed(&probe);
        if info.version.is_some() {
            return Some((cand, info));
        }
    }
    None
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
    // Add-agent form modal state
    pub show_add_form: bool,
    pub add_form: AddAgentForm,
    // Update action state
    pub show_update_confirm: bool,
    /// Agents queued in the confirm modal (cleared when it closes).
    pub update_targets: Vec<UpdateTarget>,
    /// Per-agent update lifecycle (absent = idle / never updated this session).
    pub update_states: HashMap<String, AgentUpdateState>,
    /// Per-agent captured update output (bounded to `UPDATE_LOG_CAP` lines).
    pub update_logs: HashMap<String, Vec<String>>,
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
    /// Inner list rect of the tracker modal (borders excluded), cached for click
    /// hit-testing. `Cell` so the `&App` render path can write it.
    pub picker_area: std::cell::Cell<Option<Rect>>,
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
            let mut agent = custom.to_agent();
            // If no binary was recorded (e.g. added before install-detection, or
            // not installed at add time), best-effort infer+detect it now so a
            // later install shows up without re-adding the agent.
            let installed = if agent.cli_binary.is_none() {
                match detect_custom_agent(&custom.name, &custom.repo) {
                    Some((bin, info)) => {
                        agent.cli_binary = Some(bin);
                        agent.version_command = vec!["--version".to_string()];
                        if agent.categories.is_empty() {
                            agent.categories = vec!["cli".to_string()];
                        }
                        info
                    }
                    None => InstalledInfo::default(),
                }
            } else {
                detect_installed(&agent)
            };
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
            loading_github: true,
            pending_github_fetches: pending_fetches,
            agent_list_area: None,
            detail_area: None,
            picker_area: std::cell::Cell::new(None),
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

    // Add-agent form methods
    pub fn open_add_form(&mut self) {
        self.show_add_form = true;
        self.add_form = AddAgentForm::default();
    }

    pub fn close_add_form(&mut self) {
        self.show_add_form = false;
        self.add_form = AddAgentForm::default();
    }

    pub fn add_form_input(&mut self, c: char) {
        // Ignore control chars; the field accepts plain text only.
        if c.is_control() {
            return;
        }
        self.add_form.error = None;
        self.add_form.active_mut().push(c);
    }

    pub fn add_form_backspace(&mut self) {
        self.add_form.error = None;
        self.add_form.active_mut().pop();
    }

    pub fn add_form_toggle_field(&mut self) {
        self.add_form.field = self.add_form.field.toggled();
    }

    /// Validate the form, persist a `CustomAgent` to config, and create a tracked
    /// `AgentEntry` in-memory. Returns `(id, repo)` for the GitHub fetch the main
    /// loop will spawn, or an error message to show inline (the form stays open on
    /// validation errors so the user can correct them; it closes on success).
    pub fn add_agent_save(&mut self, config: &mut Config) -> Result<(String, String), String> {
        let name = self.add_form.name.trim().to_string();
        let repo = self.add_form.repo.trim().to_string();

        if name.is_empty() {
            self.add_form.error = Some("Name is required".to_string());
            self.add_form.field = AddAgentField::Name;
            return Err("Name is required".to_string());
        }
        if !is_valid_repo_slug(&repo) {
            self.add_form.error = Some("Repo must be in owner/name form".to_string());
            self.add_form.field = AddAgentField::Repo;
            return Err("Repo must be in owner/name form".to_string());
        }

        // Id is derived from the name exactly as `AgentsApp::new` derives it for
        // config-loaded custom agents, so a restart re-resolves to the same id.
        let id = name.to_lowercase().replace(' ', "-");
        if self.entries.iter().any(|e| e.id == id) {
            let msg = format!("An agent named \"{}\" already exists", name);
            self.add_form.error = Some(msg.clone());
            return Err(msg);
        }

        // Best-effort install detection: infer a binary from the repo/name and
        // probe `<bin> --version`. If it resolves, record the binary + CLI type so
        // the entry shows installed/path (and the saved CustomAgent re-detects on
        // restart). The form stays name+repo only — this just uses what we can
        // derive. `None` keeps the agent detection-less, exactly as before.
        let detected = detect_custom_agent(&name, &repo);
        let (binary, version_command, agent_type, installed) = match detected {
            Some((bin, info)) => (
                Some(bin),
                Some(vec!["--version".to_string()]),
                Some("cli".to_string()),
                info,
            ),
            None => (None, None, None, InstalledInfo::default()),
        };

        let custom = crate::config::CustomAgent {
            name: name.clone(),
            repo: repo.clone(),
            agent_type,
            binary,
            version_command,
        };
        let agent = custom.to_agent();
        config.agents.custom.push(custom);
        // Persist as tracked so it survives a restart (custom agents are only
        // shown when their id is in config.agents.tracked).
        config.set_tracked(&id, true);
        if let Err(e) = config.save() {
            // Roll back the in-memory config mutation so a retry doesn't duplicate.
            config.agents.custom.retain(|c| c.name != name);
            return Err(format!("Failed to save config: {}", e));
        }

        self.entries.push(AgentEntry {
            id: id.clone(),
            agent,
            github: GitHubData::default(),
            installed,
            tracked: true,
            fetch_status: FetchStatus::Loading,
        });
        // Keep the by-name sort invariant the constructor establishes.
        self.entries.sort_by(|a, b| a.agent.name.cmp(&b.agent.name));

        self.close_add_form();
        self.update_filtered();
        Ok((id, repo))
    }

    // Update-action methods

    /// Build a confirm-modal target for the currently selected agent, if it has a
    /// verified updater. Returns `Err` with a status message otherwise (no updater,
    /// or an update already running for it).
    pub fn request_update_selected(&mut self) -> Result<(), String> {
        let entry = self
            .current_entry()
            .ok_or_else(|| "No agent selected".to_string())?;
        let command = match entry.resolved_update_command() {
            Some(c) => c,
            None => return Err(format!("No in-app updater for {}", entry.agent.name)),
        };
        // Nothing to update if it isn't installed — running the updater would just
        // fail with "binary not found". (`U`/update-all already gates on
        // update_available(), which requires a detected install.)
        if entry.installed.version.is_none() {
            return Err(format!(
                "{} is not installed — nothing to update",
                entry.agent.name
            ));
        }
        if self.update_states.get(&entry.id) == Some(&AgentUpdateState::Running) {
            return Err(format!("{} is already updating", entry.agent.name));
        }
        let needs_terminal = command_needs_terminal(&command);
        self.update_targets = vec![UpdateTarget {
            id: entry.id.clone(),
            name: entry.agent.name.clone(),
            command,
            method: entry.install_method().map(|m| m.label().to_string()),
            needs_terminal,
        }];
        self.show_update_confirm = true;
        Ok(())
    }

    /// Build confirm-modal targets for every agent with an available update and a
    /// verified updater (skipping any already updating). Targets that need an
    /// interactive terminal (sudo / AUR) are excluded — `U` runs in the background,
    /// which can't answer a prompt — so the user runs those individually with `u`
    /// then `i`. `Err` if none qualify (a specific message when the only candidates
    /// were interactive-only).
    pub fn request_update_all(&mut self) -> Result<usize, String> {
        let mut interactive_only = 0usize;
        let targets: Vec<UpdateTarget> = self
            .entries
            .iter()
            .filter(|e| e.tracked && e.update_available())
            .filter(|e| self.update_states.get(&e.id) != Some(&AgentUpdateState::Running))
            .filter_map(|e| {
                let command = e.resolved_update_command()?;
                if command_needs_terminal(&command) {
                    interactive_only += 1;
                    return None;
                }
                Some(UpdateTarget {
                    id: e.id.clone(),
                    name: e.agent.name.clone(),
                    command,
                    method: e.install_method().map(|m| m.label().to_string()),
                    needs_terminal: false,
                })
            })
            .collect();
        if targets.is_empty() {
            if interactive_only > 0 {
                return Err(format!(
                    "{interactive_only} agent(s) need an interactive update — use u then i"
                ));
            }
            return Err("No agents with an available update and a known updater".to_string());
        }
        let count = targets.len();
        self.update_targets = targets;
        self.show_update_confirm = true;
        Ok(count)
    }

    pub fn cancel_update(&mut self) {
        self.show_update_confirm = false;
        self.update_targets.clear();
    }

    /// Confirm the queued updates: mark each Running, reset its log, close the
    /// modal, and return the `(id, command)` pairs the main loop will spawn.
    pub fn confirm_update(&mut self) -> Vec<(String, Vec<String>)> {
        let mut spawned = Vec::new();
        for target in std::mem::take(&mut self.update_targets) {
            self.update_states
                .insert(target.id.clone(), AgentUpdateState::Running);
            self.update_logs.insert(target.id.clone(), Vec::new());
            spawned.push((target.id, target.command));
        }
        self.show_update_confirm = false;
        spawned
    }

    /// Confirm a single queued update to run **interactively** (suspend-and-run).
    /// Only valid for exactly one target (the `u` path); returns `(id, command)`
    /// for the main loop, or `None` otherwise. Marks the agent `Running` and
    /// resets its log so the detail panel reflects the in-flight update.
    pub fn confirm_update_interactive(&mut self) -> Option<(String, Vec<String>)> {
        if self.update_targets.len() != 1 {
            return None;
        }
        let target = self.update_targets.remove(0);
        self.update_states
            .insert(target.id.clone(), AgentUpdateState::Running);
        self.update_logs.insert(target.id.clone(), Vec::new());
        self.show_update_confirm = false;
        Some((target.id, target.command))
    }

    /// Append a captured output line for an in-progress update (bounded buffer).
    pub fn push_update_output(&mut self, id: &str, line: String) {
        let log = self.update_logs.entry(id.to_string()).or_default();
        log.push(line);
        if log.len() > UPDATE_LOG_CAP {
            let overflow = log.len() - UPDATE_LOG_CAP;
            log.drain(0..overflow);
        }
    }

    /// Record an update's terminal result and append a summary line to its log.
    pub fn finish_update(&mut self, id: &str, success: bool, message: String) {
        self.update_states.insert(
            id.to_string(),
            if success {
                AgentUpdateState::Succeeded
            } else {
                AgentUpdateState::Failed
            },
        );
        self.push_update_output(id, message);
    }

    /// True when the agent has a *finished* (non-Running) update result that can
    /// be dismissed.
    pub fn has_finished_update(&self, id: &str) -> bool {
        matches!(
            self.update_states.get(id),
            Some(AgentUpdateState::Succeeded | AgentUpdateState::Failed)
        )
    }

    /// Clear an agent's update state + captured log (the detail-panel Update
    /// section then disappears). Used to auto-expire finished results and for the
    /// `x` dismiss.
    pub fn clear_update(&mut self, id: &str) {
        self.update_states.remove(id);
        self.update_logs.remove(id);
    }

    /// Apply a freshly re-detected installed version after a successful update so
    /// the status dot flips (green/blue) without restarting the app.
    pub fn apply_redetected(&mut self, id: &str, installed: InstalledInfo) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.installed = installed;
        }
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
                update_command: vec![],
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

    #[test]
    fn binary_candidates_prioritizes_repo_segment() {
        assert_eq!(binary_candidates("Amp", "sourcegraph/amp"), vec!["amp"]);
        assert_eq!(
            binary_candidates("My Agent", "owner/my-agent"),
            vec!["my-agent", "myagent"]
        );
        // Repo segment first, then distinct name forms (kebab dups the segment
        // here, so only the space-joined form is added).
        assert_eq!(
            binary_candidates("Gemini CLI", "google-gemini/gemini-cli"),
            vec!["gemini-cli", "geminicli"]
        );
    }

    #[test]
    fn repo_slug_validation() {
        assert!(is_valid_repo_slug("owner/repo"));
        assert!(is_valid_repo_slug("My-Org/my.repo_2"));
        assert!(!is_valid_repo_slug("noslash"));
        assert!(!is_valid_repo_slug("owner/"));
        assert!(!is_valid_repo_slug("/repo"));
        assert!(!is_valid_repo_slug("a/b/c"));
        assert!(!is_valid_repo_slug("own er/repo"));
        assert!(!is_valid_repo_slug("https://github.com/owner/repo"));
        assert!(!is_valid_repo_slug(""));
    }

    #[test]
    fn add_agent_rejects_empty_name_without_saving() {
        // Error paths return before config.save(), so no filesystem touch.
        let mut app = test_app(vec![]);
        let mut config = Config::default();
        app.open_add_form();
        app.add_form.name = "  ".to_string();
        app.add_form.repo = "owner/repo".to_string();
        assert!(app.add_agent_save(&mut config).is_err());
        // Form stays open with an error so the user can correct it.
        assert!(app.show_add_form);
        assert_eq!(app.add_form.field, AddAgentField::Name);
        assert!(app.add_form.error.is_some());
        assert!(config.agents.custom.is_empty());
    }

    #[test]
    fn add_agent_rejects_bad_repo_slug() {
        let mut app = test_app(vec![]);
        let mut config = Config::default();
        app.open_add_form();
        app.add_form.name = "My Agent".to_string();
        app.add_form.repo = "not-a-slug".to_string();
        assert!(app.add_agent_save(&mut config).is_err());
        assert_eq!(app.add_form.field, AddAgentField::Repo);
        assert!(app.show_add_form);
        assert!(config.agents.custom.is_empty());
    }

    /// Build an entry with an installed version, a (newer) latest release, and an
    /// optional updater command — the shape the update actions key off.
    fn updatable_entry(
        id: &str,
        name: &str,
        installed: &str,
        latest: &str,
        cmd: &[&str],
    ) -> AgentEntry {
        let mut e = agent_entry(id, name, None);
        e.agent.update_command = cmd.iter().map(|s| s.to_string()).collect();
        e.installed.version = Some(installed.to_string());
        e.github.releases[0].version = latest.to_string();
        e
    }

    #[test]
    fn request_update_selected_targets_agent_with_updater() {
        let mut app = test_app(vec![updatable_entry(
            "claude-code",
            "Claude Code",
            "1.0.0",
            "2.0.0",
            &["claude", "update"],
        )]);
        assert!(app.request_update_selected().is_ok());
        assert!(app.show_update_confirm);
        assert_eq!(app.update_targets.len(), 1);
        assert_eq!(app.update_targets[0].command, vec!["claude", "update"]);
    }

    #[test]
    fn request_update_selected_errors_when_not_installed() {
        // Has an updater command but no detected install → nothing to update.
        let mut e = agent_entry("claude-code", "Claude Code", None);
        e.agent.update_command = vec!["claude".to_string(), "update".to_string()];
        // installed.version stays None (agent_entry leaves it default).
        let mut app = test_app(vec![e]);
        let err = app.request_update_selected().unwrap_err();
        assert!(err.contains("not installed"), "got: {err}");
        assert!(!app.show_update_confirm);
    }

    #[test]
    fn request_update_selected_errors_without_updater() {
        // No update_command → no in-app updater.
        let mut app = test_app(vec![updatable_entry("zed", "Zed", "1.0.0", "2.0.0", &[])]);
        assert!(app.request_update_selected().is_err());
        assert!(!app.show_update_confirm);
        assert!(app.update_targets.is_empty());
    }

    #[test]
    fn request_update_all_collects_only_updatable_with_updater() {
        let mut app = test_app(vec![
            updatable_entry("a", "A", "1.0.0", "2.0.0", &["a", "update"]), // updatable + cmd → yes
            updatable_entry("b", "B", "2.0.0", "2.0.0", &["b", "update"]), // up to date → no
            updatable_entry("c", "C", "1.0.0", "2.0.0", &[]),              // no updater → no
        ]);
        let count = app.request_update_all().expect("at least one updatable");
        assert_eq!(count, 1);
        assert_eq!(app.update_targets.len(), 1);
        assert_eq!(app.update_targets[0].id, "a");
    }

    #[test]
    fn request_update_all_errors_when_none_qualify() {
        let mut app = test_app(vec![updatable_entry(
            "b",
            "B",
            "2.0.0",
            "2.0.0",
            &["b", "update"],
        )]);
        assert!(app.request_update_all().is_err());
    }

    #[test]
    fn confirm_update_marks_running_and_returns_commands() {
        let mut app = test_app(vec![updatable_entry(
            "claude-code",
            "Claude Code",
            "1.0.0",
            "2.0.0",
            &["claude", "update"],
        )]);
        app.request_update_selected().unwrap();
        let spawned = app.confirm_update();
        assert_eq!(spawned.len(), 1);
        assert_eq!(spawned[0].0, "claude-code");
        assert_eq!(spawned[0].1, vec!["claude", "update"]);
        assert!(!app.show_update_confirm);
        assert_eq!(
            app.update_states.get("claude-code"),
            Some(&AgentUpdateState::Running)
        );
    }

    #[test]
    fn confirm_update_interactive_returns_single_target_and_marks_running() {
        let mut app = test_app(vec![updatable_entry(
            "claude-code",
            "Claude Code",
            "1.0.0",
            "2.0.0",
            &["claude", "update"],
        )]);
        app.request_update_selected().unwrap();
        let got = app.confirm_update_interactive();
        assert_eq!(
            got,
            Some((
                "claude-code".to_string(),
                vec!["claude".to_string(), "update".to_string()]
            ))
        );
        assert!(!app.show_update_confirm);
        assert_eq!(
            app.update_states.get("claude-code"),
            Some(&AgentUpdateState::Running)
        );
    }

    #[test]
    fn confirm_update_interactive_noops_for_multi_target() {
        let mut app = test_app(vec![
            updatable_entry("a", "A", "1.0.0", "2.0.0", &["a", "update"]),
            updatable_entry("b", "B", "1.0.0", "2.0.0", &["b", "update"]),
        ]);
        app.request_update_all().unwrap();
        assert!(app.confirm_update_interactive().is_none());
        // Modal stays open so the user can still confirm the background run.
        assert!(app.show_update_confirm);
    }

    #[test]
    fn request_update_selected_blocks_while_already_running() {
        let mut app = test_app(vec![updatable_entry(
            "a",
            "A",
            "1.0.0",
            "2.0.0",
            &["a", "update"],
        )]);
        app.update_states
            .insert("a".to_string(), AgentUpdateState::Running);
        assert!(app.request_update_selected().is_err());
    }

    #[test]
    fn update_output_is_capped() {
        let mut app = test_app(vec![]);
        for i in 0..(UPDATE_LOG_CAP + 50) {
            app.push_update_output("a", format!("line {i}"));
        }
        let log = app.update_logs.get("a").unwrap();
        assert_eq!(log.len(), UPDATE_LOG_CAP);
        assert_eq!(
            log.last().unwrap(),
            &format!("line {}", UPDATE_LOG_CAP + 49)
        );
    }

    #[test]
    fn finish_update_records_state_and_message() {
        let mut app = test_app(vec![]);
        app.finish_update("a", false, "boom".to_string());
        assert_eq!(app.update_states.get("a"), Some(&AgentUpdateState::Failed));
        assert_eq!(app.update_logs.get("a").unwrap().last().unwrap(), "boom");
    }

    #[test]
    fn clear_update_removes_finished_state_and_log() {
        let mut app = test_app(vec![]);
        app.update_states
            .insert("a".to_string(), AgentUpdateState::Running);
        assert!(!app.has_finished_update("a")); // Running is not "finished"
        app.finish_update("a", true, "done".to_string());
        assert!(app.has_finished_update("a"));
        app.clear_update("a");
        assert!(!app.has_finished_update("a"));
        assert!(app.update_states.get("a").is_none());
        assert!(app.update_logs.get("a").is_none());
    }

    #[test]
    fn apply_redetected_updates_installed_version() {
        let mut app = test_app(vec![updatable_entry(
            "a",
            "A",
            "1.0.0",
            "2.0.0",
            &["a", "update"],
        )]);
        app.apply_redetected(
            "a",
            InstalledInfo {
                version: Some("2.0.0".to_string()),
                path: None,
                ..Default::default()
            },
        );
        let entry = app.entries.iter().find(|e| e.id == "a").unwrap();
        assert_eq!(entry.installed.version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn add_agent_rejects_id_collision_with_existing_agent() {
        // "Claude Code" → id "claude-code"; collide with an existing entry.
        let mut app = test_app(vec![agent_entry("claude-code", "Claude Code", None)]);
        let mut config = Config::default();
        app.open_add_form();
        app.add_form.name = "Claude Code".to_string();
        app.add_form.repo = "someone/fork".to_string();
        assert!(app.add_agent_save(&mut config).is_err());
        assert!(app.add_form.error.is_some());
        assert!(config.agents.custom.is_empty());
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
