use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::data::{
    infer_install_method, package_from_canonical_path, Agent, InstallMethod, InstalledInfo,
};

/// Max wall-clock for a system-package ownership subprocess (`pacman -Qo` etc.).
/// Detection runs off the UI thread, but a wedged package DB shouldn't hang it.
const OWNERSHIP_TIMEOUT: Duration = Duration::from_secs(2);

pub fn detect_installed(agent: &Agent) -> InstalledInfo {
    detect_inner(agent, true)
}

/// Like [`detect_installed`] but skips the system-package ownership probe. The CLI
/// (`models agents`) never runs updates, so that subprocess would be pure cost.
pub fn detect_installed_cli(agent: &Agent) -> InstalledInfo {
    detect_inner(agent, false)
}

fn detect_inner(agent: &Agent, probe_system_pm: bool) -> InstalledInfo {
    // Detect any agent with a cli_binary (CLI tools, IDEs with launchers, etc.)
    let binary = match &agent.cli_binary {
        Some(b) => b,
        None => return InstalledInfo::default(),
    };

    // Try primary binary first, then fall back to alt_binaries
    // (e.g., "zed" on macOS vs "zeditor" on Arch Linux)
    let binaries_to_try =
        std::iter::once(binary.as_str()).chain(agent.alt_binaries.iter().map(|s| s.as_str()));

    for bin in binaries_to_try {
        let (version, path) =
            get_version_and_path(bin, &agent.version_command, agent.version_regex.as_deref());
        if version.is_some() || path.is_some() {
            let (method, package, aur_helper) =
                resolve_install_facts(path.as_deref(), probe_system_pm);
            return InstalledInfo {
                version,
                path,
                method,
                package,
                aur_helper,
            };
        }
    }

    InstalledInfo::default()
}

/// Resolve how a binary was installed and what package/formula the update targets.
/// Canonicalizes the path (so a brew bin symlink resolves into its Cellar, and an
/// npm shim into `node_modules`) before the path heuristic, then — only for an
/// unrecognized binary in a system dir — asks the OS package manager who owns it.
fn resolve_install_facts(
    path: Option<&str>,
    probe_system_pm: bool,
) -> (Option<InstallMethod>, Option<String>, Option<String>) {
    let path = match path {
        Some(p) => p,
        None => return (None, None, None),
    };
    let canon = std::fs::canonicalize(path).ok();
    let resolved = canon.as_deref().and_then(|p| p.to_str()).unwrap_or(path);

    if let Some(method) = infer_install_method(resolved) {
        let package = package_from_canonical_path(resolved, method);
        return (Some(method), package, None);
    }

    // Unrecognized language-PM path. If it's a system location, ask the distro
    // package manager who owns it (Arch/AUR, Debian, Fedora).
    if probe_system_pm && is_system_dir(resolved) {
        if let Some(facts) = system_package_owner(resolved) {
            return facts;
        }
    }
    (None, None, None)
}

