use std::collections::HashMap;

use super::schema::ModelRow;
use crate::data::Provider;

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
}

impl ModelTraits {
    fn from_model(model: &crate::data::Model) -> Self {
        Self {
            open_weights: model.open_weights,
            context_window: model.limit.as_ref().and_then(|l| l.context),
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
                });
            }
        }

        // Stage 3: Known creator overrides for providers absent from models.dev
        known_creator_openness(creator).map(|ow| ModelTraits {
            open_weights: ow,
            context_window: None,
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
