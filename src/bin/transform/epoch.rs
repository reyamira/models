//! `transform epoch` — Epoch AI benchmark CSV directory -> v2 `SourceFile`.
//!
//! Input is the *unzipped* `benchmark_data.zip` directory: one wide CSV per
//! benchmark. Mirrors the structure / error-handling / if-changed discipline of
//! `aa.rs` (the canonical transform).
//!
//! Ground-truth observations baked into the parsing (verified against the live
//! `https://epoch.ai/data/benchmark_data.zip`, 2026-06-10):
//! - Uniform prefix is *mostly* `Model version, <score>, Release date, ...`, but
//!   a handful of CSVs insert a categorical column (`Agent`, `Tools`,
//!   `Scaffold`) before the score. So the score column is found as the **first
//!   numeric-parseable column after `Model version`**, not a fixed index.
//! - `Organization` / `Country` are present in the header but **empty in every
//!   row** of the current export. The schema field mapping still reads them
//!   (`creator` <- slugified Organization), it just yields empty strings today.
//! - Score cells are plain fractions (`0.42`), the Epoch Capabilities Index
//!   (`ECI Score`, ~62..159, kind Index), or the documented
//!   `scorer:mean±stderr` form (handled even though absent from the current
//!   export — the format is stable across Epoch versions).
//! - Several CSVs carry RFC-4180 quoted fields with embedded commas AND embedded
//!   newlines (e.g. ECI `Description`, simplebench `Notes`) — hence the `csv`
//!   crate rather than a hand-rolled split.
//!
//! AUTO-PRUNE: a CSV is included only if its newest row date (best run/eval date
//! column, falling back to `Release date`) is <= 60 days old at transform time.
//! The metric set therefore adapts run-to-run; the data-driven UI absorbs it.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Datelike, NaiveDate, Utc};

use crate::schema::{
    MetricDef, MetricKind, ModelRow, ReasoningStatus, ScoreCell, SourceFile, SourceMeta,
};

/// Staleness cutoff (plan §8 flagged default): a CSV is pruned when its newest
/// row date is older than this many days at transform time.
const PRUNE_DAYS: i64 = 60;

// ---------------------------------------------------------------------------
// Static metric registry — id (CSV stem) -> (label, kind, group).
//
// `id` is the CSV filename stem snake_cased. Only CSVs that survive the 60-day
// prune become metrics, so this table is a *superset* lookup: stems not present
// here fall back to a humanized label + the "Academic" group (data-driven UI
// absorbs new benchmarks without a code change). The group map keeps each
// intended radar group (Frontier / Agentic / Academic) at 3-6 higher_is_better
// metrics so it renders as a radar preset (>=3 hib, first 6 are axes).
// ---------------------------------------------------------------------------

