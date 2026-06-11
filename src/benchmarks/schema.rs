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
    /// Curated 1-2 sentence explanation of what the benchmark tests, shown in
    /// the glossary popup. Set by the transforms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    /// Tool-use capability, backfilled at runtime from a models.dev match
    /// (never emitted by the transforms — None at serialize time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    /// Max output tokens, backfilled at runtime from a models.dev match
    /// (never emitted by the transforms — None at serialize time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
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
    /// Sample size behind the score (Arena: head-to-head vote count for this
    /// board). A confidence signal; omitted by sources that don't report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub votes: Option<u64>,
}

// The name-parsing facility below (PAREN_RE/DATE_RE/EFFORT_ONLY_RE,
// extract_effort, ParsedName, parse_name_metadata) is LIVE in the transform
// binary, which `#[path]`-includes this very file and calls
// `parse_name_metadata` while building `data/v2/aa.json`. In the `models`
// binary the v2 data arrives pre-parsed, so these items are exercised only by
// the unit tests below — hence the item-level `dead_code` allows.
// TODO(phase-3+): if the app ever parses raw names at runtime (e.g. a source
// without a transform pass), these allows can be dropped.

// Matches a parenthetical group: captures the content inside parens.
#[allow(dead_code)]
static PAREN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(([^)]*)\)").expect("valid regex"));

// Matches a date like "Dec '24", "Feb 2026", "June '24", "March 2025"
#[allow(dead_code)]
static DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(January|February|March|April|May|June|July|August|September|October|November|December|Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s*'?\s*\d{2,4}$")
        .expect("valid regex")
});

// Matches a standalone effort keyword (entire content is just the keyword)
#[allow(dead_code)]
static EFFORT_ONLY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(high|low|medium|xhigh|minimal)(\s+effort)?$").expect("valid regex")
});

#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct ParsedName {
    pub display_name: String,
    pub reasoning_status: ReasoningStatus,
    pub effort_level: Option<String>,
    pub variant_tag: Option<String>,
}

/// Pure metadata-parsing function shared by the app and the transform binary.
/// `initial_reasoning` seeds the status so the "only set if None" branches
/// (pure-effort, base-name fallback) preserve a caller-provided value.
#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::{parse_name_metadata, ReasoningStatus};

    /// Parse a raw name with no caller-provided reasoning seed (the AA pipeline
    /// passes `ReasoningStatus::None`).
    fn parse(name: &str) -> super::ParsedName {
        parse_name_metadata(name, ReasoningStatus::None)
    }

    #[test]
    fn test_parse_reasoning() {
        let p = parse("Claude 4.5 Haiku (Reasoning)");
        assert_eq!(p.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(p.display_name, "Claude 4.5 Haiku");
        assert!(p.variant_tag.is_none());
    }

    #[test]
    fn test_parse_non_reasoning() {
        let p = parse("Claude 4.5 Haiku (Non-reasoning, Low Effort)");
        assert_eq!(p.reasoning_status, ReasoningStatus::NonReasoning);
        assert_eq!(p.effort_level, Some("low".to_string()));
        assert_eq!(p.display_name, "Claude 4.5 Haiku");
    }

    #[test]
    fn test_parse_adaptive() {
        let p = parse("Claude Opus 4.6 (Adaptive Reasoning, Max Effort)");
        assert_eq!(p.reasoning_status, ReasoningStatus::Adaptive);
        assert_eq!(p.effort_level, Some("max".to_string()));
        assert_eq!(p.display_name, "Claude Opus 4.6");
    }

    #[test]
    fn test_parse_date_dropped() {
        let p = parse("GPT-4o (Nov '24)");
        assert_eq!(p.reasoning_status, ReasoningStatus::None);
        assert!(p.variant_tag.is_none());
        assert_eq!(p.display_name, "GPT-4o");
    }

    #[test]
    fn test_parse_variant_tag() {
        let p = parse("Some Model (Preview)");
        assert_eq!(p.variant_tag, Some("Preview".to_string()));
        assert_eq!(p.display_name, "Some Model");
    }

    #[test]
    fn test_parse_thinking() {
        let p = parse("Gemini 2.0 Flash (Thinking)");
        assert_eq!(p.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(p.display_name, "Gemini 2.0 Flash");
    }

    #[test]
    fn test_parse_effort_implies_reasoning() {
        let p = parse("o4-mini (high)");
        assert_eq!(p.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(p.effort_level, Some("high".to_string()));
        assert_eq!(p.display_name, "o4-mini");
    }

    #[test]
    fn test_parse_reasoning_in_base_name() {
        let p = parse("Grok 3 mini Reasoning (high)");
        assert_eq!(p.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(p.effort_level, Some("high".to_string()));
        assert_eq!(p.display_name, "Grok 3 mini Reasoning");
    }

    #[test]
    fn test_parse_thinking_in_base_name() {
        let p = parse("Gemini 2.0 Flash Thinking Experimental (Jan '25)");
        assert_eq!(p.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(p.display_name, "Gemini 2.0 Flash Thinking Experimental");
    }

    #[test]
    fn test_parse_full_month_date() {
        let p = parse("Claude 3.5 Sonnet (June '24)");
        assert_eq!(p.reasoning_status, ReasoningStatus::None);
        assert!(p.variant_tag.is_none());
        assert_eq!(p.display_name, "Claude 3.5 Sonnet");
    }

    #[test]
    fn test_parse_date_with_variant() {
        let p = parse("GPT-4o (March 2025, chatgpt-4o-latest)");
        assert_eq!(p.variant_tag, Some("chatgpt-4o-latest".to_string()));
        assert_eq!(p.display_name, "GPT-4o");
    }

    #[test]
    fn test_parse_initial_reasoning_preserved() {
        // A caller-provided reasoning status survives when the name carries no
        // reasoning/effort/thinking signal (base-name fallback path).
        let p = parse_name_metadata("GPT-4o", ReasoningStatus::Reasoning);
        assert_eq!(p.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(p.display_name, "GPT-4o");
    }
}
