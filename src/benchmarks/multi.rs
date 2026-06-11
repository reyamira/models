//! Multi-source benchmark store and registry-driven view helpers.
//!
//! `MultiStore` holds one `SourceState` per [`crate::benchmarks::sources::SOURCES`]
//! entry, tracking each source's progressive load state. The free functions in
//! this module are the registry-driven primitives the TUI views render against:
//! kind-based value formatting, group ordering, radar-eligible groups, the
//! per-source default sort, and the reasoning filter (ported to `ModelRow`).

use super::schema::{MetricKind, ModelRow, ReasoningStatus, SourceFile};
use super::sources::{SourceDescriptor, SOURCES};
use crate::formatting::parse_date;

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
        // Carry the unit: speed sits in mixed-direction groups (AA Performance:
        // speed ↑, latency ↓) whose section header shows no kind blurb, so the
        // bare number would be unitless everywhere it matters.
        MetricKind::TokensPerSec => format!("{value:.0} tok/s"),
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

// --- Comparator-column computations -----------------------------------------
//
// All operate over the source's FULL model list (not the filtered view), per
// metric, using only models that carry a value for that metric. They power the
// detail-panel comparator cell (field avg / peer avg / rank) and are pure so
// they can be unit-tested in isolation.

/// Half-window (in days) for the [`peer_avg`] release-date neighborhood: a peer
/// is any other model released within ±6 months of the selected model.
const PEER_WINDOW_DAYS: i64 = 183;

/// The id of the metric at `metric_idx`, if it exists.
fn metric_id_at(file: &SourceFile, metric_idx: usize) -> Option<&str> {
    file.metrics.get(metric_idx).map(|m| m.id.as_str())
}

/// Iterate the values of `metric_idx` across all models that have one.
fn metric_values<'a>(file: &'a SourceFile, metric_id: &'a str) -> impl Iterator<Item = f64> + 'a {
    file.models
        .iter()
        .filter_map(move |m| m.scores.get(metric_id).map(|c| c.value))
}

/// Arithmetic mean of `metric_idx` over every model with a value. `None` when
/// the metric index is stale or no model carries the metric.
pub fn field_avg(file: &SourceFile, metric_idx: usize) -> Option<f64> {
    let metric_id = metric_id_at(file, metric_idx)?;
    let mut sum = 0.0;
    let mut n = 0usize;
    for v in metric_values(file, metric_id) {
        sum += v;
        n += 1;
    }
    (n > 0).then(|| sum / n as f64)
}

/// Mean of `metric_idx` over models released within ±183 days of `model`'s
/// release date, **excluding `model` itself**. Returns `(mean, peer_count)`.
///
/// `None` when: the metric index is stale, `model` has no parseable release
/// date, or no peer (other dated model in-window) carries the metric.
pub fn peer_avg(file: &SourceFile, metric_idx: usize, model: &ModelRow) -> Option<(f64, usize)> {
    let metric_id = metric_id_at(file, metric_idx)?;
    let anchor = model.release_date.as_deref().and_then(parse_date)?;

    let mut sum = 0.0;
    let mut n = 0usize;
    for peer in &file.models {
        // Exclude the selected model itself (by identity).
        if std::ptr::eq(peer, model) {
            continue;
        }
        let Some(date) = peer.release_date.as_deref().and_then(parse_date) else {
            continue;
        };
        if (date - anchor).num_days().abs() > PEER_WINDOW_DAYS {
            continue;
        }
        if let Some(cell) = peer.scores.get(metric_id) {
            sum += cell.value;
            n += 1;
        }
    }
    (n > 0).then_some((sum / n as f64, n))
}

