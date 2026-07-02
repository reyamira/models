use crate::formatting;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub npm: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, Model>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub temperature: bool,
    #[serde(default)]
    pub modalities: Option<Modalities>,
    #[serde(default)]
    pub cost: Option<Cost>,
    #[serde(default)]
    pub limit: Option<Limits>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub last_updated: Option<String>,
    #[serde(default)]
    pub knowledge: Option<String>,
    #[serde(default)]
    pub open_weights: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// `Option` rather than `bool`: only ~49% of models.dev entries carry this
    /// key, so absent must stay distinguishable from an explicit `false`.
    #[serde(default)]
    pub structured_output: Option<bool>,
    #[serde(default)]
    pub reasoning_options: Vec<ReasoningOption>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub reasoning: Option<f64>,
    #[serde(default)]
    pub input_audio: Option<f64>,
    #[serde(default)]
    pub output_audio: Option<f64>,
    #[serde(default)]
    pub tiers: Vec<CostTier>,
}

/// A single reasoning-mode option (models.dev `reasoning_options[]`). Modeled
/// permissively — `type` stays a raw string so a future tag beyond the current
/// `budget_tokens`/`effort`/`toggle` set never fails deserialization.
/// `Serialize` so the CLI can emit reasoning_options in `--json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReasoningOption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Effort levels (e.g. `["low","medium","high"]`). May contain `null` — the
    /// "off"/disable choice — which is dropped for display.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Option<String>>,
}

/// A pricing tier (models.dev `cost.tiers[]`) — e.g. higher rates above a
/// context-size threshold. All fields optional for forward-compat.
/// `Serialize` so the CLI can emit tiers in `--json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CostTier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<TierSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TierSpec {
    /// Always `"context"` today; kept as a raw string for forward-compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub input: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Modalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

impl Model {
    /// Returns true if this model outputs text (or has no modalities specified).
    /// Non-text models (image gen, video gen, embeddings) return false.
    #[cfg(test)]
    pub fn is_text_model(&self) -> bool {
        match &self.modalities {
            Some(m) => m.output.iter().any(|o| o == "text"),
            None => true,
        }
    }

    pub fn context_str(&self) -> String {
        self.limit
            .as_ref()
            .and_then(|l| l.context)
            .map(formatting::format_tokens)
            .unwrap_or_else(|| formatting::EM_DASH.to_string())
    }

    pub fn output_str(&self) -> String {
        self.limit
            .as_ref()
            .and_then(|l| l.output)
            .map(formatting::format_tokens)
            .unwrap_or_else(|| formatting::EM_DASH.to_string())
    }

    pub fn input_limit_str(&self) -> String {
        self.limit
            .as_ref()
            .and_then(|l| l.input)
            .map(formatting::format_tokens)
            .unwrap_or_else(|| formatting::EM_DASH.to_string())
    }

    pub fn is_free(&self) -> bool {
        match &self.cost {
            None => true,
            Some(c) => c.input.unwrap_or(0.0) == 0.0 && c.output.unwrap_or(0.0) == 0.0,
        }
    }

    pub fn cost_str(&self) -> String {
        match &self.cost {
            Some(c) => {
                let input = c
                    .input
                    .map(|v| format!("${}", v))
                    .unwrap_or(formatting::EM_DASH.to_string());
                let output = c
                    .output
                    .map(|v| format!("${}", v))
                    .unwrap_or(formatting::EM_DASH.to_string());
                format!("{}/{}", input, output)
            }
            None => format!("{}/{}", formatting::EM_DASH, formatting::EM_DASH),
        }
    }

    /// Compact cost string for list columns (rounded to 1 decimal place).
    pub fn cost_short(value: Option<f64>) -> String {
        match value {
            Some(v) if v >= 100.0 => format!("${:.0}", v),
            Some(v) if v >= 1.0 => format!("${:.1}", v),
            Some(v) if v >= 0.01 => format!("${:.2}", v),
            Some(v) => format!("${:.3}", v),
            None => "\u{2014}".to_string(),
        }
    }