/// Whether a path lives in a distro-managed binary directory (vs a user/language-PM
/// prefix like `~/.local/bin` or `/opt/homebrew`).
fn is_system_dir(path: &str) -> bool {
    const SYSTEM_DIRS: [&str; 5] = [
        "/usr/bin/",
        "/bin/",
        "/usr/local/bin/",
        "/usr/sbin/",
        "/sbin/",
    ];
    SYSTEM_DIRS.iter().any(|d| path.starts_with(d))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DistroFamily {
    Arch,
    Debian,
    Rhel,
}

/// Classify the running distro from `/etc/os-release` (`ID` + `ID_LIKE`). We pick
/// the ownership tool by *family*, never by "which `pacman` is on PATH" — on Debian
/// a `pacman` binary is the arcade game (`/usr/games/pacman`), not the Arch PM.
fn distro_family() -> Option<DistroFamily> {
    let os = std::fs::read_to_string("/etc/os-release").ok()?;
    let mut ids = String::new();
    for line in os.lines() {
        if let Some(v) = line
            .strip_prefix("ID=")
            .or_else(|| line.strip_prefix("ID_LIKE="))
        {
            ids.push(' ');
            ids.push_str(v.trim().trim_matches('"'));
        }
    }
    classify_distro(&ids)
}

/// Map the whitespace-joined `ID`/`ID_LIKE` tokens to a package-manager family.
fn classify_distro(ids: &str) -> Option<DistroFamily> {
    let has = |kw: &str| ids.split_whitespace().any(|t| t == kw);
    if has("arch") || has("manjaro") || has("endeavouros") {
        Some(DistroFamily::Arch)
    } else if has("debian") || has("ubuntu") {
        Some(DistroFamily::Debian)
    } else if has("fedora") || has("rhel") || has("centos") {
        Some(DistroFamily::Rhel)
    } else {
        None
    }
}

/// Ask the system package manager which package owns `path`. Returns the install
/// method, the owning package (`None` for a foreign/AUR package with no helper —
/// non-auto-updatable), and the AUR helper (Pacman only).
fn system_package_owner(
    path: &str,
) -> Option<(Option<InstallMethod>, Option<String>, Option<String>)> {
    match distro_family()? {
        DistroFamily::Arch => {
            let out = run_with_timeout(pm_command("pacman", &["-Qo", path]), OWNERSHIP_TIMEOUT)?;
            if !out.status.success() {
                return None;
            }
            let owner = parse_pacman_owner(&String::from_utf8_lossy(&out.stdout))?;
            let helper = detect_aur_helper();
            if pacman_is_foreign(&owner) && helper.is_none() {
                // AUR package but no helper installed → surface pacman as the method
                // (so the UI shows it's distro-managed) but no command to run.
                return Some((Some(InstallMethod::Pacman), None, None));
            }
            Some((Some(InstallMethod::Pacman), Some(owner), helper))
        }
        DistroFamily::Debian => {
            let out = run_with_timeout(pm_command("dpkg", &["-S", path]), OWNERSHIP_TIMEOUT)?;
            if !out.status.success() {
                return None;
            }
            // "pkgname: /usr/bin/foo" (pkgname may carry an `:arch` suffix).
            let stdout = String::from_utf8_lossy(&out.stdout);
            let owner = stdout.split(':').next()?.trim();
            (!owner.is_empty()).then(|| (Some(InstallMethod::Apt), Some(owner.to_string()), None))
        }
        DistroFamily::Rhel => {
            let out = run_with_timeout(
                pm_command("rpm", &["-qf", "--queryformat", "%{NAME}", path]),
                OWNERSHIP_TIMEOUT,
            )?;
            if !out.status.success() {
                return None;
            }
            let owner = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!owner.is_empty()).then_some((Some(InstallMethod::Dnf), Some(owner), None))
        }
    }
}

/// Parse `pacman -Qo` output: "/usr/bin/opencode is owned by opencode-bin 1.17.11-1".
fn parse_pacman_owner(stdout: &str) -> Option<String> {
    const MARKER: &str = " is owned by ";
    let idx = stdout.find(MARKER)?;
    stdout[idx + MARKER.len()..]
        .split_whitespace()
        .next()
        .map(str::to_string)
}

/// Whether `pkg` is a foreign (AUR / manually-installed) package — `pacman -Qm`
/// lists "name version" per line. Such packages need an AUR helper, not `pacman -S`.
fn pacman_is_foreign(pkg: &str) -> bool {
    match run_with_timeout(pm_command("pacman", &["-Qm"]), OWNERSHIP_TIMEOUT) {
        Some(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.split_whitespace().next() == Some(pkg)),
        _ => false,
    }
}

/// The preferred installed AUR helper (`paru` then `yay`), if any.
fn detect_aur_helper() -> Option<String> {
    ["paru", "yay"]
        .into_iter()
        .find(|h| which_binary(h).is_some())
        .map(str::to_string)
}

fn pm_command(program: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args);
    c
}

/// Run a read-only command with a wall-clock cap (std has no built-in timeout).
/// Polls `try_wait`; kills and reaps on timeout so a wedged package tool can't hang
/// a detect thread.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<std::process::Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {}
            Err(_) => return None,
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn which_binary(name: &str) -> Option<PathBuf> {
    let output = Command::new("which").arg(name).output().ok()?;

    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path_str.is_empty() {
            return Some(PathBuf::from(path_str));
        }
    }

    None
}

