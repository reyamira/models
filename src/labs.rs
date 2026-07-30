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
//! 4. otherwise unlinked
//!
//! For offerings left unlinked by those authoritative tiers, two deliberately
//! narrow resolvers may infer canonical identity. The first requires a unique
//! normalized canonical name, unanimous authoritative targets for that name,
//! and an exact token fingerprint already observed for that target. The second
//! reconciles provider records that qualify both name and id with the creator
//! (`Anthropic Claude Fable 5` / `anthropic-claude-fable-5`) when that dual key
//! selects exactly one canonical target. Neither inference lane overrides an
//! authoritative result.
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
//! Lab inference remains a presentation fallback and is never canonical
//! identity evidence. Canonical inference uses only authoritative name/id
//! anchors; compatible leftovers may be grouped separately as explicitly
//! non-canonical peers by the Models view.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use crate::data::{Modalities, Model, ProvidersMap};
use crate::model_refs::BaseModelRefsFile;

/// One record of the canonical `models/` registry. Only the fields the
/// resolver needs — everything else is ignored.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CanonicalModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    modalities: Option<Modalities>,
}

#[derive(Debug, Clone)]
struct CanonicalTarget {
    id: String,
    name: String,
    lab: String,
    output_modalities: Vec<String>,
}

/// How a provider offering acquired canonical identity. The first three tiers
/// reproduce models.dev's website resolver; the inferred variants are local
/// reconstruction and must remain visible as such in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalResolutionKind {
    AuthoritativeRef,
    AuthoritativeDirectId,
    AuthoritativeScopedId,
    InferredCanonical,
    InferredQualifiedCanonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceRejection {
    EmptyIdentity,
    NoCanonicalName,
    AmbiguousCanonicalName,
    AmbiguousQualifiedIdentity,
    NoAuthoritativeNameAnchor,
    ConflictingAuthoritativeNameAnchors,
    UnseenIdFingerprint,
    CreatorConflict,
    DisjointOutputModalities,
}

impl InferenceRejection {
    pub fn label(self) -> &'static str {
        match self {
            Self::EmptyIdentity => "empty identity",
            Self::NoCanonicalName => "no canonical name match",
            Self::AmbiguousCanonicalName => "ambiguous canonical name",
            Self::AmbiguousQualifiedIdentity => "ambiguous creator-qualified identity",
            Self::NoAuthoritativeNameAnchor => "no authoritative name anchor",
            Self::ConflictingAuthoritativeNameAnchors => "conflicting name anchors",
            Self::UnseenIdFingerprint => "unseen model-id fingerprint",
            Self::CreatorConflict => "creator conflict",
            Self::DisjointOutputModalities => "output-modality conflict",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CanonicalResolution<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub lab: &'a str,
    pub kind: CanonicalResolutionKind,
}

#[derive(Debug, Clone, Copy)]
pub enum ModelIdentity<'a> {
    Canonical(CanonicalResolution<'a>),
    Unlinked(InferenceRejection),
}

/// Audit-only canonical candidate lanes. These never alter grouping; they
/// preserve distinct evidence classes so each can be evaluated before any
/// future activation decision.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowCandidateKind {
    ExactAuthoritativePair,
    OneSidedCreatorQualified,
    CrossAuthoritativeAliases,
}

