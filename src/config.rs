use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Routing discriminant for symlink aliases -- not a config field, not serde-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasKind {
    Agents,
    Benchmarks,
    Status,
}

fn default_agents_alias() -> String {
    "agents".to_string()
}

fn default_benchmarks_alias() -> String {
    "benchmarks".to_string()
}

fn default_status_alias() -> String {
    "mstatus".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AliasesConfig {
    #[serde(default = "default_agents_alias")]
    pub agents: String,
    #[serde(default = "default_benchmarks_alias")]
    pub benchmarks: String,
    #[serde(default = "default_status_alias")]
    pub status: String,
}

impl Default for AliasesConfig {
    fn default() -> Self {
        Self {
            agents: default_agents_alias(),
            benchmarks: default_benchmarks_alias(),
            status: default_status_alias(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub config_version: u32,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub status: StatusConfig,
    #[serde(default)]
    pub aliases: AliasesConfig,
    #[serde(default)]
    pub benchmarks: BenchmarksConfig,
}

/// Benchmarks-tab persistence.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BenchmarksConfig {
    /// Visible metric columns per data-source id (`aa` / `epoch` / `arena` /
    /// `llmstats`), saved when the column picker applies. Values are metric
    /// **ids**, not indices — Epoch's auto-prune shifts metric positions
    /// between pipeline runs, so ids are the stable handle. Ids whose metric
    /// no longer exists in the loaded file are silently dropped at resolve
    /// time.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub columns: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomAgent {
    pub name: String,
    pub repo: String,
    #[serde(default)]
    pub agent_type: Option<String>, // "cli" or "ide"
    #[serde(default)]
    pub binary: Option<String>,
    #[serde(default)]
    pub version_command: Option<Vec<String>>,
}

impl CustomAgent {
    pub fn to_agent(&self) -> crate::agents::Agent {
        crate::agents::Agent {
            name: self.name.clone(),
            repo: self.repo.clone(),
            categories: self
                .agent_type
                .as_ref()
                .map(|t| vec![t.clone()])
                .unwrap_or_default(),
            cli_binary: self.binary.clone(),
            alt_binaries: vec![],
            version_command: self.version_command.clone().unwrap_or_default(),
            update_command: vec![],
            installation_method: self.agent_type.clone(),
            pricing: None,
            supported_providers: vec![],
            platform_support: vec![],
            open_source: true,
            version_regex: None,
            config_files: vec![],
            homepage: None,
            docs: None,
        }
    }
}

/// Default starter agents for new users
fn default_tracked_agents() -> HashSet<String> {
    ["claude-code", "codex", "gemini-cli", "opencode"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentsConfig {
    #[serde(default = "default_tracked_agents")]
    pub tracked: HashSet<String>,
    #[serde(default)]
    pub excluded: HashSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<CustomAgent>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            tracked: default_tracked_agents(),
            excluded: HashSet::new(),
            custom: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheConfig {
    #[serde(default = "default_github_ttl")]
    pub github_ttl_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            github_ttl_seconds: default_github_ttl(),
        }
    }
}

fn default_github_ttl() -> u64 {
    3600
}

/// Default: all status providers tracked.
fn default_tracked_providers() -> HashSet<String> {
    crate::status::STATUS_REGISTRY
        .iter()
        .map(|e| e.slug.to_string())
        .collect()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusConfig {
    #[serde(default = "default_tracked_providers")]
    pub tracked: HashSet<String>,
}

impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            tracked: default_tracked_providers(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DisplayConfig {
    #[serde(default)]
    pub default_tab: Option<String>,
}

impl Config {
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("models").join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        let path = match Self::config_path() {
            Some(p) => p,
            None => return Ok(Self::default()),
        };

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;

        // Strip legacy `custom = []` lines that conflict with [[agents.custom]] blocks.
        // Older versions of save() serialized the empty Vec as an inline array, which
        // causes a TOML parse error if the user later adds [[agents.custom]] entries.
        let content: String = content
            .lines()
            .filter(|line| line.trim() != "custom = []")
            .collect::<Vec<_>>()
            .join("\n");

        toml::from_str(&content).context("Failed to parse config.toml")
    }

    pub fn save(&self) -> Result<()> {
        let path = match Self::config_path() {
            Some(p) => p,
            None => anyhow::bail!("Could not determine config directory"),
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir: {}", parent.display()))?;
        }

        let mut content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        // Append a commented example for custom agents when none are configured,
        // so users know the syntax without needing to look up docs.
        if self.agents.custom.is_empty() {
            content.push_str(
                "\n# To track custom agents, add [[agents.custom]] blocks:\n\
                 #\n\
                 # [[agents.custom]]\n\
                 # name = \"My Agent\"\n\
                 # repo = \"owner/repo\"\n\
                 # agent_type = \"cli\"       # optional: \"cli\" or \"ide\"\n\
                 # binary = \"myagent\"       # optional: for version detection\n\
                 # version_command = [\"myagent\", \"--version\"]  # optional\n\
                 #\n\
                 # Add multiple agents with additional [[agents.custom]] blocks.\n\
                 # See: https://github.com/reyamira/models/wiki/Configuration#custom-agents\n",
            );
        }

        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))?;

        Ok(())
    }

    pub fn is_tracked(&self, agent_id: &str) -> bool {
        if self.agents.excluded.contains(agent_id) {
            return false;
        }
        self.agents.tracked.contains(agent_id)
    }

    pub fn set_tracked(&mut self, agent_id: &str, tracked: bool) {
        if tracked {
            self.agents.tracked.insert(agent_id.to_string());
            self.agents.excluded.remove(agent_id);
        } else {
            self.agents.tracked.remove(agent_id);
            self.agents.excluded.insert(agent_id.to_string());
        }
    }

    /// Returns the list of (alias_name, alias_kind) tuples for symlink operations.
    pub fn alias_names(&self) -> Vec<(&str, AliasKind)> {
        vec![
            (&self.aliases.agents, AliasKind::Agents),
            (&self.aliases.benchmarks, AliasKind::Benchmarks),
            (&self.aliases.status, AliasKind::Status),
        ]
    }

    /// Given a binary name (from argv[0]), returns which alias it matches, if any.
    pub fn match_alias(&self, binary_name: &str) -> Option<AliasKind> {
        if binary_name == self.aliases.agents {
            return Some(AliasKind::Agents);
        }
        if binary_name == self.aliases.benchmarks {
            return Some(AliasKind::Benchmarks);
        }
        if binary_name == self.aliases.status {
            return Some(AliasKind::Status);
        }
        None
    }

    pub fn is_status_tracked(&self, slug: &str) -> bool {
        self.status.tracked.contains(slug)
    }

    pub fn set_status_tracked(&mut self, slug: &str, tracked: bool) {
        if tracked {
            self.status.tracked.insert(slug.to_string());
        } else {
            self.status.tracked.remove(slug);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmarks_columns_roundtrip_and_default_empty() {
        let mut config = Config::default();
        config
            .benchmarks
            .columns
            .insert("aa".to_string(), vec!["gpqa".to_string()]);
        let serialized = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(
            back.benchmarks.columns.get("aa").unwrap(),
            &vec!["gpqa".to_string()]
        );
        // A config without the [benchmarks] section deserializes to empty.
        let none: Config = toml::from_str("").unwrap();
        assert!(none.benchmarks.columns.is_empty());
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.cache.github_ttl_seconds, 3600);
        // Default includes starter agents
        assert_eq!(config.agents.tracked.len(), 4);
        assert!(config.agents.tracked.contains("claude-code"));
        assert!(config.agents.tracked.contains("codex"));
        assert!(config.agents.tracked.contains("gemini-cli"));
        assert!(config.agents.tracked.contains("opencode"));
    }

    #[test]
    fn test_is_tracked_default() {
        let config = Config::default();
        // Default tracked agents
        assert!(config.is_tracked("claude-code"));
        assert!(config.is_tracked("codex"));
        // Not in default list
        assert!(!config.is_tracked("cursor"));
    }

    #[test]
    fn test_is_tracked_excluded() {
        let mut config = Config::default();
        config.agents.excluded.insert("claude-code".to_string());
        // Excluded even though in tracked list
        assert!(!config.is_tracked("claude-code"));
        // Still tracked
        assert!(config.is_tracked("codex"));
    }

    #[test]
    fn test_status_default_tracks_all_providers() {
        use crate::status::STATUS_REGISTRY;
        let config = Config::default();
        assert_eq!(config.status.tracked.len(), STATUS_REGISTRY.len());
        for entry in STATUS_REGISTRY {
            assert!(config.is_status_tracked(entry.slug));
        }
    }

    #[test]
    fn test_set_status_tracked() {
        let mut config = Config::default();
        // Untrack a provider
        config.set_status_tracked("openai", false);
        assert!(!config.is_status_tracked("openai"));
        // Re-track it
        config.set_status_tracked("openai", true);
        assert!(config.is_status_tracked("openai"));
    }

    #[test]
    fn test_default_aliases() {
        let config = Config::default();
        assert_eq!(config.aliases.agents, "agents");
        assert_eq!(config.aliases.benchmarks, "benchmarks");
        assert_eq!(config.aliases.status, "mstatus");
    }

    #[test]
    fn test_match_alias() {
        let config = Config::default();
        assert_eq!(config.match_alias("agents"), Some(AliasKind::Agents));
        assert_eq!(
            config.match_alias("benchmarks"),
            Some(AliasKind::Benchmarks)
        );
        assert_eq!(config.match_alias("mstatus"), Some(AliasKind::Status));
        assert_eq!(config.match_alias("models"), None);
        assert_eq!(config.match_alias("status"), None);
        assert_eq!(config.match_alias(""), None);
    }

    #[test]
    fn test_alias_names_returns_all_three() {
        let config = Config::default();
        let names = config.alias_names();
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], ("agents", AliasKind::Agents));
        assert_eq!(names[1], ("benchmarks", AliasKind::Benchmarks));
        assert_eq!(names[2], ("mstatus", AliasKind::Status));
    }

    #[test]
    fn test_aliases_config_deserializes_with_defaults_when_section_absent() {
        let toml = r#"
config_version = 1
"#;
        let config: Config = toml::from_str(toml).expect("should parse");
        assert_eq!(config.aliases.agents, "agents");
        assert_eq!(config.aliases.benchmarks, "benchmarks");
        assert_eq!(config.aliases.status, "mstatus");
    }

    #[test]
    fn test_aliases_config_custom_values() {
        let toml = r#"
[aliases]
agents = "myagents"
benchmarks = "bench"
status = "mystatus"
"#;
        let config: Config = toml::from_str(toml).expect("should parse");
        assert_eq!(config.aliases.agents, "myagents");
        assert_eq!(config.aliases.benchmarks, "bench");
        assert_eq!(config.aliases.status, "mystatus");
        assert_eq!(config.match_alias("myagents"), Some(AliasKind::Agents));
        assert_eq!(config.match_alias("mystatus"), Some(AliasKind::Status));
        assert_eq!(config.match_alias("agents"), None);
    }
}
