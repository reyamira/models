//! Wire format for the generated models.dev provider-offering → canonical-model
//! reference map.
//!
//! The file is generated from models.dev's public `providers/*/models/**/*.toml`
//! sources. It carries only the identity edge that `catalog.json` omits; the
//! live catalog remains authoritative for provider and canonical-model data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const UPSTREAM_REPOSITORY: &str = "https://github.com/anomalyco/models.dev";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BaseModelRefsFile {
    pub schema_version: u32,
    pub upstream_repository: String,
    pub upstream_commit: String,
    pub refs: BTreeMap<String, String>,
}

impl BaseModelRefsFile {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported base-model refs schema {}, expected {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.upstream_repository != UPSTREAM_REPOSITORY {
            return Err(format!(
                "unexpected upstream repository: {}",
                self.upstream_repository
            ));
        }
        if self.upstream_commit.len() != 40
            || !self
                .upstream_commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("upstream_commit must be a 40-character Git commit hash".to_string());
        }
        if self.refs.is_empty() {
            return Err("base-model refs must not be empty".to_string());
        }
        for (offering, canonical) in &self.refs {
            if !offering.contains('/') || canonical.split_once('/').is_none() {
                return Err(format!(
                    "invalid base-model reference: {offering} -> {canonical}"
                ));
            }
        }
        Ok(())
    }
}