    pub fn capabilities_str(&self) -> String {
        let mut caps = Vec::new();
        if self.reasoning {
            caps.push("reasoning");
        }
        if self.tool_call {
            caps.push("tools");
        }
        if self.attachment {
            caps.push("files");
        }
        if self.temperature {
            caps.push("temperature");
        }
        if self.structured_output == Some(true) {
            caps.push("structured");
        }
        if caps.is_empty() {
            formatting::EM_DASH.to_string()
        } else {
            caps.join(", ")
        }
    }

    pub fn modalities_str(&self) -> String {
        match &self.modalities {
            Some(m) => {
                let input = if m.input.is_empty() {
                    "text".to_string()
                } else {
                    m.input.join(", ")
                };
                let output = if m.output.is_empty() {
                    "text".to_string()
                } else {
                    m.output.join(", ")
                };
                format!("{} -> {}", input, output)
            }
            None => "text -> text".to_string(),
        }
    }
}

/// `(label, value)` pairs for a model's **reasoning controls** — the API knobs
/// for controlling reasoning — one pair per `reasoning_options` entry, shaped to
/// slot into the same `Label: value` layout as the other capabilities:
/// `("Budget", "0–24.6k")`, `("Effort", "low, medium, high")`, `("Toggle", "Yes")`.
/// Budget ranges are rounded with `format_tokens` (Limits number style); effort
/// `null` levels (the "off" choice) are dropped; an unknown future type keeps its
/// (capitalized) raw name with value `"Yes"` (permissive — never fails). Empty
/// when there are no `reasoning_options`. Shared by the TUI detail panel and the
/// CLI `models show`.
pub fn reasoning_controls(opts: &[ReasoningOption]) -> Vec<(String, String)> {
    opts.iter()
        .map(|opt| {
            let raw = opt.r#type.as_deref().unwrap_or("reasoning");
            match raw {
                "budget_tokens" => {
                    let v = match (opt.min, opt.max) {
                        (Some(min), Some(max)) => format!(
                            "{}–{}",
                            formatting::format_tokens(min as u64),
                            formatting::format_tokens(max as u64)
                        ),
                        (None, Some(max)) => format!("≤{}", formatting::format_tokens(max as u64)),
                        (Some(min), None) => format!("≥{}", formatting::format_tokens(min as u64)),
                        (None, None) => "Yes".to_string(),
                    };
                    ("Budget".to_string(), v)
                }
                "effort" => {
                    let levels: Vec<&str> =
                        opt.values.iter().filter_map(|v| v.as_deref()).collect();
                    let v = if levels.is_empty() {
                        "Yes".to_string()
                    } else {
                        levels.join(", ")
                    };
                    ("Effort".to_string(), v)
                }
                "toggle" => ("Toggle".to_string(), "Yes".to_string()),
                other => (capitalize(other), "Yes".to_string()),
            }
        })
        .collect()
}

