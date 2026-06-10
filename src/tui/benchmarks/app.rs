use std::collections::HashMap;

use ratatui::style::Color;
use ratatui::widgets::ListState;

use crate::benchmarks::creator_openness;
use crate::benchmarks::multi::{
    default_sort, format_metric_value, radar_groups, MultiStore, ReasoningFilter, SortKey,
    SourceLoad,
};
use crate::benchmarks::schema::{MetricKind, ModelRow, SourceFile};
use crate::benchmarks::sources::SourceDescriptor;
use crate::formatting::{cmp_opt_f64, parse_date_to_numeric};
use crate::tui::widgets::scroll_offset::ScrollOffset;

/// Page size for page up/down navigation
const PAGE_SIZE: usize = 10;

/// A single entry in the dynamic sort picker, paired with the metric index it
/// targets (only meaningful for `SortKey::Metric`).
#[derive(Debug, Clone)]
pub struct SortOption {
    pub key: SortKey,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BenchmarkFocus {
    Creators,
    #[default]
    List,
    Details,
    Compare,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreatorListItem {
    All,
    GroupHeader(String), // non-selectable section header
    Creator(String),     // creator slug
}

/// Per-model source filter: prefers the model's own `open_weights`, falling back
/// to the creator-openness map. Entries with unknown openness (model `None` and
/// creator absent from the map) are excluded when filtering by Open or Closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceFilter {
    #[default]
    All,
    Open,
    Closed,
}

impl SourceFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Open,
            Self::Open => Self::Closed,
            Self::Closed => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Open => "Open",
            Self::Closed => "Closed",
        }
    }

    /// Check if a model passes the filter. Openness is resolved per-model first
    /// (`ModelRow.open_weights`), falling back to the creator-openness map.
    /// Models with unknown openness (model `None` and creator absent from the
    /// map) are excluded when filtering by Open or Closed.
    pub fn matches(self, model: &ModelRow, openness: &HashMap<String, bool>) -> bool {
        let open = model
            .open_weights
            .or_else(|| openness.get(&model.creator).copied());
        match self {
            Self::All => true,
            Self::Open => open == Some(true),
            Self::Closed => open == Some(false),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatorRegion {
    US,
    China,
    Europe,
    MiddleEast,
    SouthKorea,
    Canada,
    Other,
}

impl CreatorRegion {
    pub fn label(self) -> &'static str {
        match self {
            Self::US => "US",
            Self::China => "China",
            Self::Europe => "Europe",
            Self::MiddleEast => "Middle East",
            Self::SouthKorea => "S. Korea",
            Self::Canada => "Canada",
            Self::Other => "Other",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::US => "US",
            Self::China => "CN",
            Self::Europe => "EU",
            Self::MiddleEast => "ME",
            Self::SouthKorea => "KR",
            Self::Canada => "CA",
            Self::Other => "??",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::US => Color::Blue,
            Self::China => Color::Red,
            Self::Europe => Color::Magenta,
            Self::MiddleEast => Color::Yellow,
            Self::SouthKorea => Color::Cyan,
            Self::Canada => Color::Green,
            Self::Other => Color::DarkGray,
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "US" => Some(Self::US),
            "China" => Some(Self::China),
            "Europe" => Some(Self::Europe),
            "Middle East" => Some(Self::MiddleEast),
            "S. Korea" => Some(Self::SouthKorea),
            "Canada" => Some(Self::Canada),
            "Other" => Some(Self::Other),
            _ => None,
        }
    }

    pub fn from_creator(slug: &str) -> Self {
        match slug {
            // United States
            "openai" | "anthropic" | "google" | "meta" | "xai" | "aws" | "nvidia"
            | "perplexity" | "azure" | "ibm" | "databricks" | "servicenow" | "snowflake"
            | "liquidai" | "nous-research" | "ai2" | "prime-intellect" | "deepcogito"
            | "reka-ai" => Self::US,
            // China
            "deepseek" | "alibaba" | "kimi" | "minimax" | "stepfun" | "baidu"
            | "bytedance_seed" | "xiaomi" | "inclusionai" | "kwaikat" | "zai" | "openchat" => {
                Self::China
            }
            // Europe
            "mistral" => Self::Europe,
            // Middle East (UAE, Israel)
            "tii-uae" | "mbzuai" | "ai21-labs" => Self::MiddleEast,
            // South Korea
            "naver" | "korea-telecom" | "lg" | "upstage" | "motif-technologies" => Self::SouthKorea,
            // Canada
            "cohere" => Self::Canada,
            // Other
            _ => Self::Other,
        }
    }
}

/// How creators are grouped in the sidebar (toggle, not filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CreatorGrouping {
    #[default]
    None,
    ByRegion,
    ByType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatorType {
    Startup,
    Giant,
    Research,
}

impl CreatorType {
    pub fn color(self) -> Color {
        match self {
            Self::Startup => Color::Green,
            Self::Giant => Color::Blue,
            Self::Research => Color::Magenta,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Startup => "Startup",
            Self::Giant => "Big Tech",
            Self::Research => "Research",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Startup => "SU",
            Self::Giant => "BT",
            Self::Research => "RS",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "Startup" => Some(Self::Startup),
            "Big Tech" => Some(Self::Giant),
            "Research" => Some(Self::Research),
            _ => None,
        }
    }

    pub fn from_creator(slug: &str) -> Self {
        match slug {
            // Big tech / large corporations
            "google" | "meta" | "aws" | "nvidia" | "alibaba" | "azure" | "ibm" | "servicenow"
            | "snowflake" | "baidu" | "bytedance_seed" | "xiaomi" | "naver" | "korea-telecom"
            | "lg" | "kwaikat" | "databricks" | "zai" | "inclusionai" => Self::Giant,
            // Research labs / institutes / nonprofits
            "tii-uae" | "mbzuai" | "nous-research" | "ai2" | "openchat" => Self::Research,
            // AI-focused startups (default)
            _ => Self::Startup,
        }
    }
}

/// Pre-computed creator info: display name and model counts.
struct CreatorInfo {
    display_name: String,
    count: usize,
    filtered_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BottomView {
    #[default]
    Detail,
    H2H,
    Scatter,
    Radar,
}

pub struct BenchmarksApp {
    /// Index into [`crate::benchmarks::sources::SOURCES`] / `MultiStore::sources`.
    pub active_source: usize,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub list_state: ListState,
    pub focus: BenchmarkFocus,
    pub sort_key: SortKey,
    pub sort_descending: bool,
    pub search_query: String,
    // Creator sidebar
    pub creator_list_items: Vec<CreatorListItem>,
    pub selected_creator: usize,
    pub creator_list_state: ListState,
    pub source_filter: SourceFilter,
    pub reasoning_filter: ReasoningFilter,
    pub creator_grouping: CreatorGrouping,
    creator_info: HashMap<String, CreatorInfo>,
    /// Creator -> openness, derived from the active source's models.
    creator_openness: HashMap<String, bool>,
    pub bottom_view: BottomView,
    pub h2h_scroll: ScrollOffset,
    pub show_detail_overlay: bool,
    pub show_creators_in_compare: bool,
    /// Scatter axis metric indices into the active source's `metrics`.
    pub scatter_x: usize,
    pub scatter_y: usize,
    /// Index into `radar_groups(file)` of the active radar preset group.
    pub radar_group: usize,
    pub show_sort_picker: bool,
    pub sort_picker_selected: usize,
    pub loading: bool,
    pub detail_scroll: ScrollOffset,
}

impl BenchmarksApp {
    /// Construct against the active source's file (or `None` while it loads).
    pub fn new(file: Option<&SourceFile>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        let mut creator_list_state = ListState::default();
        creator_list_state.select(Some(0));

        let sort_key = file.map(default_sort).unwrap_or(SortKey::ReleaseDate);

        let mut app = Self {
            active_source: 0,
            filtered_indices: Vec::new(),
            selected: 0,
            list_state,
            focus: BenchmarkFocus::default(),
            sort_key,
            sort_descending: true,
            search_query: String::new(),
            creator_list_items: Vec::new(),
            selected_creator: 0,
            creator_list_state,
            source_filter: SourceFilter::default(),
            reasoning_filter: ReasoningFilter::default(),
            creator_grouping: CreatorGrouping::default(),
            creator_info: HashMap::new(),
            creator_openness: HashMap::new(),
            bottom_view: BottomView::default(),
            h2h_scroll: ScrollOffset::default(),
            show_detail_overlay: false,
            show_creators_in_compare: false,
            scatter_x: 0,
            scatter_y: 1,
            radar_group: 0,
            show_sort_picker: false,
            sort_picker_selected: 0,
            loading: true,
            detail_scroll: ScrollOffset::default(),
        };

        if let Some(file) = file {
            app.creator_openness = creator_openness(&file.models);
            app.build_creator_list(file);
            app.update_filtered(file);
        }
        app
    }

    // --- Active source / descriptor accessors ---

    /// Descriptor of the active source.
    pub fn active_descriptor(
        store: &MultiStore,
        active: usize,
    ) -> Option<&'static SourceDescriptor> {
        store.sources.get(active).map(|s| s.descriptor)
    }

    /// The active source's `SourceFile`, if loaded.
    pub fn active_file(store: &MultiStore, active: usize) -> Option<&SourceFile> {
        store.file(active)
    }

    /// `true` when the active source is still loading.
    pub fn active_is_loading(store: &MultiStore, active: usize) -> bool {
        matches!(
            store.sources.get(active).map(|s| &s.load),
            Some(SourceLoad::Loading)
        )
    }

    /// `true` when the active source failed to load.
    pub fn active_is_failed(store: &MultiStore, active: usize) -> bool {
        matches!(
            store.sources.get(active).map(|s| &s.load),
            Some(SourceLoad::Failed)
        )
    }

    /// Whether the active source has any model carrying a reasoning status, used
    /// to hide the `7` reasoning filter when it would be a no-op. Returns `false`
    /// when the source is not loaded.
    pub fn reasoning_filter_available(file: Option<&SourceFile>) -> bool {
        file.map(|f| {
            f.models
                .iter()
                .any(|m| !matches!(m.reasoning_status, crate::benchmarks::ReasoningStatus::None))
        })
        .unwrap_or(false)
    }

    /// Snapshot of the active source's creator-openness map (for render).
    pub fn creator_openness(&self) -> &HashMap<String, bool> {
        &self.creator_openness
    }

    // --- Source switching ---

    /// Switch to the next/previous source (wrapping). Resets all per-source view
    /// state: clears selections-related state, search, sort, scroll, and rebuilds
    /// the creator list + filtered indices against the newly active file.
    ///
    /// Selecting a Loading/Failed source is allowed — the views render the
    /// loading/error state. `App` must clear its shared `selections` separately.
    pub fn switch_source(&mut self, store: &MultiStore, forward: bool) {
        let count = store.sources.len();
        if count == 0 {
            return;
        }
        self.active_source = if forward {
            (self.active_source + 1) % count
        } else {
            (self.active_source + count - 1) % count
        };
        self.reset_for_source(store);
    }

    /// Reset all view state for the currently active source. Called by
    /// `switch_source` and after the active source first loads.
    pub fn reset_for_source(&mut self, store: &MultiStore) {
        let active = self.active_source;
        self.search_query.clear();
        self.source_filter = SourceFilter::default();
        self.reasoning_filter = ReasoningFilter::default();
        self.creator_grouping = CreatorGrouping::default();
        self.selected = 0;
        self.selected_creator = 0;
        self.list_state.select(Some(0));
        self.creator_list_state.select(Some(0));
        self.scatter_x = 0;
        self.scatter_y = 1;
        self.radar_group = 0;
        self.bottom_view = BottomView::default();
        self.show_detail_overlay = false;
        self.show_sort_picker = false;
        self.h2h_scroll.jump_top();
        self.reset_detail_scroll();

        if let Some(file) = store.file(active) {
            self.loading = false;
            self.sort_key = default_sort(file);
            self.sort_descending = Self::default_descending(file, self.sort_key);
            self.creator_openness = creator_openness(&file.models);
            self.build_creator_list(file);
            self.update_filtered(file);
        } else {
            self.loading = !Self::active_is_failed(store, active);
            self.sort_key = SortKey::ReleaseDate;
            self.sort_descending = true;
            self.creator_openness = HashMap::new();
            self.creator_info = HashMap::new();
            self.creator_list_items = vec![CreatorListItem::All];
            self.filtered_indices = Vec::new();
        }
    }

    // --- Rebuilds (require the active file) ---

    /// Rebuild all derived state after the active source's file first lands.
    /// Re-derives creator list, filtered indices, openness, applies the
    /// per-source default sort, and resets selection.
    pub fn rebuild(&mut self, file: &SourceFile) {
        self.loading = false;
        self.sort_key = default_sort(file);
        self.sort_descending = Self::default_descending(file, self.sort_key);
        self.creator_openness = creator_openness(&file.models);
        self.build_creator_list(file);
        self.selected_creator = 0;
        self.creator_list_state.select(Some(0));
        self.selected = 0;
        self.update_filtered(file);
        self.reset_detail_scroll();
    }

    /// Rebuild creator list and filtered entries after any search/filter change.
    /// Preserves the selected creator if it's still visible.
    pub fn rebuild_after_filter_change(&mut self, file: &SourceFile) {
        // Remember which creator was selected
        let prev_creator_slug = match self.creator_list_items.get(self.selected_creator) {
            Some(CreatorListItem::Creator(slug)) => Some(slug.clone()),
            _ => None, // All or GroupHeader
        };

        self.build_creator_list(file);

        // Try to find the previously selected creator in the new list
        let new_pos = prev_creator_slug.and_then(|prev_slug| {
            self.creator_list_items.iter().position(
                |item| matches!(item, CreatorListItem::Creator(slug) if *slug == prev_slug),
            )
        });

        self.selected_creator = new_pos.unwrap_or(0);
        self.creator_list_state.select(Some(self.selected_creator));
        self.selected = 0;
        self.update_filtered(file);
        self.reset_detail_scroll();
    }

    fn has_active_filters(&self) -> bool {
        !self.search_query.is_empty()
            || self.source_filter != SourceFilter::All
            || self.reasoning_filter != ReasoningFilter::default()
    }

    fn model_matches_filters(&self, model: &ModelRow) -> bool {
        if !self.source_filter.matches(model, &self.creator_openness) {
            return false;
        }
        if !self.reasoning_filter.matches(model) {
            return false;
        }
        if !self.search_query.is_empty() {
            let query_lower = self.search_query.to_lowercase();
            return model.display_name.to_lowercase().contains(&query_lower)
                || model.name.to_lowercase().contains(&query_lower)
                || model.creator.to_lowercase().contains(&query_lower)
                || model.creator_name.to_lowercase().contains(&query_lower);
        }
        true
    }

    fn build_creator_list(&mut self, file: &SourceFile) {
        let mut info: HashMap<String, CreatorInfo> = HashMap::new();
        let filtering = self.has_active_filters();

        for model in &file.models {
            if model.creator.is_empty() {
                continue;
            }
            let passes = !filtering || self.model_matches_filters(model);
            info.entry(model.creator.clone())
                .and_modify(|i| {
                    i.count += 1;
                    if passes {
                        i.filtered_count += 1;
                    }
                })
                .or_insert_with(|| CreatorInfo {
                    display_name: if model.creator_name.is_empty() {
                        model.creator.clone()
                    } else {
                        model.creator_name.clone()
                    },
                    count: 1,
                    filtered_count: if passes { 1 } else { 0 },
                });
        }

        let mut creators: Vec<String> = if filtering {
            info.iter()
                .filter(|(_, i)| i.filtered_count > 0)
                .map(|(k, _)| k.clone())
                .collect()
        } else {
            info.keys().cloned().collect()
        };
        creators.sort_by(|a, b| {
            let name_a = &info[a].display_name;
            let name_b = &info[b].display_name;
            name_a.to_lowercase().cmp(&name_b.to_lowercase())
        });

        self.creator_list_items = Vec::with_capacity(creators.len() + 1);
        self.creator_list_items.push(CreatorListItem::All);

        match self.creator_grouping {
            CreatorGrouping::None => {
                for slug in creators {
                    self.creator_list_items.push(CreatorListItem::Creator(slug));
                }
            }
            CreatorGrouping::ByRegion => {
                let regions = [
                    CreatorRegion::US,
                    CreatorRegion::China,
                    CreatorRegion::Europe,
                    CreatorRegion::MiddleEast,
                    CreatorRegion::SouthKorea,
                    CreatorRegion::Canada,
                    CreatorRegion::Other,
                ];
                for region in &regions {
                    let group: Vec<&String> = creators
                        .iter()
                        .filter(|s| CreatorRegion::from_creator(s) == *region)
                        .collect();
                    if group.is_empty() {
                        continue;
                    }
                    self.creator_list_items
                        .push(CreatorListItem::GroupHeader(region.label().to_string()));
                    for slug in group {
                        self.creator_list_items
                            .push(CreatorListItem::Creator(slug.clone()));
                    }
                }
            }
            CreatorGrouping::ByType => {
                let types = [
                    CreatorType::Startup,
                    CreatorType::Giant,
                    CreatorType::Research,
                ];
                for ct in &types {
                    let group: Vec<&String> = creators
                        .iter()
                        .filter(|s| CreatorType::from_creator(s) == *ct)
                        .collect();
                    if group.is_empty() {
                        continue;
                    }
                    self.creator_list_items
                        .push(CreatorListItem::GroupHeader(ct.label().to_string()));
                    for slug in group {
                        self.creator_list_items
                            .push(CreatorListItem::Creator(slug.clone()));
                    }
                }
            }
        }

        self.creator_info = info;
    }

    /// Get (display_name, count) for a creator slug.
    /// Returns filtered count when search/filters are active, total count otherwise.
    pub fn creator_display<'a>(&'a self, slug: &'a str) -> (&'a str, usize) {
        self.creator_info
            .get(slug)
            .map(|i| {
                let count = if self.has_active_filters() {
                    i.filtered_count
                } else {
                    i.count
                };
                (i.display_name.as_str(), count)
            })
            .unwrap_or((slug, 0))
    }

    /// Total filtered count across all visible creators.
    pub fn filtered_creator_count(&self) -> usize {
        if self.has_active_filters() {
            self.creator_list_items
                .iter()
                .filter_map(|item| match item {
                    CreatorListItem::Creator(slug) => {
                        self.creator_info.get(slug).map(|i| i.filtered_count)
                    }
                    _ => None,
                })
                .sum()
        } else {
            self.creator_info.values().map(|i| i.count).sum()
        }
    }

    /// Get the currently selected creator slug, or None for "All".
    fn selected_creator_slug(&self) -> Option<&str> {
        match self.creator_list_items.get(self.selected_creator) {
            Some(CreatorListItem::Creator(slug)) => Some(slug),
            _ => None,
        }
    }

    /// Get the display name of the currently selected creator, or None for "All".
    pub fn selected_creator_name(&self) -> Option<&str> {
        let slug = self.selected_creator_slug()?;
        Some(self.creator_display(slug).0)
    }

    pub fn update_filtered(&mut self, file: &SourceFile) {
        let query_lower = self.search_query.to_lowercase();
        let creator_slug = self.selected_creator_slug().map(|s| s.to_owned());
        let source_filter = self.source_filter;
        let reasoning_filter = self.reasoning_filter;

        self.filtered_indices = file
            .models
            .iter()
            .enumerate()
            .filter(|(_, model)| {
                // Per-model source filter (open/closed)
                if !source_filter.matches(model, &self.creator_openness) {
                    return false;
                }
                // Reasoning filter
                if !reasoning_filter.matches(model) {
                    return false;
                }
                // Creator filter
                if let Some(ref slug) = creator_slug {
                    if model.creator != *slug {
                        return false;
                    }
                }
                // Search filter
                if !query_lower.is_empty() {
                    return model.display_name.to_lowercase().contains(&query_lower)
                        || model.name.to_lowercase().contains(&query_lower)
                        || model.creator.to_lowercase().contains(&query_lower)
                        || model.creator_name.to_lowercase().contains(&query_lower);
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        // Null-filter: hide models missing data for the active sort key. Matches
        // the legacy behavior — every key except `Name` drops rows with no value
        // for the active column (ReleaseDate drops dateless models, Metric drops
        // models with no score for that metric).
        match self.sort_key {
            SortKey::Name => {}
            SortKey::ReleaseDate => {
                self.filtered_indices
                    .retain(|&i| file.models[i].release_date.is_some());
            }
            SortKey::Metric(mi) => {
                if let Some(metric) = file.metrics.get(mi) {
                    let metric_id = metric.id.clone();
                    self.filtered_indices
                        .retain(|&i| file.models[i].scores.contains_key(&metric_id));
                }
            }
        }

        self.apply_sort(file);

        if self.selected >= self.filtered_indices.len() {
            self.selected = 0;
        }
        self.list_state.select(Some(self.selected));
        self.reset_detail_scroll();
    }

    /// Extract the numeric value of `key` for a model (None when missing).
    fn extract(file: &SourceFile, model: &ModelRow, key: SortKey) -> Option<f64> {
        match key {
            SortKey::Name => Some(0.0),
            SortKey::ReleaseDate => model
                .release_date
                .as_ref()
                .and_then(|d| parse_date_to_numeric(d)),
            SortKey::Metric(mi) => file
                .metrics
                .get(mi)
                .and_then(|m| model.scores.get(&m.id))
                .map(|cell| cell.value),
        }
    }

    pub fn apply_sort(&mut self, file: &SourceFile) {
        let key = self.sort_key;
        let desc = self.sort_descending;

        self.filtered_indices.sort_by(|&a, &b| {
            let ma = &file.models[a];
            let mb = &file.models[b];

            let ord = match key {
                SortKey::Name => ma.name.cmp(&mb.name),
                _ => cmp_opt_f64(Self::extract(file, ma, key), Self::extract(file, mb, key)),
            };

            if desc {
                ord.reverse()
            } else {
                ord
            }
        });
    }

    pub fn toggle_sort_direction(&mut self, file: &SourceFile) {
        self.sort_descending = !self.sort_descending;
        self.apply_sort(file);
    }

    /// Default descending for everything except Name and lower-is-better metrics
    /// (latency, pricing).
    fn default_descending(file: &SourceFile, key: SortKey) -> bool {
        match key {
            SortKey::Name => false,
            SortKey::ReleaseDate => true,
            SortKey::Metric(mi) => file
                .metrics
                .get(mi)
                .map(|m| m.higher_is_better)
                .unwrap_or(true),
        }
    }

    /// Jump directly to a sort key. If already on that key, toggle direction.
    pub fn quick_sort(&mut self, key: SortKey, file: &SourceFile) {
        if self.sort_key == key {
            self.sort_descending = !self.sort_descending;
            self.apply_sort(file);
        } else {
            self.sort_key = key;
            self.sort_descending = Self::default_descending(file, key);
            self.update_filtered(file);
        }
    }

    /// Build the dynamic sort-picker option list for the active file:
    /// `[ReleaseDate, Name, Metric(0..n) in file order]`.
    pub fn sort_options(file: &SourceFile) -> Vec<SortOption> {
        let mut opts = Vec::with_capacity(file.metrics.len() + 2);
        opts.push(SortOption {
            key: SortKey::ReleaseDate,
            label: "Release Date".to_string(),
        });
        opts.push(SortOption {
            key: SortKey::Name,
            label: "Name".to_string(),
        });
        for (i, metric) in file.metrics.iter().enumerate() {
            opts.push(SortOption {
                key: SortKey::Metric(i),
                label: metric.label.clone(),
            });
        }
        opts
    }

    /// Short label for the active sort key, shown in the list title.
    pub fn sort_label(file: Option<&SourceFile>, key: SortKey) -> String {
        match key {
            SortKey::Name => "Name".to_string(),
            SortKey::ReleaseDate => "Date".to_string(),
            SortKey::Metric(mi) => file
                .and_then(|f| f.metrics.get(mi))
                .map(|m| m.label.clone())
                .unwrap_or_else(|| "—".to_string()),
        }
    }

    /// The quick-sort metric key for `1` (first metric), if the source has one.
    pub fn quick_sort_metric_first(file: &SourceFile) -> Option<SortKey> {
        if file.metrics.is_empty() {
            None
        } else {
            Some(SortKey::Metric(0))
        }
    }

    /// The quick-sort key for `3` (first TokensPerSec metric), if any.
    pub fn quick_sort_speed(file: &SourceFile) -> Option<SortKey> {
        file.metrics
            .iter()
            .position(|m| m.kind == MetricKind::TokensPerSec)
            .map(SortKey::Metric)
    }

    pub fn current_model<'a>(&self, file: &'a SourceFile) -> Option<&'a ModelRow> {
        self.filtered_indices
            .get(self.selected)
            .and_then(|&i| file.models.get(i))
    }

    /// Format the value of `metric_idx` for `model`, or em-dash when missing.
    pub fn formatted_score(
        file: &SourceFile,
        model: &ModelRow,
        metric_idx: usize,
    ) -> Option<String> {
        let metric = file.metrics.get(metric_idx)?;
        let cell = model.scores.get(&metric.id)?;
        Some(format_metric_value(metric.kind, cell.value))
    }

    // --- List navigation ---

    pub fn reset_detail_scroll(&self) {
        self.detail_scroll.jump_top();
    }

    pub fn next(&mut self) {
        if self.selected < self.filtered_indices.len().saturating_sub(1) {
            self.selected += 1;
            self.list_state.select(Some(self.selected));
            self.reset_detail_scroll();
        }
    }

    pub fn prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.list_state.select(Some(self.selected));
            self.reset_detail_scroll();
        }
    }

    pub fn select_first(&mut self) {
        if self.selected > 0 {
            self.selected = 0;
            self.list_state.select(Some(self.selected));
            self.reset_detail_scroll();
        }
    }

    pub fn select_last(&mut self) {
        let last = self.filtered_indices.len().saturating_sub(1);
        if self.selected < last {
            self.selected = last;
            self.list_state.select(Some(self.selected));
            self.reset_detail_scroll();
        }
    }

    pub fn page_down(&mut self) {
        let last_index = self.filtered_indices.len().saturating_sub(1);
        self.selected = (self.selected + PAGE_SIZE).min(last_index);
        self.list_state.select(Some(self.selected));
        self.reset_detail_scroll();
    }

    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(PAGE_SIZE);
        self.list_state.select(Some(self.selected));
        self.reset_detail_scroll();
    }

    pub fn cycle_source_filter(&mut self, file: &SourceFile) {
        self.source_filter = self.source_filter.next();
        self.rebuild_after_filter_change(file);
    }

    pub fn cycle_reasoning_filter(&mut self, file: &SourceFile) {
        self.reasoning_filter = self.reasoning_filter.next();
        self.rebuild_after_filter_change(file);
    }

    pub fn toggle_region_grouping(&mut self, file: &SourceFile) {
        self.creator_grouping = if self.creator_grouping == CreatorGrouping::ByRegion {
            CreatorGrouping::None
        } else {
            CreatorGrouping::ByRegion
        };
        self.build_creator_list(file);
        self.selected_creator = 0;
        self.creator_list_state.select(Some(0));
    }

    pub fn toggle_type_grouping(&mut self, file: &SourceFile) {
        self.creator_grouping = if self.creator_grouping == CreatorGrouping::ByType {
            CreatorGrouping::None
        } else {
            CreatorGrouping::ByType
        };
        self.build_creator_list(file);
        self.selected_creator = 0;
        self.creator_list_state.select(Some(0));
    }

    // --- Creator sidebar navigation ---

    fn is_header(&self, idx: usize) -> bool {
        matches!(
            self.creator_list_items.get(idx),
            Some(CreatorListItem::GroupHeader(_))
        )
    }

    /// Move to the next selectable item, skipping headers.
    fn skip_to_selectable(&mut self, start: usize, forward: bool) {
        let max = self.creator_list_items.len().saturating_sub(1);
        let mut idx = start;
        while self.is_header(idx) {
            if forward {
                if idx >= max {
                    return; // can't go further
                }
                idx += 1;
            } else {
                if idx == 0 {
                    return;
                }
                idx -= 1;
            }
        }
        self.selected_creator = idx;
        self.creator_list_state.select(Some(idx));
    }

    pub fn next_creator(&mut self) {
        let max = self.creator_list_items.len().saturating_sub(1);
        if self.selected_creator < max {
            self.skip_to_selectable(self.selected_creator + 1, true);
        }
    }

    pub fn prev_creator(&mut self) {
        if self.selected_creator > 0 {
            self.skip_to_selectable(self.selected_creator - 1, false);
        }
    }

    pub fn select_first_creator(&mut self) {
        self.skip_to_selectable(0, true);
    }

    pub fn select_last_creator(&mut self) {
        let max = self.creator_list_items.len().saturating_sub(1);
        self.skip_to_selectable(max, false);
    }

    pub fn page_down_creator(&mut self) {
        let max = self.creator_list_items.len().saturating_sub(1);
        let target = (self.selected_creator + PAGE_SIZE).min(max);
        self.skip_to_selectable(target, true);
    }

    pub fn page_up_creator(&mut self) {
        let target = self.selected_creator.saturating_sub(PAGE_SIZE);
        self.skip_to_selectable(target, true);
    }

    pub fn cycle_bottom_view(&mut self) {
        self.bottom_view = match self.bottom_view {
            BottomView::H2H => BottomView::Scatter,
            BottomView::Scatter => BottomView::Radar,
            BottomView::Radar => BottomView::H2H,
            BottomView::Detail => BottomView::H2H,
        };
    }

    /// Cycle the scatter X-axis metric (wraps over the active file's metrics).
    pub fn cycle_scatter_x(&mut self, file: &SourceFile) {
        let n = file.metrics.len();
        if n > 0 {
            self.scatter_x = (self.scatter_x + 1) % n;
        }
    }

    /// Cycle the scatter Y-axis metric (wraps over the active file's metrics).
    pub fn cycle_scatter_y(&mut self, file: &SourceFile) {
        let n = file.metrics.len();
        if n > 0 {
            self.scatter_y = (self.scatter_y + 1) % n;
        }
    }

    /// Cycle the radar preset group (wraps over the active file's radar groups).
    pub fn cycle_radar_group(&mut self, file: &SourceFile) {
        let n = radar_groups(file).len();
        if n > 0 {
            self.radar_group = (self.radar_group + 1) % n;
        }
    }

    /// Auto-transition bottom view based on selection count.
    pub fn update_bottom_view(&mut self, selection_count: usize) {
        if selection_count >= 2 && self.bottom_view == BottomView::Detail {
            self.bottom_view = BottomView::H2H;
            self.h2h_scroll.jump_top();
        } else if selection_count < 2 && self.bottom_view != BottomView::Detail {
            self.bottom_view = BottomView::Detail;
            self.show_detail_overlay = false;
            self.h2h_scroll.jump_top();
        }
    }

    // --- Focus ---

    pub fn focus_right(&mut self, has_compare: bool) {
        self.focus = if has_compare {
            let left = if self.show_creators_in_compare {
                BenchmarkFocus::Creators
            } else {
                BenchmarkFocus::List
            };
            match self.focus {
                BenchmarkFocus::Compare => left,
                _ => BenchmarkFocus::Compare,
            }
        } else {
            match self.focus {
                BenchmarkFocus::Creators => BenchmarkFocus::List,
                BenchmarkFocus::List => BenchmarkFocus::Details,
                BenchmarkFocus::Details => BenchmarkFocus::Creators,
                BenchmarkFocus::Compare => BenchmarkFocus::Creators,
            }
        };
    }

    pub fn focus_left(&mut self, has_compare: bool) {
        self.focus = if has_compare {
            let left = if self.show_creators_in_compare {
                BenchmarkFocus::Creators
            } else {
                BenchmarkFocus::List
            };
            match self.focus {
                BenchmarkFocus::Compare => left,
                _ => BenchmarkFocus::Compare,
            }
        } else {
            match self.focus {
                BenchmarkFocus::Creators => BenchmarkFocus::Details,
                BenchmarkFocus::List => BenchmarkFocus::Creators,
                BenchmarkFocus::Details => BenchmarkFocus::List,
                BenchmarkFocus::Compare => BenchmarkFocus::List,
            }
        };
    }

    // --- H2H Scroll ---

    pub fn scroll_h2h_down(&mut self) {
        self.h2h_scroll.increment(1);
    }

    pub fn scroll_h2h_up(&mut self) {
        self.h2h_scroll.decrement(1);
    }

    pub fn scroll_h2h_top(&mut self) {
        self.h2h_scroll.jump_top();
    }

    pub fn scroll_h2h_page_down(&mut self, page: usize) {
        self.h2h_scroll.increment(page as u16);
    }

    pub fn scroll_h2h_page_up(&mut self, page: usize) {
        self.h2h_scroll.decrement(page as u16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::schema::{MetricDef, ReasoningStatus, ScoreCell, SourceMeta};
    use std::collections::BTreeMap;

    fn meta() -> SourceMeta {
        SourceMeta {
            id: "test".into(),
            name: "Test".into(),
            url: "https://example.com".into(),
            fetched_at: "2026-06-10T00:00:00+00:00".into(),
            verified: true,
        }
    }

    fn metric(id: &str, kind: MetricKind, group: &str, hib: bool) -> MetricDef {
        MetricDef {
            id: id.into(),
            label: id.to_uppercase(),
            kind,
            group: group.into(),
            higher_is_better: hib,
            last_updated: None,
        }
    }

    fn model(
        id: &str,
        creator: &str,
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
                },
            );
        }
        ModelRow {
            id: id.into(),
            name: id.into(),
            display_name: id.into(),
            creator: creator.into(),
            creator_name: creator.to_uppercase(),
            release_date: release.map(str::to_string),
            reasoning_status: reasoning,
            effort_level: None,
            variant_tag: None,
            open_weights: None,
            context_window: None,
            scores: score_map,
        }
    }

    /// AA-shaped file with index + percentage + speed + price metrics, 3 models.
    fn sample_file() -> SourceFile {
        SourceFile {
            source: meta(),
            metrics: vec![
                metric("intelligence_index", MetricKind::Index, "Indexes", true),
                metric("coding_index", MetricKind::Index, "Indexes", true),
                metric("math_index", MetricKind::Index, "Indexes", true),
                metric("gpqa", MetricKind::Percentage, "Academic", true),
                metric("mmlu_pro", MetricKind::Percentage, "Academic", true),
                metric("hle", MetricKind::Percentage, "Academic", true),
                metric("output_tps", MetricKind::TokensPerSec, "Performance", true),
                metric("price_input", MetricKind::UsdPerMTok, "Pricing", false),
            ],
            models: vec![
                model(
                    "alpha-1",
                    "openai",
                    ReasoningStatus::Reasoning,
                    Some("2026-01-01"),
                    &[
                        ("intelligence_index", 70.0),
                        ("output_tps", 120.0),
                        ("price_input", 2.0),
                    ],
                ),
                model(
                    "beta-1",
                    "meta",
                    ReasoningStatus::NonReasoning,
                    Some("2026-02-01"),
                    &[("intelligence_index", 60.0), ("price_input", 1.0)],
                ),
                model(
                    "gamma-1",
                    "anthropic",
                    ReasoningStatus::None,
                    None,
                    &[("coding_index", 80.0)],
                ),
            ],
        }
    }

    fn store_with(file: SourceFile) -> MultiStore {
        // Load the same sample file into every registered source slot so that
        // cycling/switching lands on a slot that always has a file regardless of
        // how many sources the registry compiles in.
        let mut store = MultiStore::new();
        for idx in 0..store.sources.len() {
            store.set_loaded(idx, file.clone());
        }
        store
    }

    #[test]
    fn new_with_no_file_is_empty() {
        let app = BenchmarksApp::new(None);
        assert!(app.filtered_indices.is_empty());
        assert_eq!(app.active_source, 0);
        assert_eq!(app.sort_key, SortKey::ReleaseDate);
        assert!(app.loading);
    }

    #[test]
    fn new_builds_filtered_and_creators() {
        let file = sample_file();
        let app = BenchmarksApp::new(Some(&file));
        // Default sort = ReleaseDate, which drops dateless models (gamma-1).
        // alpha-1 + beta-1 remain.
        assert_eq!(app.filtered_indices.len(), 2);
        // Creator sidebar is built over ALL models (no sort null-filter):
        // "All" + 3 creators.
        assert_eq!(app.creator_list_items.len(), 4);
    }

    #[test]
    fn default_sort_release_date_descending() {
        let file = sample_file();
        let app = BenchmarksApp::new(Some(&file));
        assert_eq!(app.sort_key, SortKey::ReleaseDate);
        // ReleaseDate drops gamma-1 (no date). Descending: beta-1 (2026-02-01)
        // before alpha-1 (2026-01-01).
        let first = app.current_model(&file).unwrap();
        assert_eq!(first.id, "beta-1");
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    fn metric_sort_drops_missing() {
        let file = sample_file();
        let mut app = BenchmarksApp::new(Some(&file));
        // Sort by Metric(0) = intelligence_index. gamma-1 lacks it -> dropped.
        app.quick_sort(SortKey::Metric(0), &file);
        assert_eq!(app.filtered_indices.len(), 2);
        // Descending (higher_is_better) -> alpha-1 (70) before beta-1 (60).
        let first = app.current_model(&file).unwrap();
        assert_eq!(first.id, "alpha-1");
    }

    #[test]
    fn quick_sort_toggles_direction_when_same_key() {
        let file = sample_file();
        let mut app = BenchmarksApp::new(Some(&file));
        app.quick_sort(SortKey::Metric(0), &file);
        assert!(app.sort_descending);
        app.quick_sort(SortKey::Metric(0), &file);
        assert!(!app.sort_descending);
    }

    #[test]
    fn quick_sort_speed_finds_tokens_per_sec() {
        let file = sample_file();
        // output_tps is metric index 6.
        assert_eq!(
            BenchmarksApp::quick_sort_speed(&file),
            Some(SortKey::Metric(6))
        );
    }

    #[test]
    fn quick_sort_speed_none_without_tokens_metric() {
        let mut file = sample_file();
        file.metrics.retain(|m| m.kind != MetricKind::TokensPerSec);
        assert_eq!(BenchmarksApp::quick_sort_speed(&file), None);
    }

    #[test]
    fn quick_sort_metric_first_present() {
        let file = sample_file();
        assert_eq!(
            BenchmarksApp::quick_sort_metric_first(&file),
            Some(SortKey::Metric(0))
        );
        let empty = SourceFile {
            source: meta(),
            metrics: vec![],
            models: vec![],
        };
        assert_eq!(BenchmarksApp::quick_sort_metric_first(&empty), None);
    }

    #[test]
    fn sort_options_order() {
        let file = sample_file();
        let opts = BenchmarksApp::sort_options(&file);
        // ReleaseDate, Name, then 8 metrics in order.
        assert_eq!(opts.len(), 2 + 8);
        assert_eq!(opts[0].key, SortKey::ReleaseDate);
        assert_eq!(opts[1].key, SortKey::Name);
        assert_eq!(opts[2].key, SortKey::Metric(0));
        assert_eq!(opts[2].label, "INTELLIGENCE_INDEX");
    }

    #[test]
    fn sort_label_uses_metric_label() {
        let file = sample_file();
        assert_eq!(
            BenchmarksApp::sort_label(Some(&file), SortKey::Metric(6)),
            "OUTPUT_TPS"
        );
        assert_eq!(
            BenchmarksApp::sort_label(Some(&file), SortKey::ReleaseDate),
            "Date"
        );
        assert_eq!(
            BenchmarksApp::sort_label(Some(&file), SortKey::Name),
            "Name"
        );
    }

    #[test]
    fn reasoning_filter_available_helper() {
        let file = sample_file();
        assert!(BenchmarksApp::reasoning_filter_available(Some(&file)));

        // A file where every model has reasoning_status None.
        let mut plain = sample_file();
        for m in &mut plain.models {
            m.reasoning_status = ReasoningStatus::None;
        }
        assert!(!BenchmarksApp::reasoning_filter_available(Some(&plain)));
        assert!(!BenchmarksApp::reasoning_filter_available(None));
    }

    #[test]
    fn reasoning_filter_filters_models() {
        let file = sample_file();
        let mut app = BenchmarksApp::new(Some(&file));
        app.cycle_reasoning_filter(&file); // -> Reasoning
        assert_eq!(app.reasoning_filter, ReasoningFilter::Reasoning);
        // Only alpha-1 is Reasoning.
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.current_model(&file).unwrap().id, "alpha-1");
    }

    #[test]
    fn search_filters_by_name_and_creator() {
        let file = sample_file();
        let mut app = BenchmarksApp::new(Some(&file));
        // Sort by Name so the date null-filter doesn't drop dateless gamma-1.
        app.quick_sort(SortKey::Name, &file);
        app.search_query = "anthropic".to_string();
        app.rebuild_after_filter_change(&file);
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.current_model(&file).unwrap().id, "gamma-1");
    }

    #[test]
    fn creator_filter_restricts_list() {
        let file = sample_file();
        let mut app = BenchmarksApp::new(Some(&file));
        // Find the "meta" creator position.
        let pos = app
            .creator_list_items
            .iter()
            .position(|i| matches!(i, CreatorListItem::Creator(s) if s == "meta"))
            .unwrap();
        app.selected_creator = pos;
        app.update_filtered(&file);
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.current_model(&file).unwrap().id, "beta-1");
    }

    #[test]
    fn cycle_scatter_wraps() {
        let file = sample_file(); // 8 metrics
        let mut app = BenchmarksApp::new(Some(&file));
        assert_eq!(app.scatter_x, 0);
        assert_eq!(app.scatter_y, 1);
        for _ in 0..8 {
            app.cycle_scatter_x(&file);
        }
        assert_eq!(app.scatter_x, 0); // wrapped
    }

    #[test]
    fn cycle_radar_group_wraps() {
        let file = sample_file();
        // radar_groups: Indexes (3 hib), Academic (3 hib). Performance/Pricing excluded.
        assert_eq!(radar_groups(&file).len(), 2);
        let mut app = BenchmarksApp::new(Some(&file));
        assert_eq!(app.radar_group, 0);
        app.cycle_radar_group(&file);
        assert_eq!(app.radar_group, 1);
        app.cycle_radar_group(&file);
        assert_eq!(app.radar_group, 0);
    }

    #[test]
    fn switch_source_resets_state() {
        let file = sample_file();
        let store = store_with(file.clone());
        let mut app = BenchmarksApp::new(Some(&file));

        // Dirty the view state.
        app.search_query = "anthropic".to_string();
        app.rebuild_after_filter_change(&file);
        app.sort_key = SortKey::Metric(3);
        app.sort_descending = false;
        app.source_filter = SourceFilter::Open;
        app.reasoning_filter = ReasoningFilter::Reasoning;
        app.scatter_x = 4;
        app.scatter_y = 5;
        app.radar_group = 1;
        app.selected = 0;
        app.show_detail_overlay = true;

        // Switching forward advances to the next registered source and resets
        // per-source view state. With the same sample file loaded into every
        // slot, the rebuild assertions below hold regardless of the landed index.
        app.switch_source(&store, true);
        assert_eq!(app.active_source, 1);
        assert!(app.search_query.is_empty());
        assert_eq!(app.source_filter, SourceFilter::All);
        assert_eq!(app.reasoning_filter, ReasoningFilter::All);
        assert_eq!(app.sort_key, SortKey::ReleaseDate);
        assert!(app.sort_descending);
        assert_eq!(app.scatter_x, 0);
        assert_eq!(app.scatter_y, 1);
        assert_eq!(app.radar_group, 0);
        assert!(!app.show_detail_overlay);
        assert_eq!(app.creator_grouping, CreatorGrouping::None);
        // Filters cleared; default ReleaseDate sort drops dateless gamma-1,
        // leaving alpha-1 + beta-1.
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    fn reset_for_source_loading_shows_empty() {
        // A second slot would be Loading; with one source we simulate via a fresh
        // store where index 0 is Loading (not loaded).
        let store = MultiStore::new(); // index 0 = Loading
        let mut app = BenchmarksApp::new(None);
        app.reset_for_source(&store);
        assert!(app.loading);
        assert!(app.filtered_indices.is_empty());
        assert_eq!(app.creator_list_items, vec![CreatorListItem::All]);
    }

    #[test]
    fn open_weights_filter_prefers_model_then_creator() {
        let mut file = sample_file();
        // Per-model override on alpha-1 that diverges from its creator map entry:
        // creator "openai" is closed, but this specific model is open. The filter
        // must follow the model-level value, not the creator aggregate.
        file.models[0].open_weights = Some(true);
        let mut app = BenchmarksApp::new(Some(&file));
        // Seed openness: openai closed, meta open, anthropic unknown.
        app.creator_openness =
            HashMap::from([("openai".to_string(), false), ("meta".to_string(), true)]);

        app.source_filter = SourceFilter::Open;
        app.update_filtered(&file);
        // alpha-1 (model-level open, overriding closed creator) + beta-1 (creator
        // open, no model-level value) both pass; gamma-1 (anthropic unknown) drops.
        // gamma-1 is also dateless, so the default ReleaseDate sort would drop it
        // regardless; the openness logic is what excludes it here.
        let open_ids: Vec<&str> = app
            .filtered_indices
            .iter()
            .map(|&i| file.models[i].id.as_str())
            .collect();
        assert_eq!(app.filtered_indices.len(), 2);
        assert!(open_ids.contains(&"alpha-1"));
        assert!(open_ids.contains(&"beta-1"));

        app.source_filter = SourceFilter::Closed;
        app.update_filtered(&file);
        // No model is closed: alpha-1 is model-level open, beta-1 is creator open,
        // gamma-1 is unknown.
        assert_eq!(app.filtered_indices.len(), 0);
    }

    #[test]
    fn formatted_score_and_missing() {
        let file = sample_file();
        let alpha = &file.models[0];
        // intelligence_index = 70.0 (Index) -> "70.0"
        assert_eq!(
            BenchmarksApp::formatted_score(&file, alpha, 0),
            Some("70.0".to_string())
        );
        // gpqa (idx 3) missing on alpha -> None
        assert_eq!(BenchmarksApp::formatted_score(&file, alpha, 3), None);
    }

    #[test]
    fn focus_right_browse_mode_cycles() {
        let mut app = BenchmarksApp::new(None);
        app.focus = BenchmarkFocus::Creators;
        app.focus_right(false);
        assert_eq!(app.focus, BenchmarkFocus::List);
        app.focus_right(false);
        assert_eq!(app.focus, BenchmarkFocus::Details);
        app.focus_right(false);
        assert_eq!(app.focus, BenchmarkFocus::Creators);
    }

    #[test]
    fn cycle_bottom_view_order() {
        let mut app = BenchmarksApp::new(None);
        app.bottom_view = BottomView::H2H;
        app.cycle_bottom_view();
        assert_eq!(app.bottom_view, BottomView::Scatter);
        app.cycle_bottom_view();
        assert_eq!(app.bottom_view, BottomView::Radar);
        app.cycle_bottom_view();
        assert_eq!(app.bottom_view, BottomView::H2H);
    }

    #[test]
    fn h2h_scroll_methods() {
        let mut app = BenchmarksApp::new(None);
        assert_eq!(app.h2h_scroll.get(), 0);
        app.scroll_h2h_down();
        assert_eq!(app.h2h_scroll.get(), 1);
        app.scroll_h2h_page_down(10);
        assert_eq!(app.h2h_scroll.get(), 11);
        app.scroll_h2h_page_up(100);
        assert_eq!(app.h2h_scroll.get(), 0);
    }
}
