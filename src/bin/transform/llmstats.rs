//! `transform llmstats` — LLM Stats API responses -> v2 `SourceFile`.
//!
//! Mirrors `aa.rs` in structure, error handling, deterministic ordering, and
//! the if-changed write semantics. Differences are all driven by the *real*
//! LLM Stats API shape (probed at implementation, 2026-06-10):
//!
//! - The plan's "~11 category scores" are NOT in `/v1/scores` (which is 14.6k
//!   raw per-benchmark rows tagged with category labels). They are the
//!   per-category **TrueSkill `conservative_rating`** values returned by
//!   `/v1/rankings?category=<cat>` (`method: "trueskill"`). That is the curated,
//!   headline composite signal — one rating per (model, category). We ingest the
//!   11 plan-named categories as the curated metric set.
//! - `/v1/rankings` is hard-capped at `limit=50` (requesting more silently
//!   returns an empty list — an upstream quirk; see the workflow fetch note).
//! - Rankings carry `organization` as a bare slug string and `open_weight`, but
//!   NOT `release_date` / `context_window`. Those come from the OPTIONAL second
//!   input (`/v1/models`, cursor-paginated). The join key is rankings
//!   `model_id` == models `id`.
//!
//! `source.verified = true` — LLM Stats aggregates third-party benchmark
//! results, and its published methodology excludes provider self-reported
//! numbers from the rankings ingested here, so no "self-reported" badge is
//! shown (plan amendment 2026-06-11; was previously `false`).
//!
//! Input contract (assembled by the workflow's bounded fetch loop):
//! - `rankings`: a single JSON object `{ "rankings": [ <RankingsResponse>, ... ] }`
//!   — one `RankingsResponse` per category the workflow fetched.
//! - `models` (optional): the `/v1/models` list response `{ "models": [...] }`
//!   used purely to enrich rows with `release_date` / `context_window` /
//!   `creator_name`. A model present in the rankings but absent from this file
//!   still appears (metadata fields left `None`, `creator_name` falls back to
//!   the rankings org slug).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::schema::{
    MetricDef, MetricKind, ModelRow, ReasoningStatus, ScoreCell, SourceFile, SourceMeta,
};

// ---------------------------------------------------------------------------
// Raw API shapes (typed serde; #[serde(default)] over Value spelunking).
// ---------------------------------------------------------------------------

/// Wrapper around the collected `/v1/rankings` responses (one per category).
#[derive(Debug, Deserialize)]
struct RawRankingsFile {
    #[serde(default)]
    rankings: Vec<RawRanking>,
}

/// One `RankingsResponse` (`/v1/rankings?category=<id>`).
#[derive(Debug, Deserialize)]
struct RawRanking {
    #[serde(default)]
    category: String,
    /// Per-category timestamp of the ranking computation.
    #[serde(default)]
    ranked_at: Option<String>,
    #[serde(default)]
    models: Vec<RawRankedModel>,
}

#[derive(Debug, Deserialize)]
struct RawRankedModel {
    #[serde(default)]
    model_id: String,
    #[serde(default)]
    model_name: String,
    /// Organization slug (bare string in the rankings payload).
    #[serde(default)]
    organization: String,
    /// TrueSkill conservative rating — the curated metric value.
    #[serde(default)]
    conservative_rating: Option<f64>,
    #[serde(default)]
    open_weight: Option<bool>,
}

/// `/v1/models` list response (optional metadata-join input).
#[derive(Debug, Default, Deserialize)]
struct RawModelsFile {
    #[serde(default)]
    models: Vec<RawModelMeta>,
}