/// Capitalize the first character (for unknown reasoning-control type names).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub type ProvidersMap = HashMap<String, Provider>;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(output_modalities: Option<Vec<&str>>) -> Model {
        Model {
            id: "test".into(),
            name: "Test".into(),
            family: None,
            reasoning: false,
            tool_call: false,
            attachment: false,
            temperature: false,
            modalities: output_modalities.map(|out| Modalities {
                input: vec!["text".into()],
                output: out.into_iter().map(|s| s.to_string()).collect(),
            }),
            cost: None,
            limit: None,
            release_date: None,
            last_updated: None,
            knowledge: None,
            open_weights: false,
            status: None,
            description: None,
            structured_output: None,
            reasoning_options: Vec::new(),
        }
    }

    #[test]
    fn test_is_text_model_none_modalities() {
        let m = make_model(None);
        assert!(m.is_text_model(), "No modalities should default to text");
    }

    #[test]
    fn test_is_text_model_text_output() {
        let m = make_model(Some(vec!["text"]));
        assert!(m.is_text_model());
    }

    #[test]
    fn test_is_text_model_multimodal_with_text() {
        let m = make_model(Some(vec!["text", "image"]));
        assert!(m.is_text_model(), "Multimodal with text should be text");
    }

    #[test]
    fn test_is_text_model_image_only() {
        let m = make_model(Some(vec!["image"]));
        assert!(!m.is_text_model(), "Image-only model is not text");
    }

    #[test]
    fn test_is_text_model_video_only() {
        let m = make_model(Some(vec!["video"]));
        assert!(!m.is_text_model(), "Video-only model is not text");
    }

    #[test]
    fn test_is_text_model_empty_output() {
        let m = make_model(Some(vec![]));
        assert!(!m.is_text_model(), "Empty output modalities is not text");
    }

    /// Deserialize a model carrying every new field shape, including an
    /// **unknown** reasoning-option `type` — proves the permissive modeling
    /// (no tagged enum) tolerates a future models.dev tag rather than failing
    /// the whole parse.
    #[test]
    fn test_new_fields_deserialize_permissively() {
        let json = r#"{
            "id": "test-model",
            "name": "Test Model",
            "description": "A model for testing new fields",
            "structured_output": true,
            "reasoning_options": [
                {"type": "budget_tokens", "min": 0, "max": 24576},
                {"type": "effort", "values": [null, "low", "high"]},
                {"type": "toggle"},
                {"type": "some_future_mode", "max": 1000}
            ],
            "cost": {
                "input": 5.0,
                "output": 15.0,
                "reasoning": 1.147,
                "input_audio": 3.584,
                "output_audio": 7.0,
                "tiers": [
                    {"input": 2.5, "output": 15.0, "cache_read": 0.25,
                     "tier": {"type": "context", "size": 200000}}
                ]
            }
        }"#;
        let m: Model = serde_json::from_str(json).expect("should deserialize");

        assert_eq!(
            m.description.as_deref(),
            Some("A model for testing new fields")
        );
        assert_eq!(m.structured_output, Some(true));
        assert_eq!(m.reasoning_options.len(), 4);
        // Unknown type round-trips to its raw string instead of failing.
        assert_eq!(
            m.reasoning_options[3].r#type.as_deref(),
            Some("some_future_mode")
        );

        let cost = m.cost.as_ref().unwrap();
        assert_eq!(cost.reasoning, Some(1.147));
        assert_eq!(cost.input_audio, Some(3.584));
        assert_eq!(cost.output_audio, Some(7.0));
        assert_eq!(cost.tiers.len(), 1);
        assert_eq!(cost.tiers[0].tier.as_ref().unwrap().size, Some(200000));

        // Controls: budget rounded, effort levels listed (null dropped), toggle
        // as Yes, unknown type capitalized with value Yes (permissive).
        assert_eq!(
            reasoning_controls(&m.reasoning_options),
            vec![
                ("Budget".to_string(), "0–24.6k".to_string()),
                ("Effort".to_string(), "low, high".to_string()),
                ("Toggle".to_string(), "Yes".to_string()),
                ("Some_future_mode".to_string(), "Yes".to_string()),
            ]
        );
        assert!(m.capabilities_str().contains("structured"));
    }

    /// Absent `structured_output` stays `None` (unknown), never collapses to
    /// `Some(false)` — the 49%-coverage guarantee that justified `Option<bool>`.
    #[test]
    fn test_structured_output_absent_is_none() {
        let m: Model = serde_json::from_str(r#"{"id": "x", "name": "X"}"#).unwrap();
        assert_eq!(m.structured_output, None);
        assert!(m.reasoning_options.is_empty());
        assert!(reasoning_controls(&m.reasoning_options).is_empty());
    }
}
