use serde::{Deserialize, Serialize};

use super::schema::{parse_name_metadata, ReasoningStatus};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BenchmarkEntry {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub creator_id: String,
    #[serde(default)]
    pub creator_name: String,
    #[serde(default)]
    pub release_date: Option<String>,
    pub intelligence_index: Option<f64>,
    pub coding_index: Option<f64>,
    pub math_index: Option<f64>,
    pub mmlu_pro: Option<f64>,
    pub gpqa: Option<f64>,
    pub hle: Option<f64>,
    pub livecodebench: Option<f64>,
    pub scicode: Option<f64>,
    pub ifbench: Option<f64>,
    pub lcr: Option<f64>,
    pub terminalbench_hard: Option<f64>,
    pub tau2: Option<f64>,
    pub math_500: Option<f64>,
    #[serde(default)]
    pub aime: Option<f64>,
    pub aime_25: Option<f64>,
    pub output_tps: Option<f64>,
    pub ttft: Option<f64>,
    #[serde(default)]
    pub ttfat: Option<f64>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub price_blended: Option<f64>,
    #[serde(default)]
    pub reasoning_status: ReasoningStatus,
    #[serde(default)]
    pub effort_level: Option<String>,
    #[serde(default)]
    pub variant_tag: Option<String>,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub tool_call: Option<bool>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_output: Option<u64>,
}

impl BenchmarkEntry {
    pub fn parse_metadata(&mut self) {
        let parsed = parse_name_metadata(&self.name, self.reasoning_status.clone());
        self.display_name = parsed.display_name;
        self.reasoning_status = parsed.reasoning_status;
        self.effort_level = parsed.effort_level;
        self.variant_tag = parsed.variant_tag;
    }
}

pub struct BenchmarkStore {
    entries: Vec<BenchmarkEntry>,
}

