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
//! For offerings left unlinked by those authoritative tiers, deliberately
//! narrow resolvers may infer canonical identity. The first requires a unique
//! normalized canonical name, unanimous authoritative targets for that name,
//! and an exact token fingerprint already observed for that target. The second
//! reconciles provider records that qualify both name and id with the creator
//! (`Anthropic Claude Fable 5` / `anthropic-claude-fable-5`) when that dual key
//! selects exactly one canonical target. Four reconciliation lanes then use
//! exact, uniquely targeted evidence learned only from authoritative provider
//! records: a matching name/id pair, a creator-qualified name plus id alias,
//! agreeing name and id aliases observed on separate records, or a complete id
//! alias whose target does not contradict canonical or authoritative name
//! evidence. A canonical self-anchor lane then covers offerings models.dev has
//! never anchored at all: the registry's own name and leaf id must select the
//! same single record, and no authoritative alias may point anywhere else. A
//! final creator-prefixed-id lane mirrors the one-sided creator lane from the
//! id side — the id must spell the target's own lab tokens followed by its
//! canonical leaf id, and the plain display name must independently select the
//! same record. No inferred result seeds another match, and no inference lane
//! overrides an authoritative result.
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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
    InferredExactPairCanonical,
    InferredOneSidedCreatorCanonical,
    InferredCrossAliasCanonical,
    InferredFullIdCanonical,
    InferredSelfAnchorCanonical,
    InferredCreatorPrefixedCanonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceRejection {
    EmptyIdentity,
    NoCanonicalName,
    AmbiguousCanonicalName,
    AmbiguousQualifiedIdentity,
    AmbiguousCreatorPrefixedId,
    NoAuthoritativeNameAnchor,
    ConflictingAuthoritativeNameAnchors,
    ConflictingCanonicalName,
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
            Self::AmbiguousCreatorPrefixedId => "ambiguous creator-prefixed id",
            Self::NoAuthoritativeNameAnchor => "no authoritative name anchor",
            Self::ConflictingAuthoritativeNameAnchors => "conflicting name anchors",
            Self::ConflictingCanonicalName => "canonical name conflicts with id alias",
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

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct ReconciliationEvidence<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub lab: &'a str,
    pub kind: CanonicalResolutionKind,
    pub pair_witnesses: usize,
    pub name_witnesses: usize,
    pub id_witnesses: usize,
    pub full_id_witnesses: usize,
    /// `"leaf"` / `"full"` for a creator-prefixed resolution, `None` otherwise.
    /// The full-id branch had no live firing when the lane shipped; the audit
    /// prints the branch so a first one is visible.
    pub creator_prefixed_key: Option<&'static str>,
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
    /// Normalized canonical leaf-id fingerprint -> every canonical id whose own
    /// leaf id spells it. Seeded exclusively from the canonical registry — a
    /// provider alias must never widen this key set, or the self-anchor lane's
    /// uniqueness guard would inherit provider spelling collisions. Gated on a
    /// named record like every sibling index (the registry has none without a
    /// name; an unnamed twin could not be selected by name anyway).
    canonical_leaf_id_candidates: HashMap<String, HashSet<String>>,
    /// Creator-prefixed canonical id fingerprint -> every canonical id it
    /// spells. Keys are `P(lab)` tokens followed by the record's own canonical
    /// leaf-id tokens (see `creator_prefixes` for the pinned prefix rule),
    /// joined token-preserving and order-sensitive — never the compacted
    /// fingerprint the dual creator-qualified lane uses, and never a
    /// string-prefix test. Seeded exclusively from the canonical registry, so a
    /// provider spelling (least of all an inferred one) can never widen it.
    /// Measured on the live registry 2026-07-31: 581 keys, 0 internal
    /// collisions and none against canonical or observed authoritative leaf
    /// ids.
    creator_prefixed_id_candidates: HashMap<String, HashSet<String>>,
    /// `(creator-qualified display name, full model id)` -> canonical targets.
    /// Both components use the separator-compacted semantic fingerprint; the
    /// target must still be unique at resolution time.
    qualified_identity_candidates: HashMap<(String, String), HashSet<String>>,
    /// Creator-qualified canonical display name -> canonical targets, kept
    /// separate from the dual-field lane for one-sided qualification.
    qualified_name_candidates: HashMap<String, HashSet<String>>,
    /// Exact provider-observed leaf-id fingerprint -> authoritative targets.
    /// Unlike `canonical_id_fingerprints`, this is never seeded from the
    /// canonical registry itself.
    authoritative_id_targets: HashMap<String, HashSet<String>>,
    /// Exact provider-observed `(name, leaf id)` pair -> authoritative targets.
    authoritative_pair_targets: HashMap<(String, String), HashSet<String>>,
    /// Exact token-preserving complete model-id fingerprint -> authoritative
    /// targets. Unlike the leaf-id map, this retains creator namespaces.
    authoritative_full_id_targets: HashMap<String, HashSet<String>>,
    /// Provider witnesses for each authoritative alias edge. Provider count is
    /// not assumed to mean source independence, but is retained for audit.
    #[cfg(test)]
    authoritative_name_witnesses: HashMap<(String, String), HashSet<String>>,
    #[cfg(test)]
    authoritative_id_witnesses: HashMap<(String, String), HashSet<String>>,
    #[cfg(test)]
    authoritative_pair_witnesses: HashMap<(String, String, String), HashSet<String>>,
    #[cfg(test)]
    authoritative_full_id_witnesses: HashMap<(String, String), HashSet<String>>,
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
                        .insert(id_fingerprint.clone());
                    cat.canonical_leaf_id_candidates
                        .entry(id_fingerprint.clone())
                        .or_default()
                        .insert(cid.clone());
                    for prefix in creator_prefixes(lab) {
                        cat.creator_prefixed_id_candidates
                            .entry(format!("{prefix}/{id_fingerprint}"))
                            .or_default()
                            .insert(cid.clone());
                    }
                }
                let full_id_fingerprint = compact_model_id_fingerprint(cid);
                if !full_id_fingerprint.is_empty() {
                    for creator in [lab.to_string(), lab_display(lab)] {
                        let qualified_name =
                            compact_identity_fingerprint(&format!("{creator} {}", m.name));
                        if !qualified_name.is_empty() {
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
            let anchors: Vec<(String, String, String, String, String)> = providers
                .iter()
                .flat_map(|(provider_id, provider)| {
                    provider.models.iter().filter_map(|(model_id, model)| {
                        let resolution = cat.resolve_authoritative(provider_id, model_id)?;
                        let name = identity_fingerprint(&model.name);
                        let id = model_id_fingerprint(model_id);
                        let full_id = full_model_id_fingerprint(model_id);
                        (!name.is_empty() && !id.is_empty() && !full_id.is_empty()).then(|| {
                            (
                                provider_id.clone(),
                                name,
                                resolution.id.to_string(),
                                id,
                                full_id,
                            )
                        })
                    })
                })
                .collect();
            for (_provider_id, name, canonical_id, id_fingerprint, full_id_fingerprint) in anchors {
                cat.authoritative_name_targets
                    .entry(name.clone())
                    .or_default()
                    .insert(canonical_id.clone());
                cat.canonical_id_fingerprints
                    .entry(canonical_id.clone())
                    .or_default()
                    .insert(id_fingerprint.clone());
                cat.authoritative_id_targets
                    .entry(id_fingerprint.clone())
                    .or_default()
                    .insert(canonical_id.clone());
                cat.authoritative_pair_targets
                    .entry((name.clone(), id_fingerprint.clone()))
                    .or_default()
                    .insert(canonical_id.clone());
                cat.authoritative_full_id_targets
                    .entry(full_id_fingerprint.clone())
                    .or_default()
                    .insert(canonical_id.clone());
                #[cfg(test)]
                {
                    cat.authoritative_name_witnesses
                        .entry((name.clone(), canonical_id.clone()))
                        .or_default()
                        .insert(_provider_id.clone());
                    cat.authoritative_id_witnesses
                        .entry((id_fingerprint.clone(), canonical_id.clone()))
                        .or_default()
                        .insert(_provider_id.clone());
                    cat.authoritative_pair_witnesses
                        .entry((name, id_fingerprint, canonical_id.clone()))
                        .or_default()
                        .insert(_provider_id.clone());
                    cat.authoritative_full_id_witnesses
                        .entry((full_id_fingerprint, canonical_id))
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
                    None => {
                        // Reconciliation may fill a missing evidence tier, but
                        // it must never bypass an earlier ambiguity or conflict.
                        if matches!(
                            anchored_rejection,
                            InferenceRejection::NoCanonicalName
                                | InferenceRejection::NoAuthoritativeNameAnchor
                                | InferenceRejection::UnseenIdFingerprint
                        ) {
                            match self.resolve_reconciliation_inference(
                                provider_id,
                                model_id,
                                model,
                            ) {
                                Some(Ok(resolution)) => ModelIdentity::Canonical(resolution),
                                Some(Err(rejection)) => ModelIdentity::Unlinked(rejection),
                                None => ModelIdentity::Unlinked(anchored_rejection),
                            }
                        } else {
                            ModelIdentity::Unlinked(anchored_rejection)
                        }
                    }
                }
            }
        }
    }

    /// Resolve the strongest exact reconciliation lane. Every alias index is
    /// built exclusively from authoritative provider records (the last two
    /// lanes' key indexes solely from the canonical registry), so inferred
    /// results can never seed or transitively expand another match.
    fn resolve_reconciliation_inference<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
    ) -> Option<Result<CanonicalResolution<'a>, InferenceRejection>> {
        let name = identity_fingerprint(&model.name);
        let id = model_id_fingerprint(model_id);
        if name.is_empty() || id.is_empty() {
            return None;
        }

        if let Some(target) = unique_target(
            self.authoritative_pair_targets
                .get(&(name.clone(), id.clone())),
        ) {
            return Some(self.finish_inferred_resolution(
                provider_id,
                model_id,
                model,
                target,
                CanonicalResolutionKind::InferredExactPairCanonical,
            ));
        }

        let qualified_name = compact_identity_fingerprint(&model.name);
        if let (Some(name_target), Some(id_target)) = (
            unique_target(self.qualified_name_candidates.get(&qualified_name)),
            unique_target(self.authoritative_id_targets.get(&id)),
        ) {
            if name_target == id_target {
                return Some(self.finish_inferred_resolution(
                    provider_id,
                    model_id,
                    model,
                    name_target,
                    CanonicalResolutionKind::InferredOneSidedCreatorCanonical,
                ));
            }
        }

        if let (Some(name_target), Some(id_target)) = (
            unique_target(self.authoritative_name_targets.get(&name)),
            unique_target(self.authoritative_id_targets.get(&id)),
        ) {
            if name_target == id_target {
                return Some(self.finish_inferred_resolution(
                    provider_id,
                    model_id,
                    model,
                    name_target,
                    CanonicalResolutionKind::InferredCrossAliasCanonical,
                ));
            }
        }

        let full_id = full_model_id_fingerprint(model_id);
        if let Some(target) = unique_target(self.authoritative_full_id_targets.get(&full_id)) {
            if self
                .authoritative_name_targets
                .get(&name)
                .is_some_and(|targets| targets.len() != 1 || !targets.contains(target))
            {
                return Some(Err(InferenceRejection::ConflictingAuthoritativeNameAnchors));
            }
            if self
                .canonical_name_candidates
                .get(&name)
                .is_some_and(|targets| !targets.contains(target))
            {
                return Some(Err(InferenceRejection::ConflictingCanonicalName));
            }
            return Some(self.finish_inferred_resolution(
                provider_id,
                model_id,
                model,
                target,
                CanonicalResolutionKind::InferredFullIdCanonical,
            ));
        }

        if let Some(resolution) =
            self.resolve_self_anchor_inference(provider_id, model_id, model, &name, &id, &full_id)
        {
            return Some(resolution);
        }

        self.resolve_creator_prefixed_inference(provider_id, model_id, model, &name, &id, &full_id)
    }

    /// Canonical self-anchor. Last resort, and the only lane whose evidence
    /// is entirely registry-derived: the offering's name selects one
    /// canonical record and its leaf id spells that same record's own leaf
    /// id. Registry self-agreement is NOT a substitute for a provider
    /// anchor — `moonshotai/kimi-k2.7-code-highspeed` is a canonical record
    /// whose name and leaf id both match an offering upstream links to the
    /// *base* `moonshotai/kimi-k2.7-code`. Anchor vacuity is therefore the
    /// primary guard: the lane may only claim names models.dev has never
    /// anchored, and any authoritative id alias pointing elsewhere refuses.
    ///
    /// Two conditions are measurably redundant today and are kept explicit
    /// anyway. `own_leaf_id` is entailed by `unique_canonical_leaf` because
    /// the leaf index is keyed by each record's own fingerprint — it must
    /// survive any change to that keying. `anchor_vacant` is currently
    /// unreachable as the sole refuser: the caller only admits
    /// `NoCanonicalName`/`NoAuthoritativeNameAnchor`/`UnseenIdFingerprint`,
    /// and the first two are excluded here by the name lookup while the
    /// third cannot coexist with `own_leaf_id`. The id-alias conditions
    /// carry the contradicted-variant refusal; none of the three may be
    /// dropped on the grounds that another currently covers it.
    ///
    /// Separated from `resolve_reconciliation_inference` only so the
    /// leave-one-out gate can evaluate this contract without the earlier
    /// lanes preempting it; the caller's ordering is unchanged.
    fn resolve_self_anchor_inference<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
        name: &str,
        id: &str,
        full_id: &str,
    ) -> Option<Result<CanonicalResolution<'a>, InferenceRejection>> {
        let candidate_id = unique_target(self.canonical_name_candidates.get(name))?;
        let anchor_vacant = !self.authoritative_name_targets.contains_key(name);
        let own_leaf_id = model_id_fingerprint(candidate_id) == id;
        let unique_canonical_leaf =
            unique_target(self.canonical_leaf_id_candidates.get(id)) == Some(candidate_id);
        let id_alias_agrees = |aliases: &HashMap<String, HashSet<String>>, key: &str| {
            aliases
                .get(key)
                .is_none_or(|targets| targets.len() == 1 && targets.contains(candidate_id))
        };
        (anchor_vacant
            && own_leaf_id
            && unique_canonical_leaf
            && id_alias_agrees(&self.authoritative_id_targets, id)
            && id_alias_agrees(&self.authoritative_full_id_targets, full_id))
        .then(|| {
            self.finish_inferred_resolution(
                provider_id,
                model_id,
                model,
                candidate_id,
                CanonicalResolutionKind::InferredSelfAnchorCanonical,
            )
        })
    }

    /// Creator-prefixed id with a plain display name. Mirror of the
    /// one-sided creator lane, which qualifies the *name* instead: here the
    /// id must spell the target's own lab tokens followed by the target's
    /// canonical leaf id, and the name must independently select the same
    /// record. A provider self-prefix (`databricks-`) or a junk prefix
    /// (`deep-`) is excluded by construction — the consumed tokens have to
    /// spell the target's lab, so a wrong target generally fails the key,
    /// whereas a self-prefix is compatible with every possible target and
    /// carries no evidence at all.
    ///
    /// Registry name uniqueness is the entire correctness barrier on the
    /// three index keys whose upstream disposition is split
    /// (`anthropic/claude/opus/4/1`, `anthropic/claude/sonnet/4/5`,
    /// `deepseek/deepseek/r/1`): there the undated id selects the rolling
    /// record while models.dev links the dated snapshot. It must never be
    /// relaxed — a canonical name's trailing "(latest)" is precisely what
    /// separates a rolling record from its dated twin, so stripping it (or
    /// accepting an id-only match) would systematically prefer the wrong
    /// release.
    ///
    /// Separated from `resolve_reconciliation_inference` only so the
    /// leave-one-out gate can evaluate this contract without the earlier
    /// lanes preempting it; the caller's ordering is unchanged.
    fn resolve_creator_prefixed_inference<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
        name: &str,
        id: &str,
        full_id: &str,
    ) -> Option<Result<CanonicalResolution<'a>, InferenceRejection>> {
        let leaf_candidates = self.creator_prefixed_id_candidates.get(id);
        let full_candidates = self.creator_prefixed_id_candidates.get(full_id);
        if leaf_candidates.is_some()
            && full_candidates.is_some()
            && leaf_candidates != full_candidates
        {
            return Some(Err(InferenceRejection::AmbiguousCreatorPrefixedId));
        }
        if let Some(candidate_id) = unique_target(leaf_candidates.or(full_candidates)) {
            match self.canonical_name_candidates.get(name) {
                Some(targets) if targets.len() == 1 && targets.contains(candidate_id) => {
                    if self
                        .authoritative_name_targets
                        .get(name)
                        .is_some_and(|targets| {
                            targets.len() != 1 || !targets.contains(candidate_id)
                        })
                    {
                        return Some(Err(InferenceRejection::ConflictingAuthoritativeNameAnchors));
                    }
                    // `unique_target` declines rather than errors on an
                    // ambiguous alias set, so the cross-alias and complete-id
                    // lanes fall through silently and control reaches a lane
                    // whose own key evidence is registry-derived. The id-side
                    // conflict check therefore has to run here.
                    let id_alias_agrees = |aliases: &HashMap<String, HashSet<String>>,
                                           key: &str| {
                        aliases.get(key).is_none_or(|targets| {
                            targets.len() == 1 && targets.contains(candidate_id)
                        })
                    };
                    if id_alias_agrees(&self.authoritative_id_targets, id)
                        && id_alias_agrees(&self.authoritative_full_id_targets, full_id)
                    {
                        return Some(self.finish_inferred_resolution(
                            provider_id,
                            model_id,
                            model,
                            candidate_id,
                            CanonicalResolutionKind::InferredCreatorPrefixedCanonical,
                        ));
                    }
                }
                // An id-only match must never resolve: that would degrade the
                // lane into a complete-id lane keyed on registry spelling.
                None => {}
                Some(_) => return Some(Err(InferenceRejection::ConflictingCanonicalName)),
            }
        }

        None
    }

    /// Audit receipt for an offering resolved by one of the five final
    /// reconciliation lanes. Witness counts remain test-only diagnostics and
    /// do not participate in identity decisions. A self-anchor row reports
    /// zero *name* witnesses by construction (C3 anchor vacuity), but its
    /// id-side witnesses may be nonzero — C6/C6b accept an authoritative
    /// alias that agrees with the target, and one of the lane's live
    /// recoveries carries exactly such a leaf-id witness. Do not tighten
    /// C6 to `is_none()` on the assumption agreement cannot happen.
    #[cfg(test)]
    pub fn reconciliation_evidence<'a>(
        &'a self,
        provider_id: &str,
        model_id: &str,
        model: &Model,
    ) -> Option<ReconciliationEvidence<'a>> {
        let ModelIdentity::Canonical(resolution) =
            self.resolve_model_identity(provider_id, model_id, model)
        else {
            return None;
        };
        if !matches!(
            resolution.kind,
            CanonicalResolutionKind::InferredExactPairCanonical
                | CanonicalResolutionKind::InferredOneSidedCreatorCanonical
                | CanonicalResolutionKind::InferredCrossAliasCanonical
                | CanonicalResolutionKind::InferredFullIdCanonical
                | CanonicalResolutionKind::InferredSelfAnchorCanonical
                | CanonicalResolutionKind::InferredCreatorPrefixedCanonical
        ) {
            return None;
        }
        let name_fingerprint = identity_fingerprint(&model.name);
        let id_fingerprint = model_id_fingerprint(model_id);
        let full_id_fingerprint = full_model_id_fingerprint(model_id);
        let pair_witnesses = self
            .authoritative_pair_witnesses
            .get(&(
                name_fingerprint.to_string(),
                id_fingerprint.to_string(),
                resolution.id.to_string(),
            ))
            .map_or(0, HashSet::len);
        let name_witnesses = self
            .authoritative_name_witnesses
            .get(&(name_fingerprint.to_string(), resolution.id.to_string()))
            .map_or(0, HashSet::len);
        let id_witnesses = self
            .authoritative_id_witnesses
            .get(&(id_fingerprint.to_string(), resolution.id.to_string()))
            .map_or(0, HashSet::len);
        let full_id_witnesses = self
            .authoritative_full_id_witnesses
            .get(&(full_id_fingerprint.to_string(), resolution.id.to_string()))
            .map_or(0, HashSet::len);
        Some(ReconciliationEvidence {
            id: resolution.id,
            name: resolution.name,
            lab: resolution.lab,
            kind: resolution.kind,
            pair_witnesses,
            name_witnesses,
            id_witnesses,
            full_id_witnesses,
            creator_prefixed_key: matches!(
                resolution.kind,
                CanonicalResolutionKind::InferredCreatorPrefixedCanonical
            )
            .then(|| {
                if self
                    .creator_prefixed_id_candidates
                    .contains_key(&id_fingerprint)
                {
                    "leaf"
                } else {
                    "full"
                }
            }),
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
    ///
    /// A namespace prefix that is *not* a recognized lab still attributes the
    /// model away from the serving provider (`nvidia/qwen/qwen3-…` is Qwen's
    /// model served by NVIDIA), so it must not fall through to the
    /// provider-id heuristic — that fallback wrongly claimed the provider as
    /// creator and blocked otherwise-exact identity matches.
    pub fn independent_lab<'a>(
        &'a self,
        provider_id: &'a str,
        model_id: &'a str,
    ) -> Option<&'a str> {
        if let Some((prefix, _)) = model_id.split_once('/') {
            return self.lab_slugs.contains(prefix).then_some(prefix);
        }
        self.lab_slugs.contains(provider_id).then_some(provider_id)
    }

    /// Presentation tokens that spell this lab's identity: slug tokens,
    /// display-name tokens, and the generic corporate suffix "ai". Used by the
    /// peer name-relaxation lane to recognize that a name difference such as
    /// `Meta: Llama 3.2 3B Instruct` vs `Llama 3.2 3B Instruct` is creator
    /// attribution, not model identity. Family names are deliberately
    /// excluded — families are version-line names (`claude-opus`), and
    /// treating their tokens as neutral could erase a real variant.
    pub(crate) fn creator_alias_tokens(&self, lab: &str) -> HashSet<String> {
        let mut tokens = HashSet::new();
        for source in [lab.to_string(), lab_display(lab)] {
            tokens.extend(fingerprint_tokens(&identity_fingerprint(&source)));
        }
        tokens.insert("ai".to_string());
        tokens
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

/// Pinned prefix rule `P(lab)` for `creator_prefixed_id_candidates`: the lab's
/// slug fingerprint and its display-name fingerprint, each additionally
/// extended by **one** trailing `ai` token when its last token is not already
/// literally `ai` (`moonshotai` also yields `moonshotai/ai`; `moonshot/ai` and
/// `z/ai` are left alone). Last-token equality, never a string `ends_with`:
/// the two readings differ by 83 keys on the live registry, and this is the
/// 581-key one. Family names and provider ids are deliberately absent — only
/// tokens that spell the target's own lab are falsifiable evidence.
fn creator_prefixes(lab: &str) -> BTreeSet<String> {
    let mut prefixes = BTreeSet::new();
    for source in [lab.to_string(), lab_display(lab)] {
        let fingerprint = identity_fingerprint(&source);
        if fingerprint.is_empty() {
            continue;
        }
        if fingerprint.rsplit('/').next() != Some("ai") {
            prefixes.insert(format!("{fingerprint}/ai"));
        }
        prefixes.insert(fingerprint);
    }
    prefixes
}

/// The token set of a fingerprint produced by `identity_fingerprint`.
pub(crate) fn fingerprint_tokens(fingerprint: &str) -> HashSet<String> {
    fingerprint
        .split('/')
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
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

        // Creator attribution in the name alone leaves the id unqualified and
        // the offering unlinked.
        assert!(matches!(
            cat.resolve_model_identity(
                "name-only",
                "other-claude-fable-5",
                provider_model(&providers, "name-only", "other-claude-fable-5")
            ),
            ModelIdentity::Unlinked(_)
        ));
        // Creator attribution in the id alone still fails the dual lane; the
        // later creator-prefixed lane claims it on its own evidence (the id
        // spells the target's lab plus its canonical leaf id, and the plain
        // name independently selects that same record).
        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "id-only",
            "anthropic-claude-fable-5",
            provider_model(&providers, "id-only", "anthropic-claude-fable-5"),
        ) else {
            panic!("creator-prefixed id with a plain name should resolve");
        };
        assert_eq!(resolution.id, "anthropic/claude-fable-5");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredCreatorPrefixedCanonical
        );
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
    fn exact_pair_reconciliation_requires_provider_observed_pair() {
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

        let ModelIdentity::Canonical(resolution) =
            cat.resolve_model_identity("candidate", "alias-5", model)
        else {
            panic!("exact authoritative pair should reconcile");
        };
        assert_eq!(resolution.id, "creator/canonical-5");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredExactPairCanonical
        );
        let evidence = cat
            .reconciliation_evidence("candidate", "alias-5", model)
            .expect("exact pair evidence");
        assert_eq!(evidence.pair_witnesses, 1);
        assert_eq!(evidence.name_witnesses, 1);
        assert_eq!(evidence.id_witnesses, 1);
    }

    #[test]
    fn one_sided_creator_qualification_reconciles_exact_id_alias() {
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

        let ModelIdentity::Canonical(resolution) =
            cat.resolve_model_identity("helicone", "gpt-5", model)
        else {
            panic!("one-sided creator qualification should reconcile");
        };
        assert_eq!(resolution.id, "openai/gpt-5");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredOneSidedCreatorCanonical
        );
        let evidence = cat
            .reconciliation_evidence("helicone", "gpt-5", model)
            .expect("one-sided creator evidence");
        assert_eq!(evidence.pair_witnesses, 0);
        assert_eq!(evidence.id_witnesses, 1);
    }

    #[test]
    fn cross_aliases_reconcile_nemotron() {
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

        let ModelIdentity::Canonical(resolution) =
            cat.resolve_model_identity("digitalocean", "nemotron-3-ultra-550b", model)
        else {
            panic!("cross-record aliases should reconcile Nemotron");
        };
        assert_eq!(resolution.id, target);
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredCrossAliasCanonical
        );
        let evidence = cat
            .reconciliation_evidence("digitalocean", "nemotron-3-ultra-550b", model)
            .expect("cross-record evidence");
        assert_eq!(evidence.pair_witnesses, 0);
        assert_eq!(evidence.name_witnesses, 1);
        assert_eq!(evidence.id_witnesses, 1);
    }

    #[test]
    fn full_id_alias_reconciles_different_provider_display_name() {
        let providers = providers(
            r#"{
                "google":{"id":"google","name":"Google","models":{
                    "gemini-3.1-flash-image-preview":{"id":"gemini-3.1-flash-image-preview","name":"Nano Banana 2","modalities":{"output":["image"]}}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "gemini-3.1-flash-image-preview":{"id":"gemini-3.1-flash-image-preview","name":"Gemini 3.1 Flash Image Preview","modalities":{"output":["image"]}}
                }}
            }"#,
        );
        let target = "google/gemini-3.1-flash-image-preview";
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(target, "Nano Banana 2", None)],
            &providers,
            &[],
        );
        let model = provider_model(&providers, "candidate", "gemini-3.1-flash-image-preview");

        let ModelIdentity::Canonical(resolution) =
            cat.resolve_model_identity("candidate", "gemini-3.1-flash-image-preview", model)
        else {
            panic!("complete authoritative id alias should reconcile");
        };
        assert_eq!(resolution.id, target);
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredFullIdCanonical
        );
        let evidence = cat
            .reconciliation_evidence("candidate", "gemini-3.1-flash-image-preview", model)
            .expect("full-id evidence");
        assert_eq!(evidence.full_id_witnesses, 1);
    }

    #[test]
    fn full_id_alias_cannot_override_authoritative_name_conflict() {
        let providers = providers(
            r#"{
                "id-anchor":{"id":"id-anchor","name":"ID Anchor","models":{
                    "anthropic/claude-sonnet-4":{"id":"anthropic/claude-sonnet-4","name":"Alias Four"}
                }},
                "name-anchor":{"id":"name-anchor","name":"Name Anchor","models":{
                    "dated-alias":{"id":"dated-alias","name":"Claude Sonnet 4.5"}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "anthropic/claude-sonnet-4":{"id":"anthropic/claude-sonnet-4","name":"Claude Sonnet 4.5"}
                }}
            }"#,
        );
        let base = "anthropic/claude-sonnet-4-0";
        let dated = "anthropic/claude-sonnet-4-5-20250929";
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                (base, "Claude Sonnet 4", Some("claude-sonnet")),
                (dated, "Claude Sonnet 4.5", Some("claude-sonnet")),
            ],
            &providers,
            &[
                ("id-anchor/anthropic/claude-sonnet-4", base),
                ("name-anchor/dated-alias", dated),
            ],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "candidate",
                "anthropic/claude-sonnet-4",
                provider_model(&providers, "candidate", "anthropic/claude-sonnet-4")
            ),
            ModelIdentity::Unlinked(InferenceRejection::ConflictingAuthoritativeNameAnchors)
        ));
    }

    #[test]
    fn full_id_alias_cannot_override_canonical_name_conflict() {
        let providers = providers(
            r#"{
                "anchor":{"id":"anchor","name":"Anchor","models":{
                    "shared/model":{"id":"shared/model","name":"Provider Alias"}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "shared/model":{"id":"shared/model","name":"Canonical B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("creator/model-a", "Canonical A", None),
                ("creator/model-b", "Canonical B", None),
            ],
            &providers,
            &[("anchor/shared/model", "creator/model-a")],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "candidate",
                "shared/model",
                provider_model(&providers, "candidate", "shared/model")
            ),
            ModelIdentity::Unlinked(InferenceRejection::ConflictingCanonicalName)
        ));
    }

    #[test]
    fn colliding_full_id_aliases_fail_closed() {
        let providers = providers(
            r#"{
                "alpha":{"id":"alpha","name":"Alpha","models":{
                    "shared/model":{"id":"shared/model","name":"Alpha Alias"}
                }},
                "beta":{"id":"beta","name":"Beta","models":{
                    "shared/model":{"id":"shared/model","name":"Beta Alias"}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "shared/model":{"id":"shared/model","name":"Unknown Alias"}
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
                ("alpha/shared/model", "creator-a/model-a"),
                ("beta/shared/model", "creator-b/model-b"),
            ],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "candidate",
                "shared/model",
                provider_model(&providers, "candidate", "shared/model")
            ),
            ModelIdentity::Unlinked(_)
        ));
    }

    #[test]
    fn reconciliation_alias_collisions_fail_closed() {
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

        assert!(matches!(
            cat.resolve_model_identity("candidate", "shared", model),
            ModelIdentity::Unlinked(_)
        ));
    }

    #[test]
    fn semantic_preview_cross_match_uses_models_dev_base_semantics() {
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

        let ModelIdentity::Canonical(resolution) =
            cat.resolve_model_identity("candidate", "gemini-3-pro", model)
        else {
            panic!("models.dev aliases should reconcile the preview canonical");
        };
        assert_eq!(resolution.id, target);
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredCrossAliasCanonical
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

    /// LANE A (canonical self-anchor) — an offering models.dev has never
    /// anchored under any spelling. The registry alone selects the target.
    #[test]
    fn self_anchor_resolves_with_no_provider_evidence_at_all() {
        let providers = providers(
            r#"{
                "llmgateway":{"id":"llmgateway","name":"LLM Gateway","models":{
                    "llama-4-scout-17b-instruct":{"id":"llama-4-scout-17b-instruct","name":"Llama 4 Scout 17B Instruct"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(
                "meta/llama-4-scout-17b-instruct",
                "Llama 4 Scout 17B Instruct",
                None,
            )],
            &providers,
            &[],
        );

        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "llmgateway",
            "llama-4-scout-17b-instruct",
            provider_model(&providers, "llmgateway", "llama-4-scout-17b-instruct"),
        ) else {
            panic!("registry name + own leaf id should self-anchor");
        };
        assert_eq!(resolution.id, "meta/llama-4-scout-17b-instruct");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredSelfAnchorCanonical
        );
    }

    /// C3 asks only that *this name* carries no anchor. An authoritative link
    /// filed under a different display name leaves the name vacuous.
    #[test]
    fn self_anchor_resolves_when_the_only_anchor_spells_the_name_differently() {
        let providers = providers(
            r#"{
                "sap-ai-core":{"id":"sap-ai-core","name":"SAP AI Core","models":{
                    "anthropic--claude-3.7-sonnet":{"id":"anthropic--claude-3.7-sonnet","name":"Anthropic Claude 3.7 Sonnet"}
                }},
                "abacus":{"id":"abacus","name":"Abacus","models":{
                    "claude-3-7-sonnet-20250219":{"id":"claude-3-7-sonnet-20250219","name":"Claude Sonnet 3.7"}
                }}
            }"#,
        );
        let target = "anthropic/claude-3-7-sonnet-20250219";
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(target, "Claude Sonnet 3.7", Some("claude-sonnet"))],
            &providers,
            &[("sap-ai-core/anthropic--claude-3.7-sonnet", target)],
        );

        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "abacus",
            "claude-3-7-sonnet-20250219",
            provider_model(&providers, "abacus", "claude-3-7-sonnet-20250219"),
        ) else {
            panic!("a differently-spelled anchor must not block the name");
        };
        assert_eq!(resolution.id, target);
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredSelfAnchorCanonical
        );
    }

    /// The worst case, taken from upstream: `moonshotai/kimi-k2.7-code-highspeed`
    /// is a canonical record whose name *and* leaf id both match an offering
    /// models.dev links to the base `moonshotai/kimi-k2.7-code`. Registry
    /// self-agreement is real here and still wrong — only the anchor evidence
    /// separates them, so the lane must refuse whenever any authoritative
    /// alias for that offering points at the base.
    #[test]
    fn self_anchor_refuses_variant_canonical_contradicted_by_upstream() {
        let base = "moonshotai/kimi-k2.7-code";
        let variant = "moonshotai/kimi-k2.7-code-highspeed";
        let canonical = &[
            (base, "Kimi K2.7 Code", Some("kimi-k2")),
            (variant, "Kimi K2.7 Code Highspeed", Some("kimi-k2")),
        ];

        // (a) upstream's own spelling: the anchor shares the offering's display
        // name, so the anchored lane is terminal before reconciliation runs.
        let same_name = providers(
            r#"{
                "moonshotai":{"id":"moonshotai","name":"Moonshot AI","models":{
                    "kimi-k2.7-code-highspeed":{"id":"kimi-k2.7-code-highspeed","name":"Kimi K2.7 Code Highspeed"}
                }},
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "kimi-k2.7-code-highspeed":{"id":"kimi-k2.7-code-highspeed","name":"Kimi K2.7 Code Highspeed"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            canonical,
            &same_name,
            &[("moonshotai/kimi-k2.7-code-highspeed", base)],
        );
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "kimi-k2.7-code-highspeed",
                provider_model(&same_name, "gateway", "kimi-k2.7-code-highspeed")
            ),
            ModelIdentity::Unlinked(InferenceRejection::ConflictingAuthoritativeNameAnchors)
        ));

        // (b) the anchor names the model differently and the candidate keeps a
        // namespace, so the name is vacuous and the complete-id lane declines:
        // control reaches this lane, and the leaf-id alias refuses it.
        let renamed_anchor = providers(
            r#"{
                "moonshotai":{"id":"moonshotai","name":"Moonshot AI","models":{
                    "kimi-k2.7-code-highspeed":{"id":"kimi-k2.7-code-highspeed","name":"Kimi K2.7 Code (High Speed)"}
                }},
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "moonshot/kimi-k2.7-code-highspeed":{"id":"moonshot/kimi-k2.7-code-highspeed","name":"Kimi K2.7 Code Highspeed"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            canonical,
            &renamed_anchor,
            &[("moonshotai/kimi-k2.7-code-highspeed", base)],
        );
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "moonshot/kimi-k2.7-code-highspeed",
                provider_model(
                    &renamed_anchor,
                    "gateway",
                    "moonshot/kimi-k2.7-code-highspeed"
                )
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));

        // (c) control: drop the contradicting upstream link and the same
        // offering self-anchors on the variant record. The refusal above is
        // caused by upstream evidence, not by the fixture's shape.
        let cat = LabCatalog::from_test_catalog_with_refs(canonical, &renamed_anchor, &[]);
        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "gateway",
            "moonshot/kimi-k2.7-code-highspeed",
            provider_model(
                &renamed_anchor,
                "gateway",
                "moonshot/kimi-k2.7-code-highspeed",
            ),
        ) else {
            panic!("without the contradicting anchor the variant self-anchors");
        };
        assert_eq!(resolution.id, variant);
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredSelfAnchorCanonical
        );
    }

    /// A rolling provider alias is not the dated snapshot it points at: the
    /// canonical record's own leaf id still carries the date.
    #[test]
    fn self_anchor_refuses_rolling_alias_of_a_dated_snapshot() {
        let providers = providers(
            r#"{
                "opencode":{"id":"opencode","name":"OpenCode","models":{
                    "claude-3-5-haiku":{"id":"claude-3-5-haiku","name":"Claude Haiku 3.5"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(
                "anthropic/claude-3-5-haiku-20241022",
                "Claude Haiku 3.5",
                Some("claude-haiku"),
            )],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "opencode",
                "claude-3-5-haiku",
                provider_model(&providers, "opencode", "claude-3-5-haiku")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// Leaf-id fingerprints are order-sensitive; a reordered id is not the
    /// canonical record's own leaf id even when every token survives.
    #[test]
    fn self_anchor_refuses_reordered_id_tokens() {
        let providers = providers(
            r#"{
                "poe":{"id":"poe","name":"Poe","models":{
                    "anthropic/claude-sonnet-3.7":{"id":"anthropic/claude-sonnet-3.7","name":"Claude Sonnet 3.7"}
                }}
            }"#,
        );
        for canonical_id in [
            "anthropic/claude-3-7-sonnet-20250219",
            // Same token multiset as the offering, different order only.
            "anthropic/claude-3-7-sonnet",
        ] {
            let cat = LabCatalog::from_test_catalog_with_refs(
                &[(canonical_id, "Claude Sonnet 3.7", Some("claude-sonnet"))],
                &providers,
                &[],
            );
            assert!(
                matches!(
                    cat.resolve_model_identity(
                        "poe",
                        "anthropic/claude-sonnet-3.7",
                        provider_model(&providers, "poe", "anthropic/claude-sonnet-3.7")
                    ),
                    ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
                ),
                "reordered id must not self-anchor onto {canonical_id}"
            );
        }
    }

    /// C4 compares against the canonical record's OWN leaf id, never the broad
    /// `canonical_id_fingerprints` set — that set also absorbs provider alias
    /// spellings, which would make the lane accept an alias as identity.
    #[test]
    fn self_anchor_requires_the_canonical_records_own_leaf_id() {
        let providers = providers(
            r#"{
                "sap-ai-core":{"id":"sap-ai-core","name":"SAP AI Core","models":{
                    "anthropic--claude-3.7-sonnet":{"id":"anthropic--claude-3.7-sonnet","name":"Anthropic Claude 3.7 Sonnet"}
                }},
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "sap/anthropic--claude-3.7-sonnet":{"id":"sap/anthropic--claude-3.7-sonnet","name":"Claude Sonnet 3.7"}
                }}
            }"#,
        );
        let target = "anthropic/claude-3-7-sonnet-20250219";
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[(target, "Claude Sonnet 3.7", Some("claude-sonnet"))],
            &providers,
            &[("sap-ai-core/anthropic--claude-3.7-sonnet", target)],
        );

        // Premise: the anchor did seed the alias into the broad fingerprint set,
        // so a membership test would have accepted this offering.
        assert!(cat
            .canonical_id_fingerprints
            .get(target)
            .is_some_and(|fingerprints| fingerprints
                .contains(&identity_fingerprint("anthropic--claude-3.7-sonnet"))));
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "sap/anthropic--claude-3.7-sonnet",
                provider_model(&providers, "gateway", "sap/anthropic--claude-3.7-sonnet")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// C2: a display name two canonical records share selects nothing.
    #[test]
    fn self_anchor_refuses_preview_and_ga_sharing_one_name() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
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
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "nano-banana-pro",
                provider_model(&providers, "gateway", "nano-banana-pro")
            ),
            ModelIdentity::Unlinked(InferenceRejection::AmbiguousCanonicalName)
        ));
    }

    /// A semantic suffix the registry never spells is a different model, even
    /// with no anchor anywhere to contradict it.
    #[test]
    fn self_anchor_refuses_unseen_semantic_suffix() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "gpt-5-2-pro":{"id":"gpt-5-2-pro","name":"GPT-5.2"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("openai/gpt-5.2", "GPT-5.2", Some("gpt"))],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "gpt-5-2-pro",
                provider_model(&providers, "gateway", "gpt-5-2-pro")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// C5: two canonical records spelling one leaf id make the leaf useless as
    /// a cross-creator guard, so the lane declines.
    #[test]
    fn self_anchor_refuses_colliding_canonical_leaf_ids() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "model-x":{"id":"model-x","name":"Alpha Model X"}
                }}
            }"#,
        );
        let colliding = LabCatalog::from_test_catalog_with_refs(
            &[
                ("creator-a/model-x", "Alpha Model X", None),
                ("creator-b/model-x", "Beta Model X", None),
            ],
            &providers,
            &[],
        );
        assert!(matches!(
            colliding.resolve_model_identity(
                "gateway",
                "model-x",
                provider_model(&providers, "gateway", "model-x")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));

        // Control: the collision is the only difference.
        let unique = LabCatalog::from_test_catalog_with_refs(
            &[("creator-a/model-x", "Alpha Model X", None)],
            &providers,
            &[],
        );
        let ModelIdentity::Canonical(resolution) = unique.resolve_model_identity(
            "gateway",
            "model-x",
            provider_model(&providers, "gateway", "model-x"),
        ) else {
            panic!("an uncontested canonical leaf id should self-anchor");
        };
        assert_eq!(resolution.id, "creator-a/model-x");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredSelfAnchorCanonical
        );
    }

    /// C6: models.dev filed this leaf-id spelling under a different target.
    #[test]
    fn self_anchor_refuses_when_an_authoritative_id_alias_points_elsewhere() {
        let providers = providers(
            r#"{
                "venice":{"id":"venice","name":"Venice","models":{
                    "openai/gpt-5.2":{"id":"openai/gpt-5.2","name":"Venice GPT 5.2 Pro"}
                }},
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "gpt-5-2":{"id":"gpt-5-2","name":"GPT-5.2"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("openai/gpt-5.2", "GPT-5.2", Some("gpt")),
                ("openai/gpt-5.2-pro", "GPT-5.2 Pro", Some("gpt")),
            ],
            &providers,
            &[("venice/openai/gpt-5.2", "openai/gpt-5.2-pro")],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "gpt-5-2",
                provider_model(&providers, "gateway", "gpt-5-2")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// C6b: a complete-id spelling with two authoritative targets makes the
    /// complete-id lane decline (not error), so this lane must run the id-side
    /// conflict check itself instead of inheriting one.
    #[test]
    fn self_anchor_refuses_when_an_authoritative_full_id_alias_points_elsewhere() {
        let providers = providers(
            r#"{
                "alpha":{"id":"alpha","name":"Alpha","models":{
                    "gpt/oss-120b":{"id":"gpt/oss-120b","name":"Alpha Alias"}
                }},
                "beta":{"id":"beta","name":"Beta","models":{
                    "gpt/oss-120-b":{"id":"gpt/oss-120-b","name":"Beta Alias"}
                }},
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "gpt-oss-120b":{"id":"gpt-oss-120b","name":"GPT OSS 120B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("openai/gpt-oss-120b", "GPT OSS 120B", None),
                ("creator-a/model-a", "Model A", None),
                ("creator-b/model-b", "Model B", None),
            ],
            &providers,
            &[
                ("alpha/gpt/oss-120b", "creator-a/model-a"),
                ("beta/gpt/oss-120-b", "creator-b/model-b"),
            ],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "gpt-oss-120b",
                provider_model(&providers, "gateway", "gpt-oss-120b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// C7: the shared creator and output-modality blockers still apply.
    #[test]
    fn self_anchor_respects_creator_and_output_blockers() {
        let foreign_lab = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "nvidia/llama-4-scout-17b-instruct":{"id":"nvidia/llama-4-scout-17b-instruct","name":"Llama 4 Scout 17B Instruct"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                (
                    "meta/llama-4-scout-17b-instruct",
                    "Llama 4 Scout 17B Instruct",
                    None,
                ),
                ("nvidia/nemotron-3", "Nemotron 3", None),
            ],
            &foreign_lab,
            &[],
        );
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "nvidia/llama-4-scout-17b-instruct",
                provider_model(&foreign_lab, "gateway", "nvidia/llama-4-scout-17b-instruct")
            ),
            ModelIdentity::Unlinked(InferenceRejection::CreatorConflict)
        ));

        let image_providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "nano-banana-3":{"id":"nano-banana-3","name":"Nano Banana 3","modalities":{"output":["image"]}}
                }}
            }"#,
        );
        let canonical: HashMap<String, CanonicalModel> = serde_json::from_str(
            r#"{
                "google/nano-banana-3": {
                    "name":"Nano Banana 3",
                    "modalities":{"output":["text"]}
                }
            }"#,
        )
        .expect("valid canonical json");
        let cat = LabCatalog::from_canonical_and_refs(
            &canonical,
            BTreeMap::new(),
            Some(&image_providers),
        );
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "nano-banana-3",
                provider_model(&image_providers, "gateway", "nano-banana-3")
            ),
            ModelIdentity::Unlinked(InferenceRejection::DisjointOutputModalities)
        ));
    }

    /// Placement lock: the lane is last, so an earlier reconciliation lane
    /// keeps both the target and its own provenance even where every
    /// self-anchor condition also holds.
    #[test]
    fn self_anchor_never_preempts_an_earlier_lane() {
        let anchored = providers(
            r#"{
                "google":{"id":"google","name":"Google","models":{
                    "gemini-3.1-flash-image-preview":{"id":"gemini-3.1-flash-image-preview","name":"Nano Banana 2","modalities":{"output":["image"]}}
                }},
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "gemini-3.1-flash-image-preview":{"id":"gemini-3.1-flash-image-preview","name":"Gemini 3.1 Flash Image Preview","modalities":{"output":["image"]}}
                }}
            }"#,
        );
        let target = "google/gemini-3.1-flash-image-preview";
        let entries = &[(target, "Gemini 3.1 Flash Image Preview", None)][..];
        let cat = LabCatalog::from_test_catalog_with_refs(entries, &anchored, &[]);
        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "candidate",
            "gemini-3.1-flash-image-preview",
            provider_model(&anchored, "candidate", "gemini-3.1-flash-image-preview"),
        ) else {
            panic!("the complete-id lane should resolve this offering");
        };
        assert_eq!(resolution.id, target);
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredFullIdCanonical
        );

        // Without the authoritative offering the very same row self-anchors —
        // proving the lane was eligible and simply ran later.
        let solo = providers(
            r#"{
                "candidate":{"id":"candidate","name":"Candidate","models":{
                    "gemini-3.1-flash-image-preview":{"id":"gemini-3.1-flash-image-preview","name":"Gemini 3.1 Flash Image Preview","modalities":{"output":["image"]}}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(entries, &solo, &[]);
        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "candidate",
            "gemini-3.1-flash-image-preview",
            provider_model(&solo, "candidate", "gemini-3.1-flash-image-preview"),
        ) else {
            panic!("self-anchor should resolve the unanchored row");
        };
        assert_eq!(resolution.id, target);
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredSelfAnchorCanonical
        );
    }

    /// LANE B (creator-prefixed id) — the live `digitalocean/alibaba-qwen3-32b`
    /// row. The id spells Alibaba's lab tokens plus the canonical leaf id while
    /// the display name is plain, the mirror image of the one-sided creator
    /// lane.
    #[test]
    fn creator_prefixed_id_with_plain_name_infers_canonical() {
        let providers = providers(
            r#"{
                "digitalocean":{"id":"digitalocean","name":"DigitalOcean","models":{
                    "alibaba-qwen3-32b":{"id":"alibaba-qwen3-32b","name":"Qwen3-32B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("alibaba/qwen3-32b", "Qwen3 32B", None)],
            &providers,
            &[],
        );

        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "digitalocean",
            "alibaba-qwen3-32b",
            provider_model(&providers, "digitalocean", "alibaba-qwen3-32b"),
        ) else {
            panic!("creator-prefixed id with a plain name should resolve");
        };
        assert_eq!(resolution.id, "alibaba/qwen3-32b");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredCreatorPrefixedCanonical
        );
    }

    /// Both lab spellings are keys, and the `ai` extension covers the corporate
    /// suffix providers add or drop (`z-ai-`, `moonshot-ai-`).
    #[test]
    fn creator_prefix_accepts_display_and_ai_spellings() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "z-ai-glm-5-turbo":{"id":"z-ai-glm-5-turbo","name":"GLM-5-Turbo"},
                    "moonshot-ai-kimi-k2.6":{"id":"moonshot-ai-kimi-k2.6","name":"Kimi K2.6"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("zhipuai/glm-5-turbo", "GLM-5-Turbo", None),
                ("moonshotai/kimi-k2.6", "Kimi K2.6", None),
            ],
            &providers,
            &[],
        );

        for (model_id, expected) in [
            ("z-ai-glm-5-turbo", "zhipuai/glm-5-turbo"),
            ("moonshot-ai-kimi-k2.6", "moonshotai/kimi-k2.6"),
        ] {
            let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
                "gateway",
                model_id,
                provider_model(&providers, "gateway", model_id),
            ) else {
                panic!("{model_id} should resolve through its creator prefix");
            };
            assert_eq!(resolution.id, expected);
            assert_eq!(
                resolution.kind,
                CanonicalResolutionKind::InferredCreatorPrefixedCanonical
            );
        }
    }

    /// A path-spelled namespace leaves the leaf id bare, so only the complete
    /// id carries the creator. The second lab spelling the same canonical leaf
    /// id is what keeps the self-anchor lane out — and is exactly the collision
    /// a lab-qualified key resolves.
    #[test]
    fn path_namespace_display_spelling_uses_the_full_id_key() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "z-ai/glm-5-turbo":{"id":"z-ai/glm-5-turbo","name":"GLM-5-Turbo"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("zhipuai/glm-5-turbo", "GLM-5-Turbo", None),
                ("meituan/glm-5-turbo", "Meituan GLM 5 Turbo", None),
            ],
            &providers,
            &[],
        );

        let model = provider_model(&providers, "gateway", "z-ai/glm-5-turbo");
        let ModelIdentity::Canonical(resolution) =
            cat.resolve_model_identity("gateway", "z-ai/glm-5-turbo", model)
        else {
            panic!("the complete id spells the creator prefix");
        };
        assert_eq!(resolution.id, "zhipuai/glm-5-turbo");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredCreatorPrefixedCanonical
        );
        let evidence = cat
            .reconciliation_evidence("gateway", "z-ai/glm-5-turbo", model)
            .expect("creator-prefixed receipt");
        assert_eq!(evidence.creator_prefixed_key, Some("full"));
    }

    /// Pins the `ai`-extension reading of `P(lab)`. The live registry's 581-key
    /// count cannot be asserted offline and would rot as the registry grows, so
    /// pin the rule structurally instead: a slug whose last token is not `ai`
    /// is extended, an already-`ai`-terminated display spelling is not. A
    /// string `ends_with("ai")` reading would drop both `*/ai/*` keys below.
    #[test]
    fn creator_prefixed_index_key_rule_is_pinned() {
        let cat = LabCatalog::from_test_entries_with_refs(
            &[
                ("moonshotai/kimi-k2.6", "Kimi K2.6", None),
                ("openai/gpt-5.2", "GPT-5.2", None),
            ],
            &[],
        );

        let keys: BTreeSet<&str> = cat
            .creator_prefixed_id_candidates
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                // slug "moonshotai" — last token is not `ai`, so extended.
                "moonshotai/kimi/k/2/6",
                "moonshotai/ai/kimi/k/2/6",
                // display "Moonshot AI" — already `ai`-terminated, left alone.
                "moonshot/ai/kimi/k/2/6",
                // slug and display "OpenAI" fingerprint identically.
                "openai/gpt/5/2",
                "openai/ai/gpt/5/2",
            ])
        );
    }

    /// A provider self-prefix is compatible with every possible target and adds
    /// no evidence, so it is excluded by construction: only tokens spelling the
    /// target's own lab produce a key.
    #[test]
    fn provider_self_prefix_is_not_creator_evidence() {
        let providers = providers(
            r#"{
                "databricks":{"id":"databricks","name":"Databricks","models":{
                    "databricks-gpt-oss-120b":{"id":"databricks-gpt-oss-120b","name":"GPT OSS 120B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("openai/gpt-oss-120b", "GPT OSS 120B", None)],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "databricks",
                "databricks-gpt-oss-120b",
                provider_model(&providers, "databricks", "databricks-gpt-oss-120b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// Junk prefixes fail a fortiori — token `deep` is not token `deepseek`.
    #[test]
    fn unknown_prefix_token_refuses() {
        let providers = providers(
            r#"{
                "aihubmix":{"id":"aihubmix","name":"AiHubMix","models":{
                    "deep-deepseek-v4-flash":{"id":"deep-deepseek-v4-flash","name":"DeepSeek V4 Flash"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("deepseek/deepseek-v4-flash", "DeepSeek V4 Flash", None)],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "aihubmix",
                "deep-deepseek-v4-flash",
                provider_model(&providers, "aihubmix", "deep-deepseek-v4-flash")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// A creator prefix is falsifiable: spelling the wrong lab produces no key
    /// at all, and the right prefix under a foreign creator namespace still
    /// meets the shared creator blocker.
    #[test]
    fn creator_prefix_of_a_different_lab_refuses() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "openai-qwen3-32b":{"id":"openai-qwen3-32b","name":"Qwen3-32B"},
                    "nvidia/alibaba-qwen3-32b":{"id":"nvidia/alibaba-qwen3-32b","name":"Qwen3-32B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("alibaba/qwen3-32b", "Qwen3 32B", None),
                ("nvidia/nemotron-3", "Nemotron 3", None),
                ("openai/gpt-5.2", "GPT-5.2", None),
            ],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "openai-qwen3-32b",
                provider_model(&providers, "gateway", "openai-qwen3-32b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "nvidia/alibaba-qwen3-32b",
                provider_model(&providers, "gateway", "nvidia/alibaba-qwen3-32b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::CreatorConflict)
        ));
    }

    /// The key consumes the prefix and the canonical leaf id exactly; a
    /// semantic token beyond it is a different model.
    #[test]
    fn semantic_suffix_after_canonical_leaf_refuses() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "alibaba-qwen3-32b-thinking":{"id":"alibaba-qwen3-32b-thinking","name":"Qwen3-32B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("alibaba/qwen3-32b", "Qwen3 32B", None)],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "alibaba-qwen3-32b-thinking",
                provider_model(&providers, "gateway", "alibaba-qwen3-32b-thinking")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// `+` stays the semantic token `plus` on both sides of the key, so the two
    /// Command R records never trade places — and a `plus` id under the plain
    /// name is refused by the name channel rather than resolved by the id.
    #[test]
    fn plus_token_is_preserved() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "cohere-command-r":{"id":"cohere-command-r","name":"Command R"},
                    "cohere-command-r+":{"id":"cohere-command-r+","name":"Command R+"},
                    "cohere-command-r-plus":{"id":"cohere-command-r-plus","name":"Command R"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("cohere/command-r", "Command R", None),
                ("cohere/command-r-plus", "Command R+", None),
            ],
            &providers,
            &[],
        );

        for (model_id, expected) in [
            ("cohere-command-r", "cohere/command-r"),
            ("cohere-command-r+", "cohere/command-r-plus"),
        ] {
            let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
                "gateway",
                model_id,
                provider_model(&providers, "gateway", model_id),
            ) else {
                panic!("{model_id} should resolve to its own record");
            };
            assert_eq!(resolution.id, expected);
        }
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "cohere-command-r-plus",
                provider_model(&providers, "gateway", "cohere-command-r-plus")
            ),
            ModelIdentity::Unlinked(InferenceRejection::ConflictingCanonicalName)
        ));
    }

    /// The registry attaches the **plain** name to the **dated** record and
    /// "… (latest)" to the rolling one, so an undated creator-prefixed id
    /// selecting the rolling record is contradicted by its own display name.
    /// Dated and rolling are separate rows per the product contract.
    #[test]
    fn dated_rolling_name_conflict_fails_closed() {
        let providers = providers(
            r#"{
                "fastrouter":{"id":"fastrouter","name":"FastRouter","models":{
                    "anthropic/claude-opus-4.1":{"id":"anthropic/claude-opus-4.1","name":"Claude Opus 4.1"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                (
                    "anthropic/claude-opus-4-1",
                    "Claude Opus 4.1 (latest)",
                    None,
                ),
                (
                    "anthropic/claude-opus-4-1-20250805",
                    "Claude Opus 4.1",
                    None,
                ),
            ],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "fastrouter",
                "anthropic/claude-opus-4.1",
                provider_model(&providers, "fastrouter", "anthropic/claude-opus-4.1")
            ),
            ModelIdentity::Unlinked(InferenceRejection::ConflictingCanonicalName)
        ));
    }

    /// The four measured counterexamples where the creator-prefixed index
    /// disagrees with upstream. `fastrouter/anthropic/claude-opus-4.1` reaches
    /// the lane and C5 refuses it. The other three spell a literal canonical id
    /// (`anthropic/claude-opus-4-1`, `anthropic/claude-sonnet-4-5`,
    /// `deepseek/deepseek-r1`), so models.dev's own authoritative tiers claim
    /// them before any inference runs — the assertion there is that the
    /// explicit ref wins, plus the same evidence in a spelling the lane can
    /// actually see.
    #[test]
    fn creator_prefixed_measured_counterexamples_fail_closed() {
        let canonical = &[
            (
                "anthropic/claude-opus-4-1",
                "Claude Opus 4.1 (latest)",
                None,
            ),
            (
                "anthropic/claude-opus-4-1-20250805",
                "Claude Opus 4.1",
                None,
            ),
            (
                "anthropic/claude-sonnet-4-5",
                "Claude Sonnet 4.5 (latest)",
                None,
            ),
            (
                "anthropic/claude-sonnet-4-5-20250929",
                "Claude Sonnet 4.5",
                None,
            ),
            ("deepseek/deepseek-r1", "DeepSeek-R1", None),
            ("deepseek/deepseek-reasoner", "DeepSeek Reasoner", None),
        ][..];

        // (a) The two rows whose id is a literal canonical id. The pinned ref
        // is the only thing keeping them off the rolling / `-r1` record — the
        // direct-id tier below shows what happens without it.
        let literal = providers(
            r#"{
                "requesty":{"id":"requesty","name":"Requesty","models":{
                    "anthropic/claude-opus-4-1":{"id":"anthropic/claude-opus-4-1","name":"Claude Opus 4.1"},
                    "anthropic/claude-sonnet-4-5":{"id":"anthropic/claude-sonnet-4-5","name":"Claude Sonnet 4.5"}
                }},
                "anyapi":{"id":"anyapi","name":"AnyAPI","models":{
                    "deepseek/deepseek-r1":{"id":"deepseek/deepseek-r1","name":"DeepSeek Reasoner"}
                }}
            }"#,
        );
        let refs = &[
            (
                "requesty/anthropic/claude-opus-4-1",
                "anthropic/claude-opus-4-1-20250805",
            ),
            (
                "requesty/anthropic/claude-sonnet-4-5",
                "anthropic/claude-sonnet-4-5-20250929",
            ),
            ("anyapi/deepseek/deepseek-r1", "deepseek/deepseek-reasoner"),
        ][..];
        let cat = LabCatalog::from_test_catalog_with_refs(canonical, &literal, refs);
        for (provider, model_id, expected) in [
            (
                "requesty",
                "anthropic/claude-opus-4-1",
                "anthropic/claude-opus-4-1-20250805",
            ),
            (
                "requesty",
                "anthropic/claude-sonnet-4-5",
                "anthropic/claude-sonnet-4-5-20250929",
            ),
            (
                "anyapi",
                "deepseek/deepseek-r1",
                "deepseek/deepseek-reasoner",
            ),
        ] {
            let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
                provider,
                model_id,
                provider_model(&literal, provider, model_id),
            ) else {
                panic!("{provider}/{model_id} carries an explicit ref");
            };
            assert_eq!(resolution.id, expected);
            assert_eq!(resolution.kind, CanonicalResolutionKind::AuthoritativeRef);
        }

        // Drop the refs and models.dev's own direct-id tier — which this
        // resolver mirrors exactly — claims the rolling record. That is an
        // argument for refreshing the pinned artifact, not something an
        // inference lane may second-guess.
        let unpinned = LabCatalog::from_test_catalog_with_refs(canonical, &literal, &[]);
        let ModelIdentity::Canonical(resolution) = unpinned.resolve_model_identity(
            "requesty",
            "anthropic/claude-opus-4-1",
            provider_model(&literal, "requesty", "anthropic/claude-opus-4-1"),
        ) else {
            panic!("a literal canonical id always resolves");
        };
        assert_eq!(resolution.id, "anthropic/claude-opus-4-1");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::AuthoritativeDirectId
        );

        // (b) The same evidence in spellings the lane can actually see. Only
        // `fastrouter/anthropic/claude-opus-4.1` is a live catalog row; the
        // other two are synthetic re-spellings of the live rows above, since a
        // literal canonical id never reaches inference. The id selects the
        // rolling / `-r1` record, the plain display name selects the dated /
        // reasoner record, and the name channel refuses.
        let lane_visible = providers(
            r#"{
                "fastrouter":{"id":"fastrouter","name":"FastRouter","models":{
                    "anthropic/claude-opus-4.1":{"id":"anthropic/claude-opus-4.1","name":"Claude Opus 4.1"}
                }},
                "requesty":{"id":"requesty","name":"Requesty","models":{
                    "anthropic/claude-sonnet-4.5":{"id":"anthropic/claude-sonnet-4.5","name":"Claude Sonnet 4.5"}
                }},
                "anyapi":{"id":"anyapi","name":"AnyAPI","models":{
                    "deepseek-deepseek-r1":{"id":"deepseek-deepseek-r1","name":"DeepSeek Reasoner"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(canonical, &lane_visible, &[]);
        for (provider, model_id) in [
            ("fastrouter", "anthropic/claude-opus-4.1"),
            ("requesty", "anthropic/claude-sonnet-4.5"),
            ("anyapi", "deepseek-deepseek-r1"),
        ] {
            assert!(
                matches!(
                    cat.resolve_model_identity(
                        provider,
                        model_id,
                        provider_model(&lane_visible, provider, model_id)
                    ),
                    ModelIdentity::Unlinked(InferenceRejection::ConflictingCanonicalName)
                ),
                "{provider}/{model_id} must fail closed on the name channel"
            );
        }
    }

    /// C5 is mandatory: without a canonical name selecting the same record the
    /// lane falls through rather than degrading into a complete-id lane.
    #[test]
    fn id_only_match_never_resolves() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "alibaba-qwen3-32b":{"id":"alibaba-qwen3-32b","name":"Gateway Turbo 32B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[("alibaba/qwen3-32b", "Qwen3 32B", None)],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "alibaba-qwen3-32b",
                provider_model(&providers, "gateway", "alibaba-qwen3-32b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoCanonicalName)
        ));
    }

    /// Ambiguity raised by an earlier tier stays terminal — the caller never
    /// admits `AmbiguousCanonicalName` to reconciliation, so a unique key here
    /// cannot reopen it.
    #[test]
    fn prior_canonical_name_ambiguity_stays_terminal() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "alibaba-qwen3-32b":{"id":"alibaba-qwen3-32b","name":"Qwen3-32B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("alibaba/qwen3-32b", "Qwen3 32B", None),
                ("alibaba/qwen3-32b-preview", "Qwen3 32B", None),
            ],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "alibaba-qwen3-32b",
                provider_model(&providers, "gateway", "alibaba-qwen3-32b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::AmbiguousCanonicalName)
        ));
    }

    /// Two canonical records spelling one creator-prefixed key make the key
    /// useless as evidence, so the lane declines.
    #[test]
    fn colliding_creator_prefixed_keys_fail_closed() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "moonshot-ai-kimi-k2":{"id":"moonshot-ai-kimi-k2","name":"Kimi K2"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("moonshotai/kimi-k2", "Kimi K2", None),
                ("moonshot/ai-kimi-k2", "Moonshot AI Kimi K2", None),
            ],
            &providers,
            &[],
        );

        // Premise: the display spelling of one lab collides with the other's
        // slug plus a leading `ai` token.
        assert_eq!(
            cat.creator_prefixed_id_candidates
                .get("moonshot/ai/kimi/k/2")
                .map_or(0, HashSet::len),
            2
        );
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "moonshot-ai-kimi-k2",
                provider_model(&providers, "gateway", "moonshot-ai-kimi-k2")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));
    }

    /// C3: leaf and complete id are both keys and disagree — terminal, because
    /// picking either would be arbitrary.
    #[test]
    fn ambiguous_creator_prefixed_leaf_and_full_keys_fail_closed() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "openai/moonshot-ai-kimi-k2":{"id":"openai/moonshot-ai-kimi-k2","name":"Gateway Kimi"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("moonshotai/kimi-k2", "Kimi K2", None),
                ("openai/moonshot-ai-kimi-k-2", "OpenAI Moonshot Kimi", None),
            ],
            &providers,
            &[],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "openai/moonshot-ai-kimi-k2",
                provider_model(&providers, "gateway", "openai/moonshot-ai-kimi-k2")
            ),
            ModelIdentity::Unlinked(InferenceRejection::AmbiguousCreatorPrefixedId)
        ));
    }

    /// C6. Currently entailed by the anchored lane, which is terminal on a
    /// conflicting name anchor before reconciliation runs; kept explicit so the
    /// guard survives any reordering.
    #[test]
    fn authoritative_name_anchor_conflict_blocks() {
        let providers = providers(
            r#"{
                "anchor":{"id":"anchor","name":"Anchor","models":{
                    "house-blend":{"id":"house-blend","name":"Qwen3-32B"}
                }},
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "alibaba-qwen3-32b":{"id":"alibaba-qwen3-32b","name":"Qwen3-32B"}
                }}
            }"#,
        );
        let cat = LabCatalog::from_test_catalog_with_refs(
            &[
                ("alibaba/qwen3-32b", "Qwen3 32B", None),
                ("openai/gpt-5.2", "GPT-5.2", None),
            ],
            &providers,
            &[("anchor/house-blend", "openai/gpt-5.2")],
        );

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "alibaba-qwen3-32b",
                provider_model(&providers, "gateway", "alibaba-qwen3-32b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::ConflictingAuthoritativeNameAnchors)
        ));
    }

    /// C6-id: models.dev filed this leaf-id spelling under a different target.
    /// The cross-alias and complete-id lanes decline rather than error on an
    /// ambiguous alias set, so the check has to run in this lane.
    #[test]
    fn authoritative_id_anchor_conflict_blocks() {
        let providers = providers(
            r#"{
                "anchor":{"id":"anchor","name":"Anchor","models":{
                    "alibaba-qwen3-32b":{"id":"alibaba-qwen3-32b","name":"Other Thing"}
                }},
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "foo/alibaba-qwen3-32b":{"id":"foo/alibaba-qwen3-32b","name":"Qwen3-32B"}
                }}
            }"#,
        );
        let entries = &[
            ("alibaba/qwen3-32b", "Qwen3 32B", None),
            ("openai/gpt-5.2", "Other Thing", None),
        ][..];
        let cat = LabCatalog::from_test_catalog_with_refs(
            entries,
            &providers,
            &[("anchor/alibaba-qwen3-32b", "openai/gpt-5.2")],
        );
        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "foo/alibaba-qwen3-32b",
                provider_model(&providers, "gateway", "foo/alibaba-qwen3-32b")
            ),
            ModelIdentity::Unlinked(InferenceRejection::NoAuthoritativeNameAnchor)
        ));

        // Control: the contradicting alias is the only difference.
        let cat = LabCatalog::from_test_catalog_with_refs(entries, &providers, &[]);
        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "gateway",
            "foo/alibaba-qwen3-32b",
            provider_model(&providers, "gateway", "foo/alibaba-qwen3-32b"),
        ) else {
            panic!("without the contradicting alias the key resolves");
        };
        assert_eq!(resolution.id, "alibaba/qwen3-32b");
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredCreatorPrefixedCanonical
        );
    }

    /// C7: the shared output-modality blocker still applies.
    #[test]
    fn output_modality_conflict_blocks() {
        let providers = providers(
            r#"{
                "gateway":{"id":"gateway","name":"Gateway","models":{
                    "google-nano-banana-3":{"id":"google-nano-banana-3","name":"Nano Banana 3","modalities":{"output":["image"]}}
                }}
            }"#,
        );
        let canonical: HashMap<String, CanonicalModel> = serde_json::from_str(
            r#"{
                "google/nano-banana-3": {
                    "name":"Nano Banana 3",
                    "modalities":{"output":["text"]}
                }
            }"#,
        )
        .expect("valid canonical json");
        let cat =
            LabCatalog::from_canonical_and_refs(&canonical, BTreeMap::new(), Some(&providers));

        assert!(matches!(
            cat.resolve_model_identity(
                "gateway",
                "google-nano-banana-3",
                provider_model(&providers, "gateway", "google-nano-banana-3")
            ),
            ModelIdentity::Unlinked(InferenceRejection::DisjointOutputModalities)
        ));
    }

    /// The index is built from the canonical registry alone, so a provider
    /// spelling — least of all an inferred one — can never widen it.
    #[test]
    fn inferred_result_never_seeds_the_index() {
        let providers = providers(
            r#"{
                "digitalocean":{"id":"digitalocean","name":"DigitalOcean","models":{
                    "alibaba-qwen3-32b":{"id":"alibaba-qwen3-32b","name":"Qwen3-32B"}
                }}
            }"#,
        );
        let entries = &[("alibaba/qwen3-32b", "Qwen3 32B", None)][..];
        let cat = LabCatalog::from_test_catalog_with_refs(entries, &providers, &[]);

        // Premise: this snapshot really does resolve an offering by inference.
        let ModelIdentity::Canonical(resolution) = cat.resolve_model_identity(
            "digitalocean",
            "alibaba-qwen3-32b",
            provider_model(&providers, "digitalocean", "alibaba-qwen3-32b"),
        ) else {
            panic!("fixture must exercise the lane");
        };
        assert_eq!(
            resolution.kind,
            CanonicalResolutionKind::InferredCreatorPrefixedCanonical
        );
        let registry_only = LabCatalog::from_test_entries_with_refs(entries, &[]);
        assert_eq!(
            cat.creator_prefixed_id_candidates,
            registry_only.creator_prefixed_id_candidates
        );
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
    /// only correct active targets for the held-out explicit edges.
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
        let mut inferred_exact_pair = 0usize;
        let mut inferred_one_sided = 0usize;
        let mut inferred_cross = 0usize;
        let mut inferred_full_id = 0usize;
        let mut inferred_self_anchor = 0usize;
        let mut inferred_creator_prefixed = 0usize;
        let mut exact_conflicts = Vec::new();
        let mut active_wrong = Vec::new();

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
                        CanonicalResolutionKind::InferredExactPairCanonical => {
                            inferred_exact_pair += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                    "{offering_key}: exact-pair inferred {} but explicit target is {expected_target}",
                                    resolution.id
                                ));
                            }
                        }
                        CanonicalResolutionKind::InferredOneSidedCreatorCanonical => {
                            inferred_one_sided += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                    "{offering_key}: one-sided creator inferred {} but explicit target is {expected_target}",
                                    resolution.id
                                ));
                            }
                        }
                        CanonicalResolutionKind::InferredCrossAliasCanonical => {
                            inferred_cross += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                    "{offering_key}: cross-alias inferred {} but explicit target is {expected_target}",
                                    resolution.id
                                ));
                            }
                        }
                        CanonicalResolutionKind::InferredFullIdCanonical => {
                            inferred_full_id += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                    "{offering_key}: full-id inferred {} but explicit target is {expected_target}",
                                    resolution.id
                                ));
                            }
                        }
                        CanonicalResolutionKind::InferredSelfAnchorCanonical => {
                            inferred_self_anchor += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                    "{offering_key}: self-anchor inferred {} but explicit target is {expected_target}",
                                    resolution.id
                                ));
                            }
                        }
                        CanonicalResolutionKind::InferredCreatorPrefixedCanonical => {
                            inferred_creator_prefixed += 1;
                            if resolution.id != expected_target {
                                active_wrong.push(format!(
                                    "{offering_key}: creator-prefixed inferred {} but explicit target is {expected_target}",
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
                    ModelIdentity::Unlinked(_) => {}
                }
            }
        }

        println!(
            "provider-holdout audit: {audited} current explicit refs; active = {exact} exact ({} conflicts where the held-out explicit ref is more specific) + {inferred} anchored + {inferred_qualified} dual creator + {inferred_exact_pair} exact-pair + {inferred_one_sided} one-sided creator + {inferred_cross} cross-alias + {inferred_full_id} full-id + {inferred_self_anchor} self-anchor + {inferred_creator_prefixed} creator-prefixed, {} wrong inferred",
            exact_conflicts.len(),
            active_wrong.len()
        );
        // Receipt, never an assertion — the canonical registry grows.
        let registry_only =
            LabCatalog::from_canonical_and_refs(&snapshot.models, BTreeMap::new(), None);
        println!(
            "creator-prefixed index: {} keys, {} colliding",
            registry_only.creator_prefixed_id_candidates.len(),
            registry_only
                .creator_prefixed_id_candidates
                .values()
                .filter(|targets| targets.len() > 1)
                .count()
        );
        for conflict in exact_conflicts {
            println!("held-out exact conflict: {conflict}");
        }
        assert!(
            inferred + inferred_qualified > 0,
            "provider holdout must exercise active inferred canonical matches"
        );
        assert!(
            inferred_exact_pair + inferred_one_sided + inferred_cross + inferred_full_id > 0,
            "provider holdout must exercise exact reconciliation matches"
        );
        // The self-anchor and creator-prefixed lanes are registry-seeded, so
        // their indexes survive the holdout intact and this audit yields no
        // signal for either (the creator-prefixed lane was measured to fire
        // zero times under it) — the leave-one-out gate in the synthesis report
        // is their validation. Their counters are printed, never asserted.
        assert!(
            active_wrong.is_empty(),
            "wrong active targets:\n{}",
            active_wrong.join("\n")
        );
    }

    /// Live, explicitly-invoked leave-one-out gate for the two registry-seeded
    /// lanes. The provider holdout above cannot validate them: their key
    /// indexes (`canonical_leaf_id_candidates`, `creator_prefixed_id_candidates`)
    /// are seeded from the canonical registry alone, so holding a provider out
    /// leaves both intact while the earlier reconciliation lanes — which do
    /// consume the surviving providers' anchors — resolve the offering first.
    /// Both lanes fire zero times there; that count is printed here as the
    /// receipt explaining why this gate exists rather than as its result.
    ///
    /// The gate evaluates each lane's own evidence contract directly, against a
    /// catalog rebuilt with the held-out provider's contribution to every
    /// authoritative alias index removed. Masking is provider-level (the
    /// synthesis report measured the per-offering variant identical), so a
    /// held-out offering's name, leaf-id, pair and complete-id anchors are all
    /// absent together. Lane firings are not a partition: direct evaluation
    /// shows every offering to both lanes, and neither lane's precondition of
    /// an earlier-lane rejection is applied — applying it is exactly what
    /// drives the resolver-gated count to zero.
    #[test]
    #[ignore = "live models.dev leave-one-out gate for the registry-seeded lanes"]
    fn live_leave_one_out_gate_for_registry_seeded_lanes() {
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
        let mut self_anchor_fired = 0usize;
        // Structurally zero today — the self-anchor lane falls through rather
        // than refusing terminally. Counted anyway so a future terminal
        // rejection shows up in the audit line instead of being swallowed.
        let mut self_anchor_refused = 0usize;
        let mut creator_prefixed_fired = 0usize;
        let mut creator_prefixed_refused = 0usize;
        let mut creator_prefixed_leaf_key = 0usize;
        let mut creator_prefixed_full_key = 0usize;
        let mut creator_prefixed_guarded = 0usize;
        let mut creator_prefixed_unguarded = 0usize;
        let mut resolver_self_anchor = 0usize;
        let mut resolver_creator_prefixed = 0usize;
        let mut wrong = Vec::new();

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
                let name = identity_fingerprint(&model.name);
                let id = model_id_fingerprint(model_id);
                // The caller's own precondition — an offering with no usable
                // name or leaf id never reaches either lane.
                if name.is_empty() || id.is_empty() {
                    continue;
                }
                let full_id = full_model_id_fingerprint(model_id);
                audited += 1;

                match masked.resolve_model_identity(held_out_provider, model_id, model) {
                    ModelIdentity::Canonical(resolution) => match resolution.kind {
                        CanonicalResolutionKind::InferredSelfAnchorCanonical => {
                            resolver_self_anchor += 1;
                            if resolution.id != expected_target {
                                wrong.push(format!(
                                    "{offering_key}: resolver self-anchor {} but explicit target is {expected_target}",
                                    resolution.id
                                ));
                            }
                        }
                        CanonicalResolutionKind::InferredCreatorPrefixedCanonical => {
                            resolver_creator_prefixed += 1;
                            if resolution.id != expected_target {
                                wrong.push(format!(
                                    "{offering_key}: resolver creator-prefixed {} but explicit target is {expected_target}",
                                    resolution.id
                                ));
                            }
                        }
                        _ => {}
                    },
                    ModelIdentity::Unlinked(_) => {}
                }

                match masked.resolve_self_anchor_inference(
                    held_out_provider,
                    model_id,
                    model,
                    &name,
                    &id,
                    &full_id,
                ) {
                    Some(Ok(resolution)) => {
                        self_anchor_fired += 1;
                        if resolution.id != expected_target {
                            wrong.push(format!(
                                "{offering_key}: self-anchor inferred {} but explicit target is {expected_target}",
                                resolution.id
                            ));
                        }
                    }
                    Some(Err(_)) => self_anchor_refused += 1,
                    None => {}
                }

                match masked.resolve_creator_prefixed_inference(
                    held_out_provider,
                    model_id,
                    model,
                    &name,
                    &id,
                    &full_id,
                ) {
                    Some(Ok(resolution)) => {
                        creator_prefixed_fired += 1;
                        if masked.creator_prefixed_id_candidates.contains_key(&id) {
                            creator_prefixed_leaf_key += 1;
                        } else {
                            creator_prefixed_full_key += 1;
                        }
                        // The creator blocker is inert wherever the offering
                        // attributes no independent lab; there the key's own
                        // lab tokens are the only creator evidence.
                        if masked
                            .independent_lab(held_out_provider, model_id)
                            .is_some()
                        {
                            creator_prefixed_guarded += 1;
                        } else {
                            creator_prefixed_unguarded += 1;
                        }
                        if resolution.id != expected_target {
                            wrong.push(format!(
                                "{offering_key}: creator-prefixed inferred {} but explicit target is {expected_target}",
                                resolution.id
                            ));
                        }
                    }
                    Some(Err(_)) => creator_prefixed_refused += 1,
                    None => {}
                }
            }
        }

        println!(
            "leave-one-out gate: {audited} held-out explicit refs; self-anchor {self_anchor_fired} fired / {self_anchor_refused} refused (no terminal refusal exists in this lane); creator-prefixed {creator_prefixed_fired} fired ({creator_prefixed_leaf_key} leaf key, {creator_prefixed_full_key} full key; {creator_prefixed_guarded} guarded, {creator_prefixed_unguarded} unguarded) / {creator_prefixed_refused} refused; {} wrong"
        , wrong.len());
        // Receipt for why this gate replaces the provider holdout, never an
        // assertion — an earlier lane resolving these offerings first is the
        // designed behavior, not a failure.
        println!(
            "same offerings through the full resolver: {resolver_self_anchor} self-anchor + {resolver_creator_prefixed} creator-prefixed"
        );
        assert!(
            self_anchor_fired + creator_prefixed_fired > 0,
            "leave-one-out gate must exercise both registry-seeded lanes"
        );
        assert!(wrong.is_empty(), "wrong targets:\n{}", wrong.join("\n"));
    }

    #[test]
    fn independent_lab_ignores_provider_fallback_for_foreign_namespace() {
        let cat = LabCatalog::from_canonical(&canon(&[("nvidia/foo-1", "Foo 1", None)]));
        // A namespaced id attributes the model away from the provider — even
        // when the provider id is itself a lab, "qwen/…" is not NVIDIA's.
        assert_eq!(cat.independent_lab("nvidia", "qwen/bar-2"), None);
        // A recognized namespace still wins; bare ids keep the provider
        // fallback.
        assert_eq!(cat.independent_lab("vultr", "nvidia/bar-2"), Some("nvidia"));
        assert_eq!(cat.independent_lab("nvidia", "bar-2"), Some("nvidia"));
    }

    #[test]
    fn creator_alias_tokens_cover_slug_display_and_ai_only() {
        let cat = LabCatalog::from_canonical(&canon(&[("moonshotai/kimi-x9", "Kimi X9", None)]));
        let tokens = cat.creator_alias_tokens("moonshotai");
        for expected in ["moonshotai", "moonshot", "ai"] {
            assert!(tokens.contains(expected), "missing token {expected}");
        }
        // Family/brand names are version-line vocabulary, never neutral.
        assert!(!tokens.contains("kimi"));
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