/// `(label, kind, group, description)` for a known CSV stem.
type MetricMeta = (&'static str, MetricKind, &'static str, &'static str);

/// Label + kind + group + description overrides keyed by CSV filename stem.
fn metric_meta(stem: &str) -> Option<MetricMeta> {
    let m: MetricMeta = match stem {
        // ---- Frontier (hardest reasoning / capability frontier) ----
        "epoch_capabilities_index" => (
            "Epoch Capabilities Index",
            MetricKind::Index,
            "Frontier",
            "Epoch's composite that stitches 40+ benchmarks into one general-capability scale \
             using Item Response Theory, so models stay comparable as individual benchmarks \
             saturate. Open-ended linear scale with no maximum (recent frontier models score ~130-160); higher is better.",
        ),
        "frontiermath" => (
            "FrontierMath",
            MetricKind::Percentage,
            "Frontier",
            "Hundreds of original, expert-crafted research-level math problems spanning number \
             theory, analysis, algebraic geometry, and more — each taking specialists hours to \
             days. Scored as accuracy (% of problems solved); higher is better.",
        ),
        "frontiermath_tier_4" => (
            "FrontierMath Tier 4",
            MetricKind::Percentage,
            "Frontier",
            "The 50 hardest, research-level problems of FrontierMath — its most difficult \
             expansion tier. Scored as accuracy (% of problems solved); higher is better.",
        ),
        "arc_agi" => (
            "ARC-AGI",
            MetricKind::Percentage,
            "Frontier",
            "Abstract visual grid puzzles where the model must infer a transformation rule from \
             a few input-output demonstrations and apply it to a novel case. Scored as accuracy \
             (% of tasks solved); higher is better.",
        ),
        "arc_agi_2" => (
            "ARC-AGI-2",
            MetricKind::Percentage,
            "Frontier",
            "A substantially harder successor to ARC-AGI: abstract grid puzzles stressing \
             compositional and symbolic reasoning, two attempts per task (pass@2). Scored as \
             accuracy (% of tasks solved); higher is better.",
        ),
        "hle" => (
            "Humanity's Last Exam",
            MetricKind::Percentage,
            "Frontier",
            "Expert-authored questions across 100+ academic subjects, requiring graduate-level \
             knowledge that cannot be quickly looked up online. Scored as accuracy (% correct); \
             higher is better.",
        ),
        // ---- Agentic (coding / agent / tool-use) ----
        "swe_bench_verified" => (
            "SWE-bench Verified",
            MetricKind::Percentage,
            "Agentic",
            "Real-world software engineering: the model is given a repository and a GitHub issue \
             from popular Python projects and must edit the codebase to fix it, graded by unit \
             tests. Scored as % of issues resolved; higher is better.",
        ),
        "terminalbench" => (
            "Terminal-Bench",
            MetricKind::Percentage,
            "Agentic",
            "Agentic command-line tasks the model completes autonomously inside a sandboxed \
             Docker terminal, checked by a test script. Scored as % of tasks solved; higher is \
             better.",
        ),
        "gso" => (
            "GSO",
            MetricKind::Percentage,
            "Agentic",
            "Software performance engineering: the model rewrites real GitHub code to speed it \
             up. Scored OPT@K (% of trials reaching at least 95% of the human speed-up); higher \
             is better.",
        ),
        "apex_agents" => (
            "APEX Agents",
            MetricKind::Percentage,
            "Agentic",
            "Professional deliverables across investment banking, consulting, law, and primary \
             care that would take practitioners hours, graded against pass/fail rubric criteria. \
             Scored as % of rubric criteria satisfied; higher is better.",
        ),
        "posttrainbench" => (
            "PostTrainBench",
            MetricKind::Percentage,
            "Agentic",
            "AI R&D automation: a CLI agent must post-train a small base LLM on a single GPU \
             within a time budget to raise its downstream scores. Scored as the average \
             improvement across base models and evaluations; higher is better.",
        ),
        "weirdml" => (
            "WeirdML",
            MetricKind::Percentage,
            "Agentic",
            "Unconventional machine-learning engineering tasks where the agent must write and \
             run code to train a model on provided data, with several iterations allowed. Scored \
             as accuracy on the held-out task; higher is better.",
        ),
        "aider_polyglot" => (
            "Aider Polyglot",
            MetricKind::Percentage,
            "Agentic",
            "Multi-language code editing on Exercism problems (C++, Go, Java, JavaScript, \
             Python, Rust) with a second attempt after seeing test failures. Scored as % of \
             exercises passing; higher is better.",
        ),
        "gdpval" => (
            "GDPval",
            MetricKind::Percentage,
            "Agentic",
            "Economically valuable real-world deliverables drawn from 44 occupations across \
             nine U.S. sectors, judged by experts in blind comparisons against human work. \
             Scored as win-rate versus the human baseline; higher is better.",
        ),
        // ---- Academic (general reasoning / knowledge / math) ----
        "gpqa_diamond" => (
            "GPQA Diamond",
            MetricKind::Percentage,
            "Academic",
            "198 'Google-proof' graduate-level biology, chemistry, and physics questions that \
             stump non-experts even with web access. Scored as accuracy (% correct); higher is \
             better.",
        ),
        "simpleqa_verified" => (
            "SimpleQA Verified",
            MetricKind::Percentage,
            "Academic",
            "A cleaned 1,000-prompt factuality benchmark of short fact-seeking questions, \
             measuring parametric knowledge and resistance to hallucination. Scored as accuracy \
             (% answered correctly); higher is better.",
        ),
        "otis_mock_aime_2024_2025" => (
            "OTIS Mock AIME",
            MetricKind::Percentage,
            "Academic",
            "45 competition-style math problems from the 2024-2025 OTIS Mock AIME exams \
             (integer answers 0-999), harder than MATH Level 5 but easier than FrontierMath. \
             Scored as accuracy (% correct); higher is better.",
        ),
        "chess_puzzles" => (
            "Chess Puzzles",
            MetricKind::Percentage,
            "Academic",
            "100 programmatically generated chess positions (in FEN) where the model must name \
             the single best move as judged by Stockfish. Scored as accuracy (% of positions \
             with the correct move); higher is better.",
        ),
        "simplebench" => (
            "SimpleBench",
            MetricKind::Percentage,
            "Academic",
            "Common-sense reasoning 'trick' questions about space, time, and social cues that \
             are easy for people (84% human baseline) but hard for models. Scored as accuracy \
             (% correct); higher is better.",
        ),
        _ => return None,
    };
    Some(m)
}

/// Humanize an unknown CSV stem: `swe_bench_verified` -> `Swe Bench Verified`.
fn humanize(stem: &str) -> String {
    stem.split('_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolved metric metadata for a stem. For an UNKNOWN stem `kind` is `None`
/// (inferred from the data via [`infer_kind`]), the label is humanized, the
/// group defaults to `Academic`, and there is no curated `description`.
struct ResolvedMeta {
    label: String,
    kind: Option<MetricKind>,
    group: String,
    description: Option<String>,
}

/// Resolve label / kind / group / description for a stem.
fn resolve_metric_meta(stem: &str) -> ResolvedMeta {
    match metric_meta(stem) {
        Some((label, kind, group, description)) => ResolvedMeta {
            label: label.to_string(),
            kind: Some(kind),
            group: group.to_string(),
            description: Some(description.to_string()),
        },
        None => ResolvedMeta {
            label: humanize(stem),
            kind: None,
            group: "Academic".to_string(),
            description: None,
        },
    }
}

/// Infer a [`MetricKind`] from the maximum observed score value for an unknown
/// benchmark. Epoch scores are either fractions (<=1, Percentage), the ECI-style
/// index scale (~60..160, Index), or Elo-style arena scores (>=1000, Elo).
fn infer_kind(max_value: f64) -> MetricKind {
    if max_value <= 1.0 {
        MetricKind::Percentage
    } else if max_value >= 1000.0 {
        MetricKind::Elo
    } else {
        // 1..1000: ECI-style capability index or a 0-100 percent scale. Treat as
        // Index (formatted `{:.1}`), which is the safe non-fractional default.
        MetricKind::Index
    }
}

// ---------------------------------------------------------------------------
// Model-version cleaning: strip effort / context suffixes into structured
// metadata. Epoch uses *underscore* suffixes (`_high`, `_32K`), unlike AA's
// parenthetical names, so `parse_name_metadata` (paren-based) doesn't fit — but
// all the underscore logic lives in ONE tested helper, `clean_model_version`.
// ---------------------------------------------------------------------------

/// Result of cleaning a raw `Model version` string.
#[derive(Debug, Clone, PartialEq)]
struct CleanedVersion {
    /// Slug with effort/context/placeholder suffixes removed (the model id).
    slug: String,
    /// Effort level (`high`/`low`/`medium`/`max`/`minimal`) if a suffix encoded one.
    effort_level: Option<String>,
    /// Context-window variant tag (e.g. `32K`) if a `_<digits>k` suffix was present.
    variant_tag: Option<String>,
    /// `Reasoning` if a `_thinking` suffix was present, else `None`.
    reasoning_status: ReasoningStatus,
}

/// Strip a single trailing `_suffix` from `raw`, returning structured metadata.
///
/// Recognized suffixes (case-insensitive on the keyword, applied once):
/// - effort: `_high` `_low` `_medium` `_xhigh` `_max` `_minimal` -> `effort_level`
///   (`xhigh` normalizes to `max`, matching the AA effort vocabulary)
/// - context: `_<digits>k` (e.g. `_32K`, `_128k`) -> `variant_tag`
/// - reasoning: `_thinking` -> `reasoning_status = Reasoning`
/// - placeholder: `_unknown` `_none` `_default` -> stripped, no metadata
///   (Epoch's "effort not specified" sentinel)
///
/// Anything else (parameter sizes like `_7b`, `_res448`, `_mistral`, `_5`) is
/// part of the model identity and left untouched.
fn clean_model_version(raw: &str) -> CleanedVersion {
    let trimmed = raw.trim();
    let mut effort_level = None;
    let mut variant_tag = None;
    let mut reasoning_status = ReasoningStatus::None;

    let Some(us) = trimmed.rfind('_') else {
        return CleanedVersion {
            slug: trimmed.to_string(),
            effort_level,
            variant_tag,
            reasoning_status,
        };
    };

    let stem = &trimmed[..us];
    let suffix = &trimmed[us + 1..];
    let lower = suffix.to_lowercase();

    let stripped = match lower.as_str() {
        "high" | "low" | "medium" | "minimal" => {
            effort_level = Some(lower.clone());
            true
        }
        "xhigh" | "max" => {
            effort_level = Some("max".to_string());
            true
        }
        "thinking" => {
            reasoning_status = ReasoningStatus::Reasoning;
            true
        }
        "unknown" | "none" | "default" => true,
        _ => {
            // Context suffix: all digits followed by a `k` (e.g. `32k`, `128k`).
            let is_context = lower.len() >= 2
                && lower.ends_with('k')
                && lower[..lower.len() - 1].chars().all(|c| c.is_ascii_digit());
            if is_context {
                // Preserve original casing of the tag (`32K`).
                variant_tag = Some(suffix.to_string());
                true
            } else {
                false
            }
        }
    };

    let slug = if stripped {
        stem.to_string()
    } else {
        trimmed.to_string()
    };

    CleanedVersion {
        slug,
        effort_level,
        variant_tag,
        reasoning_status,
    }
}

/// Slugify an organization name for the `creator` field: lowercase, non-alnum
/// runs -> single `-`, trimmed. Empty in -> empty out.
fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in s.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

// ---------------------------------------------------------------------------
// Date handling.
// ---------------------------------------------------------------------------

/// Parse a date cell into a `NaiveDate`. Accepts ISO-8601 timestamps
/// (`2026-05-27T22:28:21.000Z`, with or without millis/offset) and plain
/// `YYYY-MM-DD`. Returns `None` for empty / unparseable cells.
fn parse_date(cell: &str) -> Option<NaiveDate> {
    let s = cell.trim();
    if s.is_empty() {
        return None;
    }
    // Plain date first (the common `Release date` / `Run date` form).
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    // RFC-3339 timestamp (`Started at`, `Created`).
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.date_naive());
    }
    // Some `Z` timestamps carry millis but chrono's rfc3339 handles those; a
    // trailing-`Z` form without offset colon is already covered. Date-only
    // prefix fallback for anything timestamp-shaped.
    if s.len() >= 10 {
        if let Ok(d) = NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d") {
            return Some(d);
        }
    }
    None
}

