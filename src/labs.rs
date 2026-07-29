//! Lab (canonical model creator) resolution.
//!
//! models.dev's repo links provider model entries to a canonical `models/`
//! registry via `base_model` references, and its site derives the "Lab"
//! column from the canonical id's namespace (`anthropic/claude-opus-5` →
//! Anthropic). No published endpoint exposes that linkage per offering
//! (api.json and catalog.json both resolve and strip `base_model`), so the
//! lab is *reconstructed* here from the published signals, in order:
//!
//! 1. exact display-name match into the canonical registry (`models.json`)
//! 2. name with a trailing parenthetical stripped ("Claude Opus 5 (EU)")
//! 3. `family` → lab (canonical families map 1:1 to labs; a curated table
//!    covers families the 279-model registry doesn't)
//! 4. model-id namespace prefix (`moonshotai/Kimi-K3`)
//!
//! Measured coverage on live data (2026-07-29): ~79% of 5,813 offerings.
//! Unresolved models get no lab (rendered as an em-dash).

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

const MODELS_JSON_URL: &str = "https://models.dev/models.json";

/// One record of the canonical `models/` registry. Only the fields the
/// resolver needs — everything else is ignored.
#[derive(Debug, Deserialize)]
struct CanonicalModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    family: Option<String>,
}

/// Families → lab slugs for model lines the canonical registry doesn't
/// (yet) cover. Only entries with an unambiguous owner belong here.
const CURATED_FAMILY_LABS: &[(&str, &str)] = &[
    ("claude", "anthropic"),
    ("command", "cohere"),
    ("codestral", "mistral"),
    ("devstral", "mistral"),
    ("doubao", "bytedance"),
    ("ernie", "baidu"),
    ("gemma", "google"),
    ("glm", "zhipuai"),
    ("granite", "ibm"),
    ("grok", "xai"),
    ("hunyuan", "tencent"),
    ("jamba", "ai21"),
    ("kimi", "moonshotai"),
    // Guards a real upstream collision: Thinking Machines' "Inkling" claims
    // family "ling", but non-canonical Ling offerings are InclusionAI's.
    ("ling", "inclusionai"),
    ("llama", "meta"),
    ("magistral", "mistral"),
    ("ministral", "mistral"),
    ("mistral", "mistral"),
    ("nova", "amazon"),
    ("phi", "microsoft"),
    ("pixtral", "mistral"),
    ("qwen", "alibaba"),
    ("seed", "bytedance"),
    ("titan", "amazon"),
];

/// Pretty display names for lab slugs; anything absent falls back to a
/// capitalized slug.
const LAB_DISPLAY: &[(&str, &str)] = &[
    ("ai21", "AI21 Labs"),
    ("alibaba", "Alibaba"),
    ("amazon", "Amazon"),
    ("anthropic", "Anthropic"),
    ("baidu", "Baidu"),
    ("bytedance", "ByteDance"),
    ("cohere", "Cohere"),
    ("deepreinforce", "DeepReinforce"),
    ("deepseek", "DeepSeek"),
    ("google", "Google"),
    ("ibm", "IBM"),
    ("inclusionai", "InclusionAI"),
    ("meituan", "Meituan"),
    ("meta", "Meta"),
    ("microsoft", "Microsoft"),
    ("minimax", "MiniMax"),
    ("mistral", "Mistral"),
    ("moonshotai", "Moonshot AI"),
    ("nvidia", "NVIDIA"),
    ("openai", "OpenAI"),
    ("perplexity", "Perplexity"),
    ("poolside", "Poolside"),
    ("sakana", "Sakana AI"),
    ("sarvam", "Sarvam AI"),
    ("stepfun", "StepFun"),
    ("tencent", "Tencent"),
    ("thinkingmachines", "Thinking Machines"),
    ("xai", "xAI"),
    ("xiaomi", "Xiaomi"),
    ("zhipuai", "Z.ai"),
];

