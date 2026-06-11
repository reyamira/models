//! Multi-source benchmark store and registry-driven view helpers.
//!
//! `MultiStore` holds one `SourceState` per [`crate::benchmarks::sources::SOURCES`]
//! entry, tracking each source's progressive load state. The free functions in
//! this module are the registry-driven primitives the TUI views render against:
//! kind-based value formatting, group ordering, radar-eligible groups, the
//! per-source default sort, and the reasoning filter (ported to `ModelRow`).

use super::schema::{MetricKind, ModelRow, ReasoningStatus, SourceFile};
use super::sources::{SourceDescriptor, SOURCES};

/// Progressive load state of a single source.
pub enum SourceLoad {
    /// Fetch in flight (or queued).
    Loading,
    /// Successfully fetched and (for AA) trait-augmented.
    Loaded(SourceFile),
    /// Fetch or parse failed.
    Failed,
}

/// One source's descriptor paired with its current load state.
pub struct SourceState {
    pub descriptor: &'static SourceDescriptor,
    pub load: SourceLoad,
}

/// Holds the load state of every compiled-in data source.
pub struct MultiStore {
    pub sources: Vec<SourceState>,
}

impl Default for MultiStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiStore {
    /// Seed one `Loading` entry per [`SOURCES`] descriptor.
    pub fn new() -> Self {
        let sources = SOURCES
            .iter()
            .map(|descriptor| SourceState {
                descriptor,
                load: SourceLoad::Loading,
            })
            .collect();
        Self { sources }
    }

    /// Mark source `idx` as loaded with `file`. No-op if `idx` is out of range.
    pub fn set_loaded(&mut self, idx: usize, file: SourceFile) {
        if let Some(state) = self.sources.get_mut(idx) {
            state.load = SourceLoad::Loaded(file);
        }
    }

    /// Mark source `idx` as failed. No-op if `idx` is out of range.
    pub fn set_failed(&mut self, idx: usize) {
        if let Some(state) = self.sources.get_mut(idx) {
            state.load = SourceLoad::Failed;
        }
    }

    /// Borrow the loaded `SourceFile` for source `idx`, if any.
    pub fn file(&self, idx: usize) -> Option<&SourceFile> {
        match self.sources.get(idx) {
            Some(SourceState {
                load: SourceLoad::Loaded(file),
                ..
            }) => Some(file),
            _ => None,
        }
    }

    /// Mutably borrow the loaded `SourceFile` for source `idx`, if any.
    pub fn file_mut(&mut self, idx: usize) -> Option<&mut SourceFile> {
        match self.sources.get_mut(idx) {
            Some(SourceState {
                load: SourceLoad::Loaded(file),
                ..
            }) => Some(file),
            _ => None,
        }
    }
}

/// A sort key over the active source. `Metric(i)` indexes `file.metrics`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    ReleaseDate,
    Name,
    Metric(usize),
}

/// Reasoning filter operating on [`ModelRow`].
/// `All -> Reasoning -> NonReasoning` cycle; `Adaptive` counts as reasoning.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReasoningFilter {
    #[default]
    All,
    Reasoning,
    NonReasoning,
}

impl ReasoningFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Reasoning,
            Self::Reasoning => Self::NonReasoning,
            Self::NonReasoning => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Reasoning => "Reasoning",
            Self::NonReasoning => "Non-reasoning",
        }
    }

    pub fn matches(self, model: &ModelRow) -> bool {
        match self {
            Self::All => true,
            Self::Reasoning => matches!(
                model.reasoning_status,
                ReasoningStatus::Reasoning | ReasoningStatus::Adaptive
            ),
            Self::NonReasoning => {
                matches!(model.reasoning_status, ReasoningStatus::NonReasoning)
            }
        }
    }
}

/// Format a metric value for display, driven by its [`MetricKind`].
///
/// AA stores percentages as fractions (0.914), so `Percentage` multiplies by
/// 100 before formatting.
pub fn format_metric_value(kind: MetricKind, value: f64) -> String {
    match kind {
        MetricKind::Percentage => format!("{:.1}%", value * 100.0),
        MetricKind::Index => format!("{value:.1}"),
        MetricKind::Elo => format!("{value:.0}"),
        MetricKind::TokensPerSec => format!("{value:.0}"),
        MetricKind::Seconds => format!("{value:.2}s"),
        MetricKind::UsdPerMTok => format!("${value:.2}"),
    }
}

/// Group names in first-appearance order over `file.metrics`.
pub fn groups_in_order(file: &SourceFile) -> Vec<&str> {
    let mut seen = Vec::new();
    for metric in &file.metrics {
        if !seen.contains(&metric.group.as_str()) {
            seen.push(metric.group.as_str());
        }
    }
    seen
}

/// Indices into `file.metrics` for every metric in `group` (display order).
pub fn metric_indices_in_group(file: &SourceFile, group: &str) -> Vec<usize> {
    file.metrics
        .iter()
        .enumerate()
        .filter(|(_, m)| m.group == group)
        .map(|(i, _)| i)
        .collect()
}

