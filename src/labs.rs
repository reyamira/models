//! Lab (canonical model creator) and canonical-model resolution.
//!
//! models.dev's repo links provider model entries to a canonical `models/`
//! registry via `base_model` references; its site groups offerings by that
//! link (one "Claude Opus 5" row covering the Bedrock regional variants and
//! the 2×-priced "Fast" endpoints — all carry
//! `base_model = "anthropic/claude-opus-5"`) and derives the "Lab" column
//! from the canonical id's namespace. No published endpoint exposes the
//! linkage per offering (api.json and catalog.json both resolve and strip
//! `base_model`), so both facts are *reconstructed* here from the published
//! signals.
//!
//! Lab tiers (`resolve`), in order:
//!
//! 1. exact display-name match into the canonical registry (`models.json`)
//! 2. name with a trailing parenthetical stripped ("Claude Opus 5 (EU)")
//! 3. `family` → lab (canonical families map 1:1 to labs; a curated table
//!    covers families the 279-model registry doesn't)
//! 4. model-id namespace prefix (`moonshotai/Kimi-K3`)
//!
//! Canonical-model tiers (`resolve_model`, the grouped view's grouping key):
//!
//! 1. direct canonical id — either the provider model id already is a
//!    canonical id, or `provider_id/model_id` is one (the same two exact
//!    fallbacks models.dev's website uses after consulting `base_model`)
//! 2. name match — exact, paren-stripped, and squashed (case/punctuation
//!    dropped, so `Claude-Opus-4.5` / `claude-opus-4-5` / "Qwen 3.7 Max" vs
//!    "Qwen3.7 Max" all converge)
//! 3. the same after stripping a vendor prefix ("Qwen: Qwen3 Max",
//!    "Anthropic: Claude 3.7 Sonnet" — gateway providers self-report names
//!    instead of inheriting the canonical spelling)
//! 4. longest canonical slug contained in the offering id at dash
//!    boundaries (`eu.anthropic.claude-opus-5` and `claude-opus-5-fast`
//!    both contain `claude-opus-5`; longest-wins keeps `gpt-5-mini` from
//!    folding into `gpt-5`)
//!
//! Measured lab coverage on live data (2026-07-29): ~79% of 5,813 offerings.
//! Unresolved models get no lab (rendered as an em-dash) and fall back to a
//! normalized name key. That fallback preserves words such as `(EU)` (only a
//! canonical match is allowed to fold a regional/fast variant into its base)
//! while collapsing provider punctuation/case differences.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

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
    /// Squashed canonical display name -> canonical id. Ambiguous normalized
    /// names are omitted (`Command R` and `Command R+`, for example).
    name_to_model: HashMap<String, String>,
    /// Unique canonical id slugs, longest first, for matching provider ids.
    model_slugs: Vec<(String, String)>,
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
        let mut cat = Self::curated_only();
        // Names or families claimed by more than one lab are ambiguous and
        // must not resolve (e.g. the Nano Banana preview/GA pairs are fine —
        // same lab — but guard anyway).
        let mut name_claims: HashMap<&str, HashSet<&str>> = HashMap::new();
        let mut family_claims: HashMap<&str, HashSet<&str>> = HashMap::new();
        let mut family_model_count: HashMap<&str, usize> = HashMap::new();
        let mut model_name_claims: HashMap<String, HashSet<&str>> = HashMap::new();
        let mut model_slug_claims: HashMap<String, HashSet<&str>> = HashMap::new();
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
                model_name_claims
                    .entry(squash_model_name(&m.name))
                    .or_default()
                    .insert(cid);
                if let Some(slug) = cid.rsplit('/').next().filter(|s| !s.is_empty()) {
                    model_slug_claims
                        .entry(slug.to_ascii_lowercase())
                        .or_default()
                        .insert(cid);
                }
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
        for (name, ids) in model_name_claims {
            if ids.len() == 1 {
                cat.name_to_model
                    .insert(name, ids.into_iter().next().unwrap().to_string());
            }
        }
        for (slug, ids) in model_slug_claims {
            if ids.len() == 1 {
                cat.model_slugs
                    .push((slug, ids.into_iter().next().unwrap().to_string()));
            }
        }
        cat.model_slugs
            .sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
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
    /// name, lab slug)`. Exact canonical ids are checked first, matching the
    /// models.dev website's public-data fallbacks. Heuristic name matching is
    /// attempted before id containment; the id slugs are longest-first so
    /// `gpt-5-mini-fast` maps to `gpt-5-mini`, never the shorter `gpt-5`.
    pub fn resolve_model<'a>(
        &'a self,
        name: &str,
        provider_id: &str,
        model_id: &str,
    ) -> Option<(&'a str, &'a str, &'a str)> {
        if let Some(target) = self.canonical_models.get(model_id) {
            return Some((
                target.id.as_str(),
                target.name.as_str(),
                target.lab.as_str(),
            ));
        }
        let provider_scoped_id = format!("{provider_id}/{model_id}");
        if let Some(target) = self.canonical_models.get(&provider_scoped_id) {
            return Some((
                target.id.as_str(),
                target.name.as_str(),
                target.lab.as_str(),
            ));
        }

        let from_name = |candidate: &str| {
            let normalized = squash_model_name(candidate);
            self.name_to_model
                .get(&normalized)
                .and_then(|id| self.canonical_models.get(id))
        };

        let mut target = from_name(name);
        if target.is_none() {
            target = strip_trailing_parenthetical(name).and_then(&from_name);
        }
        if target.is_none() {
            if let Some(unprefixed) = strip_vendor_prefix(name) {
                target = from_name(unprefixed);
                if target.is_none() {
                    target = strip_trailing_parenthetical(unprefixed).and_then(&from_name);
                }
            }
        }
        if target.is_none() {
            let id = model_id.to_ascii_lowercase();
            target = self.model_slugs.iter().find_map(|(slug, canonical_id)| {
                contains_at_id_boundary(&id, slug)
                    .then(|| self.canonical_models.get(canonical_id))
                    .flatten()
            });
        }

        target.map(|m| (m.id.as_str(), m.name.as_str(), m.lab.as_str()))
    }

    #[cfg(test)]
    pub(crate) fn from_test_entries(entries: &[(&str, &str, Option<&str>)]) -> Self {
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
        Self::from_canonical(&canonical)
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

/// Drop a gateway's echoed vendor prefix (`Qwen: Qwen3 Max`). The suffix is
/// only used as an additional lookup/grouping candidate; the displayed name
/// still comes from the canonical registry or the majority provider spelling.
fn strip_vendor_prefix(name: &str) -> Option<&str> {
    let (_, suffix) = name.split_once(':')?;
    let suffix = suffix.trim();
    (!suffix.is_empty()).then_some(suffix)
}

/// Case/punctuation-insensitive model-name key. `+` is expanded rather than
/// discarded because it carries model identity (`Command R` != `Command R+`).
fn squash_model_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c == '+' {
            out.push_str("plus");
        } else if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Public fallback key used by the grouped Models view when no canonical
/// identity can be reconstructed. Parenthetical words are deliberately kept.
pub fn normalized_model_name(name: &str) -> String {
    squash_model_name(strip_vendor_prefix(name).unwrap_or(name))
}

fn contains_at_id_boundary(id: &str, slug: &str) -> bool {
    id.match_indices(slug).any(|(start, _)| {
        let before = id[..start].chars().next_back();
        let end = start + slug.len();
        let after = id[end..].chars().next();
        before.is_none_or(|c| !c.is_ascii_alphanumeric())
            && after.is_none_or(|c| !c.is_ascii_alphanumeric())
    })
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
        let cat = LabCatalog::from_canonical(&canon(&[(
            "anthropic/claude-opus-5",
            "Claude Opus 5",
            Some("claude-opus"),
        )]));
        let expected = Some(("anthropic/claude-opus-5", "Claude Opus 5", "anthropic"));

        // Parenthetical regional spelling resolves by canonical name.
        assert_eq!(
            cat.resolve_model(
                "Claude Opus 5 (EU)",
                "amazon-bedrock",
                "eu.anthropic.claude-opus-5"
            ),
            expected
        );
        // A provider's non-parenthetical Fast spelling resolves by the
        // longest canonical slug contained in its offering id.
        assert_eq!(
            cat.resolve_model("Claude Opus 5 Fast", "venice", "claude-opus-5-fast"),
            expected
        );
    }

    #[test]
    fn direct_catalog_id_matches_precede_heuristics() {
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
            cat.resolve_model("Misleading name", "gateway", "openai/gpt-5"),
            Some(("openai/gpt-5", "GPT-5", "openai"))
        );
        // Origin providers commonly become canonical as provider/id.
        assert_eq!(
            cat.resolve_model("Misleading name", "anthropic", "claude-opus-5"),
            Some(("anthropic/claude-opus-5", "Claude Opus 5", "anthropic"))
        );
    }

    #[test]
    fn canonical_model_normalizes_punctuation_and_vendor_prefixes() {
        let cat = LabCatalog::from_canonical(&canon(&[(
            "alibaba/qwen-3-7-max",
            "Qwen 3.7 Max",
            Some("qwen"),
        )]));
        let expected = Some(("alibaba/qwen-3-7-max", "Qwen 3.7 Max", "alibaba"));

        assert_eq!(
            cat.resolve_model("Qwen3.7-Max", "gateway", "opaque"),
            expected
        );
        assert_eq!(
            cat.resolve_model("Qwen: Qwen3.7 Max", "gateway", "opaque"),
            expected
        );
    }

    #[test]
    fn canonical_id_matching_is_longest_first_and_boundary_aware() {
        let cat = LabCatalog::from_canonical(&canon(&[
            ("openai/gpt-5", "GPT-5", Some("gpt")),
            ("openai/gpt-5-mini", "GPT-5 mini", Some("gpt")),
        ]));

        assert_eq!(
            cat.resolve_model("Unknown", "gateway", "gateway/gpt-5-mini-fast"),
            Some(("openai/gpt-5-mini", "GPT-5 mini", "openai"))
        );
        assert_eq!(
            cat.resolve_model("Unknown", "gateway", "gateway/agpt-5x"),
            None
        );
    }

    #[test]
    fn ambiguous_squashed_names_do_not_claim_a_canonical_model() {
        let cat = LabCatalog::from_canonical(&canon(&[
            ("cohere/command-r", "Command R", None),
            ("other/command-r", "Command-R", None),
        ]));

        assert_eq!(cat.resolve_model("command r", "gateway", "opaque"), None);
    }

    #[test]
    fn fallback_name_normalization_preserves_identity_words() {
        assert_eq!(
            normalized_model_name("Qwen: Qwen3.7-Max"),
            normalized_model_name("Qwen 3.7 Max")
        );
        assert_ne!(
            normalized_model_name("Claude Opus 5 (EU)"),
            normalized_model_name("Claude Opus 5")
        );
        assert_ne!(
            normalized_model_name("Command R+"),
            normalized_model_name("Command R")
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