#[cfg(test)]
impl ShadowCandidateKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExactAuthoritativePair => "exact authoritative pair",
            Self::OneSidedCreatorQualified => "one-sided creator qualification",
            Self::CrossAuthoritativeAliases => "cross-record authoritative aliases",
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct ShadowCanonicalCandidate<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub lab: &'a str,
    pub kind: ShadowCandidateKind,
    pub pair_witnesses: usize,
    pub name_witnesses: usize,
    pub id_witnesses: usize,
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
    /// Normalized canonical display name -> every canonical id claiming it.
    canonical_name_candidates: HashMap<String, HashSet<String>>,
    /// Normalized provider display name -> canonical targets established by
    /// authoritative resolution. Inference requires this set to be exactly the
    /// same single candidate selected from the canonical registry.
    authoritative_name_targets: HashMap<String, HashSet<String>>,
    /// Canonical id -> exact normalized leaf-id fingerprints observed on the
    /// canonical record or on authoritatively linked provider offerings.
    canonical_id_fingerprints: HashMap<String, HashSet<String>>,
    /// `(creator-qualified display name, full model id)` -> canonical targets.
    /// Both components use the separator-compacted semantic fingerprint; the
    /// target must still be unique at resolution time.
    qualified_identity_candidates: HashMap<(String, String), HashSet<String>>,
    /// Creator-qualified canonical display name -> canonical targets. Kept
    /// separate from the dual-field production lane for audit-only one-sided
    /// qualification candidates.
    #[cfg(test)]
    qualified_name_candidates: HashMap<String, HashSet<String>>,
    /// Exact provider-observed leaf-id fingerprint -> authoritative targets.
    /// Unlike `canonical_id_fingerprints`, this is never seeded from the
    /// canonical registry itself.
    #[cfg(test)]
    authoritative_id_targets: HashMap<String, HashSet<String>>,
    /// Exact provider-observed `(name, leaf id)` pair -> authoritative targets.
    #[cfg(test)]
    authoritative_pair_targets: HashMap<(String, String), HashSet<String>>,
    /// Provider witnesses for each authoritative alias edge. Provider count is
    /// not assumed to mean source independence, but is retained for audit.
    #[cfg(test)]
    authoritative_name_witnesses: HashMap<(String, String), HashSet<String>>,
    #[cfg(test)]
    authoritative_id_witnesses: HashMap<(String, String), HashSet<String>>,
    #[cfg(test)]
    authoritative_pair_witnesses: HashMap<(String, String, String), HashSet<String>>,
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

    #[cfg(test)]
    pub(crate) fn from_canonical(canonical: &HashMap<String, CanonicalModel>) -> Self {
        Self::from_catalog_parts(canonical, None)
    }

    /// Build canonical/lab resolution plus conservative inference anchors from
    /// the coherent `catalog.json` provider + canonical snapshot.
    pub(crate) fn from_catalog(
        canonical: &HashMap<String, CanonicalModel>,
        providers: &ProvidersMap,
    ) -> Self {
        Self::from_catalog_parts(canonical, Some(providers))
    }

    fn from_catalog_parts(
        canonical: &HashMap<String, CanonicalModel>,
        providers: Option<&ProvidersMap>,
    ) -> Self {
        let artifact: BaseModelRefsFile = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/models-dev-base-model-refs.json"
        )))
        .expect("embedded models.dev base-model refs must be valid JSON");
        artifact
            .validate()
            .expect("embedded models.dev base-model refs must match the supported schema");
        Self::from_canonical_and_refs(canonical, artifact.refs, providers)
    }

    fn from_canonical_and_refs(
        canonical: &HashMap<String, CanonicalModel>,
        base_model_refs: BTreeMap<String, String>,
        providers: Option<&ProvidersMap>,
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
                let normalized_name = identity_fingerprint(&m.name);
                if !normalized_name.is_empty() {
                    cat.canonical_name_candidates
                        .entry(normalized_name)
                        .or_default()
                        .insert(cid.clone());
                }
                let id_fingerprint = model_id_fingerprint(cid);
                if !id_fingerprint.is_empty() {
                    cat.canonical_id_fingerprints
                        .entry(cid.clone())
                        .or_default()
                        .insert(id_fingerprint);
                }
                let full_id_fingerprint = compact_model_id_fingerprint(cid);
                if !full_id_fingerprint.is_empty() {
                    for creator in [lab.to_string(), lab_display(lab)] {
                        let qualified_name =
                            compact_identity_fingerprint(&format!("{creator} {}", m.name));
                        if !qualified_name.is_empty() {
                            #[cfg(test)]
                            cat.qualified_name_candidates
                                .entry(qualified_name.clone())
                                .or_default()
                                .insert(cid.clone());
                            cat.qualified_identity_candidates
                                .entry((qualified_name, full_id_fingerprint.clone()))
                                .or_default()
                                .insert(cid.clone());
                        }
                    }
                }
                cat.canonical_models.insert(
                    cid.clone(),
                    CanonicalTarget {
                        id: cid.clone(),
                        name: m.name.clone(),
                        lab: lab.to_string(),
                        output_modalities: m
                            .modalities
                            .as_ref()
                            .map(|modalities| modalities.output.clone())
                            .unwrap_or_default(),
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

        // Provider names and ids are needed only for the conservative local
        // inference layer. Exact models.dev resolution works without them, so
        // older unit helpers can still construct a canonical-only catalog.
        if let Some(providers) = providers {
            let anchors: Vec<(String, String, String, String)> = providers
                .iter()
                .flat_map(|(provider_id, provider)| {
                    provider.models.iter().filter_map(|(model_id, model)| {
                        let resolution = cat.resolve_authoritative(provider_id, model_id)?;
                        let name = identity_fingerprint(&model.name);
                        let id = model_id_fingerprint(model_id);
                        (!name.is_empty() && !id.is_empty())
                            .then(|| (provider_id.clone(), name, resolution.id.to_string(), id))
                    })
                })
                .collect();
            for (_provider_id, name, canonical_id, id_fingerprint) in anchors {
                cat.authoritative_name_targets
                    .entry(name.clone())
                    .or_default()
                    .insert(canonical_id.clone());
                cat.canonical_id_fingerprints
                    .entry(canonical_id.clone())
                    .or_default()
                    .insert(id_fingerprint.clone());
                #[cfg(test)]
                {
                    cat.authoritative_id_targets
                        .entry(id_fingerprint.clone())
                        .or_default()
                        .insert(canonical_id.clone());
                    cat.authoritative_pair_targets
                        .entry((name.clone(), id_fingerprint.clone()))
                        .or_default()
                        .insert(canonical_id.clone());
                    cat.authoritative_name_witnesses
                        .entry((name.clone(), canonical_id.clone()))
                        .or_default()
                        .insert(_provider_id.clone());
                    cat.authoritative_id_witnesses
                        .entry((id_fingerprint.clone(), canonical_id.clone()))
                        .or_default()
                        .insert(_provider_id.clone());
                    cat.authoritative_pair_witnesses
                        .entry((name, id_fingerprint, canonical_id))
                        .or_default()
                        .insert(_provider_id);
                }
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

    fn resolve_authoritative<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
    ) -> Option<CanonicalResolution<'a>> {
        let provider_scoped_id = format!("{provider_id}/{model_id}");
        if let Some(target) = self
            .base_model_refs
            .get(&provider_scoped_id)
            .and_then(|canonical_id| self.canonical_models.get(canonical_id))
        {
            return Some(CanonicalResolution {
                id: target.id.as_str(),
                name: target.name.as_str(),
                lab: target.lab.as_str(),
                kind: CanonicalResolutionKind::AuthoritativeRef,
            });
        }
        if let Some(target) = self.canonical_models.get(model_id) {
            return Some(CanonicalResolution {
                id: target.id.as_str(),
                name: target.name.as_str(),
                lab: target.lab.as_str(),
                kind: CanonicalResolutionKind::AuthoritativeDirectId,
            });
        }
        if let Some(target) = self.canonical_models.get(&provider_scoped_id) {
            return Some(CanonicalResolution {
                id: target.id.as_str(),
                name: target.name.as_str(),
                lab: target.lab.as_str(),
                kind: CanonicalResolutionKind::AuthoritativeScopedId,
            });
        }

        None
    }

    /// Resolve one provider offering to `(canonical id, canonical display
    /// name, lab slug)` using only models.dev's repository-defined resolver
    /// order. Kept separate from local inference so authoritative parity stays
    /// independently testable.
    #[cfg(test)]
    pub fn resolve_model<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
    ) -> Option<(&'a str, &'a str, &'a str)> {
        self.resolve_authoritative(provider_id, model_id)
            .map(|resolution| (resolution.id, resolution.name, resolution.lab))
    }

    /// Resolve canonical identity without ever allowing inferred evidence to
    /// override models.dev's explicit/direct/scoped resolver tiers.
    pub fn resolve_model_identity<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
    ) -> ModelIdentity<'a> {
        if let Some(resolution) = self.resolve_authoritative(provider_id, model_id) {
            return ModelIdentity::Canonical(resolution);
        }

        match self.resolve_anchored_inference(provider_id, model_id, model) {
            Ok(resolution) => ModelIdentity::Canonical(resolution),
            Err(anchored_rejection) => {
                match self.resolve_qualified_inference(provider_id, model_id, model) {
                    Some(Ok(resolution)) => ModelIdentity::Canonical(resolution),
                    Some(Err(qualified_rejection)) => ModelIdentity::Unlinked(qualified_rejection),
                    None => ModelIdentity::Unlinked(anchored_rejection),
                }
            }
        }
    }

    /// Return the strongest audit-only canonical candidate for an offering
    /// that remains unresolved by every active canonical lane. Candidate
    /// evidence is derived exclusively from authoritatively resolved provider
    /// offerings; this method never mutates identity or seeds another match.
    #[cfg(test)]
    pub fn shadow_canonical_candidate<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
    ) -> Option<ShadowCanonicalCandidate<'a>> {
        if matches!(
            self.resolve_model_identity(provider_id, model_id, model),
            ModelIdentity::Canonical(_)
        ) {
            return None;
        }

        let name = identity_fingerprint(&model.name);
        let id = model_id_fingerprint(model_id);
        if name.is_empty() || id.is_empty() {
            return None;
        }

        if let Some(target) = unique_target(
            self.authoritative_pair_targets
                .get(&(name.clone(), id.clone())),
        ) {
            return self.build_shadow_candidate(
                provider_id,
                model_id,
                model,
                target,
                ShadowCandidateKind::ExactAuthoritativePair,
            );
        }

        let qualified_name = compact_identity_fingerprint(&model.name);
        if let (Some(name_target), Some(id_target)) = (
            unique_target(self.qualified_name_candidates.get(&qualified_name)),
            unique_target(self.authoritative_id_targets.get(&id)),
        ) {
            if name_target == id_target {
                return self.build_shadow_candidate(
                    provider_id,
                    model_id,
                    model,
                    name_target,
                    ShadowCandidateKind::OneSidedCreatorQualified,
                );
            }
        }

        if let (Some(name_target), Some(id_target)) = (
            unique_target(self.authoritative_name_targets.get(&name)),
            unique_target(self.authoritative_id_targets.get(&id)),
        ) {
            if name_target == id_target {
                return self.build_shadow_candidate(
                    provider_id,
                    model_id,
                    model,
                    name_target,
                    ShadowCandidateKind::CrossAuthoritativeAliases,
                );
            }
        }

        None
    }

    #[cfg(test)]
    fn build_shadow_candidate<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
        candidate_id: &str,
        kind: ShadowCandidateKind,
    ) -> Option<ShadowCanonicalCandidate<'a>> {
        let target = self
            .validate_inferred_target(provider_id, model_id, model, candidate_id)
            .ok()?;
        let name_fingerprint = identity_fingerprint(&model.name);
        let id_fingerprint = model_id_fingerprint(model_id);
        let pair_witnesses = self
            .authoritative_pair_witnesses
            .get(&(
                name_fingerprint.to_string(),
                id_fingerprint.to_string(),
                target.id.clone(),
            ))
            .map_or(0, HashSet::len);
        let name_witnesses = self
            .authoritative_name_witnesses
            .get(&(name_fingerprint.to_string(), target.id.clone()))
            .map_or(0, HashSet::len);
        let id_witnesses = self
            .authoritative_id_witnesses
            .get(&(id_fingerprint.to_string(), target.id.clone()))
            .map_or(0, HashSet::len);
        Some(ShadowCanonicalCandidate {
            id: target.id.as_str(),
            name: target.name.as_str(),
            lab: target.lab.as_str(),
            kind,
            pair_witnesses,
            name_witnesses,
            id_witnesses,
        })
    }

    fn resolve_anchored_inference<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
    ) -> Result<CanonicalResolution<'a>, InferenceRejection> {
        let normalized_name = identity_fingerprint(&model.name);
        let id_fingerprint = model_id_fingerprint(model_id);
        if normalized_name.is_empty() || id_fingerprint.is_empty() {
            return Err(InferenceRejection::EmptyIdentity);
        }

        let Some(candidates) = self.canonical_name_candidates.get(&normalized_name) else {
            return Err(InferenceRejection::NoCanonicalName);
        };
        if candidates.len() != 1 {
            return Err(InferenceRejection::AmbiguousCanonicalName);
        }
        let candidate_id = candidates.iter().next().expect("one candidate");

        let Some(authoritative_targets) = self.authoritative_name_targets.get(&normalized_name)
        else {
            return Err(InferenceRejection::NoAuthoritativeNameAnchor);
        };
        if authoritative_targets.len() != 1 || !authoritative_targets.contains(candidate_id) {
            return Err(InferenceRejection::ConflictingAuthoritativeNameAnchors);
        }

        if !self
            .canonical_id_fingerprints
            .get(candidate_id)
            .is_some_and(|fingerprints| fingerprints.contains(&id_fingerprint))
        {
            return Err(InferenceRejection::UnseenIdFingerprint);
        }

        self.finish_inferred_resolution(
            provider_id,
            model_id,
            model,
            candidate_id,
            CanonicalResolutionKind::InferredCanonical,
        )
    }

    /// Reconcile a provider record that prefixes/qualifies both its display
    /// name and complete id with the canonical creator. The dual key is
    /// derived solely from the canonical registry and must select one target;
    /// arbitrary prefix stripping and fuzzy similarity are never used.
    fn resolve_qualified_inference<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
    ) -> Option<Result<CanonicalResolution<'a>, InferenceRejection>> {
        let qualified_name = compact_identity_fingerprint(&model.name);
        let full_id = compact_model_id_fingerprint(model_id);
        if qualified_name.is_empty() || full_id.is_empty() {
            return None;
        }
        let candidates = self
            .qualified_identity_candidates
            .get(&(qualified_name, full_id))?;
        if candidates.len() != 1 {
            return Some(Err(InferenceRejection::AmbiguousQualifiedIdentity));
        }
        let candidate_id = candidates.iter().next().expect("one candidate");
        Some(self.finish_inferred_resolution(
            provider_id,
            model_id,
            model,
            candidate_id,
            CanonicalResolutionKind::InferredQualifiedCanonical,
        ))
    }

    fn finish_inferred_resolution<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
        candidate_id: &str,
        kind: CanonicalResolutionKind,
    ) -> Result<CanonicalResolution<'a>, InferenceRejection> {
        let target = self.validate_inferred_target(provider_id, model_id, model, candidate_id)?;

        Ok(CanonicalResolution {
            id: target.id.as_str(),
            name: target.name.as_str(),
            lab: target.lab.as_str(),
            kind,
        })
    }

    fn validate_inferred_target<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
        candidate_id: &str,
    ) -> Result<&'a CanonicalTarget, InferenceRejection> {
        let target = self
            .canonical_models
            .get(candidate_id)
            .expect("canonical candidate must have a target");
        if self
            .independent_lab(provider_id, model_id)
            .is_some_and(|lab| lab != target.lab)
        {
            return Err(InferenceRejection::CreatorConflict);
        }
        if outputs_are_disjoint(
            model
                .modalities
                .as_ref()
                .map(|modalities| modalities.output.as_slice())
                .unwrap_or_default(),
            &target.output_modalities,
        ) {
            return Err(InferenceRejection::DisjointOutputModalities);
        }
        Ok(target)
    }

    /// Strong creator evidence only: a canonical namespace embedded in the
    /// model id, or an origin-like provider id that is itself a known lab.
    /// Name/family presentation fallbacks are deliberately excluded because
    /// they are not independent evidence for an identity merge.
    pub fn independent_lab<'a>(
        &'a self,
        provider_id: &'a str,
        model_id: &'a str,
    ) -> Option<&'a str> {
        if let Some((prefix, _)) = model_id.split_once('/') {
            if self.lab_slugs.contains(prefix) {
                return Some(prefix);
            }
        }
        self.lab_slugs.contains(provider_id).then_some(provider_id)
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
                        modalities: None,
                    },
                )
            })
            .collect();
        let base_model_refs = refs
            .iter()
            .map(|(offering, canonical)| ((*offering).to_string(), (*canonical).to_string()))
            .collect();
        Self::from_canonical_and_refs(&canonical, base_model_refs, None)
    }

    #[cfg(test)]
    pub(crate) fn from_test_catalog_with_refs(
        entries: &[(&str, &str, Option<&str>)],
        providers: &ProvidersMap,
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
                        modalities: None,
                    },
                )
            })
            .collect();
        let base_model_refs = refs
            .iter()
            .map(|(offering, canonical)| ((*offering).to_string(), (*canonical).to_string()))
            .collect();
        Self::from_canonical_and_refs(&canonical, base_model_refs, Some(providers))
    }
}

