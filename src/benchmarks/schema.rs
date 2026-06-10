//! Shared v2 data schema for multi-source benchmark data.
//! SELF-CONTAINED on purpose: this file is compiled both as
//! crate::benchmarks::schema and, via #[path] include, into the
//! transform bin (the crate has no lib target). Do not reference
//! other crate modules from here.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum ReasoningStatus {
    #[default]
    None,
    Reasoning,
    NonReasoning,
    Adaptive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub source: SourceMeta,
    pub metrics: Vec<MetricDef>,
    pub models: Vec<ModelRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceMeta {
    pub id: String,
    pub name: String,
    pub url: String,
    pub fetched_at: String,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDef {
    pub id: String,
    pub label: String,
    pub kind: MetricKind,
    pub group: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub higher_is_better: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

fn default_true() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Percentage,
    Index,
    Elo,
    TokensPerSec,
    Seconds,
    #[serde(rename = "usd_per_mtok")]
    UsdPerMTok,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRow {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub creator: String,
    pub creator_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "reasoning_is_none")]
    pub reasoning_status: ReasoningStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    pub scores: BTreeMap<String, ScoreCell>,
}

fn reasoning_is_none(r: &ReasoningStatus) -> bool {
    *r == ReasoningStatus::None
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreCell {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci: Option<f64>,
}

// Matches a parenthetical group: captures the content inside parens.
static PAREN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(([^)]*)\)").expect("valid regex"));

// Matches a date like "Dec '24", "Feb 2026", "June '24", "March 2025"
static DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(January|February|March|April|May|June|July|August|September|October|November|December|Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s*'?\s*\d{2,4}$")
        .expect("valid regex")
});

// Matches a standalone effort keyword (entire content is just the keyword)
static EFFORT_ONLY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(high|low|medium|xhigh|minimal)(\s+effort)?$").expect("valid regex")
});

fn extract_effort(content: &str) -> Option<String> {
    // content is like "Max Effort" or "Low Effort" or just "High"
    let lower = content.to_lowercase();
    if lower.contains("max") || lower.contains("xhigh") {
        Some("max".to_string())
    } else if lower.contains("high") {
        Some("high".to_string())
    } else if lower.contains("medium") || lower.contains("med") {
        Some("medium".to_string())
    } else if lower.contains("low") {
        Some("low".to_string())
    } else if lower.contains("minimal") {
        Some("minimal".to_string())
    } else {
        None
    }
}

/// Parsed metadata extracted from a raw model name.
pub struct ParsedName {
    pub display_name: String,
    pub reasoning_status: ReasoningStatus,
    pub effort_level: Option<String>,
    pub variant_tag: Option<String>,
}

/// Pure metadata-parsing function shared by the app store and the transform
/// binary. `initial_reasoning` seeds the status so the "only set if None"
/// branches (pure-effort, base-name fallback) preserve a caller-provided value.
pub fn parse_name_metadata(name: &str, initial_reasoning: ReasoningStatus) -> ParsedName {
    let mut reasoning_status = initial_reasoning;
    let mut effort_level: Option<String> = None;
    let mut variant_parts: Vec<String> = Vec::new();

    for cap in PAREN_RE.captures_iter(name) {
        let content = cap[1].trim();

        let lower = content.to_lowercase();

        // Date pattern — drop date part; if comma-separated, keep non-date parts as variant
        if DATE_RE.is_match(content) {
            continue;
        }
        if let Some(comma_pos) = content.find(',') {
            let first = content[..comma_pos].trim();
            if DATE_RE.is_match(first) {
                let rest = content[comma_pos + 1..].trim();
                if !rest.is_empty() {
                    variant_parts.push(rest.to_string());
                }
                continue;
            }
        }

        // Adaptive Reasoning (may have comma-separated effort)
        if lower.contains("adaptive reasoning") {
            reasoning_status = ReasoningStatus::Adaptive;
            if let Some(comma_pos) = content.find(',') {
                let effort_part = content[comma_pos + 1..].trim();
                effort_level = extract_effort(effort_part);
            }
            continue;
        }

        // Non-reasoning (check before "Reasoning" to avoid substring match)
        if lower.contains("non-reasoning") {
            reasoning_status = ReasoningStatus::NonReasoning;
            if let Some(comma_pos) = content.find(',') {
                let effort_part = content[comma_pos + 1..].trim();
                effort_level = extract_effort(effort_part);
            }
            continue;
        }

        // Reasoning
        if lower.contains("reasoning") {
            reasoning_status = ReasoningStatus::Reasoning;
            if let Some(comma_pos) = content.find(',') {
                let effort_part = content[comma_pos + 1..].trim();
                effort_level = extract_effort(effort_part);
            }
            continue;
        }

        // Thinking (older AA naming)
        if lower.contains("thinking") {
            reasoning_status = ReasoningStatus::Reasoning;
            continue;
        }

        // Pure effort keyword — implies reasoning (effort controls thinking budget)
        if EFFORT_ONLY_RE.is_match(content) {
            effort_level = extract_effort(content);
            if reasoning_status == ReasoningStatus::None {
                reasoning_status = ReasoningStatus::Reasoning;
            }
            continue;
        }

        // Everything else -> variant_tag
        variant_parts.push(content.to_string());
    }

    let variant_tag = if !variant_parts.is_empty() {
        Some(variant_parts.join(", "))
    } else {
        None
    };

    // Build display_name by stripping all (...) groups
    let stripped = PAREN_RE.replace_all(name, "");
    let trimmed = stripped.trim().to_string();
    let display_name = if trimmed.is_empty() {
        name.to_string()
    } else {
        trimmed
    };

    // Check base name (outside parens) for reasoning/thinking keywords
    if reasoning_status == ReasoningStatus::None {
        let base_lower = display_name.to_lowercase();
        if base_lower.contains("reasoning") || base_lower.contains("thinking") {
            reasoning_status = ReasoningStatus::Reasoning;
        }
    }

    ParsedName {
        display_name,
        reasoning_status,
        effort_level,
        variant_tag,
    }
}
