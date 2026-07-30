use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::data::ProvidersMap;
use crate::labs::{CanonicalModel, LabCatalog};

const API_URL: &str = "https://models.dev/api.json";
const CATALOG_URL: &str = "https://models.dev/catalog.json";

#[derive(Debug)]
pub struct ModelsCatalog {
    pub providers: ProvidersMap,
    pub lab_catalog: LabCatalog,
}

#[derive(Deserialize)]
struct CatalogResponse {
    providers: ProvidersMap,
    models: HashMap<String, CanonicalModel>,
}

pub fn fetch_providers() -> Result<ProvidersMap> {
    let response =
        reqwest::blocking::get(API_URL).context("Failed to fetch data from models.dev API")?;

    let providers: ProvidersMap = response.json().context("Failed to parse API response")?;

    Ok(providers)
}

/// Fetch the provider catalog and canonical model registry as one coherent
/// models.dev snapshot. This is the TUI path; CLI commands that need provider
/// rows only continue to use the smaller `api.json` response above.
pub fn fetch_catalog() -> Result<ModelsCatalog> {
    let response = reqwest::blocking::get(CATALOG_URL)
        .context("Failed to fetch catalog from models.dev API")?;
    let catalog: CatalogResponse = response
        .json()
        .context("Failed to parse catalog response")?;

    let lab_catalog = LabCatalog::from_catalog(&catalog.models, &catalog.providers);
    Ok(ModelsCatalog {
        providers: catalog.providers,
        lab_catalog,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_response_builds_provider_and_canonical_views() {
        let response: CatalogResponse = serde_json::from_str(
            r#"{
                "providers": {
                    "amazon-bedrock": {
                        "id": "amazon-bedrock",
                        "name": "Bedrock",
                        "models": {
                            "eu.anthropic.claude-opus-5": {
                                "id": "eu.anthropic.claude-opus-5",
                                "name": "Claude Opus 5 (EU)"
                            }
                        }
                    }
                },
                "models": {
                    "anthropic/claude-opus-5": {
                        "name": "Claude Opus 5",
                        "family": "claude-opus"
                    }
                }
            }"#,
        )
        .expect("valid catalog response");
        let lab_catalog = LabCatalog::from_catalog(&response.models, &response.providers);
        let snapshot = ModelsCatalog {
            providers: response.providers,
            lab_catalog,
        };

        assert_eq!(snapshot.providers.len(), 1);
        assert_eq!(
            snapshot
                .lab_catalog
                .resolve_model("amazon-bedrock", "eu.anthropic.claude-opus-5"),
            Some(("anthropic/claude-opus-5", "Claude Opus 5", "anthropic"))
        );
    }
}