/// Stable identity fingerprint. Unicode compatibility-normalizes, lowercases,
/// preserves `+` as the semantic token `plus`, treats other punctuation as a
/// separator, and splits letter/number boundaries so `4.5` and `4-5` agree
/// without ever collapsing meaningful suffix tokens.
pub(crate) fn identity_fingerprint(raw: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TokenKind {
        Letter,
        Number,
    }

    fn flush(tokens: &mut Vec<String>, current: &mut String) {
        if !current.is_empty() {
            tokens.push(std::mem::take(current));
        }
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_kind: Option<TokenKind> = None;
    for normalized in raw.nfkc() {
        if normalized == '+' {
            flush(&mut tokens, &mut current);
            current_kind = None;
            tokens.push("plus".to_string());
            continue;
        }
        for ch in normalized.to_lowercase() {
            let kind = if ch.is_alphabetic() {
                Some(TokenKind::Letter)
            } else if ch.is_numeric() {
                Some(TokenKind::Number)
            } else {
                None
            };
            let Some(kind) = kind else {
                flush(&mut tokens, &mut current);
                current_kind = None;
                continue;
            };
            if current_kind.is_some_and(|existing| existing != kind) {
                flush(&mut tokens, &mut current);
            }
            current.push(ch);
            current_kind = Some(kind);
        }
    }
    flush(&mut tokens, &mut current);
    tokens.join("/")
}

pub(crate) fn model_id_fingerprint(model_id: &str) -> String {
    identity_fingerprint(model_id.rsplit('/').next().unwrap_or(model_id))
}

/// Namespace-aware model-id fingerprint used only as a fallback after the
/// leaf-id peer lane. Some providers preserve an origin namespace as a path
/// (`aion-labs/aion-3.0-mini`) while others flatten the same path into their
/// id (`aion-labs-aion-3-0-mini`). Normalizing the complete id makes those
/// spellings comparable without discarding semantic tokens such as `plus`.
pub(crate) fn full_model_id_fingerprint(model_id: &str) -> String {
    identity_fingerprint(model_id)
}

/// Weakest peer-only id fingerprint. Separator removal can reconcile compact
/// provider spellings such as `gpt-52` with `gpt-5.2`; semantic punctuation
/// handled by `identity_fingerprint` (notably `+` -> `plus`) remains present.
pub(crate) fn compact_model_id_fingerprint(model_id: &str) -> String {
    compact_identity_fingerprint(model_id)
}

fn compact_identity_fingerprint(raw: &str) -> String {
    identity_fingerprint(raw).replace('/', "")
}

pub(crate) fn outputs_are_disjoint(left: &[String], right: &[String]) -> bool {
    !left.is_empty()
        && !right.is_empty()
        && !left.iter().any(|left_value| {
            right
                .iter()
                .any(|right_value| left_value.eq_ignore_ascii_case(right_value))
        })
}

#[cfg(test)]
fn unique_target(candidates: Option<&HashSet<String>>) -> Option<&str> {
    let candidates = candidates?;
    (candidates.len() == 1).then(|| candidates.iter().next().expect("one candidate").as_str())
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

    fn providers(json: &str) -> ProvidersMap {
        serde_json::from_str(json).expect("valid provider fixture")
    }

    fn provider_model<'a>(providers: &'a ProvidersMap, provider: &str, model: &str) -> &'a Model {
        providers
            .get(provider)
            .and_then(|value| value.models.get(model))
            .expect("fixture offering")
    }

    fn canon(entries: &[(&str, &str, Option<&str>)]) -> HashMap<String, CanonicalModel> {
        entries
            .iter()
            .map(|(cid, name, fam)| {
                (
                    cid.to_string(),
                    CanonicalModel {
                        name: name.to_string(),
                        family: fam.map(String::from),
                        modalities: None,
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
    fn identity_fingerprint_preserves_semantics_but_normalizes_punctuation() {
        assert_eq!(identity_fingerprint("Grok 4.5"), "grok/4/5");
        assert_eq!(identity_fingerprint("grok-4-5"), "grok/4/5");
        assert_eq!(identity_fingerprint("GPT5.2Pro"), "gpt/5/2/pro");
        assert_ne!(
            identity_fingerprint("Command R"),
            identity_fingerprint("Command R+")
        );
        assert_eq!(identity_fingerprint("Command R+"), "command/r/plus");
    }

    #[test]
    fn full_id_fingerprint_normalizes_namespaces_without_dropping_tokens() {
        assert_eq!(
            full_model_id_fingerprint("aion-labs/aion-3.0-mini"),
            full_model_id_fingerprint("aion-labs-aion-3-0-mini")
        );
        assert_ne!(
            full_model_id_fingerprint("creator-a/orphan-2.0"),
            full_model_id_fingerprint("creator-b-orphan-2-0")
        );
        assert_ne!(
            full_model_id_fingerprint("cohere/command-r"),
            full_model_id_fingerprint("cohere-command-r+")
        );
        assert_eq!(
            compact_model_id_fingerprint("openai/gpt-5.2"),
            compact_model_id_fingerprint("openai-gpt-52")
        );
        assert_ne!(
            compact_model_id_fingerprint("cohere/command-r"),
            compact_model_id_fingerprint("cohere-command-r+")
        );
    }

    #[test]
    fn inferred_grok_uses_unanimous_name_and_known_id_anchors() {
        let providers = providers(
            r#"{
                "xai": {"id":"xai","name":"xAI","models":{
                    "grok-4.5":{"id":"grok-4.5","name":"Grok 4.5","family":"grok"}
                }},
                "kenari": {"id":"kenari","name":"Kenari","models":{
                    "grok-4-5":{"id":"grok-4-5","name":"Grok 4.5","family":"grok"}
                }},
                "llmgateway": {"id":"llmgateway","name":"LLM Gateway","models":{
                    "grok-4-5":{"id":"grok-4-5","name":"Grok 4.5","family":"grok"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("xai/grok-4.5", "Grok 4.5", Some("grok"))],
            &providers,
            &[("kenari/grok-4-5", "xai/grok-4.5")],
        );

        let identity = cat.resolve_model_identity(
            "llmgateway",
            "grok-4-5",
            provider_model(&providers, "llmgateway", "grok-4-5"),
        );
        let ModelIdentity::Canonical(resolution) = identity else {
            panic!("Grok should infer to the canonical target");
        };
        assert_eq!(resolution.id, "xai/grok-4.5");
        assert_eq!(resolution.kind, CanonicalResolutionKind::InferredCanonical);
    }

    #[test]
    fn creator_qualified_name_and_full_id_infer_fable_canonical_identity() {
        let providers = providers(
            r#"{
                "anthropic": {"id":"anthropic","name":"Anthropic","models":{
                    "claude-fable-5":{"id":"claude-fable-5","name":"Claude Fable 5","modalities":{"output":["text"]}}
                }},
                "digitalocean": {"id":"digitalocean","name":"DigitalOcean","models":{
                    "anthropic-claude-fable-5":{"id":"anthropic-claude-fable-5","name":"Anthropic Claude Fable 5","modalities":{"output":["text"]}}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(
                "anthropic/claude-fable-5",
                "Claude Fable 5",
                Some("claude-fable"),
            )],
            &providers,
            &[("anthropic/claude-fable-5", "anthropic/claude-fable-5")],
        );

        let identity = cat.resolve_model_identity(
            "digitalocean",
            "anthropic-claude-fable-5",
            provider_model(&providers, "digitalocean", "anthropic-claude-fable-5"),
        );
        let ModelIdentity::Canonical(resolution) = identity else {
            panic!("creator-qualified Fable offering should resolve");
        };
        assert_eq!(resolution.id, "anthropic/claude-fable-5");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredQualifiedCanonical
        );
    }

    #[test]
    fn creator_qualified_inference_requires_both_name_and_full_id() {
        let providers = providers(
            r#"{
                "origin": {"id":"origin","name":"Origin","models":{
                    "claude-fable-5":{"id":"claude-fable-5","name":"Claude Fable 5","modalities":{"output":["text"]}}
                }},
                "name-only": {"id":"name-only","name":"Name Only","models":{
                    "other-claude-fable-5":{"id":"other-claude-fable-5","name":"Anthropic Claude Fable 5","modalities":{"output":["text"]}}
                }},
                "id-only": {"id":"id-only","name":"ID Only","models":{
                    "anthropic-claude-fable-5":{"id":"anthropic-claude-fable-5","name":"Claude Fable 5","modalities":{"output":["text"]}}
                }},
                "modality": {"id":"modality","name":"Modality","models":{
                    "anthropic-claude-fable-5":{"id":"anthropic-claude-fable-5","name":"Anthropic Claude Fable 5","modalities":{"output":["image"]}}
                }}
            }"#,
        );
        let canonical: HashMap<String, CanonicalModel> = serde_json::from_str(
            r#"{
                "anthropic/claude-fable-5": {
                    "name":"Claude Fable 5",
                    "family":"claude-fable",
                    "modalities":{"output":["text"]}
                }
            }"#,
        )
        .expect("valid canonical json");
        let cat = LabCatalog::from_canonical_and_refs(
            &canonical,
            [(
                "origin/claude-fable-5".to_string(),
                "anthropic/claude-fable-5".to_string(),
            )]
            .into_iter()
            .collect(),
            Some(&providers),
        );

        for (provider, id) in [
            ("name-only", "other-claude-fable-5"),
            ("id-only", "anthropic-claude-fable-5"),
        ] {
            assert!(matches!(
                cat.resolve_model_identity(provider, id, provider_model(&providers, provider, id)),
                ModelIdentity::Unlinked(_)
            ));
        }
        assert!(matches!(
            cat.resolve_model_identity(
                "modality",
                "anthropic-claude-fable-5",
                provider_model(&providers, "modality", "anthropic-claude-fable-5")
            ),
            ModelIdentity::Unlinked(InferenceRejection::DisjointOutputModalities)
        ));
    }

    #[test]
    fn shadow_exact_pair_requires_provider_observed_pair() {
        let providers = providers(
            r#"{
                "anchor":{"id":"anchor","name":"Anchor","models":{
                    "alias-5":{"id":"alias-5","name":"Special Five","modalities":{"output":["text"]}}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "alias-5":{"id":"alias-5","name":"Special Five","modalities":{"output":["text"]}}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("creator/canonical-5", "Canonical Five", None)],
            &providers,
            &[("anchor/alias-5", "creator/canonical-5")],
        );
        let model = provider_model(&providers, "candidate", "alias-5");

        assert!(matches!(
            cat.resolve_model_identity("candidate", "alias-5", model),
            ModelIdentity::Unlinked(_)
        ));
        let shadow = cat
            .shadow_canonical_candidate("candidate", "alias-5", model)
            .expect("exact pair shadow candidate");
        assert_eq!(shadow.id, "creator/canonical-5");
        assert_eq!(shadow.kind, ShadowCandidateKind::ExactAuthoritativePair);
        assert_eq!(shadow.pair_witnesses, 1);
        assert_eq!(shadow.name_witnesses, 1);
        assert_eq!(shadow.id_witnesses, 1);
    }

    #[test]
    fn shadow_one_sided_creator_qualification_is_not_an_active_merge() {
        let providers = providers(
            r#"{
                "openai":{"id":"openai","name":"OpenAI","models":{
                    "gpt-5":{"id":"gpt-5","name":"GPT-5","modalities":{"output":["text"]}}
                }},
                "helicone":{"id":"helicone","name":"Helicone","models":{
                    "gpt-5":{"id":"gpt-5","name":"OpenAI GPT-5","modalities":{"output":["text"]}}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("openai/gpt-5", "GPT-5", None)],
            &providers,
            &[],
        );
        let model = provider_model(&providers, "helicone", "gpt-5");

        assert!(matches!(
            cat.resolve_model_identity("helicone", "gpt-5", model),
            ModelIdentity::Unlinked(_)
        ));
        let shadow = cat
            .shadow_canonical_candidate("helicone", "gpt-5", model)
            .expect("one-sided creator shadow candidate");
        assert_eq!(shadow.id, "openai/gpt-5");
        assert_eq!(shadow.kind, ShadowCandidateKind::OneSidedCreatorQualified);
        assert_eq!(shadow.pair_witnesses, 0);
        assert_eq!(shadow.id_witnesses, 1);
    }

    #[test]
    fn shadow_cross_aliases_surface_nemotron_without_merging() {
        let providers = providers(
            r#"{
                "wandb":{"id":"wandb","name":"W&B","models":{
                    "nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B":{"id":"nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B","name":"Nemotron 3 Ultra","modalities":{"output":["text"]}}
                }},
                "llmgateway":{"id":"llmgateway","name":"LLM Gateway","models":{
                    "nemotron-3-ultra-550b":{"id":"nemotron-3-ultra-550b","name":"Nemotron 3 Ultra 550B A55B","modalities":{"output":["text"]}}
                }},
                "digitalocean":{"id":"digitalocean","name":"DigitalOcean","models":{
                    "nemotron-3-ultra-550b":{"id":"nemotron-3-ultra-550b","name":"Nemotron 3 Ultra","modalities":{"output":["text"]}}
                }}
            }"#,
        );
        let target = "nvidia/nemotron-3-ultra-550b-a55b";
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(target, "Nemotron 3 Ultra 550B A55B", Some("nemotron"))],
            &providers,
            &[
                ("wandb/nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B", target),
                ("llmgateway/nemotron-3-ultra-550b", target),
            ],
        );
        let model = provider_model(&providers, "digitalocean", "nemotron-3-ultra-550b");

        assert!(matches!(
            cat.resolve_model_identity("digitalocean", "nemotron-3-ultra-550b", model),
            ModelIdentity::Unlinked(_)
        ));
        let shadow = cat
            .shadow_canonical_candidate("digitalocean", "nemotron-3-ultra-550b", model)
            .expect("cross-record shadow candidate");
        assert_eq!(shadow.id, target);
        assert_eq!(shadow.kind, ShadowCandidateKind::CrossAuthoritativeAliases);
        assert_eq!(shadow.pair_witnesses, 0);
        assert_eq!(shadow.name_witnesses, 1);
        assert_eq!(shadow.id_witnesses, 1);
    }

    #[test]
    fn shadow_alias_collisions_fail_closed() {
        let providers = providers(
            r#"{
                "alpha":{"id":"alpha","name":"Alpha","models":{
                    "shared":{"id":"shared","name":"Shared Alias"}
                }},
                "beta":{"id":"beta","name":"Beta","models":{
                    "shared":{"id":"shared","name":"Shared Alias"}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "shared":{"id":"shared","name":"Shared Alias"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("creator-a/model-a", "Model A", None),
                ("creator-b/model-b", "Model B", None),
            ],
            &providers,
            &[
                ("alpha/shared", "creator-a/model-a"),
                ("beta/shared", "creator-b/model-b"),
            ],
        );
        let model = provider_model(&providers, "candidate", "shared");

        assert!(cat
            .shadow_canonical_candidate("candidate", "shared", model)
            .is_none());
    }

    #[test]
    fn semantic_preview_cross_match_remains_shadow_only() {
        let providers = providers(
            r#"{
                "name-anchor":{"id":"name-anchor","name":"Name Anchor","models":{
                    "google/gemini-3-pro-preview":{"id":"google/gemini-3-pro-preview","name":"Gemini 3 Pro","modalities":{"output":["text"]}}
                }},
                "id-anchor":{"id":"id-anchor","name":"ID Anchor","models":{
                    "gemini-3-pro":{"id":"gemini-3-pro","name":"Gemini 3 Pro Preview","modalities":{"output":["text"]}}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "gemini-3-pro":{"id":"gemini-3-pro","name":"Gemini 3 Pro","modalities":{"output":["text"]}}
                }}
            }"#,
        );
        let target = "google/gemini-3-pro-preview";
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(target, "Gemini 3 Pro Preview", None)],
            &providers,
            &[
                ("name-anchor/google/gemini-3-pro-preview", target),
                ("id-anchor/gemini-3-pro", target),
            ],
        );
        let model = provider_model(&providers, "candidate", "gemini-3-pro");

        assert!(matches!(
            cat.resolve_model_identity("candidate", "gemini-3-pro", model),
            ModelIdentity::Unlinked(_)
        ));
        assert_eq!(
            cat.shadow_canonical_candidate("candidate", "gemini-3-pro", model)
                .expect("review-only preview candidate")
                .kind,
            ShadowCandidateKind::CrossAuthoritativeAliases
        );
    }

    #[test]
    fn colliding_creator_qualified_keys_fail_closed() {
        let providers = providers(
            r#"{
                "gateway": {"id":"gateway","name":"Gateway","models":{
                    "x-ai-model-1":{"id":"x-ai-model-1","name":"xAI Model 1","modalities":{"output":["text"]}}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("x-ai/model-1", "Model 1", None),
                ("xai/model-1", "Model 1", None),
            ],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "x-ai-model-1",
                provider_model(&providers, "gateway", "x-ai-model-1")
            ),
            ModelIdentity::Unlinked(InferenceRejection::AmbiguousQualifiedIdentity)
        ));
    }

    #[test]
    fn conflicting_authoritative_name_targets_fail_closed() {
        let providers = providers(
            r#"{
                "origin": {"id":"origin","name":"Origin","models":{
                    "gpt-5.2":{"id":"gpt-5.2","name":"GPT 5.2"}
                }},
                "vercel": {"id":"vercel","name":"Vercel","models":{
                    "openai/gpt-5.2-pro":{"id":"openai/gpt-5.2-pro","name":"GPT 5.2"}
                }},
                "unlinked": {"id":"unlinked","name":"Unlinked","models":{
                    "gpt-5-2":{"id":"gpt-5-2","name":"GPT 5.2"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("openai/gpt-5.2", "GPT-5.2", Some("gpt")),
                ("openai/gpt-5.2-pro", "GPT-5.2 Pro", Some("gpt")),
            ],
            &providers,
            &[
                ("origin/gpt-5.2", "openai/gpt-5.2"),
                ("vercel/openai/gpt-5.2-pro", "openai/gpt-5.2-pro"),
            ],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "unlinked",
                "gpt-5-2",
                provider_model(&providers, "unlinked", "gpt-5-2")
            ),
            ModelIdentity::Unlinked(InferenceRejection::ConflictingAuthoritativeNameAnchors)
        ));
    }

    #[test]
    fn ambiguous_preview_and_ga_names_fail_closed() {
        let providers = providers(
            r#"{
                "anchor": {"id":"anchor","name":"Anchor","models":{
                    "nano-banana-pro":{"id":"nano-banana-pro","name":"Nano Banana Pro"}
                }},
                "unlinked": {"id":"unlinked","name":"Unlinked","models":{
                    "nano-banana-pro":{"id":"nano-banana-pro","name":"Nano Banana Pro"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("google/nano-banana-pro", "Nano Banana Pro", Some("gemini")),
                (
                    "google/nano-banana-pro-preview",
                    "Nano Banana Pro",
                    Some("gemini"),
                ),
            ],
            &providers,
            &[("anchor/nano-banana-pro", "google/nano-banana-pro")],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "unlinked",
                "nano-banana-pro",
                provider_model(&providers, "unlinked", "nano-banana-pro")
            ),
            ModelIdentity::Unlinked(InferenceRejection::AmbiguousCanonicalName)
        ));
    }

    #[test]
    fn unseen_semantic_suffix_never_merges_into_base_model() {
        let providers = providers(
            r#"{
                "anchor": {"id":"anchor","name":"Anchor","models":{
                    "gpt-5.2":{"id":"gpt-5.2","name":"GPT 5.2"}
                }},
                "unlinked": {"id":"unlinked","name":"Unlinked","models":{
                    "gpt-5-2-pro":{"id":"gpt-5-2-pro","name":"GPT 5.2"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("openai/gpt-5.2", "GPT-5.2", Some("gpt"))],
            &providers,
            &[("anchor/gpt-5.2", "openai/gpt-5.2")],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "unlinked",
                "gpt-5-2-pro",
                provider_model(&providers, "unlinked", "gpt-5-2-pro")
            ),
            ModelIdentity::Unlinked(InferenceRejection::UnseenIdFingerprint)
        ));
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

    /// Live, explicitly-invoked audit: hold out one provider at a time by
    /// removing all of its embedded `base_model` edges *and* all of its
    /// offerings from the alias indexes. The remaining providers must recover
    /// only correct active or shadow targets for the held-out explicit edges.
    /// Ordinary `mise run test` never depends on the network.
    #[test]
    #[ignore = "live models.dev provider-holdout conformance audit"]
    fn live_provider_holdout_has_zero_wrong_inferences() {
        #[derive(Deserialize)]
        struct LiveCatalog {
            providers: ProvidersMap,
            models: HashMap<String, CanonicalModel>,
        }

        let snapshot: LiveCatalog = reqwest::blocking::get("https://models.dev/catalog.json")
            .expect("fetch live catalog")
            .json()
            .expect("parse live catalog");
        let artifact: BaseModelRefsFile = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/models-dev-base-model-refs.json"
        )))
        .expect("parse embedded refs");

        let held_out_providers: std::collections::BTreeSet<_> = artifact
            .refs
            .keys()
            .filter_map(|offering| offering.split_once('/').map(|(provider, _)| provider))
            .collect();
        let mut audited = 0usize;
        let mut exact = 0usize;
        let mut inferred = 0usize;
        let mut inferred_qualified = 0usize;
        let mut shadow_exact_pair = 0usize;
        let mut shadow_one_sided = 0usize;
        let mut shadow_cross = 0usize;
        let mut exact_conflicts = Vec::new();
        let mut active_wrong = Vec::new();
        let mut shadow_wrong = Vec::new();

        for held_out_provider in held_out_providers {
            let Some(held_out_models) = snapshot.providers.get(held_out_provider) else {
                continue;
            };
            let training_providers: ProvidersMap = snapshot
                .providers
                .iter()
                .filter(|(provider, _)| provider.as_str() != held_out_provider)
                .map(|(provider, value)| (provider.clone(), value.clone()))
                .collect();
            let masked_refs = artifact
                .refs
                .iter()
                .filter(|(offering, _)| {
                    offering
                        .split_once('/')
                        .is_none_or(|(provider, _)| provider != held_out_provider)
                })
                .map(|(offering, target)| (offering.clone(), target.clone()))
                .collect();
            let masked = LabCatalog::from_canonical_and_refs(
                &snapshot.models,
                masked_refs,
                Some(&training_providers),
            );

            for (offering_key, expected_target) in artifact.refs.iter().filter(|(offering, _)| {
                offering
                    .split_once('/')
                    .is_some_and(|(provider, _)| provider == held_out_provider)
            }) {
                if !snapshot.models.contains_key(expected_target) {
                    continue;
                }
                let Some((_, model_id)) = offering_key.split_once('/') else {
                    continue;
                };
                let Some(model) = held_out_models.models.get(model_id) else {
                    continue;
                };
                audited += 1;

                match masked.resolve_model_identity(held_out_provider, model_id, model) {
                    ModelIdentity::Canonical(resolution) => match resolution.kind {
                        CanonicalResolutionKind::InferredCanonical => {
                            inferred += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                        "{offering_key}: inferred {} but explicit target is {expected_target}",
                                        resolution.id
                                    ));
                            }
                        }
                        CanonicalResolutionKind::InferredQualifiedCanonical => {
                            inferred_qualified += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                        "{offering_key}: inferred-qualified {} but explicit target is {expected_target}",
                                        resolution.id
                                    ));
                            }
                        }
                        CanonicalResolutionKind::AuthoritativeDirectId
                        | CanonicalResolutionKind::AuthoritativeScopedId => {
                            exact += 1;
                            if resolution.id != expected_target {
                                exact_conflicts.push(format!(
                                        "{offering_key}: exact fallback {} differs from explicit target {expected_target}",
                                        resolution.id
                                    ));
                            }
                        }
                        CanonicalResolutionKind::AuthoritativeRef => {
                            panic!("held-out provider retained an explicit ref: {offering_key}")
                        }
                    },
                    ModelIdentity::Unlinked(_) => {
                        let Some(candidate) =
                            masked.shadow_canonical_candidate(held_out_provider, model_id, model)
                        else {
                            continue;
                        };
                        match candidate.kind {
                            ShadowCandidateKind::ExactAuthoritativePair => {
                                shadow_exact_pair += 1;
                            }
                            ShadowCandidateKind::OneSidedCreatorQualified => {
                                shadow_one_sided += 1;
                            }
                            ShadowCandidateKind::CrossAuthoritativeAliases => {
                                shadow_cross += 1;
                            }
                        }
                        if candidate.id != expected_target {
                            shadow_wrong.push(format!(
                                "{offering_key}: shadowed {} but explicit target is {expected_target}",
                                candidate.id
                            ));
                        }
                    }
                }
            }
        }

        println!(
            "provider-holdout audit: {audited} current explicit refs; active = {exact} exact ({} conflicts where the held-out explicit ref is more specific) + {inferred} anchored + {inferred_qualified} creator-qualified, {} wrong inferred; shadow = {shadow_exact_pair} exact-pair + {shadow_one_sided} one-sided creator + {shadow_cross} cross-record, {} wrong",
            exact_conflicts.len(),
            active_wrong.len(),
            shadow_wrong.len()
        );
        for conflict in exact_conflicts {
            println!("held-out exact conflict: {conflict}");
        }
        assert!(
            inferred + inferred_qualified > 0,
            "provider holdout must exercise active inferred canonical matches"
        );
        assert!(
            shadow_exact_pair + shadow_one_sided + shadow_cross > 0,
            "provider holdout must exercise shadow candidate matches"
        );
        assert!(
            active_wrong.is_empty(),
            "wrong active targets:\n{}",
            active_wrong.join("\n")
        );
        assert!(
            shadow_wrong.is_empty(),
            "wrong shadow targets:\n{}",
            shadow_wrong.join("\n")
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
