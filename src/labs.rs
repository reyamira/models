//! Lab (canonical model creator) and canonical-model resolution.
//!
//! models.dev links provider offerings to its canonical `models/` registry
//! through `base_model` values in `providers/*/models/**/*.toml`. Its build
//! strips that edge from `api.json`/`catalog.json`, but its website separately
//! scans the same TOMLs into a `BaseModelRefs` map. This module consumes a
//! versioned artifact generated from those public TOMLs and then follows the
//! website resolver exactly:
//!
//! 1. `BaseModelRefs[provider_id/model_id]`, when its target exists
//! 2. `model_id`, when it is already a canonical id
//! 3. `provider_id/model_id`, when it is a canonical id
//! 4. otherwise unlinked — names and partial slugs are never identity evidence
//!
//! Lab tiers (`resolve`), in order:
//!
//! 1. exact display-name match into the canonical registry (`models.json`)
//! 2. name with a trailing parenthetical stripped ("Claude Opus 5 (EU)")
//! 3. `family` → lab (canonical families map 1:1 to labs; a curated table
//!    covers families the 279-model registry doesn't)
//! 4. model-id namespace prefix (`moonshotai/Kimi-K3`)
//!
//! Measured lab coverage on live data (2026-07-29): ~79% of 5,813 offerings.
//! Lab inference remains a presentation fallback for unlinked offerings; it
//! does not affect grouping identity. Unlinked offerings keep independent
//! provider/model keys and therefore cannot be merged speculatively.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

use crate::model_refs::BaseModelRefsFile;

/// One record of the canonical `models/` registry. Only the fields the
/// resolver needs — everything else is ignored.
#[derive(Debug, Deserialize)]
pub(crate) struct CanonicalModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    family: Option<String>,
}

#[derive(Debug, Clone)]
struct CanonicalTarget {
    id: String,
    name: String,
    lab: String,
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
    /// Canonical id -> canonical identity returned by `resolve_model`.
    canonical_models: HashMap<String, CanonicalTarget>,
    /// Exact provider/model offering key -> canonical model id, generated from
    /// the same provider TOMLs models.dev's website scans at build time.
    base_model_refs: BTreeMap<String, String>,
}

impl LabCatalog {
    fn curated_only() -> Self {
        let mut cat = Self::default();
        for (fam, lab) in CURATED_FAMILY_LABS {
            cat.family_to_lab.insert((*fam).into(), (*lab).into());
            cat.lab_slugs.insert((*lab).into());
        }
        cat
    }

