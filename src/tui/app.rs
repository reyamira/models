use super::agents::AgentsApp;
use super::benchmarks::BenchmarksApp;
use super::models::ModelsApp;
use super::status::StatusApp;

/// Page size for page up/down navigation
const PAGE_SIZE: usize = 10;

pub const MAX_SELECTIONS: usize = 8;
use crate::agents::{AgentsFile, FetchStatus, GitHubData};

use crate::benchmarks::multi::{MultiStore, SortKey};
use crate::benchmarks::schema::SourceFile;
use crate::config::Config;
use crate::data::{Provider, ProvidersMap};
use crate::tui::widgets::scroll_offset::ScrollOffset;

/// Apply the active source's trait/openness augmentation and rebuild the
/// benchmarks sub-app if the freshly loaded source is the active one.
fn finalize_loaded_source(app: &mut App, idx: usize) {
    // AA-only: fill open_weights / context_window from models.dev before the
    // sub-app derives the creator-openness map.
    if app.multi_store.sources.get(idx).map(|s| s.descriptor.id) == Some("aa") {
        if let Some(file) = app.multi_store.file_mut(idx) {
            crate::benchmarks::apply_model_traits(&app.providers, &mut file.models);
        }
    }
    if idx == app.benchmarks_app.active_source {
        if let Some(file) = app.multi_store.file(idx) {
            app.benchmarks_app.rebuild(file);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Models,
    Agents,
    Benchmarks,
    Status,
}

impl Tab {
    pub fn next(self) -> Self {
        match self {
            Tab::Models => Tab::Agents,
            Tab::Agents => Tab::Benchmarks,
            Tab::Benchmarks => Tab::Status,
            Tab::Status => Tab::Models,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Tab::Models => Tab::Status,
            Tab::Agents => Tab::Models,
            Tab::Benchmarks => Tab::Agents,
            Tab::Status => Tab::Benchmarks,
        }
    }
}

#[derive(Debug)]
pub enum Message {
    Quit,
    NextProvider,
    PrevProvider,
    NextModel,
    PrevModel,
    SelectFirstProvider,
    SelectLastProvider,
    SelectFirstModel,
    SelectLastModel,
    PageDownProvider,
    PageUpProvider,
    PageDownModel,
    PageUpModel,
    EnterSearch,
    ExitSearch,
    SearchInput(char),
    SearchBackspace,
    ClearSearch,
    CopyFull,          // Copy provider/model-id
    CopyModelId,       // Copy just model-id
    CopyProviderDoc,   // Copy provider documentation URL
    CopyProviderApi,   // Copy provider API URL
    OpenProviderDoc,   // Open provider documentation URL in browser
    CycleSort,         // Cycle through sort options
    ToggleSortDir,     // Toggle sort direction (ascending/descending)
    ToggleReasoning,   // Toggle reasoning filter
    ToggleTools,       // Toggle tools filter
    ToggleOpenWeights, // Toggle open weights filter
    ToggleFree,        // Toggle free models filter
    ToggleHelp,        // Toggle help popup
    ScrollHelpUp,      // Scroll help popup up
    ScrollHelpDown,    // Scroll help popup down
    NextTab,
    PrevTab,
    // Agents tab messages
    NextAgent,
    PrevAgent,
    SelectFirstAgent,
    SelectLastAgent,
    PageDownAgent,
    PageUpAgent,
    SwitchAgentFocus,
    ToggleInstalledFilter,
    ToggleCliFilter,
    ToggleOpenSourceFilter,
    OpenAgentRepo,
    OpenAgentDocs,
    CopyAgentName,
    // Picker modal messages
    OpenPicker,
    ClosePicker,
    PickerNext,
    PickerPrev,
    PickerToggle,
    PickerSave,
    // Detail panel scrolling
    ScrollDetailUp,
    ScrollDetailDown,
    ScrollDetailTop,
    ScrollDetailBottom,
    PageScrollDetailUp,
    PageScrollDetailDown,
    // Search match navigation
    NextSearchMatch,
    PrevSearchMatch,
    // Agent sort
    CycleAgentSort,
    // Models detail panel scrolling
    ScrollModelDetailUp,
    ScrollModelDetailDown,
    ScrollModelDetailTop,
    ScrollModelDetailBottom,
    PageScrollModelDetailUp,
    PageScrollModelDetailDown,
    // Models focus
    FocusModelLeft,
    FocusModelRight,
    // Provider categories
    CycleProviderCategory,
    ToggleGrouping,
    // Benchmarks tab messages
    NextBenchmark,
    PrevBenchmark,
    SelectFirstBenchmark,
    SelectLastBenchmark,
    PageDownBenchmark,
    PageUpBenchmark,
    NextBenchmarkCreator,
    PrevBenchmarkCreator,
    SelectFirstBenchmarkCreator,
    SelectLastBenchmarkCreator,
    PageDownBenchmarkCreator,
    PageUpBenchmarkCreator,
    FocusBenchmarkLeft,
    FocusBenchmarkRight,
    // Benchmarks detail panel scrolling
    ScrollBenchmarkDetailUp,
    ScrollBenchmarkDetailDown,
    ScrollBenchmarkDetailTop,
    ScrollBenchmarkDetailBottom,
    PageScrollBenchmarkDetailUp,
    PageScrollBenchmarkDetailDown,
    CycleBenchmarkSource,
    CycleReasoningFilter,
    ToggleRegionGrouping,
    ToggleTypeGrouping,
    ToggleBenchmarkSortDir,
    OpenSortPicker,
    SortPickerNext,
    SortPickerPrev,
    SortPickerConfirm,
    CloseSortPicker,
    QuickSortIntelligence,
    QuickSortDate,
    QuickSortSpeed,
    #[allow(dead_code)]
    CopyBenchmarkName,
    OpenBenchmarkUrl,
    ToggleBenchmarkSelection,
    ClearBenchmarkSelections,
    ToggleDetailOverlay,
    ToggleComparePanel,
    CloseDetailOverlay,
    CycleBenchmarkView,
    CycleScatterX,
    CycleScatterY,
    CycleRadarPreset,
    ScrollH2HDown,
    ScrollH2HUp,
    ScrollH2HTop,
    ScrollH2HPageDown,
    ScrollH2HPageUp,
    // Status tab messages
    OpenStatusPicker,
    NextStatusProvider,
    PrevStatusProvider,
    SelectFirstStatusProvider,
    SelectLastStatusProvider,
    PageDownStatusProvider,
    PageUpStatusProvider,
    SwitchStatusFocus,
    RefreshStatus,
    OpenStatusPage,
    PrevOverallStatusPanel,
    NextOverallStatusPanel,
    ScrollStatusDetailUp,
    ScrollStatusDetailDown,
    ScrollStatusDetailTop,
    ScrollStatusDetailBottom,
    PageScrollStatusDetailUp,
    PageScrollStatusDetailDown,
    // Data-source switcher (benchmarks tab): `{` / `}` cycle prev/next
    CycleDataSourcePrev,
    CycleDataSourceNext,
    // Async data messages
    GitHubDataReceived(String, GitHubData),
    GitHubFetchFailed(String, String), // (agent_id, error_message)
    // Benchmark data: one variant per source fetch. `None` => fetch failed.
    DataSourceLoaded(usize, Option<SourceFile>),
    // Provider status data messages
    StatusDataReceived(Vec<crate::status::ProviderStatus>),
}

pub struct App {
    pub providers: Vec<(String, Provider)>,
    pub mode: Mode,
    pub status_message: Option<String>,
    pub show_help: bool,
    pub help_scroll: ScrollOffset,
    pub current_tab: Tab,
    pub models_app: ModelsApp,
    pub agents_app: Option<AgentsApp>,
    pub config: Config,
    /// Agents newly tracked that need GitHub fetches (agent_id, repo)
    pub pending_fetches: Vec<(String, String)>,
    /// Multi-source benchmark store: one load-state per compiled-in source.
    pub multi_store: MultiStore,
    pub benchmarks_app: BenchmarksApp,
    pub status_app: Option<StatusApp>,
    /// Cached detail panel height for search match scrolling
    pub last_detail_height: u16,
    /// Store indices of selected models for comparison (shared between tabs)
    pub selections: Vec<usize>,
    pub pending_status_refresh: bool,
    pub force_status_refresh: bool,
}

impl App {
    pub fn new(
        providers_map: ProvidersMap,
        agents_file: Option<&AgentsFile>,
        config: Option<Config>,
    ) -> Self {
        let mut providers: Vec<(String, Provider)> = providers_map.into_iter().collect();
        providers.sort_by(|a, b| a.0.cmp(&b.0));

        let config = config.unwrap_or_default();
        let agents_app = agents_file.map(|af| AgentsApp::new(af, &config));
        let status_app = Some(StatusApp::new(&config));
        // Sources load progressively from the CDN; nothing is loaded at startup,
        // so the benchmarks sub-app starts empty (loading state).
        let multi_store = MultiStore::new();
        let benchmarks_app = BenchmarksApp::new(None);
        let models_app = ModelsApp::new(&providers);

        Self {
            providers,
            mode: Mode::Normal,
            status_message: None,
            show_help: false,
            help_scroll: ScrollOffset::default(),
            current_tab: Tab::default(),
            models_app,
            agents_app,
            config,
            pending_fetches: Vec::new(),
            multi_store,
            benchmarks_app,
            status_app,
            last_detail_height: 0,
            selections: Vec::new(),
            pending_status_refresh: false,
            force_status_refresh: false,
        }
    }

    /// Borrow the active source's loaded `SourceFile`, if any.
    pub fn active_benchmark_file(&self) -> Option<&SourceFile> {
        self.multi_store.file(self.benchmarks_app.active_source)
    }

    pub fn toggle_selection(&mut self, store_index: usize) {
        if let Some(pos) = self.selections.iter().position(|&i| i == store_index) {
            self.selections.remove(pos);
        } else if self.selections.len() < MAX_SELECTIONS {
            self.selections.push(store_index);
        }
    }

    pub fn clear_selections(&mut self) {
        self.selections.clear();
    }

    pub fn get_copy_full(&self) -> Option<String> {
        self.models_app.get_copy_full()
    }

    pub fn get_copy_model_id(&self) -> Option<String> {
        self.models_app.get_copy_model_id()
    }

    pub fn get_provider_doc(&self) -> Option<String> {
        self.models_app.get_provider_doc(&self.providers)
    }

    pub fn get_provider_api(&self) -> Option<String> {
        self.models_app.get_provider_api(&self.providers)
    }

    pub fn update(&mut self, msg: Message) -> bool {
        match msg {
            Message::Quit => return false,
            Message::NextProvider => {
                self.models_app.next_provider(&self.providers);
            }
            Message::PrevProvider => {
                self.models_app.prev_provider(&self.providers);
            }
            Message::NextModel => {
                self.models_app.next_model();
            }
            Message::PrevModel => {
                self.models_app.prev_model();
            }
            Message::SelectFirstProvider => {
                self.models_app.select_first_provider(&self.providers);
            }
            Message::SelectLastProvider => {
                self.models_app.select_last_provider(&self.providers);
            }
            Message::SelectFirstModel => {
                self.models_app.select_first_model();
            }
            Message::SelectLastModel => {
                self.models_app.select_last_model();
            }
            Message::PageDownProvider => {
                self.models_app.page_down_provider(&self.providers);
            }
            Message::PageUpProvider => {
                self.models_app.page_up_provider(&self.providers);
            }
            Message::PageDownModel => {
                self.models_app.page_down_model();
            }
            Message::PageUpModel => {
                self.models_app.page_up_model();
            }
            Message::FocusModelLeft => {
                self.models_app.focus_left();
            }
            Message::FocusModelRight => {
                self.models_app.focus_right();
            }
            Message::ScrollModelDetailUp => {
                self.models_app.detail_scroll.decrement(1);
            }
            Message::ScrollModelDetailDown => {
                self.models_app.detail_scroll.increment(1);
            }
            Message::ScrollModelDetailTop => {
                self.models_app.detail_scroll.jump_top();
            }
            Message::ScrollModelDetailBottom => {
                self.models_app.detail_scroll.jump_bottom();
            }
            Message::PageScrollModelDetailUp => {
                self.models_app.detail_scroll.decrement(PAGE_SIZE as u16);
            }
            Message::PageScrollModelDetailDown => {
                self.models_app.detail_scroll.increment(PAGE_SIZE as u16);
            }
            Message::EnterSearch => {
                self.mode = Mode::Search;
            }
            Message::ExitSearch => {
                self.mode = Mode::Normal;
            }
            Message::SearchInput(c) => match self.current_tab {
                Tab::Models => {
                    self.models_app.search_input(c, &self.providers);
                }
                Tab::Agents => {
                    if let Some(ref mut agents_app) = self.agents_app {
                        agents_app.search_query.push(c);
                        agents_app.selected_agent = 0;
                        agents_app.update_filtered();
                    }
                }
                Tab::Benchmarks => {
                    self.benchmarks_app.search_query.push(c);
                    if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                        self.benchmarks_app.rebuild_after_filter_change(file);
                    }
                }
                Tab::Status => {
                    if let Some(ref mut status_app) = self.status_app {
                        status_app.search_query.push(c);
                        status_app.selected = 0;
                        status_app.update_filtered();
                    }
                }
            },
            Message::SearchBackspace => match self.current_tab {
                Tab::Models => {
                    self.models_app.search_backspace(&self.providers);
                }
                Tab::Agents => {
                    if let Some(ref mut agents_app) = self.agents_app {
                        agents_app.search_query.pop();
                        agents_app.selected_agent = 0;
                        agents_app.update_filtered();
                    }
                }
                Tab::Benchmarks => {
                    self.benchmarks_app.search_query.pop();
                    if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                        self.benchmarks_app.rebuild_after_filter_change(file);
                    }
                }
                Tab::Status => {
                    if let Some(ref mut status_app) = self.status_app {
                        status_app.search_query.pop();
                        status_app.selected = 0;
                        status_app.update_filtered();
                    }
                }
            },
            Message::ClearSearch => match self.current_tab {
                Tab::Models => {
                    self.models_app.clear_search(&self.providers);
                }
                Tab::Agents => {
                    if let Some(ref mut agents_app) = self.agents_app {
                        agents_app.search_query.clear();
                        agents_app.selected_agent = 0;
                        agents_app.update_filtered();
                    }
                }
                Tab::Benchmarks => {
                    self.benchmarks_app.search_query.clear();
                    if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                        self.benchmarks_app.rebuild_after_filter_change(file);
                    }
                }
                Tab::Status => {
                    if let Some(ref mut status_app) = self.status_app {
                        status_app.search_query.clear();
                        status_app.selected = 0;
                        status_app.update_filtered();
                    }
                }
            },
            // Copy and open messages are handled in the main loop
            Message::CopyFull
            | Message::CopyModelId
            | Message::CopyProviderDoc
            | Message::CopyProviderApi
            | Message::OpenProviderDoc => {}
            Message::CycleSort => {
                self.models_app.cycle_sort(&self.providers);
            }
            Message::ToggleSortDir => {
                self.models_app.toggle_sort_dir(&self.providers);
            }
            Message::ToggleReasoning => {
                self.models_app.toggle_reasoning(&self.providers);
            }
            Message::ToggleTools => {
                self.models_app.toggle_tools(&self.providers);
            }
            Message::ToggleOpenWeights => {
                self.models_app.toggle_open_weights(&self.providers);
            }
            Message::ToggleFree => {
                self.models_app.toggle_free(&self.providers);
            }
            Message::ToggleHelp => {
                self.show_help = !self.show_help;
                if self.show_help {
                    self.help_scroll.jump_top(); // Reset scroll when opening
                }
            }
            Message::ScrollHelpUp => {
                self.help_scroll.decrement(1);
            }
            Message::ScrollHelpDown => {
                // Render-time clamping in draw_help_popup() prevents scrolling
                // past content, so we just increment here.
                self.help_scroll.increment(1);
            }
            Message::NextTab => {
                self.current_tab = self.current_tab.next();
            }
            Message::PrevTab => {
                self.current_tab = self.current_tab.prev();
            }
            Message::NextAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.next_agent();
                }
            }
            Message::PrevAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.prev_agent();
                }
            }
            Message::SelectFirstAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.select_first_agent();
                }
            }
            Message::SelectLastAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.select_last_agent();
                }
            }
            Message::PageDownAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.page_down(PAGE_SIZE);
                }
            }
            Message::PageUpAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.page_up(PAGE_SIZE);
                }
            }
            Message::NextStatusProvider => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.next();
                }
            }
            Message::PrevStatusProvider => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.prev();
                }
            }
            Message::SelectFirstStatusProvider => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.select_first();
                }
            }
            Message::SelectLastStatusProvider => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.select_last();
                }
            }
            Message::PageDownStatusProvider => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.page_down();
                }
            }
            Message::PageUpStatusProvider => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.page_up();
                }
            }
            Message::SwitchStatusFocus => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.switch_focus();
                }
            }
            Message::RefreshStatus => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.loading = true;
                    status_app.last_error = None;
                    self.pending_status_refresh = true;
                    self.force_status_refresh = true;
                }
            }
            Message::OpenStatusPage => {
                // Handled in main loop
            }
            Message::PrevOverallStatusPanel => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.select_prev_overall_panel();
                    } else {
                        status_app.select_prev_detail_panel();
                    }
                }
            }
            Message::NextOverallStatusPanel => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.select_next_overall_panel();
                    } else {
                        status_app.select_next_detail_panel();
                    }
                }
            }
            Message::ScrollStatusDetailUp => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.scroll_active_overall_panel_up();
                    } else {
                        status_app.scroll_active_detail_panel_up();
                    }
                }
            }
            Message::ScrollStatusDetailDown => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.scroll_active_overall_panel_down();
                    } else {
                        status_app.scroll_active_detail_panel_down();
                    }
                }
            }
            Message::ScrollStatusDetailTop => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.scroll_active_overall_panel_top();
                    } else {
                        status_app.scroll_active_detail_panel_top();
                    }
                }
            }
            Message::ScrollStatusDetailBottom => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.scroll_active_overall_panel_bottom();
                    } else {
                        status_app.scroll_active_detail_panel_bottom();
                    }
                }
            }
            Message::PageScrollStatusDetailUp => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.page_scroll_active_overall_panel_up();
                    } else {
                        status_app.page_scroll_active_detail_panel_up();
                    }
                }
            }
            Message::PageScrollStatusDetailDown => {
                if let Some(ref mut status_app) = self.status_app {
                    if status_app.is_overall_selected() {
                        status_app.page_scroll_active_overall_panel_down();
                    } else {
                        status_app.page_scroll_active_detail_panel_down();
                    }
                }
            }
            Message::SwitchAgentFocus => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.switch_focus();
                }
            }
            Message::ToggleInstalledFilter => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.toggle_installed_filter();
                }
            }
            Message::ToggleCliFilter => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.toggle_cli_filter();
                }
            }
            Message::ToggleOpenSourceFilter => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.toggle_open_source_filter();
                }
            }
            Message::OpenAgentRepo | Message::OpenAgentDocs | Message::CopyAgentName => {
                // Handled in main loop
            }
            Message::OpenPicker => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.open_picker();
                }
            }
            Message::OpenStatusPicker => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.open_picker();
                }
            }
            Message::ClosePicker => {
                if self.current_tab == Tab::Status {
                    if let Some(ref mut status_app) = self.status_app {
                        status_app.close_picker();
                    }
                } else if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.close_picker();
                }
            }
            Message::PickerNext => {
                if self.current_tab == Tab::Status {
                    if let Some(ref mut status_app) = self.status_app {
                        status_app.picker_next();
                    }
                } else if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.picker_next();
                }
            }
            Message::PickerPrev => {
                if self.current_tab == Tab::Status {
                    if let Some(ref mut status_app) = self.status_app {
                        status_app.picker_prev();
                    }
                } else if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.picker_prev();
                }
            }
            Message::PickerToggle => {
                if self.current_tab == Tab::Status {
                    if let Some(ref mut status_app) = self.status_app {
                        status_app.picker_toggle_current();
                    }
                } else if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.picker_toggle_current();
                }
            }
            Message::PickerSave => {
                if self.current_tab == Tab::Status {
                    if let Some(ref mut status_app) = self.status_app {
                        match status_app.picker_save(&mut self.config) {
                            Ok(newly_tracked) => {
                                if newly_tracked.is_empty() {
                                    self.set_status("Tracked providers saved".to_string());
                                } else {
                                    self.set_status(format!(
                                        "Tracked providers saved, fetching {} new...",
                                        newly_tracked.len()
                                    ));
                                    self.pending_status_refresh = true;
                                    self.force_status_refresh = true;
                                }
                            }
                            Err(e) => {
                                self.set_status(e);
                            }
                        }
                    }
                } else if let Some(ref mut agents_app) = self.agents_app {
                    match agents_app.picker_save(&mut self.config) {
                        Ok(newly_tracked) => {
                            if newly_tracked.is_empty() {
                                self.set_status("Tracked agents saved".to_string());
                            } else {
                                let new_fetch_count = newly_tracked.len();
                                agents_app.pending_github_fetches = agents_app
                                    .pending_github_fetches
                                    .saturating_add(new_fetch_count);
                                agents_app.loading_github = true;
                                self.set_status(format!(
                                    "Tracked agents saved, fetching {} new...",
                                    new_fetch_count
                                ));
                                self.pending_fetches = newly_tracked;
                            }
                        }
                        Err(e) => {
                            self.set_status(e);
                        }
                    }
                }
            }
            Message::ScrollDetailUp => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.detail_scroll = agents_app.detail_scroll.saturating_sub(1);
                }
            }
            Message::ScrollDetailDown => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.detail_scroll = agents_app.detail_scroll.saturating_add(1);
                }
            }
            Message::ScrollDetailTop => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.detail_scroll = 0;
                }
            }
            Message::ScrollDetailBottom => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.detail_scroll = u16::MAX; // clamped at render time
                }
            }
            Message::PageScrollDetailUp => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.detail_scroll =
                        agents_app.detail_scroll.saturating_sub(PAGE_SIZE as u16);
                }
            }
            Message::PageScrollDetailDown => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.detail_scroll =
                        agents_app.detail_scroll.saturating_add(PAGE_SIZE as u16);
                }
            }
            Message::NextSearchMatch => {
                if let Some(ref mut agents_app) = self.agents_app {
                    if let Some(scroll) = agents_app.next_search_match(self.last_detail_height) {
                        agents_app.detail_scroll = scroll;
                    }
                }
            }
            Message::PrevSearchMatch => {
                if let Some(ref mut agents_app) = self.agents_app {
                    if let Some(scroll) = agents_app.prev_search_match(self.last_detail_height) {
                        agents_app.detail_scroll = scroll;
                    }
                }
            }
            Message::CycleAgentSort => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.cycle_sort();
                }
            }
            Message::CycleProviderCategory => {
                self.models_app.cycle_provider_category(&self.providers);
            }
            Message::ToggleGrouping => {
                self.models_app.toggle_grouping(&self.providers);
            }
            // Benchmarks tab messages
            Message::NextBenchmark => {
                self.benchmarks_app.next();
            }
            Message::PrevBenchmark => {
                self.benchmarks_app.prev();
            }
            Message::SelectFirstBenchmark => {
                self.benchmarks_app.select_first();
            }
            Message::SelectLastBenchmark => {
                self.benchmarks_app.select_last();
            }
            Message::PageDownBenchmark => {
                self.benchmarks_app.page_down();
            }
            Message::PageUpBenchmark => {
                self.benchmarks_app.page_up();
            }
            Message::NextBenchmarkCreator => {
                self.benchmarks_app.next_creator();
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.update_filtered(file);
                }
            }
            Message::PrevBenchmarkCreator => {
                self.benchmarks_app.prev_creator();
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.update_filtered(file);
                }
            }
            Message::SelectFirstBenchmarkCreator => {
                self.benchmarks_app.select_first_creator();
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.update_filtered(file);
                }
            }
            Message::SelectLastBenchmarkCreator => {
                self.benchmarks_app.select_last_creator();
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.update_filtered(file);
                }
            }
            Message::PageDownBenchmarkCreator => {
                self.benchmarks_app.page_down_creator();
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.update_filtered(file);
                }
            }
            Message::PageUpBenchmarkCreator => {
                self.benchmarks_app.page_up_creator();
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.update_filtered(file);
                }
            }
            Message::FocusBenchmarkRight => {
                let has_compare = self.selections.len() >= 2;
                self.benchmarks_app.focus_right(has_compare);
            }
            Message::FocusBenchmarkLeft => {
                let has_compare = self.selections.len() >= 2;
                self.benchmarks_app.focus_left(has_compare);
            }
            Message::ScrollBenchmarkDetailUp => {
                self.benchmarks_app.detail_scroll.decrement(1);
            }
            Message::ScrollBenchmarkDetailDown => {
                self.benchmarks_app.detail_scroll.increment(1);
            }
            Message::ScrollBenchmarkDetailTop => {
                self.benchmarks_app.detail_scroll.jump_top();
            }
            Message::ScrollBenchmarkDetailBottom => {
                self.benchmarks_app.detail_scroll.jump_bottom();
            }
            Message::PageScrollBenchmarkDetailUp => {
                self.benchmarks_app
                    .detail_scroll
                    .decrement(PAGE_SIZE as u16);
            }
            Message::PageScrollBenchmarkDetailDown => {
                self.benchmarks_app
                    .detail_scroll
                    .increment(PAGE_SIZE as u16);
            }
            Message::CycleBenchmarkSource => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.cycle_source_filter(file);
                }
            }
            Message::CycleReasoningFilter => {
                // No-op when the active source has no model carrying a reasoning
                // status (the `7` key is hidden in that case anyway).
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    if BenchmarksApp::reasoning_filter_available(Some(file)) {
                        self.benchmarks_app.cycle_reasoning_filter(file);
                    }
                }
            }
            Message::ToggleRegionGrouping => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.toggle_region_grouping(file);
                }
            }
            Message::ToggleTypeGrouping => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.toggle_type_grouping(file);
                }
            }
            Message::ToggleBenchmarkSortDir => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.toggle_sort_direction(file);
                }
            }
            Message::OpenSortPicker => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    let current = self.benchmarks_app.sort_key;
                    let options = BenchmarksApp::sort_options(file);
                    self.benchmarks_app.sort_picker_selected =
                        options.iter().position(|o| o.key == current).unwrap_or(0);
                    self.benchmarks_app.show_sort_picker = true;
                }
            }
            Message::SortPickerNext => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    let len = BenchmarksApp::sort_options(file).len().max(1);
                    self.benchmarks_app.sort_picker_selected =
                        (self.benchmarks_app.sort_picker_selected + 1).min(len - 1);
                }
            }
            Message::SortPickerPrev => {
                self.benchmarks_app.sort_picker_selected =
                    self.benchmarks_app.sort_picker_selected.saturating_sub(1);
            }
            Message::SortPickerConfirm => {
                self.benchmarks_app.show_sort_picker = false;
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    let options = BenchmarksApp::sort_options(file);
                    if let Some(opt) = options.get(self.benchmarks_app.sort_picker_selected) {
                        let key = opt.key;
                        self.benchmarks_app.quick_sort(key, file);
                    }
                }
            }
            Message::CloseSortPicker => {
                self.benchmarks_app.show_sort_picker = false;
            }
            Message::QuickSortIntelligence => {
                // `1` = first metric of the source.
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    if let Some(key) = BenchmarksApp::quick_sort_metric_first(file) {
                        self.benchmarks_app.quick_sort(key, file);
                    }
                }
            }
            Message::QuickSortDate => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.quick_sort(SortKey::ReleaseDate, file);
                }
            }
            Message::QuickSortSpeed => {
                // `3` = first TokensPerSec metric; no-op when the source has none.
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    if let Some(key) = BenchmarksApp::quick_sort_speed(file) {
                        self.benchmarks_app.quick_sort(key, file);
                    }
                }
            }
            Message::CycleDataSourceNext => {
                self.benchmarks_app.switch_source(&self.multi_store, true);
                self.clear_selections();
            }
            Message::CycleDataSourcePrev => {
                self.benchmarks_app.switch_source(&self.multi_store, false);
                self.clear_selections();
            }
            Message::ToggleBenchmarkSelection => {
                if let Some(&store_idx) = self
                    .benchmarks_app
                    .filtered_indices
                    .get(self.benchmarks_app.selected)
                {
                    self.toggle_selection(store_idx);
                    self.benchmarks_app
                        .update_bottom_view(self.selections.len());
                    // Focus reset AFTER update_bottom_view
                    if self.selections.len() < 2
                        && self.benchmarks_app.focus == super::benchmarks::BenchmarkFocus::Compare
                    {
                        self.benchmarks_app.focus = super::benchmarks::BenchmarkFocus::List;
                    }
                }
            }
            Message::ClearBenchmarkSelections => {
                self.clear_selections();
                self.benchmarks_app.update_bottom_view(0);
                // Focus reset AFTER update_bottom_view
                if self.benchmarks_app.focus == super::benchmarks::BenchmarkFocus::Compare {
                    self.benchmarks_app.focus = super::benchmarks::BenchmarkFocus::List;
                }
            }
            Message::ScrollH2HDown => {
                self.benchmarks_app.scroll_h2h_down();
            }
            Message::ScrollH2HUp => {
                self.benchmarks_app.scroll_h2h_up();
            }
            Message::ScrollH2HTop => {
                self.benchmarks_app.scroll_h2h_top();
            }
            Message::ScrollH2HPageDown => {
                self.benchmarks_app.scroll_h2h_page_down(10);
            }
            Message::ScrollH2HPageUp => {
                self.benchmarks_app.scroll_h2h_page_up(10);
            }
            Message::ToggleDetailOverlay => {
                if self.selections.len() >= 2 {
                    self.benchmarks_app.show_detail_overlay =
                        !self.benchmarks_app.show_detail_overlay;
                }
            }
            Message::CloseDetailOverlay => {
                self.benchmarks_app.show_detail_overlay = false;
            }
            Message::ToggleComparePanel => {
                self.benchmarks_app.show_creators_in_compare =
                    !self.benchmarks_app.show_creators_in_compare;
                // Update focus to match the new left panel
                if self.benchmarks_app.focus != super::benchmarks::BenchmarkFocus::Compare {
                    self.benchmarks_app.focus = if self.benchmarks_app.show_creators_in_compare {
                        super::benchmarks::BenchmarkFocus::Creators
                    } else {
                        super::benchmarks::BenchmarkFocus::List
                    };
                }
            }
            Message::CycleBenchmarkView => {
                if self.selections.len() >= 2 {
                    self.benchmarks_app.cycle_bottom_view();
                }
            }
            Message::CycleScatterX => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.cycle_scatter_x(file);
                }
            }
            Message::CycleScatterY => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.cycle_scatter_y(file);
                }
            }
            Message::CycleRadarPreset => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.cycle_radar_group(file);
                }
            }
            Message::CopyBenchmarkName | Message::OpenBenchmarkUrl => {
                // Handled in main loop
            }
            Message::GitHubDataReceived(agent_id, data) => {
                if let Some(ref mut agents_app) = self.agents_app {
                    if let Some(entry) = agents_app.entries.iter_mut().find(|e| e.id == agent_id) {
                        entry.github = data;
                        entry.fetch_status = FetchStatus::Loaded;
                    }
                    agents_app.apply_sort(); // Re-sort after data arrives

                    // Decrement pending fetches and clear loading flag when all complete
                    agents_app.pending_github_fetches =
                        agents_app.pending_github_fetches.saturating_sub(1);
                    if agents_app.pending_github_fetches == 0 {
                        agents_app.loading_github = false;
                    }
                }
            }
            Message::GitHubFetchFailed(agent_id, error) => {
                if let Some(ref mut agents_app) = self.agents_app {
                    if let Some(entry) = agents_app.entries.iter_mut().find(|e| e.id == agent_id) {
                        entry.fetch_status = FetchStatus::Failed(error);
                    }

                    // Decrement pending fetches and clear loading flag when all complete
                    agents_app.pending_github_fetches =
                        agents_app.pending_github_fetches.saturating_sub(1);
                    if agents_app.pending_github_fetches == 0 {
                        agents_app.loading_github = false;
                    }
                }
            }
            Message::DataSourceLoaded(idx, result) => {
                match result {
                    Some(file) => {
                        self.multi_store.set_loaded(idx, file);
                        // Apply traits (AA only) before the sub-app derives state,
                        // then rebuild the sub-app when this is the active source.
                        finalize_loaded_source(self, idx);
                    }
                    None => {
                        self.multi_store.set_failed(idx);
                        if idx == self.benchmarks_app.active_source {
                            self.benchmarks_app.loading = false;
                        }
                        let name = self
                            .multi_store
                            .sources
                            .get(idx)
                            .map(|s| s.descriptor.name)
                            .unwrap_or("source");
                        self.set_status(format!("Failed to fetch {name} benchmark data"));
                    }
                }
            }
            Message::StatusDataReceived(entries) => {
                if let Some(ref mut status_app) = self.status_app {
                    status_app.apply_fetch(entries);
                }
            }
        }
        true
    }

    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some(msg);
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{Agent, AgentsFile};
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_agent(name: &str, repo: &str) -> Agent {
        Agent {
            name: name.to_string(),
            repo: repo.to_string(),
            categories: vec!["cli".to_string()],
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
        }
    }

    fn test_agents_file() -> AgentsFile {
        let mut agents = HashMap::new();
        agents.insert("alpha".to_string(), test_agent("Alpha", "owner/alpha"));
        agents.insert("beta".to_string(), test_agent("Beta", "owner/beta"));
        AgentsFile {
            schema_version: 1,
            last_scraped: None,
            scrape_source: None,
            agents,
        }
    }

    fn temp_config_home() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("modelsdev-tui-app-test-{nanos}"))
    }

    struct ConfigHomeGuard {
        path: PathBuf,
        previous_xdg: Option<OsString>,
        previous_home: Option<OsString>,
    }

    impl ConfigHomeGuard {
        fn install(path: PathBuf) -> Self {
            let previous_xdg = std::env::var_os("XDG_CONFIG_HOME");
            let previous_home = std::env::var_os("HOME");
            // SAFETY: only one test uses this guard and it runs single-threaded
            // in practice. XDG_CONFIG_HOME covers Linux; HOME covers macOS,
            // where dirs::config_dir() resolves through ~/Library/Application Support.
            unsafe { std::env::set_var("XDG_CONFIG_HOME", &path) };
            unsafe { std::env::set_var("HOME", &path) };
            Self {
                path,
                previous_xdg,
                previous_home,
            }
        }
    }

    impl Drop for ConfigHomeGuard {
        fn drop(&mut self) {
            if let Some(val) = &self.previous_xdg {
                unsafe { std::env::set_var("XDG_CONFIG_HOME", val) };
            } else {
                unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
            }
            if let Some(val) = &self.previous_home {
                unsafe { std::env::set_var("HOME", val) };
            } else {
                unsafe { std::env::remove_var("HOME") };
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn picker_save_updates_agents_fetch_counters_for_newly_tracked_agents() {
        let config_home = temp_config_home();
        let _config_home_guard = ConfigHomeGuard::install(config_home);

        let mut config = Config::default();
        config.agents.tracked = HashSet::new();
        config.agents.excluded = HashSet::new();
        config.agents.custom.clear();

        let agents_file = test_agents_file();
        let mut app = App::new(HashMap::new(), Some(&agents_file), Some(config));

        {
            let agents_app = app.agents_app.as_mut().expect("agents app should exist");
            agents_app.loading_github = false;
            agents_app.pending_github_fetches = 0;
            agents_app.open_picker();
            agents_app.picker_changes.insert("alpha".to_string(), true);
            agents_app.picker_changes.insert("beta".to_string(), true);
        }

        app.update(Message::PickerSave);

        let agents_app = app.agents_app.as_ref().expect("agents app should exist");
        assert_eq!(app.pending_fetches.len(), 2);
        assert_eq!(agents_app.pending_github_fetches, 2);
        assert!(agents_app.loading_github);

        app.update(Message::GitHubDataReceived(
            "alpha".to_string(),
            GitHubData::default(),
        ));
        let agents_app = app.agents_app.as_ref().expect("agents app should exist");
        assert_eq!(agents_app.pending_github_fetches, 1);
        assert!(agents_app.loading_github);

        app.update(Message::GitHubDataReceived(
            "beta".to_string(),
            GitHubData::default(),
        ));
        let agents_app = app.agents_app.as_ref().expect("agents app should exist");
        assert_eq!(agents_app.pending_github_fetches, 0);
        assert!(!agents_app.loading_github);
    }

    fn make_test_app() -> App {
        let providers = std::collections::HashMap::new();
        App::new(providers, None, None)
    }

    #[test]
    fn test_toggle_selection_add() {
        let mut app = make_test_app();
        app.toggle_selection(5);
        assert_eq!(app.selections, vec![5]);
    }

    #[test]
    fn test_toggle_selection_remove() {
        let mut app = make_test_app();
        app.toggle_selection(5);
        app.toggle_selection(10);
        app.toggle_selection(5);
        assert_eq!(app.selections, vec![10]);
    }

    #[test]
    fn test_toggle_selection_max_capacity() {
        let mut app = make_test_app();
        for i in 0..MAX_SELECTIONS {
            app.toggle_selection(i);
        }
        assert_eq!(app.selections.len(), MAX_SELECTIONS);
        // Adding one more should be a no-op
        app.toggle_selection(100);
        assert_eq!(app.selections.len(), MAX_SELECTIONS);
        assert!(!app.selections.contains(&100));
    }

    #[test]
    fn test_clear_selections() {
        let mut app = make_test_app();
        app.toggle_selection(1);
        app.toggle_selection(2);
        app.toggle_selection(3);
        app.clear_selections();
        assert!(app.selections.is_empty());
    }

    #[test]
    fn test_update_bottom_view_transitions_to_h2h() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Detail);
        app.benchmarks_app.update_bottom_view(2);
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::H2H);
    }

    #[test]
    fn test_update_bottom_view_reverts_to_detail() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        app.benchmarks_app.update_bottom_view(2);
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::H2H);
        app.benchmarks_app.update_bottom_view(1);
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Detail);
    }

    #[test]
    fn test_update_bottom_view_closes_overlay_on_revert() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        app.benchmarks_app.update_bottom_view(2);
        app.benchmarks_app.show_detail_overlay = true;
        app.benchmarks_app.update_bottom_view(1);
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Detail);
        assert!(!app.benchmarks_app.show_detail_overlay);
    }

    #[test]
    fn test_cycle_bottom_view_order() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        // Start at H2H
        app.benchmarks_app.bottom_view = BottomView::H2H;
        app.benchmarks_app.cycle_bottom_view();
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Scatter);
        app.benchmarks_app.cycle_bottom_view();
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Radar);
        app.benchmarks_app.cycle_bottom_view();
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::H2H);
    }

    #[test]
    fn test_cycle_bottom_view_from_detail() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        app.benchmarks_app.bottom_view = BottomView::Detail;
        app.benchmarks_app.cycle_bottom_view();
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::H2H);
    }

    #[test]
    fn test_cycle_data_source_clears_selections() {
        // With a single compiled-in source, cycling wraps back to index 0 and
        // clears the shared selection vec (per the switch contract).
        let mut app = make_test_app();
        app.toggle_selection(1);
        app.toggle_selection(2);
        assert_eq!(app.selections.len(), 2);
        app.update(Message::CycleDataSourceNext);
        assert!(app.selections.is_empty());
        assert_eq!(app.benchmarks_app.active_source, 0);
    }

    #[test]
    fn test_data_source_loaded_failed_sets_status() {
        let mut app = make_test_app();
        app.update(Message::DataSourceLoaded(0, None));
        assert!(super::super::benchmarks::BenchmarksApp::active_is_failed(
            &app.multi_store,
            0
        ));
        assert!(app.status_message.is_some());
    }

    #[test]
    fn test_update_bottom_view_reverts_scatter_to_detail() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        app.benchmarks_app.bottom_view = BottomView::Scatter;
        app.benchmarks_app.update_bottom_view(1);
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Detail);
    }

    #[test]
    fn test_update_bottom_view_reverts_radar_to_detail() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        app.benchmarks_app.bottom_view = BottomView::Radar;
        app.benchmarks_app.update_bottom_view(1);
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Detail);
    }

    #[test]
    fn test_focus_right_browse_mode() {
        use super::super::benchmarks::BenchmarkFocus;
        let mut app = make_test_app();
        app.benchmarks_app.focus = BenchmarkFocus::Creators;
        app.benchmarks_app.focus_right(false);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::List);
        app.benchmarks_app.focus_right(false);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::Details);
        app.benchmarks_app.focus_right(false);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::Creators);
    }

    #[test]
    fn test_focus_left_browse_mode() {
        use super::super::benchmarks::BenchmarkFocus;
        let mut app = make_test_app();
        app.benchmarks_app.focus = BenchmarkFocus::Creators;
        app.benchmarks_app.focus_left(false);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::Details);
        app.benchmarks_app.focus_left(false);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::List);
        app.benchmarks_app.focus_left(false);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::Creators);
    }

    #[test]
    fn test_focus_right_compare_mode() {
        use super::super::benchmarks::BenchmarkFocus;
        let mut app = make_test_app();
        app.benchmarks_app.focus = BenchmarkFocus::List;
        app.benchmarks_app.focus_right(true);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::Compare);
        app.benchmarks_app.focus_right(true);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::List);
    }

    #[test]
    fn test_focus_resets_when_selections_drop_below_2() {
        use super::super::benchmarks::BenchmarkFocus;
        let mut app = make_test_app();
        app.benchmarks_app.focus = BenchmarkFocus::Compare;
        // Simulate clearing selections
        app.benchmarks_app.update_bottom_view(0);
        if app.benchmarks_app.focus == BenchmarkFocus::Compare {
            app.benchmarks_app.focus = BenchmarkFocus::List;
        }
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::List);
    }

    #[test]
    fn test_h2h_scroll_methods() {
        let mut app = make_test_app();
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 0);

        app.benchmarks_app.scroll_h2h_down();
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 1);

        app.benchmarks_app.scroll_h2h_down();
        app.benchmarks_app.scroll_h2h_down();
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 3);

        app.benchmarks_app.scroll_h2h_up();
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 2);

        app.benchmarks_app.scroll_h2h_top();
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 0);

        app.benchmarks_app.scroll_h2h_page_down(10);
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 10);

        app.benchmarks_app.scroll_h2h_page_up(5);
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 5);

        // Saturating sub: can't go below 0
        app.benchmarks_app.scroll_h2h_page_up(100);
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 0);
    }

    #[test]
    fn test_h2h_scroll_resets_on_view_change() {
        use super::super::benchmarks::BottomView;
        let mut app = make_test_app();
        app.benchmarks_app.h2h_scroll.set(15);
        app.benchmarks_app.update_bottom_view(3);
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::H2H);
        assert_eq!(app.benchmarks_app.h2h_scroll.get(), 0);
    }
}