/// 1-based rank of `model`'s `metric_idx` value among all models that carry it,
/// direction-aware via `MetricDef.higher_is_better` (rank 1 = best). Returns
/// `(rank, total)`. `None` when the metric index is stale or `model` has no
/// value for it.
pub fn rank(file: &SourceFile, metric_idx: usize, model: &ModelRow) -> Option<(usize, usize)> {
    let metric = file.metrics.get(metric_idx)?;
    let metric_id = metric.id.as_str();
    let mine = model.scores.get(metric_id)?.value;

    let mut total = 0usize;
    let mut better = 0usize;
    for v in metric_values(file, metric_id) {
        total += 1;
        let is_better = if metric.higher_is_better {
            v > mine
        } else {
            v < mine
        };
        if is_better {
            better += 1;
        }
    }
    Some((better + 1, total))
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
            supports_tools: None,
            max_output: None,
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
        assert_eq!(
            format_metric_value(MetricKind::TokensPerSec, 128.6),
            "129 tok/s"
        );
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
    fn test_committed_epoch_json_deserializes_and_invariants_hold() {
        // Guards the committed file against future hand-edits / transform drift.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/v2/epoch.json");
        let raw =
            std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display()));
        let file: SourceFile = serde_json::from_str(&raw).expect("epoch.json deserializes");

        assert_eq!(file.source.id, "epoch");
        assert!(file.source.verified);
        // Metric set drifts with the 60-day prune window; assert a plausible band.
        assert!(
            (8..=30).contains(&file.metrics.len()),
            "epoch metric count {} outside expected band",
            file.metrics.len()
        );
        assert!(
            (100..=500).contains(&file.models.len()),
            "epoch model count {} outside expected band",
            file.models.len()
        );
        // Primary-org invariant: the transform takes only the first org of a
        // comma-joined `Organization`, so no creator_name may contain a comma
        // (guards against the composite multi-org slugs regressing).
        for m in &file.models {
            assert!(
                !m.creator_name.contains(','),
                "epoch creator_name {:?} is a composite multi-org string",
                m.creator_name
            );
        }
        // Epoch ships release dates -> default sort is ReleaseDate.
        assert_eq!(default_sort(&file), SortKey::ReleaseDate);
    }

    #[test]
    fn test_committed_arena_json_deserializes_and_invariants_hold() {
        // Guards the committed file against future hand-edits / transform drift.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/v2/arena.json");
        let raw =
            std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display()));
        let file: SourceFile = serde_json::from_str(&raw).expect("arena.json deserializes");

        assert_eq!(file.source.id, "arena");
        assert!(file.source.verified);
        // Up to 6 LLM boards (a missing board is tolerated), all Elo / one group.
        assert!(
            (3..=6).contains(&file.metrics.len()),
            "arena metric count {} outside expected band",
            file.metrics.len()
        );
        assert!(
            file.metrics
                .iter()
                .all(|m| m.kind == MetricKind::Elo && m.group == "Arena Elo"),
            "arena metrics must all be Elo in the 'Arena Elo' group"
        );
        // Votes are the whole point of this file's last regen: at least some
        // cells must carry a sample size (guards a transform that drops them).
        let votes_cells = file
            .models
            .iter()
            .flat_map(|m| m.scores.values())
            .filter(|c| c.votes.is_some())
            .count();
        assert!(votes_cells > 0, "arena cells carry no vote counts");
        // Arena has no release dates -> default sort is the first metric.
        assert_eq!(default_sort(&file), SortKey::Metric(0));
    }

    // --- Comparator computations ---

    /// File with two metrics (one higher-is-better, one lower-is-better) and a
    /// roster of dated models for the comparator tests.
    fn comparator_file() -> SourceFile {
        let mut file = SourceFile {
            source: meta(),
            metrics: vec![
                metric("score", MetricKind::Index, "G", true),
                metric("price", MetricKind::UsdPerMTok, "G", false),
            ],
            models: vec![],
        };
        let mk = |id: &str, date: &str, score: Option<f64>, price: Option<f64>| {
            let mut m = model(id, ReasoningStatus::None, Some(date));
            if let Some(s) = score {
                m.scores.insert(
                    "score".into(),
                    crate::benchmarks::schema::ScoreCell {
                        value: s,
                        date: None,
                        ci: None,
                        votes: None,
                    },
                );
            }
            if let Some(p) = price {
                m.scores.insert(
                    "price".into(),
                    crate::benchmarks::schema::ScoreCell {
                        value: p,
                        date: None,
                        ci: None,
                        votes: None,
                    },
                );
            }
            m
        };
        file.models = vec![
            mk("a", "2026-01-01", Some(60.0), Some(4.0)),
            mk("b", "2026-03-01", Some(70.0), Some(2.0)),
            mk("c", "2026-04-01", Some(80.0), Some(1.0)),
            // Far-future model, outside any ±183d window of a/b/c.
            mk("d", "2027-06-01", Some(100.0), Some(10.0)),
        ];
        file
    }

    #[test]
    fn test_field_avg() {
        let file = comparator_file();
        // mean of 60,70,80,100 = 77.5
        assert_eq!(field_avg(&file, 0), Some(77.5));
        // Stale metric index -> None.
        assert_eq!(field_avg(&file, 9), None);
    }

    #[test]
    fn test_field_avg_no_values_is_none() {
        let mut file = comparator_file();
        for m in &mut file.models {
            m.scores.remove("score");
        }
        assert_eq!(field_avg(&file, 0), None);
    }

    #[test]
    fn test_peer_avg_window_and_self_exclusion() {
        let file = comparator_file();
        // Anchor on "b" (2026-03-01). Peers within ±183d: a (2026-01-01, 59d),
        // c (2026-04-01, 31d). d is far out. Self (b) excluded.
        // mean of a.score(60) + c.score(80) = 70, peer count 2.
        let b = &file.models[1];
        assert_eq!(peer_avg(&file, 0, b), Some((70.0, 2)));
    }

    #[test]
    fn test_peer_avg_boundary_183_days_inclusive() {
        // A peer exactly 183 days away is included; 184 days is excluded.
        let mut file = comparator_file();
        file.models.clear();
        let mk = |id: &str, date: &str, score: f64| {
            let mut m = model(id, ReasoningStatus::None, Some(date));
            m.scores.insert(
                "score".into(),
                crate::benchmarks::schema::ScoreCell {
                    value: score,
                    date: None,
                    ci: None,
                    votes: None,
                },
            );
            m
        };
        file.models = vec![
            mk("anchor", "2026-01-01", 50.0),
            mk("at_183", "2026-07-03", 90.0), // exactly 183 days after Jan 1
            mk("at_184", "2026-07-04", 10.0), // 184 days -> excluded
        ];
        let anchor = &file.models[0];
        // Only at_183 is in-window -> mean 90, count 1.
        assert_eq!(peer_avg(&file, 0, anchor), Some((90.0, 1)));
    }

    #[test]
    fn test_peer_avg_dateless_anchor_is_none() {
        let mut file = comparator_file();
        file.models[1].release_date = None;
        let b = &file.models[1];
        assert_eq!(peer_avg(&file, 0, b), None);
    }

    #[test]
    fn test_peer_avg_empty_peer_set_is_none() {
        let file = comparator_file();
        // Anchor on "d" (2027-06-01): no other model is within ±183d.
        let d = &file.models[3];
        assert_eq!(peer_avg(&file, 0, d), None);
    }

    #[test]
    fn test_rank_higher_is_better() {
        let file = comparator_file();
        // score higher-is-better. Values: a60 b70 c80 d100. c=80 -> rank 2/4.
        let c = &file.models[2];
        assert_eq!(rank(&file, 0, c), Some((2, 4)));
        let d = &file.models[3];
        assert_eq!(rank(&file, 0, d), Some((1, 4)));
        let a = &file.models[0];
        assert_eq!(rank(&file, 0, a), Some((4, 4)));
    }

    #[test]
    fn test_rank_lower_is_better() {
        let file = comparator_file();
        // price lower-is-better. Values: a4 b2 c1 d10. c=1 is best -> rank 1/4.
        let c = &file.models[2];
        assert_eq!(rank(&file, 1, c), Some((1, 4)));
        let d = &file.models[3]; // price 10, worst -> rank 4/4
        assert_eq!(rank(&file, 1, d), Some((4, 4)));
    }

    #[test]
    fn test_rank_missing_value_is_none() {
        let mut file = comparator_file();
        file.models[0].scores.remove("score");
        let a = &file.models[0];
        assert_eq!(rank(&file, 0, a), None);
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
