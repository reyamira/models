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

/// Installed CLI info: version + resolved binary path, plus install-method facts
/// derived at detection time. Storing the method/package/helper here (rather than
/// recomputing on demand) keeps `resolved_update_command` pure — all the I/O
/// (canonicalize, ownership query, helper lookup) happens once in `detect_installed`.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct InstalledInfo {
    pub version: Option<String>,
    pub path: Option<String>,
    /// How the binary was installed, resolved at detect time — a path heuristic for
    /// language package managers, or a system-package ownership query for
    /// `/usr/bin`-style installs. `None` when unrecognized.
    pub method: Option<InstallMethod>,
    /// The package/formula/tool/owner name the update command targets (npm package,
    /// Homebrew formula, uv/pipx tool, or system-package owner). `None` when the
    /// path doesn't encode it (e.g. a bun wrapper script) or an AUR package has no
    /// helper to update it.
    pub package: Option<String>,
    /// Preferred AUR helper (`paru`/`yay`) when `method` is `Pacman`; else `None`.
    pub aur_helper: Option<String>,
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

    /// The install method resolved at detect time. Falls back to a pure path
    /// heuristic when no method was stored (e.g. unit tests that build
    /// `InstalledInfo` directly), so callers get a best effort either way.
    pub fn install_method(&self) -> Option<InstallMethod> {
        self.installed.method.or_else(|| {
            self.installed
                .path
                .as_deref()
                .and_then(infer_install_method)
        })
    }

    /// The update argv to actually run, derived from how the binary was installed.
    /// Detection is the source of truth; the registry `update_command` is the
    /// fallback. Pure — all I/O (canonicalize, ownership query, helper lookup)
    /// happened at detect time and is read from `installed.method/.package/.aur_helper`.
    ///
    /// Priority:
    /// 1. **System-package-managed** (Pacman/Apt/Dnf) → the distro command. This
    ///    precedes a self-updater: a self-updater run against a package-manager-owned
    ///    binary either fails on root-owned files or silently desyncs the package DB,
    ///    so we never fall back to it here (a missing package → no update, not the
    ///    self-updater).
    /// 2. **Self-updater** (registry argv[0] is the agent's own binary, e.g.
    ///    `claude update`) → pin argv[0] to the detected absolute path.
    /// 3. **Registry package-manager command** → adapt to the detected method:
    ///    Homebrew → `brew upgrade <formula>`; an `npm install -g` whose binary was
    ///    actually installed by bun/pnpm/yarn → swap to that manager (portable spec).
    /// 4. **No registry command** but a detected language package manager + a derived
    ///    package → build the command (custom agents added in-app).
    /// 5. Otherwise → `None`.
    pub fn resolved_update_command(&self) -> Option<Vec<String>> {
        let method = self.install_method();
        let package = self.installed.package.as_deref();
        let helper = self.installed.aur_helper.as_deref();

        // (1) A system package manager owns the binary → only its command is safe.
        // Never fall through to a self-updater that would fight the package DB.
        if let Some(m @ (InstallMethod::Pacman | InstallMethod::Apt | InstallMethod::Dnf)) = method
        {
            return package.and_then(|pkg| derive_pm_command(m, pkg, helper));
        }

        if let Some(cmd) = self.update_command() {
            let mut cmd = cmd.to_vec();

            // (2) Self-updater → pin argv[0] to the detected path.
            if let (Some(bin), Some(path)) = (
                self.agent.cli_binary.as_deref(),
                self.installed.path.as_deref(),
            ) {
                if cmd.first().is_some_and(|first| first == bin) {
                    cmd[0] = path.to_string();
                    return Some(cmd);
                }
            }

            // (3a) Homebrew → `brew upgrade <formula>` (formula derived at detect
            // time), since the registry's npm/uv command would touch a different copy.
            if method == Some(InstallMethod::Homebrew) {
                if let Some(formula) = package {
                    return Some(vec![
                        "brew".to_string(),
                        "upgrade".to_string(),
                        formula.to_string(),
                    ]);
                }
            }

            // (3b) npm install but a different JS package manager owns it → swap to
            // it, keeping the portable package spec.
            if let Some(pkg) = npm_global_package(&cmd) {
                if let Some(swapped) = method.and_then(|m| js_pm_global_add(m, &pkg)) {
                    return Some(swapped);
                }
            }

            return Some(cmd);
        }

        // (4) No registry command but a detected language package manager + a derived
        // package → build the command (custom agents get no registry updater).
        if let (Some(m), Some(pkg)) = (method, package) {
            return derive_pm_command(m, pkg, helper);
        }

        // (5) Nothing to run.
        None
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

/// How an installed CLI was put on disk. Language package managers are inferred
/// from the detected path; the system variants (Pacman/Apt/Dnf) come from a
/// distro ownership query. Used to make the update command target the copy the
/// user is actually running.
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
    Pacman,
    Apt,
    Dnf,
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
            InstallMethod::Pacman => "pacman",
            InstallMethod::Apt => "apt",
            InstallMethod::Dnf => "dnf",
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

/// Extract the Homebrew formula name from a resolved Cellar path
/// (`…/Cellar/<formula>/<version>/…` → `<formula>`). Pure string parsing.
fn formula_from_cellar_path(resolved: &str) -> Option<String> {
    let idx = resolved.find("/Cellar/")?;
    let after = &resolved[idx + "/Cellar/".len()..];
    let formula = after.split('/').next()?;
    (!formula.is_empty()).then(|| formula.to_string())
}

/// The package name an update targets, parsed from a *canonicalized* binary path
/// (symlinks already resolved). Reads the directory segment — so package-name ≠
/// binary-name is handled for free (`Cellar/gemini-cli/` → `gemini-cli` even though
/// the binary is `gemini`). Uses the LAST `node_modules/` occurrence to skip pnpm's
/// `<pkg>@<ver>` store prefix, and joins a scoped `@scope/name`. Returns `None` when
/// the path doesn't encode a package (a bun/pnpm wrapper script, or a cargo bin
/// whose crate name isn't in the path) — that agent then has no derived updater.
pub fn package_from_canonical_path(path: &str, method: InstallMethod) -> Option<String> {
    match method {
        InstallMethod::Npm | InstallMethod::Pnpm | InstallMethod::Yarn | InstallMethod::Bun => {
            let idx = path.rfind("/node_modules/")?;
            let after = &path[idx + "/node_modules/".len()..];
            let mut segs = after.split('/').filter(|s| !s.is_empty());
            let first = segs.next()?;
            if first.starts_with('@') {
                // Scoped package: `@scope/name`.
                let name = segs.next()?;
                Some(format!("{first}/{name}"))
            } else {
                Some(first.to_string())
            }
        }
        InstallMethod::Uv => segment_after(path, "/uv/tools/"),
        InstallMethod::Pipx => segment_after(path, "/pipx/venvs/"),
        InstallMethod::Homebrew => formula_from_cellar_path(path),
        // `~/.cargo/bin/<bin>` doesn't encode the crate name; system PMs get their
        // package from the ownership query, not the path.
        InstallMethod::Cargo | InstallMethod::Pacman | InstallMethod::Apt | InstallMethod::Dnf => {
            None
        }
    }
}

/// The path segment immediately after `marker` (up to the next `/`), non-empty.
fn segment_after(path: &str, marker: &str) -> Option<String> {
    let idx = path.find(marker)?;
    path[idx + marker.len()..]
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Build the upgrade argv for a resolved install method + package. Pure: the AUR
/// helper is injected (resolved at detect time). Returns `None` when no safe
/// command exists — the caller (`detect`) only sets `Pacman` with a package when a
/// working command is possible (a foreign/AUR package with no helper is left
/// package-less, so we never emit a `sudo pacman -S <aur-pkg>` that fails with
/// "target not found").
pub fn derive_pm_command(
    method: InstallMethod,
    package: &str,
    aur_helper: Option<&str>,
) -> Option<Vec<String>> {
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let spec = format!("{package}@latest");
    Some(match method {
        InstallMethod::Npm => v(&["npm", "install", "-g", spec.as_str()]),
        InstallMethod::Pnpm => v(&["pnpm", "add", "-g", spec.as_str()]),
        InstallMethod::Yarn => v(&["yarn", "global", "add", spec.as_str()]),
        InstallMethod::Bun => v(&["bun", "add", "-g", spec.as_str()]),
        InstallMethod::Homebrew => v(&["brew", "upgrade", package]),
        InstallMethod::Uv => v(&["uv", "tool", "upgrade", package]),
        InstallMethod::Pipx => v(&["pipx", "upgrade", package]),
        InstallMethod::Cargo => v(&["cargo", "install", package]),
        InstallMethod::Pacman => match aur_helper {
            Some(h) => v(&[h, "-S", package]),
            // No AUR helper → only reached for an official-repo package (detect leaves
            // foreign packages package-less). Single-package `-S` is the Arch
            // partial-upgrade antipattern but pragmatic for a one-tool update.
            None => v(&["sudo", "pacman", "-S", package]),
        },
        InstallMethod::Apt => v(&["sudo", "apt", "install", "--only-upgrade", package]),
        InstallMethod::Dnf => v(&["sudo", "dnf", "upgrade", package]),
    })
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

    fn entry_with_installed(
        cli_binary: Option<&str>,
        update_command: &[&str],
        installed: InstalledInfo,
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
            installed,
            tracked: true,
            fetch_status: FetchStatus::Loaded,
        }
    }

    /// Convenience: an entry whose install method is *inferred from the path* (no
    /// stored method), matching how the pre-detection unit tests exercised the
    /// path-heuristic fallback in `resolved_update_command`.
    fn entry_with(
        cli_binary: Option<&str>,
        update_command: &[&str],
        installed_path: Option<&str>,
    ) -> AgentEntry {
        entry_with_installed(
            cli_binary,
            update_command,
            InstalledInfo {
                version: installed_path.map(|_| "1.0.0".to_string()),
                path: installed_path.map(str::to_string),
                ..Default::default()
            },
        )
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
        // Homebrew path but no stored formula (detection didn't run in this test) →
        // the brew branch has nothing to build, so fall back to the registry npm cmd.
        let brew = entry_with(
            Some("gemini"),
            &["npm", "install", "-g", "@google/gemini-cli@latest"],
            Some("/opt/homebrew/bin/gemini"),
        );
        assert_eq!(brew.resolved_update_command().unwrap()[0], "npm");
    }

    #[test]
    fn resolved_update_command_system_pm_preempts_self_updater() {
        // opencode ships a self-updater (`opencode upgrade`), but an AUR-managed
        // install must go through the helper — the self-updater would fight pacman.
        let e = entry_with_installed(
            Some("opencode"),
            &["opencode", "upgrade"],
            InstalledInfo {
                version: Some("1.17.11".to_string()),
                path: Some("/usr/bin/opencode".to_string()),
                method: Some(InstallMethod::Pacman),
                package: Some("opencode-bin".to_string()),
                aur_helper: Some("paru".to_string()),
            },
        );
        assert_eq!(
            e.resolved_update_command().unwrap(),
            vec!["paru", "-S", "opencode-bin"]
        );
    }

    #[test]
    fn resolved_update_command_system_pm_without_package_has_no_updater() {
        // Foreign/AUR package with no helper: detect leaves `package` unset. We must
        // NOT fall back to the self-updater (it would desync the package DB).
        let e = entry_with_installed(
            Some("opencode"),
            &["opencode", "upgrade"],
            InstalledInfo {
                version: Some("1.17.11".to_string()),
                path: Some("/usr/bin/opencode".to_string()),
                method: Some(InstallMethod::Pacman),
                package: None,
                aur_helper: None,
            },
        );
        assert_eq!(e.resolved_update_command(), None);
    }

    #[test]
    fn resolved_update_command_derives_for_custom_agent_without_registry_cmd() {
        // Custom agent (empty registry update_command) installed via uv → derive it.
        let e = entry_with_installed(
            None,
            &[],
            InstalledInfo {
                version: Some("0.1.0".to_string()),
                path: Some("/home/u/.local/share/uv/tools/foo-cli/bin/foo".to_string()),
                method: Some(InstallMethod::Uv),
                package: Some("foo-cli".to_string()),
                aur_helper: None,
            },
        );
        assert_eq!(
            e.resolved_update_command().unwrap(),
            vec!["uv", "tool", "upgrade", "foo-cli"]
        );
    }

    #[test]
    fn resolved_update_command_none_when_custom_agent_has_no_derived_package() {
        // Custom agent, language PM detected but no package parsed (e.g. a bun
        // wrapper) → no updater rather than a wrong one.
        let e = entry_with_installed(
            None,
            &[],
            InstalledInfo {
                version: Some("0.1.0".to_string()),
                path: Some("/home/u/.bun/bin/foo".to_string()),
                method: Some(InstallMethod::Bun),
                package: None,
                aur_helper: None,
            },
        );
        assert_eq!(e.resolved_update_command(), None);
    }

    #[test]
    fn package_from_canonical_path_parses_pm_layouts() {
        use InstallMethod::*;
        assert_eq!(
            package_from_canonical_path(
                "/usr/lib/node_modules/@google/gemini-cli/dist/index.js",
                Npm
            )
            .as_deref(),
            Some("@google/gemini-cli")
        );
        assert_eq!(
            package_from_canonical_path(
                "/home/u/.bun/install/global/node_modules/eve/bin/eve.js",
                Bun
            )
            .as_deref(),
            Some("eve")
        );
        // pnpm store nests `<pkg>@<ver>/node_modules/<pkg>` — take the LAST occurrence.
        assert_eq!(
            package_from_canonical_path(
                "/home/u/.local/share/pnpm/.pnpm/opencode@1.2.3/node_modules/opencode/bin/x",
                Pnpm
            )
            .as_deref(),
            Some("opencode")
        );
        assert_eq!(
            package_from_canonical_path("/home/u/.local/share/uv/tools/kimi-cli/bin/kimi", Uv)
                .as_deref(),
            Some("kimi-cli")
        );
        assert_eq!(
            package_from_canonical_path("/home/u/.local/pipx/venvs/foo/bin/foo", Pipx).as_deref(),
            Some("foo")
        );
        assert_eq!(
            package_from_canonical_path(
                "/opt/homebrew/Cellar/gemini-cli/0.46.0/bin/gemini",
                Homebrew
            )
            .as_deref(),
            Some("gemini-cli")
        );
        // A bun wrapper script (no node_modules in the path) → no package.
        assert_eq!(
            package_from_canonical_path("/home/u/.bun/bin/foo", Bun),
            None
        );
        // cargo bins don't encode the crate name.
        assert_eq!(
            package_from_canonical_path("/home/u/.cargo/bin/foo", Cargo),
            None
        );
    }

    #[test]
    fn derive_pm_command_builds_expected_argv() {
        use InstallMethod::*;
        assert_eq!(
            derive_pm_command(Bun, "eve", None).unwrap(),
            vec!["bun", "add", "-g", "eve@latest"]
        );
        assert_eq!(
            derive_pm_command(Npm, "@google/gemini-cli", None).unwrap(),
            vec!["npm", "install", "-g", "@google/gemini-cli@latest"]
        );
        assert_eq!(
            derive_pm_command(Uv, "kimi-cli", None).unwrap(),
            vec!["uv", "tool", "upgrade", "kimi-cli"]
        );
        assert_eq!(
            derive_pm_command(Homebrew, "gemini-cli", None).unwrap(),
            vec!["brew", "upgrade", "gemini-cli"]
        );
        // Pacman with a helper → helper -S; without one → sudo pacman -S (official).
        assert_eq!(
            derive_pm_command(Pacman, "opencode-bin", Some("paru")).unwrap(),
            vec!["paru", "-S", "opencode-bin"]
        );
        assert_eq!(
            derive_pm_command(Pacman, "ripgrep", None).unwrap(),
            vec!["sudo", "pacman", "-S", "ripgrep"]
        );
        assert_eq!(
            derive_pm_command(Apt, "foo", None).unwrap(),
            vec!["sudo", "apt", "install", "--only-upgrade", "foo"]
        );
        assert_eq!(
            derive_pm_command(Dnf, "foo", None).unwrap(),
            vec!["sudo", "dnf", "upgrade", "foo"]
        );
    }

    #[test]
    fn formula_from_cellar_path_extracts_formula() {
        assert_eq!(
            formula_from_cellar_path(
                "/home/linuxbrew/.linuxbrew/Cellar/gemini-cli/0.46.0/libexec/bin/gemini"
            )
            .as_deref(),
            Some("gemini-cli")
        );
        assert_eq!(
            formula_from_cellar_path("/opt/homebrew/Cellar/node/22.1.0/bin/node").as_deref(),
            Some("node")
        );
        // No Cellar segment → no formula.
        assert_eq!(formula_from_cellar_path("/usr/local/bin/gemini"), None);
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