#[derive(Debug, Default)]
pub struct LabCatalog {
    /// Canonical display name → lab slug (names mapping to >1 lab dropped).
    name_to_lab: HashMap<String, String>,
    /// Family → lab slug (canonical-derived unique mappings + curated).
    family_to_lab: HashMap<String, String>,
    /// Known lab slugs, for the id-namespace-prefix tier.
    lab_slugs: HashSet<String>,
}

impl LabCatalog {
    /// Fetch and build the catalog from models.dev's canonical registry.
    /// Best-effort: on any failure the curated-only catalog is returned, so
    /// the family and prefix tiers keep working offline.
    pub fn fetch() -> Self {
        let fetched = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .ok()
            .and_then(|c| c.get(MODELS_JSON_URL).send().ok())
            .and_then(|r| r.json::<HashMap<String, CanonicalModel>>().ok());
        match fetched {
            Some(canonical) => Self::from_canonical(&canonical),
            None => Self::curated_only(),
        }
    }

    fn curated_only() -> Self {
        let mut cat = Self::default();
        for (fam, lab) in CURATED_FAMILY_LABS {
            cat.family_to_lab.insert((*fam).into(), (*lab).into());
            cat.lab_slugs.insert((*lab).into());
        }
        cat
    }

    fn from_canonical(canonical: &HashMap<String, CanonicalModel>) -> Self {
        let mut cat = Self::curated_only();
        // Names or families claimed by more than one lab are ambiguous and
        // must not resolve (e.g. the Nano Banana preview/GA pairs are fine —
        // same lab — but guard anyway).
        let mut name_claims: HashMap<&str, HashSet<&str>> = HashMap::new();
        let mut family_claims: HashMap<&str, HashSet<&str>> = HashMap::new();
        let mut family_model_count: HashMap<&str, usize> = HashMap::new();
        for (cid, m) in canonical {
            let Some(lab) = cid.split('/').next().filter(|s| !s.is_empty()) else {
                continue;
            };
            cat.lab_slugs.insert(lab.to_string());
            if !m.name.is_empty() {
                name_claims.entry(m.name.as_str()).or_default().insert(lab);
            }
            if let Some(fam) = m.family.as_deref() {
                family_claims.entry(fam).or_default().insert(lab);
                *family_model_count.entry(fam).or_default() += 1;
            }
        }
        for (name, labs) in name_claims {
            if labs.len() == 1 {
                cat.name_to_lab.insert(
                    name.to_string(),
                    labs.into_iter().next().unwrap().to_string(),
                );
            }
        }
        for (fam, labs) in family_claims {
            // A family backed by a single canonical model is weak evidence
            // (version-suffixed one-offs, or generic coincidences — Thinking
            // Machines' lone "Inkling" claims family "ling", which would
            // mislabel InclusionAI's whole Ling line). Require >=2 models
            // before a canonical family overrides the curated table.
            if labs.len() == 1 && family_model_count.get(fam).copied().unwrap_or(0) >= 2 {
                cat.family_to_lab.insert(
                    fam.to_string(),
                    labs.into_iter().next().unwrap().to_string(),
                );
            }
        }
        cat
    }

    /// Resolve a lab slug for one offering. Tiers documented at module level.
    pub fn resolve(&self, name: &str, family: Option<&str>, model_id: &str) -> Option<&str> {
        if let Some(lab) = self.name_to_lab.get(name) {
            return Some(lab);
        }
        if let Some(stripped) = strip_trailing_parenthetical(name) {
            if let Some(lab) = self.name_to_lab.get(stripped) {
                return Some(lab);
            }
        }
        if let Some(fam) = family {
            if let Some(lab) = self.family_to_lab.get(fam) {
                return Some(lab);
            }
            // Family granularity varies per provider ("claude-opus",
            // "kimi-k3" vs bare "claude"/"kimi") — fall back to the first
            // dash segment so the curated base-name table still applies.
            if let Some(base) = fam.split('-').next() {
                if base != fam {
                    if let Some(lab) = self.family_to_lab.get(base) {
                        return Some(lab);
                    }
                }
            }
        }
        if let Some(prefix) = model_id.split('/').next() {
            if prefix != model_id {
                if let Some(lab) = self.lab_slugs.get(prefix) {
                    return Some(lab);
                }
            }
        }
        None
    }
}