/// Date columns that record *when a run/evaluation happened*, in priority order.
/// Used both for the freshness prune and for `ScoreCell.date`. `Release date` is
/// the final fallback (it's a model attribute, not a run timestamp, but it's the
/// only date several external CSVs carry).
const RUN_DATE_COLUMNS: &[&str] = &[
    "Started at",
    "Date of evaluation",
    "Evaluation date",
    "Run date",
    "Created",
    "Date added",
    "Last updated",
];

// ---------------------------------------------------------------------------
// Score parsing.
// ---------------------------------------------------------------------------

/// Parse a score cell. Handles plain numbers (`0.42`) and the
/// `scorer:mean±stderr` form (`gpt-grader:0.79±0.018` -> `0.79`). Returns the
/// mean as the value.
fn parse_score(cell: &str) -> Option<f64> {
    let s = cell.trim();
    if s.is_empty() {
        return None;
    }
    // `scorer:mean±stderr` -> take the part after the last ':' then before '±'.
    let after_scorer = s.rsplit(':').next().unwrap_or(s);
    let mean_part = after_scorer
        .split(['±', '\u{00b1}'])
        .next()
        .unwrap_or(after_scorer)
        .trim();
    mean_part.parse::<f64>().ok()
}

// ---------------------------------------------------------------------------
// Per-CSV parsing.
// ---------------------------------------------------------------------------

/// One accumulating model within a single CSV: the row plus its best score so
/// far (for the max-per-model dedup). The winning run's date lives on the
/// row's `ScoreCell.date`.
struct BenchAccum {
    row: ModelRow,
    best_value: f64,
    /// True when more than one distinct raw variant (e.g. `_high` and `_32K`)
    /// collapsed into this row — the per-variant identity fields (raw name,
    /// effort, variant tag) then describe no single upstream row and are
    /// normalized to the cleaned base identity.
    collapsed: bool,
}

/// Parsed result of one CSV: the metric def plus per-model best-score rows.
struct ParsedCsv {
    metric: MetricDef,
    /// model id -> accumulated row (best score per model, deduped).
    models: BTreeMap<String, BenchAccum>,
    /// Newest row date across the whole CSV (drives the prune + `last_updated`).
    newest_date: Option<NaiveDate>,
}

/// Locate the index of the first numeric-parseable column after `Model version`
/// (index 0). Scans column-by-column; a column qualifies if >60% of its
/// non-empty cells parse as scores. This skips categorical columns like
/// `Agent`/`Tools`/`Scaffold` that precede the score in some CSVs.
fn find_score_column(records: &[Vec<String>], ncol: usize) -> Option<usize> {
    for ci in 1..ncol {
        let mut ok = 0usize;
        let mut total = 0usize;
        for rec in records {
            if let Some(cell) = rec.get(ci) {
                if cell.trim().is_empty() {
                    continue;
                }
                total += 1;
                if parse_score(cell).is_some() {
                    ok += 1;
                }
            }
        }
        if total > 0 && (ok as f64) / (total as f64) > 0.6 {
            return Some(ci);
        }
    }
    None
}