impl BenchmarkStore {
    pub fn entries(&self) -> &[BenchmarkEntry] {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut [BenchmarkEntry] {
        &mut self.entries
    }

    /// Create an empty store (no benchmark data loaded yet).
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Create a store from runtime-fetched or cached entries.
    pub fn from_entries(mut entries: Vec<BenchmarkEntry>) -> Self {
        for entry in &mut entries {
            entry.parse_metadata();
        }
        Self { entries }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ReasoningFilter {
    #[default]
    All,
    Reasoning,
    NonReasoning,
}

impl ReasoningFilter {
    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::Reasoning,
            Self::Reasoning => Self::NonReasoning,
            Self::NonReasoning => Self::All,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::All => "",
            Self::Reasoning => "Reasoning",
            Self::NonReasoning => "Non-reasoning",
        }
    }

    pub fn matches(&self, entry: &BenchmarkEntry) -> bool {
        match self {
            Self::All => true,
            Self::Reasoning => matches!(
                entry.reasoning_status,
                ReasoningStatus::Reasoning | ReasoningStatus::Adaptive
            ),
            Self::NonReasoning => {
                matches!(entry.reasoning_status, ReasoningStatus::NonReasoning)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(overrides: impl FnOnce(&mut BenchmarkEntry)) -> BenchmarkEntry {
        let mut entry = BenchmarkEntry {
            id: String::new(),
            name: "test".to_string(),
            slug: "test".to_string(),
            creator: "openai".to_string(),
            creator_id: String::new(),
            creator_name: "OpenAI".to_string(),
            release_date: Some("2025-01-01".to_string()),
            intelligence_index: Some(1.0),
            coding_index: None,
            math_index: None,
            mmlu_pro: None,
            gpqa: None,
            hle: None,
            livecodebench: None,
            scicode: None,
            ifbench: None,
            lcr: None,
            terminalbench_hard: None,
            tau2: None,
            math_500: None,
            aime: None,
            aime_25: None,
            output_tps: None,
            ttft: None,
            ttfat: None,
            price_input: None,
            price_output: None,
            price_blended: None,
            reasoning_status: ReasoningStatus::None,
            effort_level: None,
            variant_tag: None,
            display_name: String::new(),
            tool_call: None,
            context_window: None,
            max_output: None,
        };
        overrides(&mut entry);
        entry
    }

    #[test]
    fn test_empty_store() {
        let store = BenchmarkStore::empty();
        assert!(store.entries().is_empty());
    }

    #[test]
    fn test_from_entries() {
        let entries = vec![make_entry(|e| {
            e.name = "cached-model".to_string();
            e.ttfat = Some(5.0);
        })];
        let store = BenchmarkStore::from_entries(entries);
        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.entries()[0].name, "cached-model");
        assert_eq!(store.entries()[0].ttfat, Some(5.0));
    }

    #[test]
    fn test_parse_metadata_reasoning() {
        let mut e = make_entry(|e| e.name = "Claude 4.5 Haiku (Reasoning)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(e.display_name, "Claude 4.5 Haiku");
        assert!(e.variant_tag.is_none());
    }

    #[test]
    fn test_parse_metadata_non_reasoning() {
        let mut e =
            make_entry(|e| e.name = "Claude 4.5 Haiku (Non-reasoning, Low Effort)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::NonReasoning);
        assert_eq!(e.effort_level, Some("low".to_string()));
        assert_eq!(e.display_name, "Claude 4.5 Haiku");
    }

    #[test]
    fn test_parse_metadata_adaptive() {
        let mut e =
            make_entry(|e| e.name = "Claude Opus 4.6 (Adaptive Reasoning, Max Effort)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::Adaptive);
        assert_eq!(e.effort_level, Some("max".to_string()));
        assert_eq!(e.display_name, "Claude Opus 4.6");
    }

    #[test]
    fn test_parse_metadata_date_dropped() {
        let mut e = make_entry(|e| e.name = "GPT-4o (Nov '24)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::None);
        assert!(e.variant_tag.is_none());
        assert_eq!(e.display_name, "GPT-4o");
    }

    #[test]
    fn test_parse_metadata_variant_tag() {
        let mut e = make_entry(|e| e.name = "Some Model (Preview)".to_string());
        e.parse_metadata();
        assert_eq!(e.variant_tag, Some("Preview".to_string()));
        assert_eq!(e.display_name, "Some Model");
    }

    #[test]
    fn test_parse_metadata_thinking() {
        let mut e = make_entry(|e| e.name = "Gemini 2.0 Flash (Thinking)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(e.display_name, "Gemini 2.0 Flash");
    }

    #[test]
    fn test_reasoning_filter_matches() {
        let mut reasoning_entry = make_entry(|e| e.name = "Test (Reasoning)".to_string());
        reasoning_entry.parse_metadata();

        let mut adaptive_entry = make_entry(|e| e.name = "Test (Adaptive Reasoning)".to_string());
        adaptive_entry.parse_metadata();

        let mut nr_entry = make_entry(|e| e.name = "Test (Non-reasoning)".to_string());
        nr_entry.parse_metadata();

        let mut plain_entry = make_entry(|e| e.name = "Test".to_string());
        plain_entry.parse_metadata();

        let all = ReasoningFilter::All;
        let reasoning = ReasoningFilter::Reasoning;
        let non_reasoning = ReasoningFilter::NonReasoning;

        assert!(all.matches(&reasoning_entry));
        assert!(all.matches(&plain_entry));

        assert!(reasoning.matches(&reasoning_entry));
        assert!(reasoning.matches(&adaptive_entry));
        assert!(!reasoning.matches(&nr_entry));
        assert!(!reasoning.matches(&plain_entry));

        assert!(non_reasoning.matches(&nr_entry));
        assert!(!non_reasoning.matches(&reasoning_entry));
        assert!(!non_reasoning.matches(&plain_entry));
    }

    #[test]
    fn test_parse_metadata_effort_implies_reasoning() {
        let mut e = make_entry(|e| e.name = "o4-mini (high)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(e.effort_level, Some("high".to_string()));
        assert_eq!(e.display_name, "o4-mini");
    }

    #[test]
    fn test_parse_metadata_reasoning_in_base_name() {
        let mut e = make_entry(|e| e.name = "Grok 3 mini Reasoning (high)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(e.effort_level, Some("high".to_string()));
        assert_eq!(e.display_name, "Grok 3 mini Reasoning");
    }

    #[test]
    fn test_parse_metadata_thinking_in_base_name() {
        let mut e =
            make_entry(|e| e.name = "Gemini 2.0 Flash Thinking Experimental (Jan '25)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::Reasoning);
        assert_eq!(e.display_name, "Gemini 2.0 Flash Thinking Experimental");
    }

    #[test]
    fn test_parse_metadata_full_month_date() {
        let mut e = make_entry(|e| e.name = "Claude 3.5 Sonnet (June '24)".to_string());
        e.parse_metadata();
        assert_eq!(e.reasoning_status, ReasoningStatus::None);
        assert!(e.variant_tag.is_none());
        assert_eq!(e.display_name, "Claude 3.5 Sonnet");
    }

    #[test]
    fn test_parse_metadata_date_with_variant() {
        let mut e = make_entry(|e| {
            e.name = "GPT-4o (March 2025, chatgpt-4o-latest)".to_string();
        });
        e.parse_metadata();
        assert_eq!(e.variant_tag, Some("chatgpt-4o-latest".to_string()));
        assert_eq!(e.display_name, "GPT-4o");
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
}
