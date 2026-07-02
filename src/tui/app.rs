use super::agents::AgentsApp;
use super::benchmarks::BenchmarksApp;
use super::models::ModelsApp;
use super::status::StatusApp;

/// Page size for page up/down navigation
const PAGE_SIZE: usize = 10;

pub const MAX_SELECTIONS: usize = 8;
use crate::agents::{AgentsFile, FetchStatus, GitHubData};

use crate::benchmarks::multi::MultiStore;
use crate::benchmarks::schema::SourceFile;
use crate::config::Config;
use crate::data::{Provider, ProvidersMap};
use crate::tui::widgets::scroll_offset::ScrollOffset;

/// Fill empty trait fields of source `idx` from models.dev. AA uses the
/// Jaro-Winkler matcher (its slugs need fuzzy matching); the clean-id sources
/// (epoch / arena / llmstats) use the generic exact/normalized enrichment,
/// which also backfills creator and release_date where the source omits them.
fn enrich_source(app: &mut App, idx: usize) {
    let source_id = app.multi_store.sources.get(idx).map(|s| s.descriptor.id);
    if source_id == Some("aa") {
        if let Some(file) = app.multi_store.file_mut(idx) {
            crate::benchmarks::apply_model_traits(&app.providers, &mut file.models);
        }
    } else if source_id.is_some() {
        if let Some(file) = app.multi_store.file_mut(idx) {
            crate::benchmarks::enrich_from_models_dev(&app.providers, &mut file.models);
        }
    }
}