/// Parse one CSV file into a [`ParsedCsv`]. Returns `Err` (caller skips + warns)
/// when the file can't be read, has no `Model version` header, or has no
/// identifiable score column.
fn parse_csv(stem: &str, path: &Path) -> Result<ParsedCsv, String> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;

    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| format!("headers {}: {e}", path.display()))?
        .iter()
        .map(str::to_string)
        .collect();

    if headers.first().map(String::as_str) != Some("Model version") {
        return Err(format!(
            "{}: first column is not 'Model version'",
            path.display()
        ));
    }

    let records: Vec<Vec<String>> = reader
        .records()
        .filter_map(Result::ok)
        .map(|rec| rec.iter().map(str::to_string).collect())
        .collect();

    let ncol = headers.len();
    let score_idx = find_score_column(&records, ncol)
        .ok_or_else(|| format!("{}: no numeric score column found", path.display()))?;

    // Date column indices (run dates in priority order, then Release date).
    let run_date_idx: Vec<usize> = RUN_DATE_COLUMNS
        .iter()
        .filter_map(|name| headers.iter().position(|h| h == name))
        .collect();
    let release_idx = headers.iter().position(|h| h == "Release date");
    let org_idx = headers.iter().position(|h| h == "Organization");

    let ResolvedMeta {
        label,
        kind: known_kind,
        group,
        description,
    } = resolve_metric_meta(stem);
    let higher_is_better = true; // every Epoch metric is higher-is-better.

    // Track the value range so an UNKNOWN stem's kind can be inferred from the
    // data (see `infer_kind`) rather than blindly defaulting to Percentage.
    let mut max_value = f64::NEG_INFINITY;

    // Whether this CSV carries ANY run/eval-date column. When it does, freshness
    // is judged purely on run dates (release dates don't count) — otherwise a
    // benchmark whose leaderboard stopped updating but whose models keep getting
    // released (e.g. webdev_arena: `Last updated` capped at 2026-01-05 but recent
    // `Release date`s) would falsely read as fresh. When it does NOT, the only
    // signal is release date, so freshness falls back to that.
    let has_run_date_col = !run_date_idx.is_empty();

    let mut models: BTreeMap<String, BenchAccum> = BTreeMap::new();
    let mut newest_run_date: Option<NaiveDate> = None;
    let mut newest_release_date: Option<NaiveDate> = None;

    for rec in &records {
        let Some(raw_version) = rec.first() else {
            continue;
        };
        if raw_version.trim().is_empty() {
            continue;
        }
        let Some(value) = rec.get(score_idx).and_then(|c| parse_score(c)) else {
            continue;
        };

        let cleaned = clean_model_version(raw_version);
        if cleaned.slug.is_empty() {
            continue;
        }

        if value > max_value {
            max_value = value;
        }

        // Raw run date (run/eval columns only) and release date, tracked apart.
        let row_run_date = run_date_idx
            .iter()
            .find_map(|&i| rec.get(i).and_then(|c| parse_date(c)));
        let row_release_date = release_idx.and_then(|i| rec.get(i).and_then(|c| parse_date(c)));

        if let Some(d) = row_run_date {
            if newest_run_date.is_none_or(|n| d > n) {
                newest_run_date = Some(d);
            }
        }
        if let Some(d) = row_release_date {
            if newest_release_date.is_none_or(|n| d > n) {
                newest_release_date = Some(d);
            }
        }

        // Per-row `ScoreCell.date`: best available run timestamp (run date, else
        // release date) — this is a per-model attribute, distinct from the
        // CSV-wide freshness signal computed above.
        let run_date = row_run_date.or(row_release_date);

        let release_date = release_idx
            .and_then(|i| rec.get(i).and_then(|c| parse_date(c)))
            .map(|d| d.format("%Y-%m-%d").to_string());

        // Epoch's `Organization` cell often lists several contributing orgs
        // comma-separated (e.g. "Google DeepMind,Google" or "DeepSeek,Peking
        // University"); the first is the primary developer. Use it alone for the
        // creator slug/name so the creators list isn't polluted with multi-org
        // composite slugs (which also never match the region/type tables).
        let org = org_idx
            .and_then(|i| rec.get(i))
            .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
            .unwrap_or_default();

        let entry = models.entry(cleaned.slug.clone());
        match entry {
            std::collections::btree_map::Entry::Vacant(v) => {
                let mut scores = BTreeMap::new();
                scores.insert(
                    stem.to_string(),
                    ScoreCell {
                        value,
                        date: run_date.map(|d| d.format("%Y-%m-%d").to_string()),
                        ci: None,
                        votes: None,
                    },
                );
                v.insert(BenchAccum {
                    row: ModelRow {
                        id: cleaned.slug.clone(),
                        name: raw_version.trim().to_string(),
                        display_name: cleaned.slug.clone(),
                        creator: slugify(&org),
                        creator_name: org,
                        release_date,
                        reasoning_status: cleaned.reasoning_status,
                        effort_level: cleaned.effort_level,
                        variant_tag: cleaned.variant_tag,
                        open_weights: None,
                        context_window: None,
                        supports_tools: None,
                        max_output: None,
                        scores,
                    },
                    best_value: value,
                    collapsed: false,
                });
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
                let acc = o.get_mut();
                if raw_version.trim() != acc.row.name {
                    acc.collapsed = true;
                }
                // Dedup: keep best (max, higher-is-better) score per model.
                let better = if higher_is_better {
                    value > acc.best_value
                } else {
                    value < acc.best_value
                };
                if better {
                    acc.best_value = value;
                    acc.row.scores.insert(
                        stem.to_string(),
                        ScoreCell {
                            value,
                            date: run_date.map(|d| d.format("%Y-%m-%d").to_string()),
                            ci: None,
                            votes: None,
                        },
                    );
                    // Prefer a non-empty release date / creator if this winning
                    // row carries one and the earlier didn't.
                    if acc.row.release_date.is_none() {
                        acc.row.release_date = release_idx
                            .and_then(|i| rec.get(i).and_then(|c| parse_date(c)))
                            .map(|d| d.format("%Y-%m-%d").to_string());
                    }
                }
            }
        }
    }

    // Freshness signal (prune + last_updated): run dates when the CSV has a
    // run/eval-date column, else release date.
    let newest_date = if has_run_date_col {
        newest_run_date
    } else {
        newest_release_date
    };

    // Known stems use their static kind; unknown stems infer from the data.
    let kind = known_kind.unwrap_or_else(|| infer_kind(max_value));

    let metric = MetricDef {
        id: stem.to_string(),
        label,
        kind,
        group,
        higher_is_better,
        last_updated: newest_date.map(|d| d.format("%Y-%m-%d").to_string()),
        description,
    };

    Ok(ParsedCsv {
        metric,
        models,
        newest_date,
    })
}

// ---------------------------------------------------------------------------
// Directory -> SourceFile assembly.
// ---------------------------------------------------------------------------

/// Outcome counts for the run summary.
#[derive(Default)]
struct Summary {
    pruned: usize,
    skipped: usize,
}

