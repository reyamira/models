# Models — Claude Code Instructions

## Project Overview
A Rust CLI/TUI for browsing AI models, benchmarks, and coding agents. Built with ratatui, crossterm, and tokio.

## Build & Test
```bash
mise run fmt        # Format code (required before commit)
mise run clippy     # Lint with -D warnings
mise run test       # Run tests
mise run build      # Build debug
mise run run        # Run the TUI
```

Always run the full check sequence before committing:
```bash
mise run fmt && mise run clippy && mise run test
```

## Architecture

### Tabs
- **Models Tab** (`src/tui/models/`) — browse models from models.dev API with 3-column layout (20% providers | 45% model list | 35% detail panel), RTFO capability indicators, adaptive provider panel
- **Benchmarks Tab** (`src/tui/benchmarks/`) — compare model benchmarks from Artificial Analysis with browse/compare modes, H2H table, scatter plot, radar chart views
- **Agents Tab** (`src/tui/agents/`) — track AI coding assistants with GitHub integration
- **Status Tab** (`src/tui/status/`) — live provider health monitoring with detail view for incidents, components, and scheduled maintenance

### Data Flow
- Model data: fetched from models.dev API at startup (`src/api.rs`)
- Benchmark data: fetched fresh from jsDelivr CDN on every launch (`src/benchmarks/fetch.rs`)
- Agent/GitHub data: disk-cached with ETag conditional fetching (`src/agents/cache.rs`, `src/agents/github.rs`)
- CLI agents: uses `fetch_releases_only` (1 API call, no repo metadata) — TUI uses full `fetch_conditional` (2 calls, includes stars/issues/license)
- Status data: fetched from each provider's official status page (Statuspage, BetterStack, Instatus, etc.) with apistatuscheck.com as fallback (`src/status/fetch.rs`), provider registry and strategy mapping in `src/status/registry.rs`
- Status source contract and normalization rules are documented in code comments within `src/status/` adapters

### Async Pattern
Background fetches use tokio::spawn + mpsc channels. Results arrive as `Message` variants processed in the main loop (`src/tui/mod.rs`). The app never blocks on network calls.

### Agents & CLI
See `src/agents/CLAUDE.md` and `src/cli/CLAUDE.md` for detailed module docs.
- Binary aliases: `models agents <cmd>` or `agents <cmd>` via argv[0] symlink detection. Alias names configurable via `[aliases]` in config.toml (defaults: `agents`, `benchmarks`, `mstatus`)
- Commands: `list`, `search`, `show`, `benchmarks`, `completions <shell>`, `link`, full agents suite (`status`, `latest`, `list-sources`, `<tool>`), full status suite (`list`, `show`, `status`, `sources`)
- CLI pickers use shared `PickerTerminal` infrastructure in `src/cli/picker.rs`

### Key Files

Each module has its own `CLAUDE.md` with detailed documentation. Top-level highlights:

- `src/formatting.rs` — shared utilities: `truncate`, `parse_date`, `format_tokens`, `format_stars`, `EM_DASH`, `cmp_opt_f64`
- `src/data.rs` — Provider/Model data structures from models.dev API
- `src/config.rs` — user config file (agents, cache, display, aliases settings). `AliasesConfig` struct + `AliasKind` enum for symlink routing
- `src/provider_category.rs` — provider categorization logic
- `src/benchmarks/` — `store.rs` (BenchmarkStore/Entry), `fetch.rs` (CDN fetcher), `traits.rs` (AA↔models.dev matching)
- `src/status/` — `types.rs`, `registry.rs`, `assessment.rs`, `fetch.rs`, `adapters/` (per-source-family parsers)
- `src/tui/` — `app.rs` (App state, Message enum), `event.rs` (NavAction dedup), `ui.rs` (shared helpers), `markdown.rs`, `widgets/` (ScrollablePanel, SoftCard, ScrollOffset, ComparisonLegend), per-tab subdirs: `models/`, `agents/`, `benchmarks/` (includes `radar.rs`), `status/` — each with `app.rs` (sub-app state) + `render.rs` (tab rendering)
- `src/agents/health.rs` — agent-to-status-provider mapping for service health display in the Agents tab
- `src/cli/` — `picker.rs` (shared PickerTerminal, nav helpers, style constants), `models.rs`/`benchmarks.rs`/`agents_ui.rs`/`status.rs` (inline pickers), `styles.rs`

