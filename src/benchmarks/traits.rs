use std::collections::HashMap;

use super::schema::{ModelRow, ReasoningStatus};
use crate::data::Provider;
use crate::provider_category::{provider_category, ProviderCategory};

/// Minimum Jaro-Winkler similarity to consider a match.
/// 0.85 is tuned to catch reordered tokens (e.g. "llama-3-1-instruct-405b" ↔
/// "llama-3.1-405b-instruct") while rejecting cross-family matches
/// (e.g. "gemma-3-27b" ≠ "gemini-3-pro").
const MIN_SIMILARITY: f64 = 0.85;

/// Normalize a string for matching: lowercase, strip separators.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | '.' | ' '))
        .collect()
}

/// Map AA creator slugs to models.dev provider IDs where they differ.
fn creator_to_providers(creator: &str) -> &[&str] {
    match creator {
        "meta" => &["llama"],
        "kimi" => &["moonshotai"],
        // Note: aws→amazon-bedrock and nvidia use org-prefixed model IDs
        // (e.g. "amazon.nova-2-lite-v1:0", "deepseek-ai/deepseek-r1") that
        // don't match AA slugs, but the mapping is kept for partial matches.
        "aws" => &["amazon-bedrock"],
        "azure" => &["azure"],
        "nvidia" => &["nvidia"],
        _ => &[],
    }
}

/// Hardcoded open/closed status for well-known creators that have no
/// models.dev provider. Returns `None` for unknown creators.
fn known_creator_openness(creator: &str) -> Option<bool> {
    match creator {
        // Open weight
        "ai2" => Some(true),           // OLMo, Molmo, Tülu — Allen Institute
        "ibm" => Some(true),           // Granite
        "lg" => Some(true),            // EXAONE
        "nous-research" => Some(true), // Hermes, DeepHermes
        "tii-uae" => Some(true),       // Falcon
        "databricks" => Some(true),    // DBRX
        "snowflake" => Some(true),     // Arctic
        "servicenow" => Some(true),    // Apriel
        "deepcogito" => Some(true),    // Cogito
        // Closed / proprietary API
        "ai21-labs" => Some(false),     // Jamba
        "naver" => Some(false),         // HyperCLOVA
        "korea-telecom" => Some(false), // Mi:dm
        _ => None,
    }
}

/// Model traits extracted from models.dev matching.
struct ModelTraits {
    open_weights: bool,
    context_window: Option<u64>,
    supports_tools: bool,
    max_output: Option<u64>,
    reasoning: bool,
}

impl ModelTraits {
    fn from_model(model: &crate::data::Model) -> Self {
        Self {
            open_weights: model.open_weights,
            context_window: model.limit.as_ref().and_then(|l| l.context),
            supports_tools: model.tool_call,
            max_output: model.limit.as_ref().and_then(|l| l.output),
            reasoning: model.reasoning,
        }
    }
}

/// Prebuilt model lookups derived from the models.dev provider list, reused
/// across every match so the normalization work happens once.
struct MatchIndex {
    /// Set of normalized provider IDs present in models.dev.
    provider_set: HashMap<String, ()>,
    /// Normalized provider ID → [(normalized model ID, traits)].
    model_lookup: HashMap<String, Vec<(String, ModelTraits)>>,
    /// Flat list of every model for the global fallback stage.
    all_models: Vec<(String, ModelTraits)>,
}

impl MatchIndex {
    fn build(providers: &[(String, Provider)]) -> Self {
        let provider_set: HashMap<String, ()> = providers
            .iter()
            .map(|(id, _)| (normalize(id), ()))
            .collect();

        let mut model_lookup: HashMap<String, Vec<(String, ModelTraits)>> = HashMap::new();
        for (id, provider) in providers {
            let norm_provider = normalize(id);
            let models: Vec<(String, ModelTraits)> = provider
                .models
                .iter()
                .map(|(model_id, model)| (normalize(model_id), ModelTraits::from_model(model)))
                .collect();
            model_lookup.insert(norm_provider, models);
        }

        let all_models: Vec<(String, ModelTraits)> = providers
            .iter()
            .flat_map(|(_, provider)| {
                provider
                    .models
                    .iter()
                    .map(|(model_id, model)| (normalize(model_id), ModelTraits::from_model(model)))
            })
            .collect();

        Self {
            provider_set,
            model_lookup,
            all_models,
        }
    }