/// Build a `SourceFile` from a directory of Epoch CSVs, using `now` as the
/// "transform time" anchor for the 60-day prune. Factored out of `run` so tests
/// can pin `now` deterministically against fixtures.
fn build_source_file(csv_dir: &Path, now: NaiveDate, fetched_at: String) -> (SourceFile, Summary) {
    let mut summary = Summary::default();
    let mut parsed: Vec<ParsedCsv> = Vec::new();

    let entries = match std::fs::read_dir(csv_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("warning: cannot read dir {}: {e}", csv_dir.display());
            return (empty_source(fetched_at), summary);
        }
    };

    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "csv"))
        .collect();
    paths.sort();

    for path in paths {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(metric_id_from_stem)
            .unwrap_or_default();
        if stem.is_empty() {
            summary.skipped += 1;
            eprintln!("warning: skipping {} (empty stem)", path.display());
            continue;
        }

        match parse_csv(&stem, &path) {
            Ok(pc) => {
                // Prune: newest row date must be <= PRUNE_DAYS old.
                match pc.newest_date {
                    Some(d) if (now - d).num_days() <= PRUNE_DAYS => parsed.push(pc),
                    Some(d) => {
                        summary.pruned += 1;
                        eprintln!(
                            "pruned {} (newest row {} is {} days old)",
                            stem,
                            d,
                            (now - d).num_days()
                        );
                    }
                    None => {
                        summary.pruned += 1;
                        eprintln!("pruned {stem} (no parseable dates)");
                    }
                }
            }
            Err(e) => {
                summary.skipped += 1;
                eprintln!("warning: skipping CSV: {e}");
            }
        }
    }

    // Deterministic metric order: group (first-appearance via the static group
    // ordering), then label. We sort by (group_rank, label) so the output is
    // stable regardless of directory read order.
    parsed.sort_by(|a, b| {
        group_rank(&a.metric.group)
            .cmp(&group_rank(&b.metric.group))
            .then_with(|| a.metric.label.cmp(&b.metric.label))
    });

    let metrics: Vec<MetricDef> = parsed.iter().map(|p| p.metric.clone()).collect();

    // Merge per-CSV model rows into a single roster keyed by model id.
    let mut roster: BTreeMap<String, ModelRow> = BTreeMap::new();
    for pc in &parsed {
        for (id, acc) in &pc.models {
            let mut row = acc.row.clone();
            // A row that collapsed multiple raw variants describes no single
            // upstream row: normalize its identity to the cleaned base name and
            // drop the per-variant effort/tag metadata.
            if acc.collapsed {
                row.name = row.display_name.clone();
                row.effort_level = None;
                row.variant_tag = None;
            }
            match roster.entry(id.clone()) {
                std::collections::btree_map::Entry::Vacant(v) => {
                    v.insert(row);
                }
                std::collections::btree_map::Entry::Occupied(mut o) => {
                    let existing = o.get_mut();
                    // CSVs may have kept different raw variants for the same
                    // model id — that is the same collapse situation across
                    // files, normalized the same way below.
                    let name_conflict = existing.name != row.name;
                    // Fold this CSV's score(s) into the existing roster row.
                    for (mid, cell) in row.scores {
                        existing.scores.insert(mid, cell);
                    }
                    // Fill in any identity fields the first CSV left empty.
                    if existing.release_date.is_none() {
                        existing.release_date = row.release_date;
                    }
                    if existing.creator.is_empty() && !row.creator.is_empty() {
                        existing.creator = row.creator;
                        existing.creator_name = row.creator_name;
                    }
                    if existing.reasoning_status == ReasoningStatus::None {
                        existing.reasoning_status = row.reasoning_status;
                    }
                    if name_conflict {
                        existing.name = existing.display_name.clone();
                        existing.effort_level = None;
                        existing.variant_tag = None;
                    } else {
                        if existing.effort_level.is_none() {
                            existing.effort_level = row.effort_level;
                        }
                        if existing.variant_tag.is_none() {
                            existing.variant_tag = row.variant_tag;
                        }
                    }
                }
            }
        }
    }

    // Keep only models with >= 1 score after pruning.
    let mut models: Vec<ModelRow> = roster
        .into_values()
        .filter(|m| !m.scores.is_empty())
        .collect();

    // Deterministic output ordering: newest release_date first (None last),
    // then display_name ascending, then id ascending.
    models.sort_by(|a, b| {
        b.release_date
            .cmp(&a.release_date)
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| a.id.cmp(&b.id))
    });

    let source = SourceFile {
        source: SourceMeta {
            id: "epoch".to_string(),
            name: "Epoch AI".to_string(),
            url: "https://epoch.ai/benchmarks".to_string(),
            fetched_at,
            verified: true,
        },
        metrics,
        models,
    };

    (source, summary)
}

/// Group display order (first-appearance ranking for deterministic output).
fn group_rank(group: &str) -> u8 {
    match group {
        "Frontier" => 0,
        "Agentic" => 1,
        "Academic" => 2,
        _ => 3,
    }
}

/// Metric id from a CSV filename stem: snake_case, then drop a trailing
/// `_external` marker (Epoch tags externally-sourced benchmarks that way;
/// `terminalbench_external.csv` -> metric id `terminalbench`).
fn metric_id_from_stem(stem: &str) -> String {
    let snake = snake_case(stem);
    snake
        .strip_suffix("_external")
        .map(str::to_string)
        .unwrap_or(snake)
}