/// `"Claude Opus 5 (EU)"` → `"Claude Opus 5"`. Returns `None` when there is
/// nothing to strip (avoids a redundant second lookup).
fn strip_trailing_parenthetical(name: &str) -> Option<&str> {
    let trimmed = name.trim_end();
    if !trimmed.ends_with(')') {
        return None;
    }
    let open = trimmed.rfind('(')?;
    let base = trimmed[..open].trim_end();
    (!base.is_empty()).then_some(base)
}

/// Pretty display name for a lab slug.
pub fn lab_display(slug: &str) -> String {
    for (s, d) in LAB_DISPLAY {
        if *s == slug {
            return (*d).to_string();
        }
    }
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(entries: &[(&str, &str, Option<&str>)]) -> HashMap<String, CanonicalModel> {
        entries
            .iter()
            .map(|(cid, name, fam)| {
                (
                    cid.to_string(),
                    CanonicalModel {
                        name: name.to_string(),
                        family: fam.map(String::from),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn resolves_by_exact_name() {
        let cat = LabCatalog::from_canonical(&canon(&[(
            "anthropic/claude-opus-5",
            "Claude Opus 5",
            Some("claude-opus"),
        )]));
        assert_eq!(
            cat.resolve("Claude Opus 5", None, "us.anthropic.claude-opus-5"),
            Some("anthropic")
        );
    }

    #[test]
    fn resolves_regional_variant_by_stripped_name() {
        let cat = LabCatalog::from_canonical(&canon(&[(
            "anthropic/claude-opus-5",
            "Claude Opus 5",
            None,
        )]));
        assert_eq!(
            cat.resolve("Claude Opus 5 (EU)", None, "eu.anthropic.claude-opus-5"),
            Some("anthropic")
        );
    }

    #[test]
    fn resolves_by_family_and_curated_table() {
        // Two canonical models back the family — single-model families are
        // deliberately not trusted (see the "ling" collision note).
        let cat = LabCatalog::from_canonical(&canon(&[
            ("google/gemini-3-pro", "Gemini 3 Pro", Some("gemini-pro")),
            (
                "google/gemini-3.5-pro",
                "Gemini 3.5 Pro",
                Some("gemini-pro"),
            ),
        ]));
        // Canonical-derived family.
        assert_eq!(
            cat.resolve("Some Gemini Variant", Some("gemini-pro"), "x"),
            Some("google")
        );
        // Curated family (registry doesn't carry it).
        assert_eq!(
            cat.resolve("Seed Thing", Some("seed"), "x"),
            Some("bytedance")
        );
    }

    #[test]
    fn resolves_by_id_prefix() {
        let cat = LabCatalog::from_canonical(&canon(&[("moonshotai/kimi-k3", "Kimi K3", None)]));
        assert_eq!(
            cat.resolve("Kimi K3 TEE", None, "moonshotai/Kimi-K3-TEE"),
            Some("moonshotai")
        );
    }

    #[test]
    fn ambiguous_name_does_not_resolve() {
        let cat = LabCatalog::from_canonical(&canon(&[
            ("a/model-1", "Shared Name", None),
            ("b/model-2", "Shared Name", None),
        ]));
        assert_eq!(cat.resolve("Shared Name", None, "x"), None);
    }

    #[test]
    fn unresolvable_is_none() {
        let cat = LabCatalog::curated_only();
        assert_eq!(cat.resolve("Mystery Model", None, "mystery"), None);
    }

    #[test]
    fn display_names() {
        assert_eq!(lab_display("moonshotai"), "Moonshot AI");
        assert_eq!(lab_display("zhipuai"), "Z.ai");
        assert_eq!(lab_display("somelab"), "Somelab");
    }
}