/// Groups eligible to be radar presets: those with at least 3
/// `higher_is_better` metrics. This keeps Performance/Pricing off the radar
/// (matching the legacy behavior where perf/price were never radar axes),
/// since those groups have fewer than 3 higher-is-better metrics.
pub fn radar_groups(file: &SourceFile) -> Vec<String> {
    groups_in_order(file)
        .into_iter()
        .filter(|group| {
            file.metrics
                .iter()
                .filter(|m| m.group == *group && m.higher_is_better)
                .count()
                >= 3
        })
        .map(str::to_string)
        .collect()
}

/// Default sort for a source: `ReleaseDate` if any model carries one, else
/// `Metric(0)` (first metric, descending at the call site).
pub fn default_sort(file: &SourceFile) -> SortKey {
    if file.models.iter().any(|m| m.release_date.is_some()) {
        SortKey::ReleaseDate
    } else {
        SortKey::Metric(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::schema::{MetricDef, SourceMeta};
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
            label: id.into(),
            kind,
            group: group.into(),
            higher_is_better: hib,
            last_updated: None,
            description: None,
        }
    }

    fn model(id: &str, reasoning: ReasoningStatus, release: Option<&str>) -> ModelRow {
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
            scores: BTreeMap::new(),
        }
    }

    /// AA-shaped metric set: Indexes (3 index), Agentic (3 pct),
    /// Academic (3 pct), Performance (1 tps + 2 seconds, hib=false),
    /// Pricing (3 usd, hib=false).
    fn aa_shaped_file() -> SourceFile {
        SourceFile {
            source: meta(),
            metrics: vec![
                metric("intelligence_index", MetricKind::Index, "Indexes", true),
                metric("coding_index", MetricKind::Index, "Indexes", true),
                metric("math_index", MetricKind::Index, "Indexes", true),
                metric("livecodebench", MetricKind::Percentage, "Agentic", true),
                metric("scicode", MetricKind::Percentage, "Agentic", true),
                metric(
                    "terminalbench_hard",
                    MetricKind::Percentage,
                    "Agentic",
                    true,
                ),
                metric("gpqa", MetricKind::Percentage, "Academic", true),
                metric("mmlu_pro", MetricKind::Percentage, "Academic", true),
                metric("hle", MetricKind::Percentage, "Academic", true),
                metric("output_tps", MetricKind::TokensPerSec, "Performance", true),
                metric("ttft", MetricKind::Seconds, "Performance", false),
                metric("ttfat", MetricKind::Seconds, "Performance", false),
                metric("price_input", MetricKind::UsdPerMTok, "Pricing", false),
                metric("price_output", MetricKind::UsdPerMTok, "Pricing", false),
                metric("price_blended", MetricKind::UsdPerMTok, "Pricing", false),
            ],
            models: vec![],
        }
    }

    #[test]
    fn test_format_metric_value_all_kinds() {
        // AA stores percentages as fractions; 0.914 -> "91.4%".
        assert_eq!(format_metric_value(MetricKind::Percentage, 0.914), "91.4%");
        assert_eq!(format_metric_value(MetricKind::Index, 73.25), "73.2");
        assert_eq!(format_metric_value(MetricKind::Elo, 1432.7), "1433");
        assert_eq!(format_metric_value(MetricKind::TokensPerSec, 128.6), "129");
        assert_eq!(format_metric_value(MetricKind::Seconds, 0.456), "0.46s");
        assert_eq!(format_metric_value(MetricKind::UsdPerMTok, 2.5), "$2.50");
    }

    #[test]
    fn test_groups_in_order() {
        let file = aa_shaped_file();
        assert_eq!(
            groups_in_order(&file),
            vec!["Indexes", "Agentic", "Academic", "Performance", "Pricing"]
        );
    }

    #[test]
    fn test_metric_indices_in_group() {
        let file = aa_shaped_file();
        assert_eq!(metric_indices_in_group(&file, "Indexes"), vec![0, 1, 2]);
        assert_eq!(metric_indices_in_group(&file, "Agentic"), vec![3, 4, 5]);
        assert_eq!(metric_indices_in_group(&file, "Pricing"), vec![12, 13, 14]);
        assert!(metric_indices_in_group(&file, "Nonexistent").is_empty());
    }

    #[test]
    fn test_radar_groups_excludes_performance_and_pricing() {
        let file = aa_shaped_file();
        let groups = radar_groups(&file);
        assert_eq!(groups, vec!["Indexes", "Agentic", "Academic"]);
        assert!(
            !groups.contains(&"Performance".to_string()),
            "Performance has only 1 higher_is_better metric"
        );
        assert!(
            !groups.contains(&"Pricing".to_string()),
            "Pricing has 0 higher_is_better metrics"
        );
    }

    #[test]
    fn test_default_sort_with_release_dates() {
        let mut file = aa_shaped_file();
        file.models = vec![
            model("a", ReasoningStatus::None, Some("2026-01-01")),
            model("b", ReasoningStatus::None, None),
        ];
        assert_eq!(default_sort(&file), SortKey::ReleaseDate);
    }

    #[test]
    fn test_default_sort_without_release_dates() {
        let mut file = aa_shaped_file();
        file.models = vec![
            model("a", ReasoningStatus::None, None),
            model("b", ReasoningStatus::None, None),
        ];
        assert_eq!(default_sort(&file), SortKey::Metric(0));
    }

    #[test]
    fn test_default_sort_empty_models() {
        let file = aa_shaped_file();
        assert_eq!(default_sort(&file), SortKey::Metric(0));
    }

    #[test]
    fn test_reasoning_filter_cycle() {
        let f = ReasoningFilter::All;
        let f = f.next();
        assert_eq!(f, ReasoningFilter::Reasoning);
        let f = f.next();
        assert_eq!(f, ReasoningFilter::NonReasoning);
        let f = f.next();
        assert_eq!(f, ReasoningFilter::All);
    }

    #[test]
    fn test_reasoning_filter_matches() {
        let reasoning = model("r", ReasoningStatus::Reasoning, None);
        let adaptive = model("a", ReasoningStatus::Adaptive, None);
        let non_reasoning = model("n", ReasoningStatus::NonReasoning, None);
        let plain = model("p", ReasoningStatus::None, None);

        assert!(ReasoningFilter::All.matches(&reasoning));
        assert!(ReasoningFilter::All.matches(&plain));

        assert!(ReasoningFilter::Reasoning.matches(&reasoning));
        assert!(ReasoningFilter::Reasoning.matches(&adaptive));
        assert!(!ReasoningFilter::Reasoning.matches(&non_reasoning));
        assert!(!ReasoningFilter::Reasoning.matches(&plain));

        assert!(ReasoningFilter::NonReasoning.matches(&non_reasoning));
        assert!(!ReasoningFilter::NonReasoning.matches(&reasoning));
        assert!(!ReasoningFilter::NonReasoning.matches(&adaptive));
        assert!(!ReasoningFilter::NonReasoning.matches(&plain));
    }

    #[test]
    fn test_multistore_seeds_loading() {
        let store = MultiStore::new();
        assert_eq!(store.sources.len(), SOURCES.len());
        assert!(matches!(store.sources[0].load, SourceLoad::Loading));
        assert!(store.file(0).is_none());
    }

    #[test]
    fn test_multistore_transitions() {
        let mut store = MultiStore::new();
        let file = SourceFile {
            source: meta(),
            metrics: vec![metric("m", MetricKind::Index, "G", true)],
            models: vec![model("x", ReasoningStatus::None, None)],
        };
        store.set_loaded(0, file);
        assert!(store.file(0).is_some());
        assert_eq!(store.file(0).unwrap().models.len(), 1);

        // file_mut allows in-place augmentation.
        store.file_mut(0).unwrap().models[0].open_weights = Some(true);
        assert_eq!(store.file(0).unwrap().models[0].open_weights, Some(true));

        store.set_failed(0);
        assert!(store.file(0).is_none());
        assert!(matches!(store.sources[0].load, SourceLoad::Failed));
    }

    #[test]
    fn test_committed_aa_json_deserializes_and_helpers_match() {
        // Guards the schema <-> helper contract against the real committed file.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/v2/aa.json");
        let raw =
            std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display()));
        let file: SourceFile = serde_json::from_str(&raw).expect("aa.json deserializes");

        assert_eq!(file.source.id, "aa");
        assert!(file.source.verified);
        assert_eq!(file.metrics.len(), 21);
        // Sanity range, not an exact pin: the AA model roster drifts on every
        // refresh, so assert a plausible band instead of a brittle count.
        assert!(
            (450..=650).contains(&file.models.len()),
            "aa model count {} outside expected band",
            file.models.len()
        );

        // First-appearance group order on real data.
        assert_eq!(
            groups_in_order(&file),
            vec!["Indexes", "Agentic", "Academic", "Performance", "Pricing"]
        );
        // Radar excludes Performance (1 higher-is-better metric) and Pricing (0).
        assert_eq!(radar_groups(&file), vec!["Indexes", "Agentic", "Academic"]);
        // AA has release dates -> default sort is ReleaseDate.
        assert_eq!(default_sort(&file), SortKey::ReleaseDate);
    }

    #[test]
    fn test_multistore_out_of_range_noop() {
        let mut store = MultiStore::new();
        let file = SourceFile {
            source: meta(),
            metrics: vec![],
            models: vec![],
        };
        // Should not panic.
        store.set_loaded(99, file);
        store.set_failed(99);
        assert!(store.file(99).is_none());
        assert!(store.file_mut(99).is_none());
    }
}