/// Snake-case a filename stem: lowercase, non-alnum -> `_`, collapse runs.
fn snake_case(s: &str) -> String {
    let mut out = String::new();
    let mut prev_us = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_us = false;
        } else if !out.is_empty() && !prev_us {
            out.push('_');
            prev_us = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// An empty Epoch `SourceFile` (used when the input dir is unreadable).
fn empty_source(fetched_at: String) -> SourceFile {
    SourceFile {
        source: SourceMeta {
            id: "epoch".to_string(),
            name: "Epoch AI".to_string(),
            url: "https://epoch.ai/benchmarks".to_string(),
            fetched_at,
            verified: true,
        },
        metrics: Vec::new(),
        models: Vec::new(),
    }
}

/// Two `SourceFile`s are "the same" for commit-if-changed purposes if they are
/// equal after normalizing `fetched_at` out (the timestamp changes every run).
fn unchanged(old: &SourceFile, new: &SourceFile) -> bool {
    let mut old_norm = old.clone();
    old_norm.source.fetched_at = new.source.fetched_at.clone();
    &old_norm == new
}

/// Transform a directory of Epoch benchmark CSVs into `output` (v2 `SourceFile`).
pub fn run(csv_dir: &Path, output: &Path) -> Result<(), String> {
    let now = Utc::now();
    let now_date = NaiveDate::from_ymd_opt(now.year(), now.month(), now.day())
        .ok_or_else(|| "invalid current date".to_string())?;

    let (source, summary) = build_source_file(csv_dir, now_date, Utc::now().to_rfc3339());

    if let Ok(existing_text) = std::fs::read_to_string(output) {
        if let Ok(existing) = serde_json::from_str::<SourceFile>(&existing_text) {
            if unchanged(&existing, &source) {
                println!(
                    "unchanged ({} metrics, {} pruned, {} skipped)",
                    source.metrics.len(),
                    summary.pruned,
                    summary.skipped
                );
                return Ok(());
            }
        }
    }

    let pretty =
        serde_json::to_string_pretty(&source).map_err(|e| format!("failed to serialize: {e}"))?;
    std::fs::write(output, pretty)
        .map_err(|e| format!("failed to write {}: {e}", output.display()))?;

    println!(
        "wrote {} ({} models, {} metrics, {} pruned, {} skipped)",
        output.display(),
        source.models.len(),
        source.metrics.len(),
        summary.pruned,
        summary.skipped
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/epoch")
    }

    /// Anchor "now" so prune boundaries are deterministic. Fixtures are dated
    /// relative to 2026-06-10 (newest fixture row = 2026-06-01, aider = stale).
    fn now() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()
    }

    fn build() -> SourceFile {
        let (sf, _) = build_source_file(
            &fixtures_dir(),
            now(),
            "2026-06-10T00:00:00+00:00".to_string(),
        );
        sf
    }

    fn build_with_summary() -> (SourceFile, Summary) {
        build_source_file(
            &fixtures_dir(),
            now(),
            "2026-06-10T00:00:00+00:00".to_string(),
        )
    }

    fn row<'a>(sf: &'a SourceFile, id: &str) -> &'a ModelRow {
        sf.models
            .iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("model {id} present"))
    }

    // ----- clean_model_version (suffix stripping) -----

    #[test]
    fn strips_effort_suffix() {
        let c = clean_model_version("claude-opus-4-7_high");
        assert_eq!(c.slug, "claude-opus-4-7");
        assert_eq!(c.effort_level.as_deref(), Some("high"));
        assert!(c.variant_tag.is_none());
    }

    #[test]
    fn xhigh_and_max_normalize_to_max() {
        assert_eq!(
            clean_model_version("m_xhigh").effort_level.as_deref(),
            Some("max")
        );
        assert_eq!(
            clean_model_version("m_max").effort_level.as_deref(),
            Some("max")
        );
    }

    #[test]
    fn strips_context_suffix_to_variant_tag() {
        let c = clean_model_version("claude-3-7-sonnet-20250219_32K");
        assert_eq!(c.slug, "claude-3-7-sonnet-20250219");
        assert_eq!(c.variant_tag.as_deref(), Some("32K"));
        assert!(c.effort_level.is_none());
    }

    #[test]
    fn thinking_suffix_sets_reasoning() {
        let c = clean_model_version("DeepSeek-V3.1_thinking");
        assert_eq!(c.slug, "DeepSeek-V3.1");
        assert_eq!(c.reasoning_status, ReasoningStatus::Reasoning);
    }

    #[test]
    fn placeholder_suffixes_stripped_without_metadata() {
        for raw in ["claude-opus-4-7_unknown", "x_none", "y_default"] {
            let c = clean_model_version(raw);
            assert!(c.effort_level.is_none());
            assert!(c.variant_tag.is_none());
            assert_eq!(c.reasoning_status, ReasoningStatus::None);
            assert!(!c.slug.ends_with("_unknown"));
        }
        assert_eq!(
            clean_model_version("claude-opus-4-7_unknown").slug,
            "claude-opus-4-7"
        );
    }

    #[test]
    fn param_size_and_other_tokens_preserved() {
        // `_7b`, `_res448`, `_mistral`, `_5` are identity, not suffixes.
        assert_eq!(clean_model_version("llama_7b").slug, "llama_7b");
        assert_eq!(clean_model_version("intern_res448").slug, "intern_res448");
        assert_eq!(clean_model_version("foo_mistral").slug, "foo_mistral");
        assert_eq!(clean_model_version("bar_5").slug, "bar_5");
    }

    #[test]
    fn no_suffix_left_untouched() {
        let c = clean_model_version("DeepSeek-R1");
        assert_eq!(c.slug, "DeepSeek-R1");
        assert!(c.effort_level.is_none());
    }

    // ----- parse_score (mean±stderr) -----

    #[test]
    fn parses_plain_number() {
        assert_eq!(parse_score("0.42"), Some(0.42));
        assert_eq!(parse_score(" 0.9460 "), Some(0.946));
    }

    #[test]
    fn parses_scorer_mean_stderr() {
        // `scorer:mean±stderr` -> mean.
        assert_eq!(parse_score("gpt-grader:0.79±0.018"), Some(0.79));
        assert_eq!(parse_score("0.83±0.015"), Some(0.83));
    }

    #[test]
    fn empty_or_garbage_score_is_none() {
        assert_eq!(parse_score(""), None);
        assert_eq!(parse_score("   "), None);
        assert_eq!(parse_score("vix"), None);
    }

    // ----- kind table -----

    #[test]
    fn eci_is_index_others_percentage() {
        let sf = build();
        let eci = sf
            .metrics
            .iter()
            .find(|m| m.id == "epoch_capabilities_index")
            .expect("eci metric present");
        assert_eq!(eci.kind, MetricKind::Index);
        for m in &sf.metrics {
            if m.id != "epoch_capabilities_index" {
                assert_eq!(
                    m.kind,
                    MetricKind::Percentage,
                    "{} should be Percentage",
                    m.id
                );
            }
        }
    }

    #[test]
    fn metric_labels_from_override_map() {
        let sf = build();
        let label = |id: &str| {
            sf.metrics
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.label.as_str())
        };
        assert_eq!(label("frontiermath"), Some("FrontierMath"));
        assert_eq!(label("swe_bench_verified"), Some("SWE-bench Verified"));
        assert_eq!(label("gpqa_diamond"), Some("GPQA Diamond"));
        assert_eq!(label("terminalbench"), Some("Terminal-Bench"));
    }

    #[test]
    fn humanize_fallback_for_unknown_stem() {
        assert_eq!(humanize("some_new_bench"), "Some New Bench");
        let m = resolve_metric_meta("some_new_bench");
        assert_eq!(m.label, "Some New Bench");
        assert_eq!(m.kind, None, "unknown stem kind is inferred from data");
        assert_eq!(m.group, "Academic");
        assert!(
            m.description.is_none(),
            "unknown stem has no curated description"
        );
    }

    #[test]
    fn every_known_stem_has_a_nonempty_description() {
        // Every stem the static registry recognizes must carry a curated
        // description; unknown stems legitimately have none.
        for stem in [
            "epoch_capabilities_index",
            "frontiermath",
            "frontiermath_tier_4",
            "arc_agi",
            "arc_agi_2",
            "hle",
            "swe_bench_verified",
            "terminalbench",
            "gso",
            "apex_agents",
            "posttrainbench",
            "weirdml",
            "aider_polyglot",
            "gdpval",
            "gpqa_diamond",
            "simpleqa_verified",
            "otis_mock_aime_2024_2025",
            "chess_puzzles",
            "simplebench",
        ] {
            let (_, _, _, description) =
                metric_meta(stem).unwrap_or_else(|| panic!("known stem {stem} present"));
            assert!(
                description.len() > 20,
                "known stem {stem} description too short"
            );
        }
    }

    #[test]
    fn fresh_fixture_metrics_carry_descriptions() {
        let sf = build();
        for m in &sf.metrics {
            // All fixtures use curated stems, so each should resolve a curated
            // description.
            let d = m
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("metric {} has no description", m.id));
            assert!(d.len() > 20, "metric {} description too short", m.id);
        }
    }

    #[test]
    fn infer_kind_from_value_range() {
        // Fraction scores -> Percentage.
        assert_eq!(infer_kind(0.85), MetricKind::Percentage);
        assert_eq!(infer_kind(1.0), MetricKind::Percentage);
        // Elo-scale arena scores -> Elo.
        assert_eq!(infer_kind(1566.85), MetricKind::Elo);
        // ECI / 0-100 scale -> Index.
        assert_eq!(infer_kind(159.3), MetricKind::Index);
        assert_eq!(infer_kind(72.1), MetricKind::Index);
    }

    #[test]
    fn metric_id_strips_external_suffix() {
        assert_eq!(
            metric_id_from_stem("terminalbench_external"),
            "terminalbench"
        );
        assert_eq!(metric_id_from_stem("hle_external"), "hle");
        // No `_external` -> unchanged.
        assert_eq!(
            metric_id_from_stem("swe_bench_verified"),
            "swe_bench_verified"
        );
        assert_eq!(metric_id_from_stem("gpqa_diamond"), "gpqa_diamond");
    }

    // ----- prune boundary -----

    #[test]
    fn prune_keeps_fresh_drops_stale() {
        let (sf, summary) = build_with_summary();
        let ids: Vec<&str> = sf.metrics.iter().map(|m| m.id.as_str()).collect();
        // Fresh fixtures survive.
        assert!(ids.contains(&"frontiermath"));
        assert!(ids.contains(&"swe_bench_verified"));
        assert!(ids.contains(&"terminalbench"));
        assert!(ids.contains(&"epoch_capabilities_index"));
        // aider's newest eval date (2025-10-03) is > 60 days -> pruned.
        assert!(!ids.contains(&"aider_polyglot"));
        assert!(summary.pruned >= 1, "aider should be counted as pruned");
    }

    // ----- malformed skip -----

    #[test]
    fn malformed_csv_skipped_and_counted() {
        let (sf, summary) = build_with_summary();
        // malformed.csv has no "Model version" header -> skipped, never a metric.
        assert!(!sf.metrics.iter().any(|m| m.id == "malformed"));
        assert!(
            summary.skipped >= 1,
            "malformed should be counted as skipped"
        );
    }

    // ----- internal + external merge -----

    #[test]
    fn merges_internal_and_external_rows() {
        let sf = build();
        // claude-opus-4-7 appears in frontiermath (internal), swe (internal),
        // terminalbench (external), eci (internal) -> one merged roster row.
        let m = row(&sf, "claude-opus-4-7");
        assert!(m.scores.contains_key("frontiermath"));
        assert!(m.scores.contains_key("swe_bench_verified"));
        assert!(m.scores.contains_key("terminalbench"));
        assert!(m.scores.contains_key("epoch_capabilities_index"));
    }

    // ----- best-score dedup -----

    #[test]
    fn dedup_keeps_max_score_and_its_date() {
        let sf = build();
        // swe: claude-opus-4-7_high=0.79 (2026-06-01) vs _medium=0.83 (2026-05-30)
        // -> collapse to claude-opus-4-7, keep max 0.83 + its date.
        let m = row(&sf, "claude-opus-4-7");
        let cell = &m.scores["swe_bench_verified"];
        assert_eq!(cell.value, 0.83);
        assert_eq!(cell.date.as_deref(), Some("2026-05-30"));
    }

    #[test]
    fn collapsed_variants_normalize_identity() {
        let sf = build();
        // swe collapses claude-opus-4-7_high and _medium into one row. The
        // surviving row must not carry any single variant's raw name or
        // per-variant metadata — name normalizes to the cleaned base and
        // effort/variant_tag are dropped (they describe no single upstream row).
        let m = row(&sf, "claude-opus-4-7");
        assert_eq!(m.name, "claude-opus-4-7");
        assert!(m.effort_level.is_none());
        assert!(m.variant_tag.is_none());
    }

    #[test]
    fn dedup_frontiermath_keeps_higher_of_two_efforts() {
        let sf = build();
        // frontiermath: gemini-3.5-flash_high=0.30 vs _low=0.18 -> 0.30.
        let m = row(&sf, "gemini-3.5-flash");
        assert_eq!(m.scores["frontiermath"].value, 0.30);
    }

    // ----- score column detection (categorical col before score) -----

    #[test]
    fn finds_score_after_categorical_column() {
        let sf = build();
        // terminalbench has `Agent` (categorical) at index 1; score is index 2.
        let m = row(&sf, "claude-opus-4-7");
        assert_eq!(m.scores["terminalbench"].value, 0.90);
    }

    // ----- last_updated = newest CSV row date -----

    #[test]
    fn metric_last_updated_is_newest_csv_date() {
        let sf = build();
        let swe = sf
            .metrics
            .iter()
            .find(|m| m.id == "swe_bench_verified")
            .unwrap();
        // newest run date in swe fixture = 2026-06-01.
        assert_eq!(swe.last_updated.as_deref(), Some("2026-06-01"));
    }

    // ----- groups 3-6 hib for radar -----

    #[test]
    fn radar_groups_have_three_to_six_hib_metrics() {
        let sf = build();
        let mut by_group: BTreeMap<&str, usize> = BTreeMap::new();
        for m in &sf.metrics {
            if m.higher_is_better {
                *by_group.entry(m.group.as_str()).or_default() += 1;
            }
        }
        // Every group with >=3 hib metrics is a radar group; assert none
        // exceeds 6 (only first 6 are axes, but we keep them within range by
        // design so every metric is reachable).
        for (group, count) in &by_group {
            if *count >= 3 {
                assert!(
                    *count <= 6,
                    "radar group {group} has {count} hib metrics (>6)"
                );
            }
        }
        // Fixture set: Frontier = {frontiermath, hle, epoch_capabilities_index}
        // = 3 -> radar-eligible and within 3-6. The real-data group sizing
        // (Frontier 6 / Agentic 6 / Academic 5) is asserted in the report.
        assert_eq!(
            by_group.get("Frontier").copied(),
            Some(3),
            "Frontier should have 3 hib metrics in the fixtures"
        );
    }

    #[test]
    fn all_groups_are_known() {
        let sf = build();
        for m in &sf.metrics {
            assert!(
                ["Frontier", "Agentic", "Academic"].contains(&m.group.as_str()),
                "metric {} has unexpected group {}",
                m.id,
                m.group
            );
        }
    }

    // ----- identity fields -----

    #[test]
    fn org_empty_yields_empty_creator() {
        let sf = build();
        // Epoch export has empty Organization -> empty creator strings.
        for m in &sf.models {
            assert_eq!(m.creator, "");
            assert_eq!(m.creator_name, "");
        }
    }

    #[test]
    fn multi_org_collapses_to_primary_org() {
        // Epoch lists every contributing org comma-separated; only the first
        // (primary developer) becomes the creator, so the creators list isn't
        // polluted with composite multi-org slugs.
        let dir = std::env::temp_dir().join(format!("epoch_primary_org_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_bench.csv");
        std::fs::write(
            &path,
            "Model version,Score,Organization,Release date\n\
             test-model,0.5,\"Google DeepMind,Google\",2026-05-01\n",
        )
        .unwrap();
        let parsed = parse_csv("test_bench", &path).expect("parse");
        let accum = parsed.models.get("test-model").expect("row present");
        assert_eq!(accum.row.creator, "google-deepmind");
        assert_eq!(accum.row.creator_name, "Google DeepMind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_date_carried_from_csv() {
        let sf = build();
        let m = row(&sf, "claude-opus-4-7");
        assert_eq!(m.release_date.as_deref(), Some("2026-04-16"));
    }

    #[test]
    fn open_weights_and_context_window_none() {
        let sf = build();
        for m in &sf.models {
            assert!(m.open_weights.is_none());
            assert!(m.context_window.is_none());
        }
    }

    #[test]
    fn variant_tag_preserved_through_merge() {
        let sf = build();
        // claude-3-7-sonnet-20250219_32K only appears in eci; its 32K tag should
        // survive into the merged roster.
        let m = row(&sf, "claude-3-7-sonnet-20250219");
        assert_eq!(m.variant_tag.as_deref(), Some("32K"));
        assert!(m.scores.contains_key("epoch_capabilities_index"));
    }

    // ----- deterministic ordering -----

    #[test]
    fn metrics_ordered_by_group_then_label() {
        let sf = build();
        // Frontier group (rank 0) before Agentic (1) before Academic (2).
        let ranks: Vec<u8> = sf.metrics.iter().map(|m| group_rank(&m.group)).collect();
        let mut sorted = ranks.clone();
        sorted.sort_unstable();
        assert_eq!(ranks, sorted, "metrics not group-ordered");
    }

    #[test]
    fn models_ordered_newest_release_then_name() {
        let sf = build();
        // release_date descending (None last), then display_name ascending.
        let mut prev: Option<&Option<String>> = None;
        for m in &sf.models {
            if let Some(p) = prev {
                assert!(
                    p >= &m.release_date,
                    "models not sorted by release_date desc: {p:?} then {:?}",
                    m.release_date
                );
            }
            prev = Some(&m.release_date);
        }
    }

    #[test]
    fn models_keep_only_with_scores() {
        let sf = build();
        for m in &sf.models {
            assert!(!m.scores.is_empty(), "model {} has no scores", m.id);
        }
    }

    // ----- if-changed -----

    #[test]
    fn unchanged_ignores_fetched_at() {
        let mut a = build();
        let mut b = a.clone();
        a.source.fetched_at = "2026-01-01T00:00:00+00:00".to_string();
        b.source.fetched_at = "2099-12-31T23:59:59+00:00".to_string();
        assert!(unchanged(&a, &b));

        b.models[0].name = "Different".to_string();
        assert!(!unchanged(&a, &b));
    }

    #[test]
    fn if_changed_second_run_leaves_file_untouched() {
        let dir = std::env::temp_dir().join(format!(
            "transform_epoch_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let output = dir.join("epoch.json");

        // Write a SourceFile, then write it again with only fetched_at changed.
        let (mut sf, _) = build_with_summary();
        sf.source.fetched_at = "2026-06-10T00:00:00+00:00".to_string();
        std::fs::write(&output, serde_json::to_string_pretty(&sf).unwrap()).unwrap();
        let first_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();

        // Simulate run's if-changed: new source equal except fetched_at.
        let mut new_sf = sf.clone();
        new_sf.source.fetched_at = "2026-06-10T12:00:00+00:00".to_string();
        let existing: SourceFile =
            serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
        if !unchanged(&existing, &new_sf) {
            std::fs::write(&output, serde_json::to_string_pretty(&new_sf).unwrap()).unwrap();
        }
        let second_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();
        assert_eq!(first_mtime, second_mtime, "unchanged should not rewrite");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serialize_deserialize_round_trip() {
        let sf = build();
        let json = serde_json::to_string_pretty(&sf).expect("serialize");
        let back: SourceFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sf, back);
    }

    #[test]
    fn source_meta_is_epoch() {
        let sf = build();
        assert_eq!(sf.source.id, "epoch");
        assert_eq!(sf.source.name, "Epoch AI");
        assert!(sf.source.verified);
    }
}