    /// Match a single `(creator, slug)` pair via the three-stage strategy:
    /// creator-scoped Jaro-Winkler, global fallback, then known-creator
    /// openness override. Returns `None` when nothing meets [`MIN_SIMILARITY`]
    /// and the creator has no override.
    fn match_pair(&self, creator: &str, slug: &str) -> Option<ModelTraits> {
        if creator.is_empty() || slug.is_empty() {
            return None;
        }

        let norm_creator = normalize(creator);
        let norm_slug = normalize(slug);

        // Stage 1: Creator-scoped matching
        let mapped = creator_to_providers(creator);
        let provider_ids: Vec<String> = if mapped.is_empty() {
            vec![norm_creator]
        } else {
            mapped.iter().map(|id| normalize(id)).collect()
        };

        let mut best_score: f64 = 0.0;
        let mut best_traits: Option<&ModelTraits> = None;

        for norm_provider_id in &provider_ids {
            if !self.provider_set.contains_key(norm_provider_id.as_str()) {
                continue;
            }

            if let Some(models) = self.model_lookup.get(norm_provider_id.as_str()) {
                for (norm_model_id, traits) in models {
                    let score = strsim::jaro_winkler(&norm_slug, norm_model_id);
                    if score > best_score {
                        best_score = score;
                        best_traits = Some(traits);
                        if (score - 1.0).abs() < f64::EPSILON {
                            break;
                        }
                    }
                }
            }

            if (best_score - 1.0).abs() < f64::EPSILON {
                break;
            }
        }

        // Stage 2: Global fallback — search all models if creator-scoped didn't match
        if best_score < MIN_SIMILARITY {
            for (norm_model_id, traits) in &self.all_models {
                let score = strsim::jaro_winkler(&norm_slug, norm_model_id);
                if score > best_score {
                    best_score = score;
                    best_traits = Some(traits);
                    if (score - 1.0).abs() < f64::EPSILON {
                        break;
                    }
                }
            }
        }

        if best_score >= MIN_SIMILARITY {
            if let Some(traits) = best_traits {
                return Some(ModelTraits {
                    open_weights: traits.open_weights,
                    context_window: traits.context_window,
                    supports_tools: traits.supports_tools,
                    max_output: traits.max_output,
                    reasoning: traits.reasoning,
                });
            }
        }

        // Stage 3: Known creator overrides for providers absent from models.dev
        known_creator_openness(creator).map(|ow| ModelTraits {
            open_weights: ow,
            context_window: None,
            supports_tools: false,
            max_output: None,
            reasoning: false,
        })
    }
}

