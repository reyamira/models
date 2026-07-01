# Agents Module — Claude Code Instructions

## Module Purpose
Tracks AI coding assistants (CLI tools, IDEs, plugins) with GitHub metadata, local installation detection, and changelog parsing. Powers both TUI (Agents tab) and CLI (`models agents` commands).

## Key Types

| Type | File | Purpose |
|------|------|---------|
| `Agent` | `data.rs` | Static metadata from `data/agents.json` (name, repo, categories, pricing, binaries, version detection, `update_command` self-updater argv) |
| `GitHubData` | `data.rs` | Runtime: releases, stars, issues, license, commit date. Methods: `latest_version()`, `release_frequency()`, `latest_release_date()` |
| `AgentEntry` | `data.rs` | Combined entry: Agent + GitHubData + InstalledInfo + tracked flag. Methods: `update_available()`, `new_releases()`, `latest_release_relative_time()` |
| `ChangelogBlock` | `changelog_parser.rs` | Normalized IR: `Heading(String)` \| `Bullet(String)` \| `Paragraph(String)`. Used by both CLI (`agents.rs`) and TUI preview panes |
| `Changelog` | `changelog_parser.rs` | Flat list of ChangelogBlock. Produced by `parse_changelog()` (comrak AST → IR) |
| `AgentServiceMapping` | `health.rs` | Maps agent IDs to status provider slugs and component names for service health display |
| `ResolvedHealth` | `health.rs` | Resolved health with provider name and optional component name |

## Data Flow

- **Load**: `loader.rs` — loads embedded `data/agents.json` via `include_str!`
- **Detect**: `detect.rs` — runs version commands (e.g., `claude --version`) to find installed binaries + versions
- **Update**: **detection-first** — `AgentEntry::resolved_update_command` (pure) derives the argv from how the binary was installed, stored at detect time on `InstalledInfo` (`method`/`package`/`aur_helper`); the registry `update_command` is the fallback. System-package-managed installs (`Pacman`/`Apt`/`Dnf`, from the `detect.rs` ownership query) take precedence over a self-updater; custom agents with no registry command get a command derived from the path (`derive_pm_command`). The resolved argv is run as a background subprocess by `spawn_agent_update` (`tui/mod.rs`) — or, for sudo/AUR (`command_needs_terminal`), the interactive suspend-and-run path — streamed over an `mpsc<UpdateEvent>` channel, then `detect_installed` re-runs to refresh the version. TUI-only (the `u`/`U` keys); see `.claude/rules/tui-agents-tab.md` §7c
- **Detect**: `detect.rs::detect_installed` runs the version command + `which` for the path, then `resolve_install_facts` canonicalizes it, infers the language-PM method (`infer_install_method`) + package (`package_from_canonical_path`), and — only for an unrecognized binary in a system dir — runs a `/etc/os-release`-gated ownership query (`pacman -Qo`/`dpkg -S`/`rpm -qf`, 2s timeout). `detect_installed_cli` skips that subprocess (the CLI never updates)
- **Fetch**: `github.rs` — 2-API-call flow for TUI, 1-call for CLI (releases only). ETag conditional. Spawned in background, results via mpsc Message
- **Cache**: `cache.rs` — disk cache with version sentinel (v1). Reads/writes via `load_cache()`/`save_cache()`. Path: `~/.local/share/modelsdev/github-cache.json` on Unix
- **Parse**: `changelog_parser.rs` — comrak (CommonMark/GFM) → AST → normalized IR. Skips boilerplate headers ("What's Changed", "Changelog"). Flattens nested lists

## Key Patterns

- **`comrak` only in CLI**: changelog_parser imports comrak but TUI uses regex-based markdown converter in `src/tui/markdown.rs`
- **Semver comparison**: `AgentEntry::update_available()` tries `semver::Version::parse()`, falls back to string equality
- **Release frequency**: calculates from last N release dates (e.g., "~1w"). Format: "just now" / "Nd ago" / "~Nw" / "~Nm"
- **Version detection**: each Agent has `cli_binary`, `alt_binaries`, `version_command` array, optional `version_regex`. Detection spawned in background
- **Changelog skip filter**: "What's Changed", "Changelog", "Full Changelog" headers are content wrappers, not sections — skipped by `is_skip_header()`

## Gotchas

- **Gemini releases**: flat PR list under `## What's Changed` with no sub-headers — this is their actual format, not a parser bug
- **Named lifetimes**: comrak AST uses arena, requires `fn f<'a>(node: &'a AstNode<'a>)` — `'_` won't work due to invariance
- **CLI vs TUI fetch modes**: CLI uses `fetch_releases_only()` (1 call, no repo metadata), TUI uses `fetch_conditional()` (2 calls, includes stars/issues/license)
- **Cache path**: uses `dirs::data_local_dir()` — ensure it exists before write; cache.rs creates parent dirs
- **ETag handling**: GitHub returns 304 Not Modified on conditional fetch. Code merges new releases with cached stars/issues/license