### GitHub Actions
- `ci.yml` — runs on PR/push: fmt check, clippy, test
- `build-with-nix.yml` — runs on PR/push/manual dispatch: `nix build .` then `nix flake check` across Linux, Linux ARM, and macOS. Magic Nix Cache uses GitHub Actions cache only; FlakeHub cache is disabled.
- `release.yml` — triggered by `v*` tags: builds 5 targets in parallel with Rust caching, packages .deb/.rpm via cargo-binstall (pinned versions), generates SHA256SUMS, publishes to crates.io, and updates AUR package. Homebrew Core updates are handled in `Homebrew/homebrew-core` by Homebrew automation/maintainers, not from this repo. Pre-release tags (containing `-`) skip publish/AUR and mark the GitHub release as prerelease. Scoop Extras handles Windows updates via its own autoupdate mechanism.
- `flakehub-publish-tagged.yml` — manual-only; dispatch with an existing `v<version>` tag if FlakeHub publishing is intentionally enabled. Do not assume GitHub flake availability requires FlakeHub.
- `update-benchmarks.yml` — `workflow_dispatch`-only; fetches AA API, commits if data changed. Triggered every 30 min (at `:17` and `:47` UTC) by the Cloudflare Worker in `infra/benchmark-trigger/` — the original GH `schedule:` cron was removed after proving unreliable under GitHub's cron throttling

## Conventions
- Use `mise run <task>` for all CLI operations — never run bare commands
- Keep clippy clean with `-D warnings`
- Enum-based message passing (no callbacks)
- No disk cache — benchmark data fetched fresh from CDN on every launch, empty store until CDN responds
- `BenchmarkEntry` must derive both `Serialize` and `Deserialize`
- New `BenchmarkEntry` fields require `#[serde(default)]`
- Status detail semantics use parallel `*_state` metadata on `ProviderStatus`; UI and assessment logic should use helper methods instead of inferring meaning from empty vectors

## Gotchas
- clippy `-D warnings` treats unused enum variant fields as errors — if a Message variant's payload is only passed through (e.g., error strings logged nowhere), use a unit variant instead
- `Cargo.lock` must be committed after `Cargo.toml` version bumps
- GitHub Actions `workflow_dispatch` only works when the workflow file exists on the default branch — cannot test from feature branches
- Adding a new field to `BenchmarkEntry`: add field with `#[serde(default)]` — no cache versioning needed since data is fetched fresh every launch
- The AA API uses `0` as a sentinel for missing performance data — jq transforms must convert `0` → `null` (e.g., `if . == 0 then null else . end`)
- jq transforms use null-safe access (`?.` / `// null`) for nested objects — `mise.toml` and `update-benchmarks.yml` must stay in sync
- Never use `eprintln!` in TUI mode — stderr output corrupts ratatui's alternate screen buffer, causing rendering glitches. Use `Message` variants or status bar updates instead. (`eprintln!` is fine in CLI-only code paths like `src/cli/agents.rs`)
- Agents `GitHubData` fields `open_issues`, `license`, and `last_commit` are fetched/cached but never displayed in the UI — only `stars` (detail panel + sort) and `releases` are rendered. Kept for potential future use
- `Paragraph::scroll((y, 0))` with `.wrap(Wrap { trim: false })` counts **visual (wrapped) lines**, not logical lines — scroll positions must account for line wrapping when jumping to specific content
- Use `line.width()` (unicode-aware) not `.len()` (byte count) when computing wrapped line heights — ratatui wraps on display width, not byte length. Word-wrapping needs +1 buffer per wrapped line since `div_ceil` underestimates
- TLS uses `rustls-tls-native-roots` (not `rustls-tls`) — loads certificates from the OS trust store to support corporate TLS-inspecting proxies
- Status-source quirks to preserve: Better Stack resources use `public_name`; Status.io `status_code = 400` means degraded; incident.io incidents and Instatus components need second fetches; the Google adapter is currently summary-derived rather than preserving raw incident rows

## Website (`website/`)
Astro 6 + Tailwind 4 + TypeScript landing page. See `website/CLAUDE.md` for full details.
```bash
cd website
mise run fmt && mise run typecheck && mise run build
```
Deployed to GitHub Pages at `/models`. Uses bun, not npm.

## Releasing
1. Bump version in `Cargo.toml`
2. `mise run fmt && mise run clippy && mise run test`
3. Commit `Cargo.toml` and `Cargo.lock` together
4. `git tag v<version> && git push && git push --tags`
5. Release workflow runs automatically: builds binaries, packages .deb/.rpm, publishes to crates.io, and updates AUR package. Homebrew Core bumps happen separately in `Homebrew/homebrew-core`.
6. The GitHub flake is available from the pushed tag automatically (for example, `nix run github:reyamira/models/v<version>`). FlakeHub publishing is manual-only and should only be dispatched after account/org setup is intentionally enabled.

## Secrets
- `AA_API_KEY` — Artificial Analysis API key (GitHub repo secret, local `.env`)
- `AUR_SSH_PRIVATE_KEY` — SSH key for pushing to AUR (`~/.ssh/aur`)
- `CARGO_REGISTRY_TOKEN` — crates.io publish token (GitHub repo secret)