/// Augment AA [`ModelRow`]s with traits from models.dev.
///
/// AA-only call site: the model id (the AA slug) is matched against models.dev
/// model IDs with a Jaro-Winkler + creator-scoping strategy, filling
/// `open_weights` and `context_window` from the matched models.dev model (and
/// the known-creator-openness overrides for creators absent from models.dev).
/// Existing populated fields are left untouched.
pub fn apply_model_traits(providers: &[(String, Provider)], models: &mut [ModelRow]) {
    let index = MatchIndex::build(providers);
    for model in models {
        if let Some(traits) = index.match_pair(&model.creator, &model.id) {
            if model.open_weights.is_none() {
                model.open_weights = Some(traits.open_weights);
            }
            if model.context_window.is_none() {
                model.context_window = traits.context_window;
            }
            if model.supports_tools.is_none() {
                model.supports_tools = Some(traits.supports_tools);
            }
            if model.max_output.is_none() {
                model.max_output = traits.max_output;
            }
            // Fill reasoning ONLY from a positive models.dev capability flag, and
            // only when name-parsing left it unknown. models.dev `reasoning:
            // false` is provider-specific and unreliable (the same model is
            // flagged both ways across providers), so a false is NOT mapped to
            // NonReasoning — that would launder "unknown" into a wrong verdict.
            if traits.reasoning && model.reasoning_status == ReasoningStatus::None {
                model.reasoning_status = ReasoningStatus::Reasoning;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Generic within-source enrichment (epoch / arena / llmstats)
// ---------------------------------------------------------------------------

/// Variant suffixes stripped from a source model id during normalized matching.
///
/// These denote access/budget/reasoning variants that share the same underlying
/// models.dev model. The first group is the plan-sanctioned list
/// (`-thinking`, effort levels, `-preview`); the rest are additional
/// same-model deployment variants observed in the real
/// `data/v2/{epoch,arena,llmstats}.json` ids (e.g. AA-Arena's `-search`,
/// `-grounding`, `-thinking-32k`). Longer suffixes precede shorter prefixes of
/// themselves so stripping is order-stable. Stripping is exact/normalized only —
/// no fuzzy matching is introduced.
const VARIANT_SUFFIXES: &[&str] = &[
    // Plan-sanctioned variant suffixes.
    "-thinking",
    "-high",
    "-low",
    "-medium",
    "-minimal",
    "-max",
    "-preview",
    // Same-model deployment/access variants seen in the real source ids.
    "-non-reasoning",
    "-reasoning",
    "-search",
    "-grounding",
    "-instant",
    "-beta",
    "-pre-release",
    "-web-app",
    "-webapp",
];

/// Hosting-org prefixes that some sources (notably AA-Epoch) prepend to the
/// underlying model id (e.g. `fireworks/kimi-k2`, `parasail-qwen3-...`,
/// `api-gpt-4o-search`). Stripping the host prefix exposes the real model id for
/// matching. Applied to both the source id and the models.dev id (models.dev
/// also lists org-prefixed ids like `deepseek-ai/DeepSeek-R1`).
fn strip_host_prefix(s: &str) -> &str {
    // A path-style host prefix: everything before the last `/`.
    if let Some(pos) = s.rfind('/') {
        return &s[pos + 1..];
    }
    for pre in ["parasail-", "api-"] {
        if let Some(rest) = s.strip_prefix(pre) {
            return rest;
        }
    }
    s
}

/// Normalize a model id for cross-source matching:
/// 1. lowercase,
/// 2. strip a hosting-org prefix,
/// 3. drop any parenthetical group (e.g. `(thinking-minimal)`, `(codex-harness)`),
/// 4. repeatedly strip trailing variant suffixes, trailing date stamps
///    (`-2025-12-11`, `-202512`, `-20251211`), and trailing thinking-budget
///    tags (`-32k`),
/// 5. remove the remaining separators (`-`, `_`, `.`, space).
///
/// Returns the collapsed identity string. Pure exact/normalized matching — there
/// is no similarity scoring.
pub fn normalize_id(raw: &str) -> String {
    let lower = raw.to_lowercase();
    let mut s: String = strip_host_prefix(&lower).to_string();

    // Drop parenthetical groups: keep characters outside any `(...)`.
    if s.contains('(') {
        let mut out = String::with_capacity(s.len());
        let mut depth = 0u32;
        for c in s.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        s = out;
    }

    loop {
        let before = s.len();

        for suf in VARIANT_SUFFIXES {
            if let Some(rest) = s.strip_suffix(suf) {
                s = rest.to_string();
            }
        }
        if let Some(rest) = strip_trailing_date_stamp(&s) {
            s = rest;
        }
        if let Some(rest) = strip_trailing_budget_tag(&s) {
            s = rest;
        }

        if s.len() == before {
            break;
        }
    }

    s.chars()
        .filter(|c| !matches!(c, '-' | '_' | '.' | ' '))
        .collect()
}

/// Strip a trailing date stamp: `-YYYY-MM-DD`, `-YYYY-MM`, `-YYYYMMDD`, `-YYYYMM`.
/// Returns the prefix without the stamp, or `None` when no stamp is present.
fn strip_trailing_date_stamp(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    // -YYYY-MM-DD (11 trailing chars)
    if matches_pattern(bytes, b"-dddd-dd-dd") {
        return Some(s[..s.len() - 11].to_string());
    }
    // -YYYYMMDD (9 trailing chars)
    if matches_pattern(bytes, b"-dddddddd") {
        return Some(s[..s.len() - 9].to_string());
    }
    // -YYYY-MM (8 trailing chars)
    if matches_pattern(bytes, b"-dddd-dd") {
        return Some(s[..s.len() - 8].to_string());
    }
    // -YYYYMM (7 trailing chars)
    if matches_pattern(bytes, b"-dddddd") {
        return Some(s[..s.len() - 7].to_string());
    }
    None
}

/// Strip a trailing thinking-budget tag like `-32k` / `-128k`.
fn strip_trailing_budget_tag(s: &str) -> Option<String> {
    let rest = s.strip_suffix('k')?;
    // Require at least one digit immediately before the `k`, preceded by `-`.
    let trimmed = rest.trim_end_matches(|c: char| c.is_ascii_digit());
    if trimmed.len() == rest.len() {
        return None; // no digits
    }
    let prefix = trimmed.strip_suffix('-')?;
    Some(prefix.to_string())
}

/// Check whether the tail of `bytes` matches a pattern where `d` means an ASCII
/// digit and any other byte must match literally.
fn matches_pattern(bytes: &[u8], pattern: &[u8]) -> bool {
    if bytes.len() < pattern.len() {
        return false;
    }
    let tail = &bytes[bytes.len() - pattern.len()..];
    tail.iter().zip(pattern).all(|(&b, &p)| {
        if p == b'd' {
            b.is_ascii_digit()
        } else {
            b == p
        }
    })
}

/// Traits derived from a single models.dev model + its host provider.
#[derive(Clone)]
struct EnrichCandidate {
    provider_id: String,
    open_weights: bool,
    release_date: Option<String>,
    context_window: Option<u64>,
    supports_tools: bool,
    max_output: Option<u64>,
    reasoning: bool,
}

impl EnrichCandidate {
    /// Origin-category providers are authoritative for a model's creator and
    /// open-weights status; prefer them when a model is hosted under several
    /// providers.
    fn is_origin(&self) -> bool {
        provider_category(&self.provider_id) == ProviderCategory::Origin
    }
}

/// Lookup of models.dev models keyed by exact and normalized model id, used to
/// enrich clean-id sources (epoch / arena / llmstats) without fuzzy matching.
struct EnrichIndex {
    /// Lowercased exact model id → candidates.
    exact: HashMap<String, Vec<EnrichCandidate>>,
    /// Normalized model id → candidates.
    normalized: HashMap<String, Vec<EnrichCandidate>>,
}

impl EnrichIndex {
    fn build(providers: &[(String, Provider)]) -> Self {
        let mut exact: HashMap<String, Vec<EnrichCandidate>> = HashMap::new();
        let mut normalized: HashMap<String, Vec<EnrichCandidate>> = HashMap::new();

        for (provider_id, provider) in providers {
            for (model_id, model) in &provider.models {
                let candidate = EnrichCandidate {
                    provider_id: provider_id.clone(),
                    open_weights: model.open_weights,
                    release_date: model.release_date.clone(),
                    context_window: model.limit.as_ref().and_then(|l| l.context),
                    supports_tools: model.tool_call,
                    max_output: model.limit.as_ref().and_then(|l| l.output),
                    reasoning: model.reasoning,
                };
                exact
                    .entry(model_id.to_lowercase())
                    .or_default()
                    .push(candidate.clone());
                normalized
                    .entry(normalize_id(model_id))
                    .or_default()
                    .push(candidate);
            }
        }

        Self { exact, normalized }
    }

    /// Resolve the best candidate for a source model id: exact match first, then
    /// normalized. Within the matched bucket the Origin-category provider wins
    /// (authoritative creator/openness); otherwise the first candidate is used.
    fn resolve(&self, source_id: &str) -> Option<&EnrichCandidate> {
        let pick = |bucket: &'_ [EnrichCandidate]| -> Option<usize> {
            if bucket.is_empty() {
                return None;
            }
            let origin = bucket.iter().position(EnrichCandidate::is_origin);
            Some(origin.unwrap_or(0))
        };

        if let Some(bucket) = self.exact.get(&source_id.to_lowercase()) {
            if let Some(i) = pick(bucket) {
                return Some(&bucket[i]);
            }
        }
        let norm = normalize_id(source_id);
        if let Some(bucket) = self.normalized.get(&norm) {
            if let Some(i) = pick(bucket) {
                return Some(&bucket[i]);
            }
        }
        None
    }
}

/// Generic within-source enrichment for the non-AA sources (epoch / arena /
/// llmstats). For each model, the source id is matched against models.dev model
/// ids — **exact then normalized only, no fuzzy/Jaro-Winkler matching** (the
/// plan sanctions exact/normalized for clean-id sources). On a match, ONLY
/// currently-empty fields are filled from the matched models.dev model:
/// `creator` / `creator_name` (from the matched model's host provider, with
/// Origin-category providers preferred), `release_date`, `context_window`, and
/// `open_weights`. Source-provided values are never overwritten, and models with
/// no match are left untouched (the UI shows an honest em-dash).
pub fn enrich_from_models_dev(providers: &[(String, Provider)], models: &mut [ModelRow]) {
    let index = EnrichIndex::build(providers);

    // provider id → display name, for filling creator_name.
    let provider_names: HashMap<&str, &str> = providers
        .iter()
        .map(|(id, p)| (id.as_str(), p.name.as_str()))
        .collect();

    for model in models {
        let Some(candidate) = index.resolve(&model.id) else {
            continue;
        };

        if model.creator.is_empty() {
            model.creator = candidate.provider_id.clone();
        }
        if model.creator_name.is_empty() {
            let name = provider_names
                .get(candidate.provider_id.as_str())
                .copied()
                .unwrap_or(candidate.provider_id.as_str());
            model.creator_name = name.to_string();
        }
        if model.release_date.is_none() {
            model.release_date.clone_from(&candidate.release_date);
        }
        if model.context_window.is_none() {
            model.context_window = candidate.context_window;
        }
        if model.open_weights.is_none() {
            model.open_weights = Some(candidate.open_weights);
        }
        if model.supports_tools.is_none() {
            model.supports_tools = Some(candidate.supports_tools);
        }
        if model.max_output.is_none() {
            model.max_output = candidate.max_output;
        }
        // Reasoning: positive capability flag only, and only when name-parsing
        // left it unknown. models.dev `reasoning: false` is unreliable (provider-
        // specific, the same model is flagged both ways), so false is left as
        // None rather than asserting NonReasoning. See apply_model_traits.
        if candidate.reasoning && model.reasoning_status == ReasoningStatus::None {
            model.reasoning_status = ReasoningStatus::Reasoning;
        }
    }
}

/// Derive a creator → openness map from model-level `open_weights`.
///
/// A creator maps to `true` if any of its models is open-weight, `false` if all
/// of its models with a known status are closed, and is absent entirely when no
/// model under that creator has a known status. Drives the sidebar O/C
/// indicators and the `4` weights filter.
pub fn creator_openness(models: &[ModelRow]) -> HashMap<String, bool> {
    // creator -> (any_open, any_known)
    let mut acc: HashMap<String, (bool, bool)> = HashMap::new();
    for model in models {
        if model.creator.is_empty() {
            continue;
        }
        if let Some(open) = model.open_weights {
            let entry = acc.entry(model.creator.clone()).or_insert((false, false));
            entry.0 |= open;
            entry.1 = true;
        }
    }
    acc.into_iter()
        .filter(|(_, (_, any_known))| *any_known)
        .map(|(creator, (any_open, _))| (creator, any_open))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Model, Provider, ProvidersMap};

    fn make_provider(id: &str, models: Vec<(&str, bool)>) -> (String, Provider) {
        let mut model_map = HashMap::new();
        for (model_id, open_weights) in models {
            model_map.insert(
                model_id.to_string(),
                Model {
                    id: model_id.to_string(),
                    name: model_id.to_string(),
                    open_weights,
                    ..default_model()
                },
            );
        }
        (
            id.to_string(),
            Provider {
                id: id.to_string(),
                name: id.to_string(),
                npm: None,
                env: Vec::new(),
                doc: None,
                api: None,
                models: model_map,
            },
        )
    }

    fn default_model() -> Model {
        Model {
            id: String::new(),
            name: String::new(),
            family: None,
            reasoning: false,
            tool_call: false,
            attachment: false,
            temperature: false,
            modalities: None,
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

    /// Match a single AA model row and read back its derived openness.
    fn match_openness(providers: &[(String, Provider)], creator: &str, id: &str) -> Option<bool> {
        let mut models = vec![make_model_row(creator, id)];
        apply_model_traits(providers, &mut models);
        models[0].open_weights
    }

    #[test]
    fn test_direct_match() {
        let providers = vec![make_provider(
            "llama",
            vec![("llama-3.1-70b", true), ("llama-3.1-8b", true)],
        )];
        assert_eq!(
            match_openness(&providers, "meta", "llama-3.1-70b"),
            Some(true)
        );
    }

    #[test]
    fn test_closed_model() {
        let providers = vec![make_provider(
            "openai",
            vec![("gpt-4o", false), ("gpt-4o-mini", false)],
        )];
        assert_eq!(match_openness(&providers, "openai", "gpt-4o"), Some(false));
    }

    #[test]
    fn test_unmatched_creator_not_matched() {
        let providers = vec![make_provider("openai", vec![("gpt-4o", false)])];
        assert_eq!(
            match_openness(&providers, "unknown-lab", "some-model"),
            None
        );
    }

    #[test]
    fn test_substring_match() {
        let providers = vec![make_provider(
            "mistral",
            vec![("mistral-large-2411", false)],
        )];
        assert_eq!(
            match_openness(&providers, "mistral", "mistral-large"),
            Some(false)
        );
    }

    #[test]
    fn test_creator_to_provider_mapping() {
        // meta → llama
        let providers = vec![make_provider(
            "llama",
            vec![("llama-3.1-405b", true), ("llama-3.2-1b", true)],
        )];
        assert_eq!(
            match_openness(&providers, "meta", "llama-3.1-405b"),
            Some(true)
        );
        assert_eq!(
            match_openness(&providers, "meta", "llama-3.2-1b"),
            Some(true)
        );
    }

    #[test]
    fn test_best_score_picks_closest() {
        // "claude-35-sonnet" should match a sonnet model (not haiku); both closed.
        let providers = vec![make_provider(
            "anthropic",
            vec![
                ("claude-3-5-sonnet-20240620", false),
                ("claude-3-5-sonnet-20241022", false),
                ("claude-3-5-haiku-20241022", false),
            ],
        )];
        assert_eq!(
            match_openness(&providers, "anthropic", "claude-35-sonnet"),
            Some(false)
        );
    }

    #[test]
    fn test_best_score_prefers_longer_slug_overlap() {
        // "gemini-2-5-pro" should match "gemini-2.5-pro" over the preview variant.
        let providers = vec![make_provider(
            "google",
            vec![
                ("gemini-2.5-pro", false),
                ("gemini-2.5-pro-preview-05-06", false),
            ],
        )];
        assert_eq!(
            match_openness(&providers, "google", "gemini-2-5-pro"),
            Some(false)
        );
    }

    #[test]
    fn test_reordered_tokens_match() {
        // AA: "llama-3-1-instruct-405b" vs models.dev: "llama-3.1-405b-instruct"
        // These differ in token order but should match via Jaro-Winkler.
        let providers = vec![make_provider(
            "llama",
            vec![("llama-3.1-405b-instruct", true)],
        )];
        assert_eq!(
            match_openness(&providers, "meta", "llama-3-1-instruct-405b"),
            Some(true)
        );
    }

    #[test]
    fn test_cross_family_rejected() {
        // "gemma-3-27b" should NOT match "gemini-3-pro" — different model families.
        let providers = vec![make_provider(
            "google",
            vec![("gemini-3-pro-preview", false)],
        )];
        assert_eq!(match_openness(&providers, "google", "gemma-3-27b"), None);
    }

    /// Diagnostic test: runs v2 matching against the committed AA data file + the
    /// live models.dev API.
    /// Run manually with: cargo test diagnostic_match_rate -- --ignored --nocapture
    #[test]
    #[ignore]
    fn diagnostic_match_rate() {
        use crate::benchmarks::schema::SourceFile;

        // Load AA model rows from the committed v2 data file.
        let bench_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/v2/aa.json");
        let bench_data = std::fs::read_to_string(&bench_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", bench_path.display()));
        let file: SourceFile =
            serde_json::from_str(&bench_data).expect("Failed to parse data/v2/aa.json");

        // Fetch providers from models.dev API.
        let api_url = "https://models.dev/api.json";
        let response = reqwest::blocking::get(api_url).expect("Failed to fetch models.dev API");
        let providers_map: ProvidersMap = response.json().expect("Failed to parse API response");
        let providers: Vec<(String, crate::data::Provider)> = providers_map.into_iter().collect();

        // Run matching on a clone so we observe derived openness per model.
        let mut models = file.models.clone();
        apply_model_traits(&providers, &mut models);

        let total = models.len();
        let matched = models.iter().filter(|m| m.open_weights.is_some()).count();
        let unmatched = total - matched;
        let open_count = models
            .iter()
            .filter(|m| m.open_weights == Some(true))
            .count();
        let closed_count = models
            .iter()
            .filter(|m| m.open_weights == Some(false))
            .count();

        println!("\n=== Open Weights Match Rate ===");
        println!("Total AA models:   {total}");
        println!(
            "Matched:           {matched} ({:.1}%)",
            matched as f64 / total as f64 * 100.0
        );
        println!("  Open:            {open_count}");
        println!("  Closed:          {closed_count}");
        println!(
            "Unmatched:         {unmatched} ({:.1}%)",
            unmatched as f64 / total as f64 * 100.0
        );

        // Group unmatched by creator.
        let mut unmatched_by_creator: HashMap<&str, Vec<&str>> = HashMap::new();
        for model in &models {
            if model.open_weights.is_none() {
                unmatched_by_creator
                    .entry(model.creator.as_str())
                    .or_default()
                    .push(model.id.as_str());
            }
        }
        let mut unmatched_creators: Vec<_> = unmatched_by_creator.iter().collect();
        unmatched_creators.sort_by_key(|b| std::cmp::Reverse(b.1.len()));

        println!("\n--- Unmatched by creator ---");
        for &(creator, ids) in &unmatched_creators {
            let mapped = creator_to_providers(creator);
            let mapping_note = if mapped.is_empty() {
                format!("(identity: {})", normalize(creator))
            } else {
                format!("(mapped → {mapped:?})")
            };
            println!("{creator} ({} models) {mapping_note}", ids.len());
            for id in ids {
                println!("  - {id}");
            }
        }
    }

    fn make_model_row(creator: &str, id: &str) -> ModelRow {
        ModelRow {
            id: id.to_string(),
            name: id.to_string(),
            display_name: id.to_string(),
            creator: creator.to_string(),
            creator_name: creator.to_string(),
            release_date: None,
            reasoning_status: Default::default(),
            effort_level: None,
            variant_tag: None,
            open_weights: None,
            context_window: None,
            supports_tools: None,
            max_output: None,
            scores: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn test_apply_model_traits_fills_open_weights() {
        let providers = vec![make_provider("openai", vec![("gpt-4o", false)])];
        let mut models = vec![make_model_row("openai", "gpt-4o")];
        apply_model_traits(&providers, &mut models);
        assert_eq!(models[0].open_weights, Some(false));
    }

    #[test]
    fn test_apply_model_traits_open_via_creator_mapping() {
        // meta → llama provider; llama models are open.
        let providers = vec![make_provider("llama", vec![("llama-3.1-405b", true)])];
        let mut models = vec![make_model_row("meta", "llama-3.1-405b")];
        apply_model_traits(&providers, &mut models);
        assert_eq!(models[0].open_weights, Some(true));
    }

    #[test]
    fn test_apply_model_traits_known_creator_override() {
        // ai2 has no models.dev provider but is a known open-weight creator.
        let providers = vec![make_provider("openai", vec![("gpt-4o", false)])];
        let mut models = vec![make_model_row("ai2", "olmo-2-32b")];
        apply_model_traits(&providers, &mut models);
        assert_eq!(models[0].open_weights, Some(true));
    }

    #[test]
    fn test_apply_model_traits_unmatched_left_none() {
        let providers = vec![make_provider("openai", vec![("gpt-4o", false)])];
        let mut models = vec![make_model_row("unknown-lab", "some-model")];
        apply_model_traits(&providers, &mut models);
        assert_eq!(models[0].open_weights, None);
    }

    #[test]
    fn test_apply_model_traits_preserves_existing() {
        // A pre-populated open_weights must not be overwritten by matching.
        let providers = vec![make_provider("openai", vec![("gpt-4o", false)])];
        let mut models = vec![make_model_row("openai", "gpt-4o")];
        models[0].open_weights = Some(true);
        apply_model_traits(&providers, &mut models);
        assert_eq!(models[0].open_weights, Some(true));
    }

    #[test]
    fn test_apply_model_traits_fills_context_window() {
        let mut model_map = HashMap::new();
        model_map.insert(
            "gpt-4o".to_string(),
            Model {
                id: "gpt-4o".to_string(),
                name: "gpt-4o".to_string(),
                limit: Some(crate::data::Limits {
                    context: Some(128_000),
                    input: None,
                    output: Some(16_000),
                }),
                ..default_model()
            },
        );
        let providers = vec![(
            "openai".to_string(),
            Provider {
                id: "openai".to_string(),
                name: "openai".to_string(),
                npm: None,
                env: Vec::new(),
                doc: None,
                api: None,
                models: model_map,
            },
        )];
        let mut models = vec![make_model_row("openai", "gpt-4o")];
        apply_model_traits(&providers, &mut models);
        assert_eq!(models[0].context_window, Some(128_000));
    }

    // -- enrich_from_models_dev ---------------------------------------------

    fn make_provider_full(
        id: &str,
        name: &str,
        models: Vec<(&str, bool, Option<&str>, Option<u64>)>,
    ) -> (String, Provider) {
        let mut model_map = HashMap::new();
        for (model_id, open_weights, release_date, context) in models {
            model_map.insert(
                model_id.to_string(),
                Model {
                    id: model_id.to_string(),
                    name: model_id.to_string(),
                    open_weights,
                    release_date: release_date.map(str::to_string),
                    limit: context.map(|c| crate::data::Limits {
                        context: Some(c),
                        input: None,
                        output: None,
                    }),
                    ..default_model()
                },
            );
        }
        (
            id.to_string(),
            Provider {
                id: id.to_string(),
                name: name.to_string(),
                npm: None,
                env: Vec::new(),
                doc: None,
                api: None,
                models: model_map,
            },
        )
    }

    /// Build a source row with no creator/dates/traits (the epoch shape).
    fn bare_row(id: &str) -> ModelRow {
        let mut m = make_model_row("", id);
        m.creator_name = String::new();
        m
    }

    #[test]
    fn test_enrich_exact_match_fills_empty_fields() {
        // openai is Origin-category; gpt-4o matches exactly.
        let providers = vec![make_provider_full(
            "openai",
            "OpenAI",
            vec![("gpt-4o", false, Some("2024-05-13"), Some(128_000))],
        )];
        let mut models = vec![bare_row("gpt-4o")];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].creator, "openai");
        assert_eq!(models[0].creator_name, "OpenAI");
        assert_eq!(models[0].release_date.as_deref(), Some("2024-05-13"));
        assert_eq!(models[0].context_window, Some(128_000));
        assert_eq!(models[0].open_weights, Some(false));
    }

    /// Provider with a single model carrying a specific `reasoning` flag.
    fn provider_with_reasoning(
        provider_id: &str,
        model_id: &str,
        reasoning: bool,
    ) -> (String, Provider) {
        let mut model_map = HashMap::new();
        model_map.insert(
            model_id.to_string(),
            Model {
                id: model_id.to_string(),
                name: model_id.to_string(),
                reasoning,
                ..default_model()
            },
        );
        (
            provider_id.to_string(),
            Provider {
                id: provider_id.to_string(),
                name: provider_id.to_string(),
                npm: None,
                env: Vec::new(),
                doc: None,
                api: None,
                models: model_map,
            },
        )
    }

    #[test]
    fn test_enrich_reasoning_true_fills_unknown() {
        // models.dev reasoning=true + name-parse left it None -> Reasoning.
        let providers = vec![provider_with_reasoning("openai", "gpt-5", true)];
        let mut models = vec![bare_row("gpt-5")];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].reasoning_status, ReasoningStatus::Reasoning);
    }

    #[test]
    fn test_enrich_reasoning_false_stays_none() {
        // models.dev reasoning=false is unreliable -> NOT mapped to NonReasoning;
        // the honest "unknown" (None) is preserved.
        let providers = vec![provider_with_reasoning("openai", "gpt-5", false)];
        let mut models = vec![bare_row("gpt-5")];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].reasoning_status, ReasoningStatus::None);
    }

    #[test]
    fn test_enrich_does_not_override_name_parsed_reasoning() {
        // The load-bearing guard: an explicit name-parsed status (Adaptive here,
        // and NonReasoning) is NEVER overridden by a conflicting models.dev flag.
        let providers = vec![provider_with_reasoning("openai", "gpt-5", true)];
        let mut adaptive = bare_row("gpt-5");
        adaptive.reasoning_status = ReasoningStatus::Adaptive;
        let mut nonreasoning = bare_row("gpt-5");
        nonreasoning.reasoning_status = ReasoningStatus::NonReasoning;
        let mut models = vec![adaptive, nonreasoning];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].reasoning_status, ReasoningStatus::Adaptive);
        assert_eq!(models[1].reasoning_status, ReasoningStatus::NonReasoning);
    }

    #[test]
    fn test_enrich_normalized_match_strips_suffix() {
        // Source id carries a `-thinking` variant suffix + a trailing date stamp
        // that models.dev's bare id does not.
        let providers = vec![make_provider_full(
            "anthropic",
            "Anthropic",
            vec![("claude-opus-4-8", false, Some("2026-05-28"), Some(200_000))],
        )];
        let mut models = vec![bare_row("claude-opus-4-8-thinking")];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].creator, "anthropic");
        assert_eq!(models[0].release_date.as_deref(), Some("2026-05-28"));
        assert_eq!(models[0].context_window, Some(200_000));
    }

    #[test]
    fn test_enrich_normalized_date_stamp_and_host_prefix() {
        // `-2026-03-05` date stamp + `-web-app` suffix + `fireworks/` host prefix
        // all stripped down to `gpt-5.4-pro`.
        let providers = vec![make_provider_full(
            "openai",
            "OpenAI",
            vec![("gpt-5.4-pro", false, Some("2026-03-05"), None)],
        )];
        let mut models = vec![bare_row("fireworks/gpt-5.4-pro-2026-03-05-web-app")];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].creator, "openai");
        assert_eq!(models[0].release_date.as_deref(), Some("2026-03-05"));
    }

    #[test]
    fn test_enrich_never_overwrites_source_values() {
        let providers = vec![make_provider_full(
            "openai",
            "OpenAI",
            vec![("gpt-4o", true, Some("2099-01-01"), Some(999))],
        )];
        let mut m = make_model_row("custom-creator", "gpt-4o");
        m.creator_name = "Custom Creator".to_string();
        m.release_date = Some("2024-01-01".to_string());
        m.context_window = Some(64_000);
        m.open_weights = Some(false);
        let mut models = vec![m];
        enrich_from_models_dev(&providers, &mut models);
        // Every source-provided field survives untouched.
        assert_eq!(models[0].creator, "custom-creator");
        assert_eq!(models[0].creator_name, "Custom Creator");
        assert_eq!(models[0].release_date.as_deref(), Some("2024-01-01"));
        assert_eq!(models[0].context_window, Some(64_000));
        assert_eq!(models[0].open_weights, Some(false));
    }

    #[test]
    fn test_enrich_unmatched_left_untouched() {
        let providers = vec![make_provider_full(
            "openai",
            "OpenAI",
            vec![("gpt-4o", false, Some("2024-05-13"), Some(128_000))],
        )];
        let mut models = vec![bare_row("totally-unknown-model-xyz")];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].creator, "");
        assert_eq!(models[0].creator_name, "");
        assert_eq!(models[0].release_date, None);
        assert_eq!(models[0].context_window, None);
        assert_eq!(models[0].open_weights, None);
    }

    #[test]
    fn test_enrich_prefers_origin_provider_for_creator() {
        // The same model id is hosted under an Inference provider (deepinfra,
        // listed first → first-seen would lose) and its Origin provider
        // (deepseek). Origin must win for creator + open_weights.
        let providers = vec![
            make_provider_full(
                "deepinfra",
                "DeepInfra",
                vec![("deepseek-v4-pro", false, None, None)],
            ),
            make_provider_full(
                "deepseek",
                "DeepSeek",
                vec![("deepseek-v4-pro", true, Some("2026-04-24"), Some(163_840))],
            ),
        ];
        let mut models = vec![bare_row("deepseek-v4-pro")];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].creator, "deepseek");
        assert_eq!(models[0].creator_name, "DeepSeek");
        assert_eq!(models[0].open_weights, Some(true));
        assert_eq!(models[0].release_date.as_deref(), Some("2026-04-24"));
    }

    #[test]
    fn test_enrich_fills_only_creator_when_others_present() {
        // Arena shape: creator + open_weights present, but release_date/context
        // missing. Enrichment should add the missing two without disturbing the
        // present creator/openness.
        let providers = vec![make_provider_full(
            "google",
            "Google",
            vec![("gemini-2.5-pro", false, Some("2025-03-25"), Some(1_048_576))],
        )];
        let mut m = make_model_row("google", "gemini-2.5-pro");
        m.creator_name = "Google".to_string();
        m.open_weights = Some(false);
        // release_date + context_window left None (arena omits them).
        let mut models = vec![m];
        enrich_from_models_dev(&providers, &mut models);
        assert_eq!(models[0].creator, "google");
        assert_eq!(models[0].release_date.as_deref(), Some("2025-03-25"));
        assert_eq!(models[0].context_window, Some(1_048_576));
    }

    /// Diagnostic: run generic enrichment against the committed v2 data files +
    /// the live models.dev API. Reports per-source match rates.
    /// Run with: cargo test enrich_match_rate -- --ignored --nocapture
    #[test]
    #[ignore]
    fn enrich_match_rate() {
        use crate::benchmarks::schema::SourceFile;

        let api_url = "https://models.dev/api.json";
        let response = reqwest::blocking::get(api_url).expect("Failed to fetch models.dev API");
        let providers_map: ProvidersMap = response.json().expect("Failed to parse API response");
        let providers: Vec<(String, crate::data::Provider)> = providers_map.into_iter().collect();

        for source in ["epoch", "arena", "llmstats"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("data/v2/{source}.json"));
            let data = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("Failed to read {}", path.display()));
            let file: SourceFile = serde_json::from_str(&data)
                .unwrap_or_else(|_| panic!("Failed to parse {}", path.display()));

            let total = file.models.len();
            let had_creator = file.models.iter().filter(|m| !m.creator.is_empty()).count();
            let had_release = file
                .models
                .iter()
                .filter(|m| m.release_date.is_some())
                .count();

            let mut models = file.models.clone();
            enrich_from_models_dev(&providers, &mut models);

            let creator_now = models.iter().filter(|m| !m.creator.is_empty()).count();
            let release_now = models.iter().filter(|m| m.release_date.is_some()).count();
            let ow_now = models.iter().filter(|m| m.open_weights.is_some()).count();
            let ctx_now = models.iter().filter(|m| m.context_window.is_some()).count();

            println!("\n=== {source} ({total} models) ===");
            println!("creator:        {had_creator} -> {creator_now}");
            println!("release_date:   {had_release} -> {release_now}");
            println!("open_weights:   filled {ow_now}");
            println!("context_window: filled {ctx_now}");

            // Unmatched = no field got newly filled AND none were present that
            // would suppress a match attempt; approximate via context_window
            // (always absent in sources, so reflects raw match rate).
            let matched = ctx_now;
            println!(
                "match rate (ctx proxy): {matched}/{total} ({:.0}%)",
                matched as f64 / total as f64 * 100.0
            );
        }
    }

    #[test]
    fn test_creator_openness() {
        let models = vec![
            {
                let mut m = make_model_row("meta", "llama-a");
                m.open_weights = Some(true);
                m
            },
            {
                let mut m = make_model_row("meta", "llama-b");
                m.open_weights = Some(false); // any-open wins → meta = open
                m
            },
            {
                let mut m = make_model_row("openai", "gpt-a");
                m.open_weights = Some(false);
                m
            },
            // unknown openness — should not create an entry
            make_model_row("mystery", "x"),
        ];
        let map = creator_openness(&models);
        assert_eq!(map.get("meta"), Some(&true));
        assert_eq!(map.get("openai"), Some(&false));
        assert!(!map.contains_key("mystery"));
    }
}
