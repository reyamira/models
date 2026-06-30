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

    /// The update argv to actually run, using detected install info. When the
    /// updater invokes the agent's **own** binary (a self-updater like
    /// `["claude", "update"]`) and `detect_installed` resolved an absolute path,
    /// substitute that path for argv[0] so we update the exact detected copy
    /// rather than whatever PATH happens to resolve first. Package-manager
    /// updaters (`["npm", …]`, `["uv", …]`) are returned unchanged — argv[0]
    /// isn't the agent binary, so there's nothing to pin.
    pub fn resolved_update_command(&self) -> Option<Vec<String>> {
        let mut cmd = self.update_command()?.to_vec();
        if let (Some(bin), Some(path)) = (
            self.agent.cli_binary.as_deref(),
            self.installed.path.as_deref(),
        ) {
            if cmd.first().is_some_and(|first| first == bin) {
                cmd[0] = path.to_string();
            }
        }
        Some(cmd)
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

    fn entry_with(
        cli_binary: Option<&str>,
        update_command: &[&str],
        installed_path: Option<&str>,
    ) -> AgentEntry {
        AgentEntry {
            id: "x".to_string(),
            agent: Agent {
                name: "X".to_string(),
                repo: "o/x".to_string(),
                categories: vec![],
                installation_method: None,
                pricing: None,
                supported_providers: vec![],
                platform_support: vec![],
                open_source: false,
                cli_binary: cli_binary.map(str::to_string),
                alt_binaries: vec![],
                version_command: vec![],
                update_command: update_command.iter().map(|s| s.to_string()).collect(),
                version_regex: None,
                config_files: vec![],
                homepage: None,
                docs: None,
            },
            github: GitHubData::default(),
            installed: InstalledInfo {
                version: installed_path.map(|_| "1.0.0".to_string()),
                path: installed_path.map(str::to_string),
            },
            tracked: true,
            fetch_status: FetchStatus::Loaded,
        }
    }

    #[test]
    fn resolved_update_command_pins_self_updater_to_detected_path() {
        // Self-updater whose argv[0] is the agent binary + a detected path → pin.
        let e = entry_with(
            Some("claude"),
            &["claude", "update"],
            Some("/home/u/.local/bin/claude"),
        );
        assert_eq!(
            e.resolved_update_command().unwrap(),
            vec!["/home/u/.local/bin/claude", "update"]
        );
    }

    #[test]
    fn resolved_update_command_leaves_package_manager_argv_unchanged() {
        // npm updater: argv[0] is npm, not the agent binary → unchanged even with a path.
        let e = entry_with(
            Some("gemini"),
            &["npm", "install", "-g", "@google/gemini-cli@latest"],
            Some("/opt/homebrew/bin/gemini"),
        );
        assert_eq!(
            e.resolved_update_command().unwrap(),
            vec!["npm", "install", "-g", "@google/gemini-cli@latest"]
        );
    }

    #[test]
    fn resolved_update_command_falls_back_to_bare_binary_without_path() {
        // Self-updater but no detected path → keep the bare binary (PATH lookup).
        let e = entry_with(Some("claude"), &["claude", "update"], None);
        assert_eq!(
            e.resolved_update_command().unwrap(),
            vec!["claude", "update"]
        );
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
