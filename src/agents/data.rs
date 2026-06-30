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

    /// The install method inferred from the detected binary path, if recognizable.
    pub fn install_method(&self) -> Option<InstallMethod> {
        self.installed
            .path
            .as_deref()
            .and_then(infer_install_method)
    }

    /// The update argv to actually run, made install-aware from detected info:
    ///
    /// 1. **Self-updater** (argv[0] is the agent's own binary, e.g. `claude update`):
    ///    pin argv[0] to the detected absolute path so the exact detected copy is
    ///    updated, not whatever PATH resolves first.
    /// 2. **JS package-manager updater** (`npm install -g <pkg>`): if the binary was
    ///    actually installed by a *different* JS package manager (bun / pnpm / yarn),
    ///    swap to that manager keeping the same package spec — the package id is
    ///    portable, so `bun add -g <pkg>` updates the copy you're running while
    ///    `npm i -g` would miss it. An npm install, an unrecognized method, or a
    ///    non-JS method (Homebrew/uv/…) keeps the registry command unchanged.
    pub fn resolved_update_command(&self) -> Option<Vec<String>> {
        let mut cmd = self.update_command()?.to_vec();

        // (1) Self-updater → pin to the detected path.
        if let (Some(bin), Some(path)) = (
            self.agent.cli_binary.as_deref(),
            self.installed.path.as_deref(),
        ) {
            if cmd.first().is_some_and(|first| first == bin) {
                cmd[0] = path.to_string();
                return Some(cmd);
            }
        }

        // (2) JS package-manager swap to the manager that owns the install.
        if let Some(pkg) = npm_global_package(&cmd) {
            if let Some(swapped) = self
                .install_method()
                .and_then(|m| js_pm_global_add(m, &pkg))
            {
                return Some(swapped);
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

/// How an installed CLI was put on disk, inferred from its detected path. Used to
/// make the update command target the copy the user is actually running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Homebrew,
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Uv,
    Pipx,
    Cargo,
}

impl InstallMethod {
    pub fn label(&self) -> &'static str {
        match self {
            InstallMethod::Homebrew => "Homebrew",
            InstallMethod::Npm => "npm",
            InstallMethod::Pnpm => "pnpm",
            InstallMethod::Yarn => "yarn",
            InstallMethod::Bun => "bun",
            InstallMethod::Uv => "uv",
            InstallMethod::Pipx => "pipx",
            InstallMethod::Cargo => "cargo",
        }
    }
}

/// Infer the install method from a binary's absolute path. Conservative — returns
/// `None` for ambiguous locations (e.g. a bare `~/.local/bin` native-installer
/// shim) so callers only adapt when confident. Substring checks are
/// case-insensitive and ordered most-specific first.
pub fn infer_install_method(path: &str) -> Option<InstallMethod> {
    let p = path.to_lowercase();
    // Homebrew (Intel /usr/local/Cellar, Apple-silicon /opt/homebrew, linuxbrew).
    if p.contains("/cellar/") || p.contains("/homebrew/") || p.contains("linuxbrew") {
        return Some(InstallMethod::Homebrew);
    }
    if p.contains("/.bun/") {
        return Some(InstallMethod::Bun);
    }
    if p.contains("/pnpm/") || p.contains("/.pnpm") {
        return Some(InstallMethod::Pnpm);
    }
    if p.contains("/.yarn/") || p.contains("yarn/global") {
        return Some(InstallMethod::Yarn);
    }
    // npm global prefixes: system node_modules, nvm, fnm, volta, npm-global.
    if p.contains("node_modules") || p.contains("/.nvm/") || p.contains("/.fnm/") {
        return Some(InstallMethod::Npm);
    }
    if p.contains("/uv/tools") || p.contains("share/uv") {
        return Some(InstallMethod::Uv);
    }
    if p.contains("/pipx/") {
        return Some(InstallMethod::Pipx);
    }
    if p.contains("/.cargo/") {
        return Some(InstallMethod::Cargo);
    }
    None
}

/// If `cmd` is an npm global install (`npm install|i -g|--global <pkg>`), return
/// the package spec (the last argument, e.g. `@google/gemini-cli@latest`).
fn npm_global_package(cmd: &[String]) -> Option<String> {
    let first = cmd.first().map(String::as_str)?;
    if first != "npm" {
        return None;
    }
    let is_install = cmd.iter().any(|a| a == "install" || a == "i");
    let is_global = cmd.iter().any(|a| a == "-g" || a == "--global");
    if is_install && is_global {
        cmd.last().filter(|s| !s.starts_with('-')).cloned()
    } else {
        None
    }
}

/// Build the global-add command for a JS package manager that *isn't* npm,
/// reusing the same package spec. Returns `None` for npm (no change needed) and
/// for non-JS methods (where the package spec doesn't transfer).
fn js_pm_global_add(method: InstallMethod, pkg: &str) -> Option<Vec<String>> {
    let s = |a: &str| a.to_string();
    match method {
        InstallMethod::Pnpm => Some(vec![s("pnpm"), s("add"), s("-g"), s(pkg)]),
        InstallMethod::Yarn => Some(vec![s("yarn"), s("global"), s("add"), s(pkg)]),
        InstallMethod::Bun => Some(vec![s("bun"), s("add"), s("-g"), s(pkg)]),
        // npm: command already correct. Non-JS methods: not transferable.
        _ => None,
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
    fn infer_install_method_recognizes_common_locations() {
        use InstallMethod::*;
        assert_eq!(
            infer_install_method("/opt/homebrew/bin/gemini"),
            Some(Homebrew)
        );
        assert_eq!(
            infer_install_method("/usr/local/Cellar/node/bin/gemini"),
            Some(Homebrew)
        );
        assert_eq!(infer_install_method("/home/u/.bun/bin/gemini"), Some(Bun));
        assert_eq!(
            infer_install_method("/home/u/.local/share/pnpm/gemini"),
            Some(Pnpm)
        );
        assert_eq!(
            infer_install_method("/usr/lib/node_modules/.bin/gemini"),
            Some(Npm)
        );
        assert_eq!(
            infer_install_method("/home/u/.local/share/uv/tools/kimi-cli/bin/kimi"),
            Some(Uv)
        );
        // Ambiguous native-installer shim → no confident method.
        assert_eq!(infer_install_method("/home/u/.local/bin/claude"), None);
    }

    #[test]
    fn resolved_update_command_swaps_npm_to_owning_js_pm() {
        // gemini installed via bun → run `bun add -g <pkg>`, not npm.
        let e = entry_with(
            Some("gemini"),
            &["npm", "install", "-g", "@google/gemini-cli@latest"],
            Some("/home/u/.bun/bin/gemini"),
        );
        assert_eq!(
            e.resolved_update_command().unwrap(),
            vec!["bun", "add", "-g", "@google/gemini-cli@latest"]
        );
    }

    #[test]
    fn resolved_update_command_keeps_npm_for_npm_and_unknown_installs() {
        // npm install → keep npm.
        let npm = entry_with(
            Some("gemini"),
            &["npm", "install", "-g", "@google/gemini-cli@latest"],
            Some("/usr/lib/node_modules/.bin/gemini"),
        );
        assert_eq!(npm.resolved_update_command().unwrap()[0], "npm");
        // Homebrew install (can't transfer the npm package spec to a brew formula)
        // → keep the registry npm command as the best available action.
        let brew = entry_with(
            Some("gemini"),
            &["npm", "install", "-g", "@google/gemini-cli@latest"],
            Some("/opt/homebrew/bin/gemini"),
        );
        assert_eq!(brew.resolved_update_command().unwrap()[0], "npm");
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