/// Get version and path in one operation - avoids separate `which` call
fn get_version_and_path(
    binary: &str,
    version_cmd: &[String],
    version_regex: Option<&str>,
) -> (Option<String>, Option<String>) {
    if version_cmd.is_empty() {
        return (None, None);
    }

    // Try to run the version command - if it works, the binary exists
    let output = match Command::new(binary).args(version_cmd).output() {
        Ok(o) => o,
        Err(_) => return (None, None), // Binary not found or not executable
    };

    let output_str = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        // Some tools output version to stderr
        String::from_utf8_lossy(&output.stderr).to_string()
    };

    let version = extract_version(&output_str, version_regex);

    // Only look up path if we found a version (binary definitely exists)
    let path = if version.is_some() {
        which_binary(binary).map(|p| p.to_string_lossy().to_string())
    } else {
        None
    };

    (version, path)
}

fn extract_version(output: &str, regex_pattern: Option<&str>) -> Option<String> {
    if let Some(pattern) = regex_pattern {
        // Use the provided regex pattern — expects a capture group for the version
        if let Ok(re) = regex::Regex::new(pattern) {
            for line in output.lines() {
                if let Some(captures) = re.captures(line) {
                    // Return first capture group if present, otherwise full match
                    let version = captures
                        .get(1)
                        .or_else(|| captures.get(0))
                        .map(|m| m.as_str().to_string());
                    if version.is_some() {
                        return version;
                    }
                }
            }
        }
    }

    // Default: find X.Y.Z semver pattern
    let re = regex::Regex::new(r"(\d+\.\d+\.\d+)").ok()?;
    for line in output.lines() {
        if let Some(captures) = re.captures(line) {
            if let Some(m) = captures.get(1) {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pacman_owner_extracts_package() {
        assert_eq!(
            parse_pacman_owner("/usr/bin/opencode is owned by opencode-bin 1.17.11-1").as_deref(),
            Some("opencode-bin")
        );
        assert_eq!(
            parse_pacman_owner("error: No package owns /usr/bin/foo"),
            None
        );
    }

    #[test]
    fn is_system_dir_recognizes_managed_locations() {
        assert!(is_system_dir("/usr/bin/opencode"));
        assert!(is_system_dir("/usr/local/bin/foo"));
        assert!(!is_system_dir("/home/u/.bun/bin/eve"));
        assert!(!is_system_dir("/opt/homebrew/bin/gemini"));
    }

    #[test]
    fn classify_distro_matches_family_by_id_and_id_like() {
        assert_eq!(classify_distro(" arch"), Some(DistroFamily::Arch));
        // Manjaro: ID=manjaro ID_LIKE=arch.
        assert_eq!(classify_distro(" manjaro arch"), Some(DistroFamily::Arch));
        assert_eq!(
            classify_distro(" ubuntu debian"),
            Some(DistroFamily::Debian)
        );
        assert_eq!(classify_distro(" fedora"), Some(DistroFamily::Rhel));
        assert_eq!(classify_distro(" void"), None);
    }

    #[test]
    fn test_extract_semver() {
        assert_eq!(
            extract_version("claude-code v1.0.30", None),
            Some("1.0.30".to_string())
        );
        assert_eq!(
            extract_version("opencode v0.82.1", None),
            Some("0.82.1".to_string())
        );
        assert_eq!(
            extract_version("Version: 2.3.4-beta", None),
            Some("2.3.4".to_string())
        );
    }

    #[test]
    fn test_no_version() {
        assert_eq!(extract_version("no version here", None), None);
        assert_eq!(extract_version("1.2", None), None); // Not enough parts
    }

    #[test]
    fn test_custom_version_regex_returns_capture_group() {
        assert_eq!(
            extract_version(
                "release build release-12.34.56 (abcdef)",
                Some(r"release-(\d+\.\d+\.\d+)")
            ),
            Some("12.34.56".to_string())
        );
    }

    #[test]
    fn test_custom_version_regex_invalid_pattern_falls_back_to_semver() {
        assert_eq!(
            extract_version("tool version v9.8.7", Some(r"([invalid")),
            Some("9.8.7".to_string())
        );
    }

    #[test]
    fn test_custom_version_regex_non_match_falls_back_to_default_semver() {
        assert_eq!(
            extract_version("tool version v4.5.6", Some(r"version=(\d+\.\d+\.\d+)")),
            Some("4.5.6".to_string())
        );
    }
}
