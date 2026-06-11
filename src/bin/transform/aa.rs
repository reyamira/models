//! `transform aa` — Artificial Analysis raw API response -> v2 `SourceFile`.
//!
//! Mirrors the jq transform in `.github/workflows/update-benchmarks.yml`
//! exactly for field paths and null-safety. Field semantics:
//! - `evaluations` (and every field within) may be null/missing.
//! - `model_creator` may be null.
//! - `median_output_tokens_per_second` / `median_time_to_first_token_seconds`
//!   / `median_time_to_first_answer_token` use `0` as a MISSING-DATA SENTINEL
//!   -> treated as `None` here.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::schema::{
    MetricDef, MetricKind, ModelRow, ReasoningStatus, ScoreCell, SourceFile, SourceMeta,
};

// ---------------------------------------------------------------------------
// Raw API shape (typed serde structs; #[serde(default)] over Value spelunking).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    data: Vec<RawModel>,
}

#[derive(Debug, Deserialize)]
struct RawModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    model_creator: Option<RawCreator>,
    #[serde(default)]
    evaluations: Option<RawEvaluations>,
    #[serde(default)]
    median_output_tokens_per_second: Option<f64>,
    #[serde(default)]
    median_time_to_first_token_seconds: Option<f64>,
    #[serde(default)]
    median_time_to_first_answer_token: Option<f64>,
    #[serde(default)]
    pricing: Option<RawPricing>,
}