/// Apply the active source's trait/openness augmentation and rebuild the
/// benchmarks sub-app if the freshly loaded source is the active one.
fn finalize_loaded_source(app: &mut App, idx: usize) {
    enrich_source(app, idx);
    if idx == app.benchmarks_app.active_source {
        if let Some(file) = app.multi_store.file(idx) {
            app.benchmarks_app.rebuild(file);
        }
        app.restore_saved_columns(idx);
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

    /// Parse a config `display.default_tab` value. Case-insensitive; unknown
    /// or missing values fall back to the default tab (Models).
    pub fn from_config(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_lowercase).as_deref() {
            Some("models") => Tab::Models,
            Some("agents") => Tab::Agents,
            Some("benchmarks") => Tab::Benchmarks,
            Some("status") => Tab::Status,
            _ => Tab::default(),
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
    // Add-agent form modal messages
    OpenAddAgent,
    CloseAddAgent,
    AddAgentInput(char),
    AddAgentBackspace,
    AddAgentToggleField,
    AddAgentSave,
    // Update-action messages
    RequestUpdateAgent,
    RequestUpdateAll,
    ConfirmUpdate,
    ConfirmUpdateInteractive,
    CancelUpdate,
    /// Cancel the in-flight background update for the selected agent. The actual
    /// child-kill is performed in the main loop (which holds the cancel handles);
    /// `update()` is a no-op for this variant.
    RequestCancelUpdate,
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
    #[allow(dead_code)]
    CopyBenchmarkName,
    OpenBenchmarkUrl,
    /// An async URL open resolved to a final URL (Epoch 404-fallback path);
    /// the main loop reports it to the status bar.
    BenchmarkUrlOpened(String),
    ToggleBenchmarkSelection,
    ClearBenchmarkSelections,
    ToggleDetailOverlay,
    ToggleComparePanel,
    CloseDetailOverlay,
    CycleBenchmarkView,
    CycleScatterX,
    CycleScatterY,
    CycleRadarPreset,
    /// `a` in browse mode — cycle the detail-panel comparator column
    /// (field avg → peers → rank → off).
    CycleComparator,
    ScrollH2HDown,
    ScrollH2HUp,
    ScrollH2HTop,
    ScrollH2HPageDown,
    ScrollH2HPageUp,
    // Benchmark glossary popup (`i`)
    ToggleGlossary,
    ScrollGlossaryUp,
    ScrollGlossaryDown,
    // Column visibility picker (`C`, browse mode)
    OpenColumnPicker,
    ColumnPickerNext,
    ColumnPickerPrev,
    ColumnPickerFirst,
    ColumnPickerLast,
    ColumnPickerToggle,
    ColumnPickerSave,
    ColumnPickerCancel,
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
    // `r` on Models tab — re-fetch models.dev data. The fetch is spawned in
    // the main loop; its result arrives as `ProvidersRefreshed`.
    RefreshModels,
    /// Result of an `r`-triggered models.dev refetch. `Some(map)` => swap in
    /// the new providers and state-preservingly rebuild; `None` => keep the
    /// current providers, report a non-fatal failure.
    ProvidersRefreshed(Option<crate::data::ProvidersMap>),
    // `R` on Agents tab — re-trigger GitHub fetches for tracked agents.
    RefreshAgents,
    // `r` — re-fetch the active source (stale-while-revalidate). The fetch is
    // spawned in the main loop; its result arrives as `DataSourceRefreshed`.
    RefreshBenchmarkSource,
    // Async data messages
    GitHubDataReceived(String, GitHubData),
    GitHubFetchFailed(String, String), // (agent_id, error_message)
    // Benchmark data: one variant per source fetch. `None` => fetch failed.
    DataSourceLoaded(usize, Option<SourceFile>),
    // Result of an `r`-triggered refresh of source `idx`. `Some` => replace the
    // loaded file and state-preservingly rebuild; `None` => keep the current
    // (good) data, report a non-fatal failure.
    DataSourceRefreshed(usize, Option<SourceFile>),
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
    /// Confirmed agent updates to spawn as background subprocesses (agent_id, argv)
    pub pending_updates: Vec<(String, Vec<String>)>,
    /// A single confirmed *interactive* update (agent_id, argv) for the main loop
    /// to run with a terminal handover (suspend-and-run). At most one at a time.
    pub pending_interactive_update: Option<(String, Vec<String>)>,
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
            current_tab: Tab::from_config(config.display.default_tab.as_deref()),
            models_app,
            agents_app,
            config,
            pending_fetches: Vec::new(),
            pending_updates: Vec::new(),
            pending_interactive_update: None,
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

    /// Switch the benchmarks data source `{`/`}`, carrying the compare
    /// `selections` over to the new source by exact id match (order-preserving;
    /// missing ids drop). Falls back to clearing when either side isn't loaded.
    /// `update_bottom_view` runs afterwards so browse/compare mode stays
    /// consistent with the surviving selection count.
    fn switch_data_source(&mut self, forward: bool) {
        let old_active = self.benchmarks_app.active_source;
        self.benchmarks_app
            .switch_source(&self.multi_store, forward);
        let new_active = self.benchmarks_app.active_source;

        match (
            self.multi_store.file(old_active),
            self.multi_store.file(new_active),
        ) {
            (Some(old_file), Some(new_file)) => {
                self.selections =
                    Self::remap_selections_by_id(&self.selections, old_file, new_file);
            }
            // Either side not loaded — there's no honest id mapping to apply.
            _ => self.clear_selections(),
        }
        self.benchmarks_app
            .update_bottom_view(self.selections.len());
        // Demote focus when the surviving selection count drops below compare.
        if self.selections.len() < 2
            && self.benchmarks_app.focus == super::benchmarks::BenchmarkFocus::Compare
        {
            self.benchmarks_app.focus = super::benchmarks::BenchmarkFocus::List;
        }
        // reset_for_source cleared visible_columns; restore this source's saved
        // selection from config (no-op while the source is still unloaded — the
        // restore re-runs in finalize_loaded_source when the file lands).
        self.restore_saved_columns(new_active);
    }

    /// Switch the active benchmark data source directly to `target` (an index
    /// into the compiled-in sources), applying the same selection remap, focus
    /// demotion, and column restore as the `{`/`}` cycle. No-op when `target` is
    /// already active or out of range. Used by mouse clicks on the source-bar
    /// labels so a click lands on the source clicked, not one step toward it.
    pub fn switch_to_data_source(&mut self, target: usize) {
        let old_active = self.benchmarks_app.active_source;
        if target == old_active || target >= self.multi_store.sources.len() {
            return;
        }
        self.benchmarks_app.active_source = target;
        self.benchmarks_app.reset_for_source(&self.multi_store);

        match (
            self.multi_store.file(old_active),
            self.multi_store.file(target),
        ) {
            (Some(old_file), Some(new_file)) => {
                self.selections =
                    Self::remap_selections_by_id(&self.selections, old_file, new_file);
            }
            _ => self.clear_selections(),
        }
        self.benchmarks_app
            .update_bottom_view(self.selections.len());
        if self.selections.len() < 2
            && self.benchmarks_app.focus == super::benchmarks::BenchmarkFocus::Compare
        {
            self.benchmarks_app.focus = super::benchmarks::BenchmarkFocus::List;
        }
        self.restore_saved_columns(target);
    }

    /// Restore `visible_columns` for source `idx` from the config-persisted
    /// metric ids (per-source `[benchmarks.columns]`). No-op when the source
    /// isn't loaded or nothing is saved for it.
    fn restore_saved_columns(&mut self, idx: usize) {
        let Some(source_id) = self.multi_store.sources.get(idx).map(|s| s.descriptor.id) else {
            return;
        };
        let Some(saved) = self.config.benchmarks.columns.get(source_id) else {
            return;
        };
        let saved = saved.clone();
        if let Some(file) = self.multi_store.file(idx) {
            self.benchmarks_app.apply_saved_columns(file, &saved);
        }
    }

    /// Mirror the active source's `visible_columns` into the in-memory config
    /// (as metric ids). An empty selection removes the source's entry so the
    /// config file doesn't accumulate `aa = []` noise. Returns `false` when
    /// there was nothing to sync (source unloaded / unknown index). Disk IO
    /// lives in `persist_visible_columns` so tests can assert the mutation
    /// without touching the real config file.
    fn sync_visible_columns_to_config(&mut self) -> bool {
        let idx = self.benchmarks_app.active_source;
        let Some(source_id) = self.multi_store.sources.get(idx).map(|s| s.descriptor.id) else {
            return false;
        };
        let Some(file) = self.multi_store.file(idx) else {
            return false;
        };
        let ids = self.benchmarks_app.visible_column_ids(file);
        if ids.is_empty() {
            self.config.benchmarks.columns.remove(source_id);
        } else {
            self.config
                .benchmarks
                .columns
                .insert(source_id.to_string(), ids);
        }
        true
    }

    /// Sync the active source's column selection into the config and write it
    /// to disk, reporting the outcome on the status bar.
    fn persist_visible_columns(&mut self) {
        if !self.sync_visible_columns_to_config() {
            return;
        }
        match self.config.save() {
            Ok(()) => self.set_status("Columns saved".to_string()),
            Err(e) => self.set_status(format!("Failed to save columns: {e}")),
        }
    }

    /// Apply the result of an `r`-triggered refresh of source `idx`.
    ///
    /// `Some(file)` => replace the loaded file, re-run models.dev enrichment, and
    /// (when `idx` is active) state-preservingly rebuild + remap the compare
    /// selections by id. `None` => keep the current loaded file untouched (a
    /// failed refresh must not discard good data) and report a non-fatal failure.
    fn apply_source_refresh(&mut self, idx: usize, result: Option<SourceFile>) {
        let name = self
            .multi_store
            .sources
            .get(idx)
            .map(|s| s.descriptor.name)
            .unwrap_or("source");
        match result {
            Some(file) => {
                let is_active = idx == self.benchmarks_app.active_source;
                // Snapshot the old selection ids AND the selected row's id against
                // the old file before it is replaced, so both the compare set and
                // the focused row can be remapped to the new file by id.
                let (old_ids, prev_model_id, had_old_file) = match self.multi_store.file(idx) {
                    Some(old_file) if is_active => {
                        let ids: Vec<String> = self
                            .selections
                            .iter()
                            .filter_map(|&i| old_file.models.get(i).map(|m| m.id.clone()))
                            .collect();
                        let sel = self.benchmarks_app.selected_model_id(old_file);
                        (ids, sel, true)
                    }
                    Some(_) => (Vec::new(), None, true),
                    None => (Vec::new(), None, false),
                };

                self.multi_store.set_loaded(idx, file);
                enrich_source(self, idx);

                if is_active {
                    // Remap selections by id when we had a prior file; if the
                    // source was previously unloaded, there's nothing to map.
                    if let Some(new_file) = self.multi_store.file(idx) {
                        self.selections = if had_old_file {
                            old_ids
                                .iter()
                                .filter_map(|id| new_file.models.iter().position(|m| &m.id == id))
                                .collect()
                        } else {
                            Vec::new()
                        };
                    }
                    if let Some(new_file) = self.multi_store.file(idx) {
                        self.benchmarks_app
                            .rebuild_preserving(new_file, prev_model_id);
                    }
                    // Re-resolve saved columns by id against the refreshed file
                    // (more robust than the index prune when metrics moved).
                    self.restore_saved_columns(idx);
                    self.benchmarks_app
                        .update_bottom_view(self.selections.len());
                    if self.selections.len() < 2
                        && self.benchmarks_app.focus == super::benchmarks::BenchmarkFocus::Compare
                    {
                        self.benchmarks_app.focus = super::benchmarks::BenchmarkFocus::List;
                    }
                }
                self.set_status(format!("Refreshed {name}"));
            }
            None => {
                // Keep the existing loaded file: a refresh failure must not
                // discard good data (do NOT set_failed here).
                self.set_status(format!("Failed to refresh {name} — keeping current data"));
            }
        }
    }

    /// Apply the result of an `r`-triggered models.dev refetch.
    ///
    /// `Some(map)` => replace `app.providers`, rebuild the models sub-app
    /// state-preservingly (keep search, filters, sort; try to keep the
    /// selected provider and model by id, falling back to index 0).
    ///
    /// `None` => keep the existing providers untouched; set a non-fatal
    /// failure status.
    ///
    /// Note: already-loaded benchmark sources keep the trait fields applied
    /// at their original load time — re-enrichment on a models refresh is out
    /// of scope (see plan §3.1).
    pub fn apply_models_refresh(&mut self, result: Option<crate::data::ProvidersMap>) {
        match result {
            Some(map) => {
                let mut providers: Vec<(String, crate::data::Provider)> = map.into_iter().collect();
                providers.sort_by(|a, b| a.0.cmp(&b.0));
                let n = providers.iter().map(|(_, p)| p.models.len()).sum::<usize>();

                // Snapshot current selection state for restore-by-id logic.
                let prev_provider_id: Option<String> = self
                    .models_app
                    .selected_provider_data(&self.providers)
                    .map(|(id, _)| id.clone());
                let prev_model_id: Option<String> =
                    self.models_app.current_model().map(|e| e.id.clone());

                self.providers = providers;
                // Rebuild state (preserves search, filters, sort; resets
                // selection to index 0 as a conservative fallback, then we
                // try to restore by id below).
                self.models_app.update_provider_list(&self.providers);
                self.models_app.update_filtered_models(&self.providers);

                // Attempt to restore the selected provider by id.
                if let Some(ref pid) = prev_provider_id {
                    if let Some(new_pos) =
                        self.models_app
                            .provider_list_items
                            .iter()
                            .position(|item| match item {
                                crate::tui::models::app::ProviderListItem::Provider(idx, _) => {
                                    self.providers.get(*idx).is_some_and(|(id, _)| id == pid)
                                }
                                _ => false,
                            })
                    {
                        self.models_app
                            .select_provider_at_index(new_pos, &self.providers);
                    } else {
                        self.models_app.select_provider_at_index(0, &self.providers);
                    }
                } else {
                    // Was on "All" — keep it.
                    self.models_app.provider_list_state.select(Some(0));
                    self.models_app.update_filtered_models(&self.providers);
                }

                // Attempt to restore the selected model by id.
                if let Some(ref mid) = prev_model_id {
                    if let Some(new_idx) = self
                        .models_app
                        .filtered_models()
                        .iter()
                        .position(|e| &e.id == mid)
                    {
                        self.models_app.selected_model = new_idx;
                        self.models_app.model_list_state.select(Some(new_idx + 1));
                    }
                }
                self.models_app.reset_detail_scroll();
                self.set_status(format!("Refreshed models ({} models)", n));
            }
            None => {
                self.set_status("Failed to refresh models — keeping current data".to_string());
            }
        }
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

    /// Remap the compare `selections` (indices into `old_file.models`) onto
    /// indices into `new_file.models`, preserving order so compare colors stay
    /// stable. Returns the new selection vector (does not mutate `self`).
    ///
    /// Two match tiers per selection: exact `ModelRow.id`, then the enrichment
    /// pipeline's `normalize_id` — the four sources spell the same model
    /// differently (`gemini-3.5-flash` / `gemini-3-5-flash`, `DeepSeek-V3-1`,
    /// `zai-org/GLM-4-7`, Arena's dated `claude-haiku-4-5-20251001`), so
    /// exact-only matching made carry-over look randomly flaky. Normalization
    /// can fold variants together (e.g. a `-thinking` suffix strips), so the
    /// exact tier runs first and resolved targets are deduped. Models with no
    /// counterpart in the new source are dropped.
    fn remap_selections_by_id(
        selections: &[usize],
        old_file: &SourceFile,
        new_file: &SourceFile,
    ) -> Vec<usize> {
        use crate::benchmarks::normalize_id;
        let mut out: Vec<usize> = Vec::with_capacity(selections.len());
        for &old_idx in selections {
            let Some(old) = old_file.models.get(old_idx) else {
                continue;
            };
            let found = new_file
                .models
                .iter()
                .position(|m| m.id == old.id)
                .or_else(|| {
                    let want = normalize_id(&old.id);
                    new_file
                        .models
                        .iter()
                        .position(|m| normalize_id(&m.id) == want)
                });
            if let Some(idx) = found {
                if !out.contains(&idx) {
                    out.push(idx);
                }
            }
        }
        out
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
            Message::OpenAddAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.open_add_form();
                }
            }
            Message::CloseAddAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.close_add_form();
                }
            }
            Message::AddAgentInput(c) => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.add_form_input(c);
                }
            }
            Message::AddAgentBackspace => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.add_form_backspace();
                }
            }
            Message::AddAgentToggleField => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.add_form_toggle_field();
                }
            }
            Message::AddAgentSave => {
                if let Some(ref mut agents_app) = self.agents_app {
                    match agents_app.add_agent_save(&mut self.config) {
                        Ok((id, repo)) => {
                            agents_app.pending_github_fetches =
                                agents_app.pending_github_fetches.saturating_add(1);
                            agents_app.loading_github = true;
                            self.set_status(format!("Added {}, fetching releases…", id));
                            self.pending_fetches.push((id, repo));
                        }
                        Err(e) => {
                            self.set_status(e);
                        }
                    }
                }
            }
            Message::RequestUpdateAgent => {
                if let Some(ref mut agents_app) = self.agents_app {
                    if let Err(e) = agents_app.request_update_selected() {
                        self.set_status(e);
                    }
                }
            }
            Message::RequestUpdateAll => {
                if let Some(ref mut agents_app) = self.agents_app {
                    if let Err(e) = agents_app.request_update_all() {
                        self.set_status(e);
                    }
                }
            }
            Message::CancelUpdate => {
                if let Some(ref mut agents_app) = self.agents_app {
                    agents_app.cancel_update();
                }
            }
            // Handled in the main loop (holds the cancel handles); no-op here.
            Message::RequestCancelUpdate => {}
            Message::ConfirmUpdate => {
                if let Some(ref mut agents_app) = self.agents_app {
                    let spawned = agents_app.confirm_update();
                    if !spawned.is_empty() {
                        let n = spawned.len();
                        self.pending_updates.extend(spawned);
                        self.set_status(if n == 1 {
                            "Updating 1 agent…".to_string()
                        } else {
                            format!("Updating {} agents…", n)
                        });
                    }
                }
            }
            Message::ConfirmUpdateInteractive => {
                // Single-agent only: hand off to the main loop's suspend-and-run.
                if let Some(ref mut agents_app) = self.agents_app {
                    if let Some((id, command)) = agents_app.confirm_update_interactive() {
                        self.pending_interactive_update = Some((id, command));
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
                        self.benchmarks_app.select_sort_key(key, file);
                    }
                }
            }
            Message::CloseSortPicker => {
                self.benchmarks_app.show_sort_picker = false;
            }
            Message::ToggleGlossary => {
                if self.current_tab == Tab::Models {
                    self.models_app.toggle_glossary();
                } else {
                    self.benchmarks_app.toggle_glossary();
                }
            }
            Message::ScrollGlossaryUp => {
                if self.current_tab == Tab::Models {
                    self.models_app.scroll_glossary_up();
                } else {
                    self.benchmarks_app.scroll_glossary_up();
                }
            }
            Message::ScrollGlossaryDown => {
                if self.current_tab == Tab::Models {
                    self.models_app.scroll_glossary_down();
                } else {
                    self.benchmarks_app.scroll_glossary_down();
                }
            }
            // --- Column visibility picker ---
            Message::OpenColumnPicker => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.open_column_picker(file);
                }
            }
            Message::ColumnPickerNext => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.column_picker_next(file);
                }
            }
            Message::ColumnPickerPrev => {
                self.benchmarks_app.column_picker_prev();
            }
            Message::ColumnPickerFirst => {
                self.benchmarks_app.column_picker_first();
            }
            Message::ColumnPickerLast => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.column_picker_last(file);
                }
            }
            Message::ColumnPickerToggle => {
                if let Some(file) = self.multi_store.file(self.benchmarks_app.active_source) {
                    self.benchmarks_app.column_picker_toggle(file);
                }
            }
            Message::ColumnPickerSave => {
                self.benchmarks_app.column_picker_save();
                // Persist the (possibly empty) selection per source as metric ids.
                self.persist_visible_columns();
            }
            Message::ColumnPickerCancel => {
                self.benchmarks_app.column_picker_cancel();
            }
            Message::CycleDataSourceNext => {
                self.switch_data_source(true);
            }
            Message::CycleDataSourcePrev => {
                self.switch_data_source(false);
            }
            Message::RefreshBenchmarkSource => {
                // Fetch is spawned in the main loop; status set there too.
            }
            Message::DataSourceRefreshed(idx, result) => {
                self.apply_source_refresh(idx, result);
            }
            Message::RefreshModels => {
                // Fetch is spawned in the main loop; status updated there since
                // the runtime isn't accessible from update().
            }
            Message::ProvidersRefreshed(result) => {
                self.apply_models_refresh(result);
            }
            Message::RefreshAgents => {
                // Mark tracked agents as Loading and increment the pending
                // counter before the spawn loop fires in the main loop.
                if let Some(ref mut agents_app) = self.agents_app {
                    let mut count = 0usize;
                    for entry in &mut agents_app.entries {
                        if entry.tracked {
                            entry.fetch_status = crate::agents::FetchStatus::Loading;
                            count += 1;
                        }
                    }
                    if count > 0 {
                        agents_app.pending_github_fetches =
                            agents_app.pending_github_fetches.saturating_add(count);
                        agents_app.loading_github = true;
                    }
                }
                self.set_status("Refreshing agents\u{2026}".to_string());
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
            Message::CycleComparator => {
                self.benchmarks_app.cycle_comparator();
            }
            Message::CopyBenchmarkName
            | Message::OpenBenchmarkUrl
            | Message::BenchmarkUrlOpened(_) => {
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
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Serializes tests that install `ConfigHomeGuard` (which mutates the process
    /// HOME / XDG_CONFIG_HOME env vars) so they can't race each other under the
    /// default parallel test runner. Recover from poisoning so one failing test
    /// doesn't cascade.
    static CONFIG_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn tab_from_config_parses_known_values_case_insensitively() {
        assert_eq!(Tab::from_config(Some("benchmarks")), Tab::Benchmarks);
        assert_eq!(Tab::from_config(Some("Benchmarks")), Tab::Benchmarks);
        assert_eq!(Tab::from_config(Some("STATUS")), Tab::Status);
        assert_eq!(Tab::from_config(Some("agents")), Tab::Agents);
        assert_eq!(Tab::from_config(Some("models")), Tab::Models);
    }

    #[test]
    fn tab_from_config_falls_back_to_default_on_unknown_or_missing() {
        assert_eq!(Tab::from_config(Some("benchmark")), Tab::default());
        assert_eq!(Tab::from_config(Some("")), Tab::default());
        assert_eq!(Tab::from_config(None), Tab::default());
    }

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
            update_command: vec![],
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
        let _env = CONFIG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    #[test]
    fn add_agent_save_persists_custom_agent_and_queues_fetch() {
        let _env = CONFIG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config_home = temp_config_home();
        let _config_home_guard = ConfigHomeGuard::install(config_home);

        let mut config = Config::default();
        config.agents.custom.clear();

        let agents_file = test_agents_file();
        let mut app = App::new(HashMap::new(), Some(&agents_file), Some(config));
        {
            let agents_app = app.agents_app.as_mut().expect("agents app should exist");
            agents_app.loading_github = false;
            agents_app.pending_github_fetches = 0;
            agents_app.open_add_form();
            agents_app.add_form.name = "My Agent".to_string();
            agents_app.add_form.repo = "owner/my-agent".to_string();
        }

        app.update(Message::AddAgentSave);

        // A GitHub fetch was queued for the new agent.
        assert_eq!(
            app.pending_fetches,
            vec![("my-agent".to_string(), "owner/my-agent".to_string())]
        );

        let agents_app = app.agents_app.as_ref().expect("agents app should exist");
        // Form closed, counters bumped.
        assert!(!agents_app.show_add_form);
        assert_eq!(agents_app.pending_github_fetches, 1);
        assert!(agents_app.loading_github);
        // The new entry exists, is tracked, and is loading.
        let entry = agents_app
            .entries
            .iter()
            .find(|e| e.id == "my-agent")
            .expect("new agent entry should exist");
        assert_eq!(entry.agent.repo, "owner/my-agent");
        assert!(entry.tracked);
        assert!(matches!(
            entry.fetch_status,
            crate::agents::FetchStatus::Loading
        ));
        // Persisted to config (tracked + custom list).
        assert!(app.config.is_tracked("my-agent"));
        assert!(app
            .config
            .agents
            .custom
            .iter()
            .any(|c| c.name == "My Agent"));
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
    fn test_cycle_data_source_clears_selections_when_unloaded() {
        // With no source loaded (the default test store), there's no honest id
        // mapping to carry selections across, so the switch falls back to
        // clearing — and still advances `active_source`.
        let mut app = make_test_app();
        app.toggle_selection(1);
        app.toggle_selection(2);
        assert_eq!(app.selections.len(), 2);
        app.update(Message::CycleDataSourceNext);
        assert!(app.selections.is_empty());
        assert_eq!(app.benchmarks_app.active_source, 1);
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

    // --- Phase 2: cross-source state persistence & in-app refresh ---

    use crate::benchmarks::multi::SortKey as TestSortKey;
    use crate::benchmarks::schema::{
        MetricDef, MetricKind, ModelRow, ReasoningStatus, ScoreCell, SourceFile, SourceMeta,
    };
    use std::collections::BTreeMap;

    fn bm_meta(id: &str) -> SourceMeta {
        SourceMeta {
            id: id.into(),
            name: id.to_uppercase(),
            url: "https://example.com".into(),
            fetched_at: "2026-06-10T00:00:00+00:00".into(),
            verified: true,
        }
    }

    fn bm_metric(id: &str, kind: MetricKind, group: &str) -> MetricDef {
        MetricDef {
            id: id.into(),
            label: id.to_uppercase(),
            kind,
            group: group.into(),
            higher_is_better: true,
            last_updated: None,
            description: None,
            short_label: None,
        }
    }

    fn bm_model(
        id: &str,
        reasoning: ReasoningStatus,
        release: Option<&str>,
        scores: &[(&str, f64)],
    ) -> ModelRow {
        let mut score_map = BTreeMap::new();
        for (mid, v) in scores {
            score_map.insert(
                (*mid).to_string(),
                ScoreCell {
                    value: *v,
                    date: None,
                    ci: None,
                    votes: None,
                },
            );
        }
        ModelRow {
            id: id.into(),
            name: id.into(),
            display_name: id.into(),
            creator: "creator".into(),
            creator_name: "Creator".into(),
            release_date: release.map(str::to_string),
            reasoning_status: reasoning,
            effort_level: None,
            variant_tag: None,
            open_weights: None,
            context_window: None,
            supports_tools: None,
            max_output: None,
            scores: score_map,
        }
    }

    /// File whose models all carry reasoning metadata, with metrics + dates.
    fn bm_file_reasoning(meta_id: &str, model_ids: &[&str]) -> SourceFile {
        SourceFile {
            source: bm_meta(meta_id),
            metrics: vec![
                bm_metric("intelligence", MetricKind::Index, "Indexes"),
                bm_metric("coding", MetricKind::Index, "Indexes"),
            ],
            models: model_ids
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    bm_model(
                        id,
                        ReasoningStatus::Reasoning,
                        Some("2026-01-01"),
                        &[("intelligence", 50.0 + i as f64)],
                    )
                })
                .collect(),
        }
    }

    /// File whose models carry NO reasoning metadata (so `7` is a no-op there).
    fn bm_file_plain(meta_id: &str, model_ids: &[&str]) -> SourceFile {
        SourceFile {
            source: bm_meta(meta_id),
            metrics: vec![bm_metric("elo", MetricKind::Elo, "Arena Elo")],
            models: model_ids
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    bm_model(
                        id,
                        ReasoningStatus::None,
                        Some("2026-01-01"),
                        &[("elo", 1000.0 + i as f64)],
                    )
                })
                .collect(),
        }
    }

    /// Build an app with two distinct sources loaded at indices 0 and 1, active 0.
    fn app_with_two_sources(file0: SourceFile, file1: SourceFile) -> App {
        let mut app = make_test_app();
        // Need at least two compiled-in sources for index 1 to exist.
        assert!(
            app.multi_store.sources.len() >= 2,
            "registry must have ≥2 sources"
        );
        app.multi_store.set_loaded(0, file0);
        app.multi_store.set_loaded(1, file1);
        app.benchmarks_app.active_source = 0;
        // Rebuild the sub-app against the active file (mimics initial load).
        if let Some(f) = app.multi_store.file(0) {
            app.benchmarks_app.rebuild(f);
        }
        app
    }

    #[test]
    fn switch_persists_search_filters_grouping() {
        use super::super::benchmarks::CreatorGrouping;
        use crate::benchmarks::multi::{ReasoningFilter, SortKey};
        let file0 = bm_file_reasoning("aa", &["alpha", "beta"]);
        let file1 = bm_file_reasoning("epoch", &["alpha", "gamma"]);
        let mut app = app_with_two_sources(file0, file1);

        // Dirty cross-source intent.
        app.benchmarks_app.search_query = "alpha".to_string();
        app.benchmarks_app.source_filter = super::super::benchmarks::SourceFilter::Open;
        app.benchmarks_app.creator_grouping = CreatorGrouping::ByRegion;
        app.benchmarks_app.reasoning_filter = ReasoningFilter::Reasoning;
        // A name sort with non-default direction must survive.
        app.benchmarks_app.sort_key = SortKey::Name;
        app.benchmarks_app.sort_descending = true;

        app.update(Message::CycleDataSourceNext);

        assert_eq!(app.benchmarks_app.active_source, 1);
        assert_eq!(app.benchmarks_app.search_query, "alpha");
        assert_eq!(
            app.benchmarks_app.source_filter,
            super::super::benchmarks::SourceFilter::Open
        );
        assert_eq!(
            app.benchmarks_app.creator_grouping,
            CreatorGrouping::ByRegion
        );
        assert_eq!(
            app.benchmarks_app.reasoning_filter,
            ReasoningFilter::Reasoning
        );
        // Name sort + direction survive.
        assert_eq!(app.benchmarks_app.sort_key, SortKey::Name);
        assert!(app.benchmarks_app.sort_descending);
    }

    #[test]
    fn switch_metric_sort_falls_back_date_sort_survives() {
        use crate::benchmarks::multi::{default_sort, SortKey};
        let file0 = bm_file_reasoning("aa", &["alpha"]);
        let file1 = bm_file_reasoning("epoch", &["alpha"]);
        let mut app = app_with_two_sources(file0, file1);

        // A metric index does not map across sources -> reset to new default.
        app.benchmarks_app.sort_key = SortKey::Metric(1);
        app.benchmarks_app.sort_descending = false;
        app.update(Message::CycleDataSourceNext);
        let new_file = app.multi_store.file(1).unwrap();
        assert_eq!(app.benchmarks_app.sort_key, default_sort(new_file));

        // Switch back: a ReleaseDate sort (with direction) survives untouched.
        app.benchmarks_app.sort_key = SortKey::ReleaseDate;
        app.benchmarks_app.sort_descending = false;
        app.update(Message::CycleDataSourcePrev);
        assert_eq!(app.benchmarks_app.active_source, 0);
        assert_eq!(app.benchmarks_app.sort_key, SortKey::ReleaseDate);
        assert!(!app.benchmarks_app.sort_descending);
    }

    #[test]
    fn switch_resets_reasoning_filter_when_target_lacks_reasoning() {
        use crate::benchmarks::multi::ReasoningFilter;
        // Source 0 has reasoning data; source 1 (plain) does not.
        let file0 = bm_file_reasoning("aa", &["alpha"]);
        let file1 = bm_file_plain("arena", &["alpha"]);
        let mut app = app_with_two_sources(file0, file1);

        app.benchmarks_app.reasoning_filter = ReasoningFilter::Reasoning;
        app.update(Message::CycleDataSourceNext);
        // Target has no reasoning metadata -> filter reset to All (avoids a stuck
        // invisible filter silently emptying the list).
        assert_eq!(app.benchmarks_app.reasoning_filter, ReasoningFilter::All);
    }

    #[test]
    fn selection_carryover_shared_ids_survive_in_order() {
        // Source 0 models: [alpha, beta, gamma]; source 1: [gamma, alpha].
        // Selecting alpha(0) + gamma(2) must map to indices in source 1
        // preserving SELECTION order: alpha -> 1, gamma -> 0 => [1, 0].
        let file0 = bm_file_reasoning("aa", &["alpha", "beta", "gamma"]);
        let file1 = bm_file_reasoning("epoch", &["gamma", "alpha"]);
        let mut app = app_with_two_sources(file0, file1);

        app.selections = vec![0, 2]; // alpha, gamma
        app.update(Message::CycleDataSourceNext);
        assert_eq!(app.benchmarks_app.active_source, 1);
        assert_eq!(app.selections, vec![1, 0]);
    }

    #[test]
    fn selection_carryover_matches_normalized_ids_across_spellings() {
        // The same model is spelled differently per source: dotted vs dashed
        // version numbers, raw-HF capitals, org prefixes, trailing date stamps.
        // Exact-id matching alone drops all of these; the normalized tier
        // (enrichment's `normalize_id`) must carry them over.
        let file0 = bm_file_reasoning(
            "epoch",
            &["gemini-3.5-flash", "zai-org/GLM-4-7", "DeepSeek-V3-1"],
        );
        let file1 = bm_file_reasoning(
            "arena",
            &["deepseek-v3-1", "gemini-3-5-flash-20260115", "glm-4-7"],
        );
        let mut app = app_with_two_sources(file0, file1);

        app.selections = vec![0, 1, 2];
        app.update(Message::CycleDataSourceNext);
        // Selection order preserved: gemini -> 1, glm -> 2, deepseek -> 0.
        assert_eq!(app.selections, vec![1, 2, 0]);
    }

    #[test]
    fn selection_carryover_dedupes_normalized_collisions() {
        // A base model and its `-thinking` variant normalize identically; both
        // selections resolve to the single counterpart in the new source and
        // must collapse to one selection, not two markers on the same row.
        let file0 = bm_file_reasoning("arena", &["opus-9", "opus-9-thinking"]);
        let file1 = bm_file_reasoning("aa", &["opus-9", "other"]);
        let mut app = app_with_two_sources(file0, file1);

        app.selections = vec![0, 1];
        app.update(Message::CycleDataSourceNext);
        assert_eq!(app.selections, vec![0]);
    }

    #[test]
    fn column_sync_persists_ids_and_clears_empty_entry() {
        let file0 = bm_file_reasoning("aa", &["alpha"]);
        let mut app = make_test_app();
        app.multi_store.set_loaded(0, file0);
        app.benchmarks_app.active_source = 0;
        // Select the "coding" column (index 1) and sync.
        app.benchmarks_app.visible_columns = vec![1];
        assert!(app.sync_visible_columns_to_config());
        assert_eq!(
            app.config.benchmarks.columns.get("aa").map(Vec::as_slice),
            Some(&["coding".to_string()][..])
        );
        // Clearing the selection removes the entry (no `aa = []` noise).
        app.benchmarks_app.visible_columns.clear();
        assert!(app.sync_visible_columns_to_config());
        assert!(!app.config.benchmarks.columns.contains_key("aa"));
    }

    #[test]
    fn switch_restores_saved_columns_per_source() {
        let file0 = bm_file_reasoning("aa", &["alpha"]);
        let file1 = bm_file_reasoning("epoch", &["alpha"]);
        let mut app = app_with_two_sources(file0, file1);
        app.config
            .benchmarks
            .columns
            .insert("aa".to_string(), vec!["intelligence".to_string()]);
        // Saved epoch entry carries one live id and one stale id (a metric the
        // source no longer ships) — the stale one must drop silently.
        app.config.benchmarks.columns.insert(
            "epoch".to_string(),
            vec!["coding".to_string(), "long-gone".to_string()],
        );

        app.update(Message::CycleDataSourceNext);
        assert_eq!(app.benchmarks_app.active_source, 1);
        assert_eq!(app.benchmarks_app.visible_columns, vec![1]);

        app.update(Message::CycleDataSourcePrev);
        assert_eq!(app.benchmarks_app.active_source, 0);
        assert_eq!(app.benchmarks_app.visible_columns, vec![0]);
    }

    #[test]
    fn selection_carryover_missing_ids_drop_and_demote() {
        use super::super::benchmarks::{BenchmarkFocus, BottomView};
        // Source 0: [alpha, beta, gamma] all selected (3 -> compare mode).
        // Source 1: only [alpha] survives -> 1 selection -> browse demotion.
        let file0 = bm_file_reasoning("aa", &["alpha", "beta", "gamma"]);
        let file1 = bm_file_reasoning("epoch", &["alpha"]);
        let mut app = app_with_two_sources(file0, file1);

        app.selections = vec![0, 1, 2];
        app.benchmarks_app.update_bottom_view(3);
        app.benchmarks_app.focus = BenchmarkFocus::Compare;
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::H2H);

        app.update(Message::CycleDataSourceNext);
        // Only alpha survives.
        assert_eq!(app.selections, vec![0]);
        // Compare -> browse demotion fired.
        assert_eq!(app.benchmarks_app.bottom_view, BottomView::Detail);
        assert_eq!(app.benchmarks_app.focus, BenchmarkFocus::List);
    }

    #[test]
    fn refresh_failure_keeps_old_file_loaded() {
        let file0 = bm_file_reasoning("aa", &["alpha", "beta"]);
        let mut app = make_test_app();
        app.multi_store.set_loaded(0, file0);
        app.benchmarks_app.active_source = 0;
        if let Some(f) = app.multi_store.file(0) {
            app.benchmarks_app.rebuild(f);
        }
        let before = app.multi_store.file(0).unwrap().models.len();

        app.update(Message::DataSourceRefreshed(0, None));
        // Old file is untouched (not set_failed) and a non-fatal status is set.
        assert!(app.multi_store.file(0).is_some());
        assert_eq!(app.multi_store.file(0).unwrap().models.len(), before);
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("keeping current data"));
    }

    #[test]
    fn refresh_success_preserves_sort_search_and_selection_by_id() {
        use crate::benchmarks::multi::SortKey;
        let file0 = bm_file_reasoning("aa", &["alpha", "beta", "gamma"]);
        let mut app = make_test_app();
        app.multi_store.set_loaded(0, file0);
        app.benchmarks_app.active_source = 0;
        if let Some(f) = app.multi_store.file(0) {
            app.benchmarks_app.rebuild(f);
        }

        // Set a Name sort (descending), an active SEARCH narrowing to gamma, and
        // select gamma by id. Search + sort + selection must all survive refresh.
        app.benchmarks_app.sort_key = SortKey::Name;
        app.benchmarks_app.sort_descending = true;
        app.benchmarks_app.search_query = "gamma".to_string();
        app.benchmarks_app
            .rebuild_after_filter_change(app.multi_store.file(0).unwrap());
        // Search narrows the filtered view to gamma only.
        assert_eq!(app.benchmarks_app.filtered_indices.len(), 1);
        app.selections = vec![2]; // gamma in old file

        // Refreshed file reorders models: gamma now at index 0.
        let refreshed = bm_file_reasoning("aa", &["gamma", "alpha", "beta"]);
        app.update(Message::DataSourceRefreshed(0, Some(refreshed)));

        // Sort + direction preserved.
        assert_eq!(app.benchmarks_app.sort_key, SortKey::Name);
        assert!(app.benchmarks_app.sort_descending);
        // SEARCH preserved + re-applied against the refreshed file.
        assert_eq!(app.benchmarks_app.search_query, "gamma");
        assert_eq!(app.benchmarks_app.filtered_indices.len(), 1);
        assert_eq!(
            app.benchmarks_app
                .current_model(app.multi_store.file(0).unwrap())
                .unwrap()
                .id,
            "gamma"
        );
        // Selection remapped by id: gamma moved to index 0.
        assert_eq!(app.selections, vec![0]);
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .starts_with("Refreshed"));
    }

    #[test]
    fn refresh_shrinking_metrics_falls_back_stale_metric_sort() {
        use crate::benchmarks::multi::{default_sort, SortKey};
        // Old file has 2 metrics; sort by Metric(1).
        let file0 = bm_file_reasoning("aa", &["alpha"]);
        let mut app = make_test_app();
        app.multi_store.set_loaded(0, file0);
        app.benchmarks_app.active_source = 0;
        if let Some(f) = app.multi_store.file(0) {
            app.benchmarks_app.rebuild(f);
        }
        app.benchmarks_app.sort_key = SortKey::Metric(1);

        // Refreshed file has only 1 metric -> Metric(1) is now out of range.
        let mut shrunk = bm_file_reasoning("aa", &["alpha"]);
        shrunk.metrics.truncate(1);
        app.update(Message::DataSourceRefreshed(0, Some(shrunk)));

        let new_file = app.multi_store.file(0).unwrap();
        assert_eq!(app.benchmarks_app.sort_key, default_sort(new_file));
        // default_sort here is ReleaseDate (models carry dates).
        assert_eq!(app.benchmarks_app.sort_key, TestSortKey::ReleaseDate);
    }

    // --- Phase 3: Models tab `r` refresh and Agents tab `R` refresh ---

    fn make_providers_map_with(provider_id: &str, model_ids: &[&str]) -> crate::data::ProvidersMap {
        use crate::data::{Model, Provider};
        let models: HashMap<String, Model> = model_ids
            .iter()
            .map(|id| {
                (
                    (*id).to_string(),
                    Model {
                        id: (*id).to_string(),
                        name: (*id).to_string(),
                        family: None,
                        release_date: None,
                        reasoning: false,
                        tool_call: false,
                        attachment: false,
                        temperature: false,
                        modalities: None,
                        open_weights: false,
                        cost: None,
                        limit: None,
                        last_updated: None,
                        knowledge: None,
                        status: None,
                        description: None,
                        structured_output: None,
                        reasoning_options: Vec::new(),
                    },
                )
            })
            .collect();
        let mut map = HashMap::new();
        map.insert(
            provider_id.to_string(),
            Provider {
                id: provider_id.to_string(),
                name: provider_id.to_string(),
                doc: None,
                api: None,
                npm: None,
                env: vec![],
                models,
            },
        );
        map
    }

    #[test]
    fn providers_refreshed_none_keeps_old_providers() {
        let initial = make_providers_map_with("openai", &["gpt-4", "gpt-3.5"]);
        let mut app = App::new(initial, None, None);
        let before_count = app.providers.len();

        // A None result must not discard the existing providers.
        app.update(Message::ProvidersRefreshed(None));

        assert_eq!(app.providers.len(), before_count);
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("keeping current data"));
    }

    #[test]
    fn providers_refreshed_some_swaps_and_sets_status() {
        let initial = make_providers_map_with("openai", &["gpt-4"]);
        let mut app = App::new(initial, None, None);

        let refreshed = make_providers_map_with("openai", &["gpt-4", "gpt-4o", "gpt-4-mini"]);
        app.update(Message::ProvidersRefreshed(Some(refreshed)));

        // Provider list now reflects the new data.
        assert!(app.providers.iter().any(|(id, _)| id == "openai"));
        let total: usize = app.providers.iter().map(|(_, p)| p.models.len()).sum();
        assert_eq!(total, 3);
        let status = app.status_message.as_deref().unwrap();
        assert!(status.contains("Refreshed models"), "status was: {status}");
        assert!(status.contains("3 models"), "status was: {status}");
    }

    #[test]
    fn providers_refreshed_preserves_search_filter_sort() {
        use crate::tui::models::app::SortOrder;
        let initial = make_providers_map_with("openai", &["gpt-4", "gpt-4o"]);
        let mut app = App::new(initial, None, None);

        // Set up non-default state that must survive the refresh.
        app.models_app.search_query = "gpt-4".to_string();
        app.models_app.filters.reasoning = true;
        app.models_app.sort_order = SortOrder::Cost;

        let refreshed = make_providers_map_with("openai", &["gpt-4", "gpt-4o", "o3"]);
        app.update(Message::ProvidersRefreshed(Some(refreshed)));

        assert_eq!(app.models_app.search_query, "gpt-4");
        assert!(app.models_app.filters.reasoning);
        assert_eq!(app.models_app.sort_order, SortOrder::Cost);
    }

    /// Verifies the in-process state transitions from `Message::RefreshAgents`:
    /// tracked entries must be flipped to `Loading` and the pending counter
    /// incremented **before** `spawn_agent_fetches` is called in `mod.rs`.
    ///
    /// This ordering is load-bearing: `spawn_agent_fetches` filters on
    /// `Loading | NotStarted`, so it must run after this state flip — not
    /// before, as was the bug (review round 1, phase 3). The unit test cannot
    /// cover the async dispatch path; that relies on the `need_refresh_agents`
    /// flag pattern in `run_app` (mod.rs) ensuring the spawn executes
    /// post-`app.update()`.
    #[test]
    fn refresh_agents_sets_tracked_entries_loading_and_increments_counter() {
        use crate::agents::{Agent, AgentsFile, FetchStatus};
        let agents_file = {
            let mut agents = HashMap::new();
            for id in &["alpha", "beta", "gamma"] {
                agents.insert(
                    id.to_string(),
                    Agent {
                        name: id.to_string(),
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
                );
            }
            AgentsFile {
                schema_version: 1,
                last_scraped: None,
                scrape_source: None,
                agents,
            }
        };

        let mut config = Config::default();
        // Track alpha and beta; leave gamma untracked.
        config.agents.tracked = HashSet::from(["alpha".to_string(), "beta".to_string()]);
        config.agents.excluded = HashSet::new();
        config.agents.custom.clear();

        let mut app = App::new(HashMap::new(), Some(&agents_file), Some(config));

        // Force all entries to Loaded so we can verify they're re-set to Loading.
        if let Some(ref mut agents_app) = app.agents_app {
            for entry in &mut agents_app.entries {
                entry.fetch_status = FetchStatus::Loaded;
            }
            agents_app.pending_github_fetches = 0;
            agents_app.loading_github = false;
        }

        app.update(Message::RefreshAgents);

        let agents_app = app.agents_app.as_ref().expect("agents_app");
        let tracked_ids: Vec<&str> = agents_app
            .entries
            .iter()
            .filter(|e| e.tracked)
            .map(|e| e.id.as_str())
            .collect();
        // All tracked entries are now Loading.
        for entry in agents_app.entries.iter().filter(|e| e.tracked) {
            assert_eq!(
                entry.fetch_status,
                FetchStatus::Loading,
                "tracked entry {} should be Loading",
                entry.id
            );
        }
        // Untracked entries are not Loading.
        for entry in agents_app.entries.iter().filter(|e| !e.tracked) {
            assert_ne!(
                entry.fetch_status,
                FetchStatus::Loading,
                "untracked entry {} should NOT be Loading",
                entry.id
            );
        }
        // Pending counter reflects tracked count.
        assert_eq!(
            agents_app.pending_github_fetches,
            tracked_ids.len(),
            "pending counter should equal tracked count"
        );
        assert!(agents_app.loading_github, "loading_github should be true");
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("Refreshing agents"));
    }
}