    pub(crate) fn from_canonical(canonical: &HashMap<String, CanonicalModel>) -> Self {
        let artifact: BaseModelRefsFile = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/models-dev-base-model-refs.json"
        )))
        .expect("embedded models.dev base-model refs must be valid JSON");
        artifact
            .validate()
            .expect("embedded models.dev base-model refs must match the supported schema");
        Self::from_canonical_and_refs(canonical, artifact.refs)
    }

    fn from_canonical_and_refs(
        canonical: &HashMap<String, CanonicalModel>,
        base_model_refs: BTreeMap<String, String>,
    ) -> Self {
        let mut cat = Self::curated_only();
        cat.base_model_refs = base_model_refs;
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
                cat.canonical_models.insert(
                    cid.clone(),
                    CanonicalTarget {
                        id: cid.clone(),
                        name: m.name.clone(),
                        lab: lab.to_string(),
                    },
                );
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

    /// Resolve one provider offering to `(canonical id, canonical display
    /// name, lab slug)` using models.dev's repository-defined resolver order.
    pub fn resolve_model<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
    ) -> Option<(&'a str, &'a str, &'a str)> {
        let provider_scoped_id = format!("{provider_id}/{model_id}");
        if let Some(target) = self
            .base_model_refs
            .get(&provider_scoped_id)
            .and_then(|canonical_id| self.canonical_models.get(canonical_id))
        {
            return Some((
                target.id.as_str(),
                target.name.as_str(),
                target.lab.as_str(),
            ));
        }
        if let Some(target) = self.canonical_models.get(model_id) {
            return Some((
                target.id.as_str(),
                target.name.as_str(),
                target.lab.as_str(),
            ));
        }
        if let Some(target) = self.canonical_models.get(&provider_scoped_id) {
            return Some((
                target.id.as_str(),
                target.name.as_str(),
                target.lab.as_str(),
            ));
        }

        None
    }

    #[cfg(test)]
    pub(crate) fn from_test_entries_with_refs(
        entries: &[(&str, &str, Option<&str>)],
        refs: &[(&str, &str)],
    ) -> Self {
        let canonical = entries
            .iter()
            .map(|(id, name, family)| {
                (
                    (*id).to_string(),
                    CanonicalModel {
                        name: (*name).to_string(),
                        family: family.map(String::from),
                    },
                )
            })
            .collect();
        let base_model_refs = refs
            .iter()
            .map(|(offering, canonical)| ((*offering).to_string(), (*canonical).to_string()))
            .collect();
        Self::from_canonical_and_refs(&canonical, base_model_refs)
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
    fn canonical_model_folds_regional_and_fast_offerings() {
        let cat = LabCatalog::from_test_entries_with_refs(
            &[(
                "anthropic/claude-opus-5",
                "Claude Opus 5",
                Some("claude-opus"),
            )],
            &[
                (
                    "amazon-bedrock/eu.anthropic.claude-opus-5",
                    "anthropic/claude-opus-5",
                ),
                ("venice/claude-opus-5-fast", "anthropic/claude-opus-5"),
            ],
        );
        let expected = Some(("anthropic/claude-opus-5", "Claude Opus 5", "anthropic"));

        assert_eq!(
            cat.resolve_model("amazon-bedrock", "eu.anthropic.claude-opus-5"),
            expected
        );
        assert_eq!(cat.resolve_model("venice", "claude-opus-5-fast"), expected);
    }

    #[test]
    fn explicit_ref_precedes_both_public_id_fallbacks() {
        let cat = LabCatalog::from_test_entries_with_refs(
            &[
                ("openai/gpt-5.6", "GPT-5.6", Some("gpt")),
                ("openai/gpt-5.6-sol", "GPT-5.6 Sol", Some("gpt")),
            ],
            &[("openai/gpt-5.6", "openai/gpt-5.6-sol")],
        );

        assert_eq!(
            cat.resolve_model("openai", "gpt-5.6"),
            Some(("openai/gpt-5.6-sol", "GPT-5.6 Sol", "openai"))
        );
    }

    #[test]
    fn missing_explicit_target_falls_through_to_direct_id() {
        let cat = LabCatalog::from_test_entries_with_refs(
            &[("openai/gpt-5", "GPT-5", Some("gpt"))],
            &[("gateway/openai/gpt-5", "missing/gpt-5")],
        );

        assert_eq!(
            cat.resolve_model("gateway", "openai/gpt-5"),
            Some(("openai/gpt-5", "GPT-5", "openai"))
        );
    }

    #[test]
    fn direct_catalog_id_fallbacks_match_website_order() {
        let cat = LabCatalog::from_canonical(&canon(&[
            ("openai/gpt-5", "GPT-5", Some("gpt")),
            (
                "anthropic/claude-opus-5",
                "Claude Opus 5",
                Some("claude-opus"),
            ),
        ]));

        // A fully qualified provider model id is already canonical.
        assert_eq!(
            cat.resolve_model("gateway", "openai/gpt-5"),
            Some(("openai/gpt-5", "GPT-5", "openai"))
        );
        // Origin providers commonly become canonical as provider/id.
        assert_eq!(
            cat.resolve_model("anthropic", "claude-opus-5"),
            Some(("anthropic/claude-opus-5", "Claude Opus 5", "anthropic"))
        );
    }

    #[test]
    fn partial_canonical_slugs_are_not_identity_evidence() {
        let cat = LabCatalog::from_canonical(&canon(&[
            ("openai/gpt-5", "GPT-5", Some("gpt")),
            ("openai/gpt-5-mini", "GPT-5 mini", Some("gpt")),
        ]));

        assert_eq!(
            cat.resolve_model("gateway", "gateway/gpt-5-mini-fast"),
            None
        );
        assert_eq!(cat.resolve_model("gateway", "gateway/agpt-5x"), None);
    }

    #[test]
    fn embedded_refs_resolve_non_obvious_opus_link() {
        let artifact: BaseModelRefsFile = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/models-dev-base-model-refs.json"
        )))
        .expect("parse embedded refs");
        artifact.validate().expect("validate embedded refs");
        assert_eq!(
            artifact.refs.get("gitlab/duo-chat-opus-5"),
            Some(&"anthropic/claude-opus-5".to_string())
        );

        let cat = LabCatalog::from_canonical(&canon(&[(
            "anthropic/claude-opus-5",
            "Claude Opus 5",
            Some("claude-opus"),
        )]));
        assert_eq!(
            cat.resolve_model("gitlab", "duo-chat-opus-5"),
            Some(("anthropic/claude-opus-5", "Claude Opus 5", "anthropic"))
        );
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