#[derive(Debug, Deserialize)]
struct RawCreator {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvaluations {
    #[serde(default)]
    artificial_analysis_intelligence_index: Option<f64>,
    #[serde(default)]
    artificial_analysis_coding_index: Option<f64>,
    #[serde(default)]
    artificial_analysis_math_index: Option<f64>,
    #[serde(default)]
    mmlu_pro: Option<f64>,
    #[serde(default)]
    gpqa: Option<f64>,
    #[serde(default)]
    hle: Option<f64>,
    #[serde(default)]
    livecodebench: Option<f64>,
    #[serde(default)]
    scicode: Option<f64>,
    #[serde(default)]
    ifbench: Option<f64>,
    #[serde(default)]
    lcr: Option<f64>,
    #[serde(default)]
    terminalbench_hard: Option<f64>,
    #[serde(default)]
    tau2: Option<f64>,
    #[serde(default)]
    math_500: Option<f64>,
    #[serde(default)]
    aime: Option<f64>,
    #[serde(default)]
    aime_25: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPricing {
    #[serde(default)]
    price_1m_input_tokens: Option<f64>,
    #[serde(default)]
    price_1m_output_tokens: Option<f64>,
    #[serde(default)]
    price_1m_blended_3_to_1: Option<f64>,
}

// ---------------------------------------------------------------------------
// Metric registry (binding) — see task spec. `group` PARTITIONS the metrics:
// each metric belongs to exactly one group. This intentionally differs from the
// current radar "Agentic" preset (which borrows `coding_index` from Indexes) —
// Phase 2 radar presets must be derived from these single-membership groups,
// not the legacy overlapping presets.
// ---------------------------------------------------------------------------

/// One entry in the AA metric registry:
/// (id, label, kind, group, higher_is_better, description).
type MetricEntry = (
    &'static str,
    &'static str,
    MetricKind,
    &'static str,
    bool,
    &'static str,
);

const METRICS: &[MetricEntry] = &[
    // Group "Indexes"
    (
        "intelligence_index",
        "Intelligence Index",
        MetricKind::Index,
        "Indexes",
        true,
        "Artificial Analysis's headline composite, averaging a suite of reasoning, \
         knowledge, math, and coding evaluations into one general-capability score. \
         Scale 0-100; higher is better.",
    ),
    (
        "coding_index",
        "Coding Index",
        MetricKind::Index,
        "Indexes",
        true,
        "Artificial Analysis's coding composite, averaging the Terminal-Bench Hard and \
         SciCode coding evaluations. Scale 0-100; higher is better.",
    ),
    (
        "math_index",
        "Math Index",
        MetricKind::Index,
        "Indexes",
        true,
        "Artificial Analysis's math composite, averaging its competition-math evaluations \
         (AIME and MATH-500). Scale 0-100; higher is better.",
    ),
    // Group "Agentic"
    (
        "livecodebench",
        "LiveCodeBench",
        MetricKind::Percentage,
        "Agentic",
        true,
        "Contamination-free competitive-programming problems harvested fresh from LeetCode, \
         AtCoder, and Codeforces. Scored pass@1 (% solved on the first attempt); higher is \
         better.",
    ),
    (
        "scicode",
        "SciCode",
        MetricKind::Percentage,
        "Agentic",
        true,
        "Scientist-curated tasks where the model writes Python to solve research-grade \
         problems across 16 scientific disciplines. Scored pass@1 (% of subproblems solved); \
         higher is better.",
    ),
    (
        "terminalbench_hard",
        "Terminal-Bench Hard",
        MetricKind::Percentage,
        "Agentic",
        true,
        "Agentic tasks the model must complete autonomously inside a sandboxed terminal \
         (software engineering, sysadmin, data processing). Scored pass@1 (% of tasks solved); \
         higher is better.",
    ),
    (
        "ifbench",
        "IFBench",
        MetricKind::Percentage,
        "Agentic",
        true,
        "Tests precise instruction-following on novel, verifiable output constraints the model \
         was not trained on (e.g. 'answer only yes or no'). Scored pass@1 (% of constraints \
         satisfied); higher is better.",
    ),
    (
        "lcr",
        "Long Context Reasoning",
        MetricKind::Percentage,
        "Agentic",
        true,
        "AA-LCR: reasoning across multiple real-world documents totalling ~100k tokens, where \
         answers must be synthesized rather than retrieved. Scored pass@1 (% correct); higher \
         is better.",
    ),
    (
        "tau2",
        "Tau2-Bench",
        MetricKind::Percentage,
        "Agentic",
        true,
        "𝜏²-Bench: a dual-control conversational benchmark where the tool-using agent and a \
         simulated user must coordinate to resolve telecom support issues. Scored pass@1 \
         (% of scenarios resolved); higher is better.",
    ),
    // Group "Academic"
    (
        "gpqa",
        "GPQA Diamond",
        MetricKind::Percentage,
        "Academic",
        true,
        "198 'Google-proof' graduate-level questions in biology, chemistry, and physics that \
         stump non-experts even with web access. Scored as accuracy (% correct); higher is \
         better.",
    ),
    (
        "mmlu_pro",
        "MMLU-Pro",
        MetricKind::Percentage,
        "Academic",
        true,
        "A harder MMLU with graduate-level questions across 14 subjects and ten answer options, \
         emphasizing reasoning over recall. Scored as accuracy (% correct); higher is better.",
    ),
    (
        "hle",
        "Humanity's Last Exam",
        MetricKind::Percentage,
        "Academic",
        true,
        "Expert-authored questions across 100+ academic subjects, designed to require \
         specialized knowledge that cannot be quickly looked up. Scored as accuracy \
         (% correct); higher is better.",
    ),
    (
        "math_500",
        "MATH-500",
        MetricKind::Percentage,
        "Academic",
        true,
        "A 500-problem subset of the MATH dataset spanning competition-level algebra, geometry, \
         number theory, and more. Scored pass@1 (% correct); higher is better.",
    ),
    (
        "aime",
        "AIME '24",
        MetricKind::Percentage,
        "Academic",
        true,
        "The 2024 American Invitational Mathematics Examination: olympiad-level problems with \
         integer answers 0-999. Scored pass@1 accuracy (% correct); higher is better.",
    ),
    (
        "aime_25",
        "AIME '25",
        MetricKind::Percentage,
        "Academic",
        true,
        "The 2025 American Invitational Mathematics Examination: olympiad-level problems with \
         integer answers 0-999. Scored pass@1 accuracy (% correct); higher is better.",
    ),
    // Group "Performance"
    (
        "output_tps",
        "Output Speed",
        MetricKind::TokensPerSec,
        "Performance",
        true,
        "Output generation speed — average tokens received per second after the first token. \
         Measured in tokens/sec; higher (faster) is better.",
    ),
    (
        "ttft",
        "TTFT",
        MetricKind::Seconds,
        "Performance",
        false,
        "Time to first token: seconds between sending the request and receiving the first \
         token of the response. Measured in seconds; lower (faster) is better.",
    ),
    (
        "ttfat",
        "TTFAT",
        MetricKind::Seconds,
        "Performance",
        false,
        "Time to first answer token: seconds until the first answer token arrives, measured \
         for reasoning models after any 'thinking' time. Measured in seconds; lower (faster) \
         is better.",
    ),
    // Group "Pricing"
    (
        "price_input",
        "Input Price",
        MetricKind::UsdPerMTok,
        "Pricing",
        false,
        "Provider list price to send prompt tokens to the model, in US dollars per million \
         input tokens. Lower (cheaper) is better.",
    ),
    (
        "price_output",
        "Output Price",
        MetricKind::UsdPerMTok,
        "Pricing",
        false,
        "Provider list price for generated tokens, in US dollars per million output tokens. \
         Lower (cheaper) is better.",
    ),
    (
        "price_blended",
        "Blended Price",
        MetricKind::UsdPerMTok,
        "Pricing",
        false,
        "Blended cost assuming a 3:1 input-to-output token ratio, in US dollars per million \
         tokens. Lower (cheaper) is better.",
    ),
];

fn metric_defs() -> Vec<MetricDef> {
    METRICS
        .iter()
        .map(|&(id, label, kind, group, hib, description)| MetricDef {
            id: id.to_string(),
            label: label.to_string(),
            kind,
            group: group.to_string(),
            higher_is_better: hib,
            last_updated: None,
            description: Some(description.to_string()),
        })
        .collect()
}

/// `0` is a missing-data sentinel for the three performance fields.
fn nonzero(v: Option<f64>) -> Option<f64> {
    v.filter(|&x| x != 0.0)
}

fn insert_score(scores: &mut BTreeMap<String, ScoreCell>, id: &str, value: Option<f64>) {
    if let Some(value) = value {
        scores.insert(
            id.to_string(),
            ScoreCell {
                value,
                date: None,
                ci: None,
                votes: None,
            },
        );
    }
}

fn model_to_row(raw: RawModel) -> ModelRow {
    let parsed = crate::schema::parse_name_metadata(&raw.name, ReasoningStatus::None);

    let (creator, creator_name) = match raw.model_creator {
        Some(c) => (c.slug.unwrap_or_default(), c.name.unwrap_or_default()),
        None => (String::new(), String::new()),
    };

    let evals = raw.evaluations.unwrap_or_default();

    let mut scores: BTreeMap<String, ScoreCell> = BTreeMap::new();
    // Indexes
    insert_score(
        &mut scores,
        "intelligence_index",
        evals.artificial_analysis_intelligence_index,
    );
    insert_score(
        &mut scores,
        "coding_index",
        evals.artificial_analysis_coding_index,
    );
    insert_score(
        &mut scores,
        "math_index",
        evals.artificial_analysis_math_index,
    );
    // Agentic
    insert_score(&mut scores, "livecodebench", evals.livecodebench);
    insert_score(&mut scores, "scicode", evals.scicode);
    insert_score(&mut scores, "terminalbench_hard", evals.terminalbench_hard);
    insert_score(&mut scores, "ifbench", evals.ifbench);
    insert_score(&mut scores, "lcr", evals.lcr);
    insert_score(&mut scores, "tau2", evals.tau2);
    // Academic
    insert_score(&mut scores, "gpqa", evals.gpqa);
    insert_score(&mut scores, "mmlu_pro", evals.mmlu_pro);
    insert_score(&mut scores, "hle", evals.hle);
    insert_score(&mut scores, "math_500", evals.math_500);
    insert_score(&mut scores, "aime", evals.aime);
    insert_score(&mut scores, "aime_25", evals.aime_25);
    // Performance (0 is a missing-data sentinel)
    insert_score(
        &mut scores,
        "output_tps",
        nonzero(raw.median_output_tokens_per_second),
    );
    insert_score(
        &mut scores,
        "ttft",
        nonzero(raw.median_time_to_first_token_seconds),
    );
    insert_score(
        &mut scores,
        "ttfat",
        nonzero(raw.median_time_to_first_answer_token),
    );
    // Pricing
    let pricing = raw.pricing.unwrap_or_default();
    insert_score(&mut scores, "price_input", pricing.price_1m_input_tokens);
    insert_score(&mut scores, "price_output", pricing.price_1m_output_tokens);
    insert_score(
        &mut scores,
        "price_blended",
        pricing.price_1m_blended_3_to_1,
    );

    let id = if raw.slug.is_empty() {
        raw.id.clone()
    } else {
        raw.slug.clone()
    };

    ModelRow {
        id,
        name: raw.name,
        display_name: parsed.display_name,
        creator,
        creator_name,
        release_date: raw.release_date,
        reasoning_status: parsed.reasoning_status,
        effort_level: parsed.effort_level,
        variant_tag: parsed.variant_tag,
        // open_weights + context_window are runtime-augmented in Phase 2
        // (ported traits.rs pass against models.dev). Left None at transform time.
        open_weights: None,
        context_window: None,
        supports_tools: None,
        max_output: None,
        scores,
    }
}

/// Build a `SourceFile` from a parsed raw AA response.
fn build_source_file(raw: RawResponse) -> SourceFile {
    let models = raw.data.into_iter().map(model_to_row).collect();

    SourceFile {
        source: SourceMeta {
            id: "aa".to_string(),
            name: "Artificial Analysis".to_string(),
            url: "https://artificialanalysis.ai".to_string(),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            verified: true,
        },
        metrics: metric_defs(),
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

pub fn run(input: &Path, output: &Path) -> Result<(), String> {
    let raw_text = std::fs::read_to_string(input)
        .map_err(|e| format!("failed to read {}: {e}", input.display()))?;
    let raw: RawResponse = serde_json::from_str(&raw_text)
        .map_err(|e| format!("failed to parse AA response from {}: {e}", input.display()))?;

    let source = build_source_file(raw);

    // If-changed: skip the write (and leave mtime/content untouched) when the
    // only difference from the existing output is the fetched_at timestamp.
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

    const FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/aa_raw_sample.json"
    ));

    fn parse_fixture() -> SourceFile {
        let raw: RawResponse = serde_json::from_str(FIXTURE).expect("fixture parses");
        build_source_file(raw)
    }

    fn row<'a>(sf: &'a SourceFile, id: &str) -> &'a ModelRow {
        sf.models
            .iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("model {id} present"))
    }

    #[test]
    fn model_count_matches_fixture() {
        let sf = parse_fixture();
        assert_eq!(sf.models.len(), 4, "fixture has 4 models");
    }

    #[test]
    fn metric_table_has_21_metrics() {
        let sf = parse_fixture();
        assert_eq!(sf.metrics.len(), 21);
    }

    #[test]
    fn every_metric_in_exactly_one_of_five_groups() {
        let sf = parse_fixture();
        let groups = ["Indexes", "Agentic", "Academic", "Performance", "Pricing"];
        let mut seen_ids: HashSet<&str> = HashSet::new();
        for m in &sf.metrics {
            assert!(
                groups.contains(&m.group.as_str()),
                "metric {} group {:?} not one of the five",
                m.id,
                m.group
            );
            assert!(
                seen_ids.insert(m.id.as_str()),
                "metric id {} appears more than once",
                m.id
            );
        }
        // Each metric appears under exactly one group (no id collisions across
        // groups) — partition invariant.
        assert_eq!(seen_ids.len(), 21);
    }

    #[test]
    fn group_distribution_partitions() {
        let sf = parse_fixture();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for m in &sf.metrics {
            *counts.entry(m.group.as_str()).or_default() += 1;
        }
        assert_eq!(counts["Indexes"], 3);
        assert_eq!(counts["Agentic"], 6);
        assert_eq!(counts["Academic"], 6);
        assert_eq!(counts["Performance"], 3);
        assert_eq!(counts["Pricing"], 3);
    }

    #[test]
    fn metric_last_updated_is_none() {
        let sf = parse_fixture();
        assert!(sf.metrics.iter().all(|m| m.last_updated.is_none()));
    }

    #[test]
    fn every_metric_has_a_nonempty_description() {
        let sf = parse_fixture();
        for m in &sf.metrics {
            let d = m
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("metric {} has no description", m.id));
            assert!(d.len() > 20, "metric {} description too short: {d:?}", m.id);
        }
    }

    #[test]
    fn source_meta_is_aa() {
        let sf = parse_fixture();
        assert_eq!(sf.source.id, "aa");
        assert_eq!(sf.source.name, "Artificial Analysis");
        assert_eq!(sf.source.url, "https://artificialanalysis.ai");
        assert!(sf.source.verified);
        assert!(!sf.source.fetched_at.is_empty());
    }

    #[test]
    fn zero_sentinel_perf_values_absent_from_scores() {
        let sf = parse_fixture();
        // "zephyr-zero" model has 0 in all three perf fields -> none present.
        let m = row(&sf, "zephyr-zero");
        assert!(!m.scores.contains_key("output_tps"));
        assert!(!m.scores.contains_key("ttft"));
        assert!(!m.scores.contains_key("ttfat"));
    }

    #[test]
    fn nonzero_perf_values_present() {
        let sf = parse_fixture();
        // "atlas-pro" has real perf numbers.
        let m = row(&sf, "atlas-pro");
        assert!(m.scores.contains_key("output_tps"));
        assert!(m.scores.contains_key("ttft"));
        assert!(m.scores.contains_key("ttfat"));
    }

    #[test]
    fn null_evaluations_absent_from_scores() {
        let sf = parse_fixture();
        // "nova-base" has evaluations: null -> no eval metrics at all.
        let m = row(&sf, "nova-base");
        for id in [
            "intelligence_index",
            "coding_index",
            "math_index",
            "livecodebench",
            "scicode",
            "terminalbench_hard",
            "ifbench",
            "lcr",
            "tau2",
            "gpqa",
            "mmlu_pro",
            "hle",
            "math_500",
            "aime",
            "aime_25",
        ] {
            assert!(
                !m.scores.contains_key(id),
                "eval metric {id} should be absent for null-evals model"
            );
        }
    }

    #[test]
    fn missing_eval_field_absent_but_others_present() {
        let sf = parse_fixture();
        // "atlas-pro" has most evals but is missing `hle` (field absent).
        let m = row(&sf, "atlas-pro");
        assert!(m.scores.contains_key("intelligence_index"));
        assert!(!m.scores.contains_key("hle"));
    }

    #[test]
    fn reasoning_effort_display_name_parsed() {
        let sf = parse_fixture();
        // "atlas-pro" name: "Atlas Pro (Reasoning, High Effort)"
        let m = row(&sf, "atlas-pro");
        assert_eq!(m.display_name, "Atlas Pro");
        assert_eq!(m.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(m.effort_level.as_deref(), Some("high"));
    }

    #[test]
    fn non_reasoning_model_parsed() {
        let sf = parse_fixture();
        // "nova-base" name: "Nova Base (Non-Reasoning)"
        let m = row(&sf, "nova-base");
        assert_eq!(m.reasoning_status, ReasoningStatus::NonReasoning);
        assert_eq!(m.display_name, "Nova Base");
    }

    #[test]
    fn date_parenthetical_stripped_from_display_name() {
        let sf = parse_fixture();
        // "orion-flash" name: "Orion Flash (Dec '24)"
        let m = row(&sf, "orion-flash");
        assert_eq!(m.display_name, "Orion Flash");
    }

    #[test]
    fn null_release_date_preserved_as_none() {
        let sf = parse_fixture();
        // "zephyr-zero" has release_date: null.
        let m = row(&sf, "zephyr-zero");
        assert!(m.release_date.is_none());
    }

    #[test]
    fn null_creator_yields_empty_strings() {
        let sf = parse_fixture();
        // "nova-base" has model_creator: null.
        let m = row(&sf, "nova-base");
        assert_eq!(m.creator, "");
        assert_eq!(m.creator_name, "");
    }

    #[test]
    fn creator_slug_and_name_populated_when_present() {
        let sf = parse_fixture();
        let m = row(&sf, "atlas-pro");
        assert_eq!(m.creator, "acme-labs");
        assert_eq!(m.creator_name, "Acme Labs");
    }

    #[test]
    fn open_weights_and_context_window_left_none() {
        let sf = parse_fixture();
        for m in &sf.models {
            assert!(m.open_weights.is_none());
            assert!(m.context_window.is_none());
        }
    }

    #[test]
    fn model_order_preserved() {
        let sf = parse_fixture();
        let ids: Vec<&str> = sf.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            ["atlas-pro", "nova-base", "orion-flash", "zephyr-zero"]
        );
    }

    #[test]
    fn serialize_deserialize_round_trip() {
        let sf = parse_fixture();
        let json = serde_json::to_string_pretty(&sf).expect("serialize");
        let back: SourceFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sf, back);
    }

    #[test]
    fn unchanged_ignores_fetched_at() {
        let mut a = parse_fixture();
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
            "transform_aa_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let input = dir.join("raw.json");
        let output = dir.join("aa.json");
        std::fs::write(&input, FIXTURE).expect("write input");

        // First run writes the file.
        run(&input, &output).expect("first run ok");
        let first_bytes = std::fs::read(&output).expect("read output");
        let first_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();

        // Second run: identical input -> only fetched_at would differ ->
        // detected as unchanged -> file is NOT rewritten.
        run(&input, &output).expect("second run ok");
        let second_bytes = std::fs::read(&output).expect("read output again");
        let second_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();

        assert_eq!(first_bytes, second_bytes, "content unchanged");
        assert_eq!(first_mtime, second_mtime, "mtime unchanged (no rewrite)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn if_changed_detects_real_change_and_rewrites() {
        let dir = std::env::temp_dir().join(format!(
            "transform_aa_change_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let input = dir.join("raw.json");
        let output = dir.join("aa.json");

        // Write an output that differs in a real (non-timestamp) field.
        let mut stale = parse_fixture();
        stale.models[0].name = "Stale Name".to_string();
        std::fs::write(
            &output,
            serde_json::to_string_pretty(&stale).expect("serialize stale"),
        )
        .expect("write stale output");
        std::fs::write(&input, FIXTURE).expect("write input");

        run(&input, &output).expect("run ok");
        let rewritten: SourceFile =
            serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).expect("parse output");
        assert_eq!(
            rewritten.models[0].name,
            "Atlas Pro (Reasoning, High Effort)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
