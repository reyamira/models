use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FetchStatus {
    #[default]
    NotStarted,
    Loading,
    Loaded,
    Failed(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub last_scraped: Option<String>,
    #[serde(default)]
    pub scrape_source: Option<String>,
    pub agents: HashMap<String, Agent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Agent {
    pub name: String,
    pub repo: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub installation_method: Option<String>,
    #[serde(default)]
    pub pricing: Option<Pricing>,
    #[serde(default)]
    pub supported_providers: Vec<String>,
    #[serde(default)]
    pub platform_support: Vec<String>,
    #[serde(default)]
    pub open_source: bool,
    #[serde(default)]
    pub cli_binary: Option<String>,
    #[serde(default)]
    pub alt_binaries: Vec<String>,
    #[serde(default)]
    pub version_command: Vec<String>,
    /// Non-interactive, argv-vec self-update command (no shell), e.g.
    /// `["claude", "update"]`. Empty = no in-app update action for this agent.
    /// Only ever populated with commands verified from the tool's official docs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub update_command: Vec<String>,
    #[serde(default)]
    pub version_regex: Option<String>,
    #[serde(default)]
    pub config_files: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub docs: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pricing {
    pub model: String,
    #[serde(default)]
    pub subscription_price: Option<f64>,
    #[serde(default)]
    pub subscription_period: Option<String>,
    #[serde(default)]
    pub free_tier: bool,
    #[serde(default)]
    pub usage_notes: Option<String>,
}

/// A single release from GitHub
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub date: Option<String>,
    pub changelog: Option<String>,
}

/// GitHub API data - fetched live and cached
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct GitHubData {
    pub releases: Vec<Release>,
    pub stars: Option<u64>,
    pub open_issues: Option<u64>,
    pub license: Option<String>,
    pub last_commit: Option<String>,
}

impl GitHubData {
    /// Get the latest release (first in the list)
    pub fn latest_release(&self) -> Option<&Release> {
        self.releases.first()
    }

    /// Get the latest version string
    pub fn latest_version(&self) -> Option<&str> {
        self.latest_release().map(|r| r.version.as_str())
    }

    pub fn latest_release_date(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.latest_release()
            .and_then(|r| r.date.as_deref())
            .and_then(crate::agents::helpers::parse_date)
    }

    pub fn release_dates(&self) -> Vec<chrono::DateTime<chrono::Utc>> {
        self.releases
            .iter()
            .filter_map(|r| r.date.as_deref())
            .filter_map(crate::agents::helpers::parse_date)
            .collect()
    }

    pub fn release_frequency(&self) -> String {
        crate::agents::helpers::calculate_release_frequency(&self.release_dates())
    }
}

/// Installed CLI info - path field for future use
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct InstalledInfo {
    pub version: Option<String>,
    pub path: Option<String>,
}

/// Agent entry combining static and runtime data
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AgentEntry {
    pub id: String,
    pub agent: Agent,
    pub github: GitHubData,
    pub installed: InstalledInfo,
    pub tracked: bool,
    pub fetch_status: FetchStatus,
}

impl AgentEntry {
    /// The agent's verified self-update argv, if it has one (the registry only
    /// ships commands confirmed from official docs). `None` = no update action.
    pub fn update_command(&self) -> Option<&[String]> {
        if self.agent.update_command.is_empty() {
            None
        } else {
            Some(&self.agent.update_command)
        }
    }

    pub fn update_available(&self) -> bool {
        match (&self.installed.version, self.github.latest_version()) {
            (Some(installed), Some(latest)) => {
                // Try semver comparison, fallback to string
                match (
                    semver::Version::parse(installed),
                    semver::Version::parse(latest),
                ) {
                    (Ok(i), Ok(l)) => l > i,
                    _ => latest != installed,
                }
            }
            _ => false,
        }
    }

    /// Find releases between installed version and latest (exclusive of installed)
    pub fn new_releases(&self) -> Vec<&Release> {
        let installed = match &self.installed.version {
            Some(v) => v,
            None => return self.github.releases.iter().collect(), // All releases are "new" if not installed
        };

        self.github
            .releases
            .iter()
            .take_while(|r| r.version != *installed)
            .collect()
    }

    pub fn latest_release_relative_time(&self) -> Option<String> {
        self.github
            .latest_release_date()
            .map(|dt| crate::agents::helpers::format_relative_time(&dt))
    }

    pub fn release_frequency(&self) -> String {
        self.github.release_frequency()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn sample_github_data(dates: &[chrono::DateTime<Utc>]) -> GitHubData {
        GitHubData {
            releases: dates
                .iter()
                .enumerate()
                .map(|(idx, date)| Release {
                    version: format!("1.0.{}", idx),
                    date: Some(date.to_rfc3339()),
                    changelog: None,
                })
                .collect(),
            ..GitHubData::default()
        }
    }

    #[test]
    fn github_release_frequency_uses_parsed_release_dates() {
        let now = Utc::now();
        let github = sample_github_data(&[now, now - Duration::days(7), now - Duration::days(14)]);

        assert_eq!(github.release_frequency(), "~1w");
    }

    #[test]
    fn latest_release_relative_time_uses_latest_release_date() {
        let now = Utc::now();
        let entry = AgentEntry {
            id: "claude".to_string(),
            agent: Agent {
                name: "Claude Code".to_string(),
                repo: "anthropics/claude-code".to_string(),
                categories: vec![],
                installation_method: None,
                pricing: None,
                supported_providers: vec![],
                platform_support: vec![],
                open_source: false,
                cli_binary: None,
                alt_binaries: vec![],
                version_command: vec![],
                update_command: vec![],
                version_regex: None,
                config_files: vec![],
                homepage: None,
                docs: None,
            },
            github: sample_github_data(&[now - Duration::days(2)]),
            installed: InstalledInfo::default(),
            tracked: true,
            fetch_status: FetchStatus::Loaded,
        };

        assert_eq!(
            entry.latest_release_relative_time(),
            Some("2d ago".to_string())
        );
    }
}