#[derive(Debug, Deserialize)]
struct RawModelMeta {
    #[serde(default)]
    id: String,
    #[serde(default)]
    organization: Option<RawOrg>,
    #[serde(default)]
    open_weight: Option<bool>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    release_date: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOrg {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

// ---------------------------------------------------------------------------
// Curated metric registry (binding) — the 11 plan-named category TrueSkill
// ratings. `group` PARTITIONS the metrics (each metric in exactly one group).
// Split 6 / 5 so each group is a valid radar preset: >= 3 higher_is_better
// metrics (renders) and <= 6 metrics (no axes dropped).
//
// The metric `id` is the LLM Stats category id (the `/v1/rankings?category=`
// value); the score map keys by that id.
// ---------------------------------------------------------------------------

/// One entry: (id == category id, label, kind, group, higher_is_better, description).
type MetricEntry = (
    &'static str,
    &'static str,
    MetricKind,
    &'static str,
    bool,
    &'static str,
);

/// Common explanation shared by every LLM Stats category rating. LLM Stats runs
/// the TrueSkill ranking system independently per category over the benchmarks
/// tagged with it, reporting the conservative estimate `μ − 3σ`; higher is
/// better. Each description below appends the domain-specific clause.
const RATING_SUFFIX: &str =
    "Score is an LLM Stats TrueSkill rating (a conservative skill estimate from the benchmarks \
     tagged with this category); higher is better.";

const METRICS: &[MetricEntry] = &[
    // Group "Categories I" (6)
    (
        "agents",
        "Agents",
        MetricKind::Index,
        "Categories I",
        true,
        "Capability on agentic, multi-step tool-using tasks.",
    ),
    (
        "code",
        "Code",
        MetricKind::Index,
        "Categories I",
        true,
        "Capability on programming and code-generation tasks.",
    ),
    (
        "finance",
        "Finance",
        MetricKind::Index,
        "Categories I",
        true,
        "Capability on finance-domain tasks.",
    ),
    (
        "frontend_development",
        "Frontend Dev",
        MetricKind::Index,
        "Categories I",
        true,
        "Capability on front-end / web-UI development tasks.",
    ),
    (
        "general",
        "General",
        MetricKind::Index,
        "Categories I",
        true,
        "Overall general-capability rating across all tracked benchmarks.",
    ),
    (
        "healthcare",
        "Healthcare",
        MetricKind::Index,
        "Categories I",
        true,
        "Capability on healthcare and medical-domain tasks.",
    ),
    // Group "Categories II" (5)
    (
        "legal",
        "Legal",
        MetricKind::Index,
        "Categories II",
        true,
        "Capability on legal-domain tasks.",
    ),
    (
        "math",
        "Math",
        MetricKind::Index,
        "Categories II",
        true,
        "Capability on mathematical problem-solving tasks.",
    ),
    (
        "multimodal",
        "Multimodal",
        MetricKind::Index,
        "Categories II",
        true,
        "Capability on multimodal tasks spanning more than one input modality.",
    ),
    (
        "reasoning",
        "Reasoning",
        MetricKind::Index,
        "Categories II",
        true,
        "Capability on logical and multi-step reasoning tasks.",
    ),
    (
        "vision",
        "Vision",
        MetricKind::Index,
        "Categories II",
        true,
        "Capability on visual understanding tasks.",
    ),
];

/// Membership test: only curated category ids are ingested. Non-curated
/// categories present in the input (e.g. "safety", "tool_calling") are dropped.
fn is_curated(category: &str) -> bool {
    METRICS.iter().any(|&(id, ..)| id == category)
}

/// Build the metric defs, carrying per-metric `last_updated` = the newest
/// `ranked_at` seen for that category across the input.
fn metric_defs(last_updated: &BTreeMap<String, String>) -> Vec<MetricDef> {
    METRICS
        .iter()
        .map(|&(id, label, kind, group, hib, description)| MetricDef {
            id: id.to_string(),
            label: label.to_string(),
            kind,
            group: group.to_string(),
            higher_is_better: hib,
            last_updated: last_updated.get(id).cloned(),
            description: Some(format!("{description} {RATING_SUFFIX}")),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Accumulator: one entry per model, scores filled from each curated ranking.
// First-seen order across rankings is preserved (deterministic).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ModelAcc {
    /// Insertion order index (first appearance across the rankings).
    order: usize,
    name: String,
    organization: String,
    open_weight: Option<bool>,
    scores: BTreeMap<String, ScoreCell>,
}

fn build_source_file(rankings: RawRankingsFile, models: Option<RawModelsFile>) -> SourceFile {
    // Metadata-join lookup: model id -> (creator_name, release_date,
    // context_window, open_weight).
    let meta: BTreeMap<String, RawModelMeta> = models
        .map(|m| m.models.into_iter().map(|r| (r.id.clone(), r)).collect())
        .unwrap_or_default();

    // Newest ranked_at per curated category -> MetricDef.last_updated.
    let mut last_updated: BTreeMap<String, String> = BTreeMap::new();

    // Accumulate models in first-seen order across curated rankings.
    let mut acc: BTreeMap<String, ModelAcc> = BTreeMap::new();
    let mut next_order = 0usize;

    for ranking in rankings.rankings {
        if !is_curated(&ranking.category) {
            continue;
        }
        if let Some(ts) = &ranking.ranked_at {
            // Keep the lexicographically-greatest (newest) RFC3339 timestamp.
            last_updated
                .entry(ranking.category.clone())
                .and_modify(|cur| {
                    if ts > cur {
                        *cur = ts.clone();
                    }
                })
                .or_insert_with(|| ts.clone());
        }

        for rm in ranking.models {
            let Some(value) = rm.conservative_rating else {
                continue;
            };
            if rm.model_id.is_empty() {
                continue;
            }
            let entry = acc.entry(rm.model_id.clone()).or_insert_with(|| {
                let order = next_order;
                next_order += 1;
                ModelAcc {
                    order,
                    name: rm.model_name.clone(),
                    organization: rm.organization.clone(),
                    open_weight: rm.open_weight,
                    scores: BTreeMap::new(),
                }
            });
            // Fill missing fields from later rankings if the first was sparse.
            if entry.name.is_empty() {
                entry.name = rm.model_name.clone();
            }
            if entry.organization.is_empty() {
                entry.organization = rm.organization.clone();
            }
            if entry.open_weight.is_none() {
                entry.open_weight = rm.open_weight;
            }
            entry.scores.insert(
                ranking.category.clone(),
                ScoreCell {
                    value,
                    date: ranking.ranked_at.clone(),
                    ci: None,
                    votes: None,
                },
            );
        }
    }

    // Emit models in first-seen order; drop any that ended up with no curated
    // scores (defensive — shouldn't happen given the insert guard).
    let mut accs: Vec<(String, ModelAcc)> = acc.into_iter().collect();
    accs.sort_by_key(|(_, a)| a.order);

    let models: Vec<ModelRow> = accs
        .into_iter()
        .filter(|(_, a)| !a.scores.is_empty())
        .map(|(id, a)| {
            let parsed = crate::schema::parse_name_metadata(&a.name, ReasoningStatus::None);
            let m = meta.get(&id);

            // creator slug: prefer the join's org id, else the rankings slug.
            let creator = m
                .and_then(|r| r.organization.as_ref())
                .and_then(|o| o.id.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| a.organization.clone());
            // creator_name: join's org name, else the rankings slug as fallback.
            let creator_name = m
                .and_then(|r| r.organization.as_ref())
                .and_then(|o| o.name.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| a.organization.clone());

            let release_date = m.and_then(|r| r.release_date.clone());
            let context_window = m.and_then(|r| r.context_window);
            // open_weights: prefer metadata join, else the rankings flag.
            let open_weights = m.and_then(|r| r.open_weight).or(a.open_weight);

            ModelRow {
                id,
                name: a.name,
                display_name: parsed.display_name,
                creator,
                creator_name,
                release_date,
                reasoning_status: parsed.reasoning_status,
                effort_level: parsed.effort_level,
                variant_tag: parsed.variant_tag,
                open_weights,
                context_window,
                supports_tools: None,
                max_output: None,
                scores: a.scores,
            }
        })
        .collect();

    SourceFile {
        source: SourceMeta {
            id: "llmstats".to_string(),
            name: "LLM Stats".to_string(),
            url: "https://llm-stats.com".to_string(),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            // LLM Stats aggregates third-party benchmark results; its published
            // methodology excludes provider self-reported numbers from the
            // rankings we ingest, so it is marked verified like the other
            // sources (no "self-reported" badge). See the multi-source plan
            // amendment (2026-06-11).
            verified: true,
        },
        metrics: metric_defs(&last_updated),
        models,
    }
}

/// Two `SourceFile`s are "the same" for commit-if-changed purposes if they are
/// equal after normalizing `fetched_at` out (the timestamp changes every run).
fn unchanged(old: &SourceFile, new: &SourceFile) -> bool {
    let mut old_norm = old.clone();
    old_norm.source.fetched_at = new.source.fetched_at.clone();
    &old_norm == new
}

/// Transform LLM Stats rankings (+ optional model metadata) into a `SourceFile`.
///
/// CLI shape (see `main.rs` wiring):
/// `transform llmstats <rankings.json> [--models <models.json>] --output <out>`
pub fn run(rankings: &Path, models: Option<&Path>, output: &Path) -> Result<(), String> {
    let rankings_text = std::fs::read_to_string(rankings)
        .map_err(|e| format!("failed to read {}: {e}", rankings.display()))?;
    let raw_rankings: RawRankingsFile = serde_json::from_str(&rankings_text).map_err(|e| {
        format!(
            "failed to parse LLM Stats rankings from {}: {e}",
            rankings.display()
        )
    })?;

    let raw_models = match models {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let parsed: RawModelsFile = serde_json::from_str(&text).map_err(|e| {
                format!(
                    "failed to parse LLM Stats models from {}: {e}",
                    path.display()
                )
            })?;
            Some(parsed)
        }
        None => None,
    };

    let source = build_source_file(raw_rankings, raw_models);

    // If-changed: skip the write when the only difference from the existing
    // output is the fetched_at timestamp.
    if let Ok(existing_text) = std::fs::read_to_string(output) {
        if let Ok(existing) = serde_json::from_str::<SourceFile>(&existing_text) {
            if unchanged(&existing, &source) {
                println!("unchanged");
                return Ok(());
            }
        }
    }

    let pretty =
        serde_json::to_string_pretty(&source).map_err(|e| format!("failed to serialize: {e}"))?;
    std::fs::write(output, pretty)
        .map_err(|e| format!("failed to write {}: {e}", output.display()))?;

    println!(
        "wrote {} ({} models, {} metrics)",
        output.display(),
        source.models.len(),
        source.metrics.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const RANKINGS_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llmstats/rankings_sample.json"
    ));
    const MODELS_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llmstats/models_sample.json"
    ));

    fn parse_fixture_with_models() -> SourceFile {
        let rankings: RawRankingsFile =
            serde_json::from_str(RANKINGS_FIXTURE).expect("rankings fixture parses");
        let models: RawModelsFile =
            serde_json::from_str(MODELS_FIXTURE).expect("models fixture parses");
        build_source_file(rankings, Some(models))
    }

    fn parse_fixture_no_models() -> SourceFile {
        let rankings: RawRankingsFile =
            serde_json::from_str(RANKINGS_FIXTURE).expect("rankings fixture parses");
        build_source_file(rankings, None)
    }

    fn row<'a>(sf: &'a SourceFile, id: &str) -> &'a ModelRow {
        sf.models
            .iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("model {id} present"))
    }

    #[test]
    fn metric_table_has_11_curated_categories() {
        let sf = parse_fixture_with_models();
        assert_eq!(sf.metrics.len(), 11);
        let ids: HashSet<&str> = sf.metrics.iter().map(|m| m.id.as_str()).collect();
        for expected in [
            "agents",
            "code",
            "finance",
            "frontend_development",
            "general",
            "healthcare",
            "legal",
            "math",
            "multimodal",
            "reasoning",
            "vision",
        ] {
            assert!(ids.contains(expected), "curated metric {expected} present");
        }
    }

    #[test]
    fn every_metric_has_a_nonempty_description_mentioning_trueskill() {
        let sf = parse_fixture_with_models();
        for m in &sf.metrics {
            let d = m
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("metric {} has no description", m.id));
            assert!(d.len() > 20, "metric {} description too short", m.id);
            assert!(
                d.contains("TrueSkill"),
                "metric {} description should mention the TrueSkill rating: {d:?}",
                m.id
            );
        }
    }

    #[test]
    fn all_metrics_are_index_kind() {
        let sf = parse_fixture_with_models();
        assert!(sf.metrics.iter().all(|m| m.kind == MetricKind::Index));
    }

    #[test]
    fn groups_partition_into_two_radar_sized_buckets() {
        let sf = parse_fixture_with_models();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for m in &sf.metrics {
            assert!(
                m.group == "Categories I" || m.group == "Categories II",
                "metric {} group {:?} unexpected",
                m.id,
                m.group
            );
            *counts.entry(m.group.as_str()).or_default() += 1;
        }
        // Both groups are valid radar presets (>=3 metrics) and within 6 axes.
        assert_eq!(counts["Categories I"], 6);
        assert_eq!(counts["Categories II"], 5);
        assert!(counts["Categories I"] >= 3 && counts["Categories I"] <= 6);
        assert!(counts["Categories II"] >= 3 && counts["Categories II"] <= 6);
    }

    #[test]
    fn every_metric_higher_is_better() {
        let sf = parse_fixture_with_models();
        assert!(sf.metrics.iter().all(|m| m.higher_is_better));
    }

    #[test]
    fn non_curated_category_excluded_from_scores() {
        // The "safety" ranking is present in the fixture but NOT curated.
        let sf = parse_fixture_with_models();
        // No metric def named "safety".
        assert!(!sf.metrics.iter().any(|m| m.id == "safety"));
        // claude-opus-4-8 appears in the safety ranking, but that score is
        // dropped — it must NOT carry a "safety" score key.
        let m = row(&sf, "claude-opus-4-8");
        assert!(!m.scores.contains_key("safety"));
    }

    #[test]
    fn curated_scores_present_with_value_and_date() {
        let sf = parse_fixture_with_models();
        let m = row(&sf, "claude-opus-4-8");
        // From the "general" ranking.
        let general = m.scores.get("general").expect("general score present");
        assert!((general.value - 67.14).abs() < 1e-9);
        assert_eq!(general.date.as_deref(), Some("2026-06-10T21:57:00.000000Z"));
        assert!(general.ci.is_none());
        // From the "code" ranking.
        let code = m.scores.get("code").expect("code score present");
        assert!((code.value - 56.63).abs() < 1e-9);
    }

    #[test]
    fn metric_last_updated_carries_ranked_at() {
        let sf = parse_fixture_with_models();
        let general = sf
            .metrics
            .iter()
            .find(|m| m.id == "general")
            .expect("general metric");
        assert_eq!(
            general.last_updated.as_deref(),
            Some("2026-06-10T21:57:00.000000Z")
        );
        // A curated category with no row in the fixture (e.g. "math") has no
        // ranked_at recorded -> last_updated None.
        let math = sf
            .metrics
            .iter()
            .find(|m| m.id == "math")
            .expect("math metric");
        assert!(math.last_updated.is_none());
    }

    #[test]
    fn source_meta_is_llmstats_and_verified() {
        let sf = parse_fixture_with_models();
        assert_eq!(sf.source.id, "llmstats");
        assert_eq!(sf.source.name, "LLM Stats");
        assert_eq!(sf.source.url, "https://llm-stats.com");
        assert!(
            sf.source.verified,
            "llmstats aggregates third-party results — no self-reported badge"
        );
        assert!(!sf.source.fetched_at.is_empty());
    }

    #[test]
    fn metadata_join_populates_release_date_and_context() {
        let sf = parse_fixture_with_models();
        let m = row(&sf, "claude-opus-4-8");
        assert_eq!(m.release_date.as_deref(), Some("2026-05-01"));
        assert_eq!(m.context_window, Some(200000));
        assert_eq!(m.creator, "anthropic");
        assert_eq!(m.creator_name, "Anthropic");
    }

    #[test]
    fn metadata_join_absent_falls_back_to_ranking_org() {
        // phantom-no-meta is in the vision ranking but has NO /v1/models row.
        let sf = parse_fixture_with_models();
        let m = row(&sf, "phantom-no-meta");
        assert!(m.release_date.is_none());
        assert!(m.context_window.is_none());
        // creator + creator_name fall back to the rankings org slug.
        assert_eq!(m.creator, "mystery-org");
        assert_eq!(m.creator_name, "mystery-org");
    }

    #[test]
    fn without_models_input_metadata_fields_are_none() {
        let sf = parse_fixture_no_models();
        let m = row(&sf, "claude-opus-4-8");
        // No models file -> release_date/context_window None, creator from
        // the rankings org slug.
        assert!(m.release_date.is_none());
        assert!(m.context_window.is_none());
        assert_eq!(m.creator, "anthropic");
        assert_eq!(m.creator_name, "anthropic");
        // open_weights still resolves from the rankings flag.
        assert_eq!(m.open_weights, Some(false));
    }

    #[test]
    fn open_weights_from_metadata_join() {
        let sf = parse_fixture_with_models();
        assert_eq!(row(&sf, "qwen3-max").open_weights, Some(true));
        assert_eq!(row(&sf, "claude-opus-4-8").open_weights, Some(false));
    }

    #[test]
    fn reasoning_effort_parsed_from_name() {
        let sf = parse_fixture_with_models();
        // "Gemini 3 Pro (Reasoning, High Effort)"
        let m = row(&sf, "gemini-3-pro");
        assert_eq!(m.display_name, "Gemini 3 Pro");
        assert_eq!(m.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(m.effort_level.as_deref(), Some("high"));
    }

    #[test]
    fn model_order_is_first_seen_across_rankings() {
        let sf = parse_fixture_with_models();
        // Order of first appearance: general ranking first
        // (claude-opus-4-8, gemini-3-pro, qwen3-max), then vision adds
        // phantom-no-meta. "safety" is non-curated and contributes no models.
        let ids: Vec<&str> = sf.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "claude-opus-4-8",
                "gemini-3-pro",
                "qwen3-max",
                "phantom-no-meta"
            ]
        );
    }

    #[test]
    fn serialize_deserialize_round_trip() {
        let sf = parse_fixture_with_models();
        let json = serde_json::to_string_pretty(&sf).expect("serialize");
        let back: SourceFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sf, back);
    }

    #[test]
    fn unchanged_ignores_fetched_at() {
        let mut a = parse_fixture_with_models();
        let mut b = a.clone();
        a.source.fetched_at = "2026-01-01T00:00:00+00:00".to_string();
        b.source.fetched_at = "2099-12-31T23:59:59+00:00".to_string();
        assert!(unchanged(&a, &b));

        // A real difference is detected.
        b.models[0].name = "Different Name".to_string();
        assert!(!unchanged(&a, &b));
    }

    #[test]
    fn if_changed_second_run_leaves_file_untouched() {
        let dir = std::env::temp_dir().join(format!(
            "transform_llmstats_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let rankings = dir.join("rankings.json");
        let models = dir.join("models.json");
        let output = dir.join("llmstats.json");
        std::fs::write(&rankings, RANKINGS_FIXTURE).expect("write rankings");
        std::fs::write(&models, MODELS_FIXTURE).expect("write models");

        // First run writes the file.
        run(&rankings, Some(&models), &output).expect("first run ok");
        let first_bytes = std::fs::read(&output).expect("read output");
        let first_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();

        // Second run: identical input -> only fetched_at would differ ->
        // detected as unchanged -> file is NOT rewritten.
        run(&rankings, Some(&models), &output).expect("second run ok");
        let second_bytes = std::fs::read(&output).expect("read output again");
        let second_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();

        assert_eq!(first_bytes, second_bytes, "content unchanged");
        assert_eq!(first_mtime, second_mtime, "mtime unchanged (no rewrite)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn if_changed_detects_real_change_and_rewrites() {
        let dir = std::env::temp_dir().join(format!(
            "transform_llmstats_change_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let rankings = dir.join("rankings.json");
        let models = dir.join("models.json");
        let output = dir.join("llmstats.json");

        // Write an output that differs in a real (non-timestamp) field.
        let mut stale = parse_fixture_with_models();
        stale.models[0].name = "Stale Name".to_string();
        std::fs::write(
            &output,
            serde_json::to_string_pretty(&stale).expect("serialize stale"),
        )
        .expect("write stale output");
        std::fs::write(&rankings, RANKINGS_FIXTURE).expect("write rankings");
        std::fs::write(&models, MODELS_FIXTURE).expect("write models");

        run(&rankings, Some(&models), &output).expect("run ok");
        let rewritten: SourceFile =
            serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).expect("parse output");
        assert_eq!(rewritten.models[0].name, "Claude Opus 4.8");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
