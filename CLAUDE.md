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
- **Benchmarks Tab** (`src/tui/benchmarks/`) — compare model benchmarks across 4 data sources (Artificial Analysis, Epoch AI, Arena, LLM Stats) via a data-source switcher (`{`/`}`; state-preserving — search/filters/sort intent and id-matched compare selections carry across sources). All views are registry-driven from per-source metric definitions shipped in the data files (no hardcoded field names). Browse/compare modes, H2H table, scatter plot, radar chart views, an `i` glossary popup with curated per-benchmark descriptions, an `a`-cycled comparator cell in the detail panel (field avg / peer avg / rank), and `r` in-app refresh of the active source (stale-while-revalidate)
- **Agents Tab** (`src/tui/agents/`) — track AI coding assistants with GitHub integration; add custom agents in-app (`A` → name + `owner/repo`, writes `config.agents.custom`) and run an agent's verified self-updater in the background (`u` selected / `U` all, behind a confirm modal; `Agent.update_command` argv per tool). See `.claude/rules/tui-agents-tab.md` §7b/§7c
- **Status Tab** (`src/tui/status/`) — live provider health monitoring with detail view for incidents, components, and scheduled maintenance

### Data Flow
- Model data: fetched from models.dev API at startup (`src/api.rs`)
- Benchmark data: each of the 4 sources is fetched fresh and in parallel on every launch (`src/benchmarks/fetch.rs` `fetch_source`), one v2 `SourceFile` JSON per source from `data/v2/*.json` on `main`. Each fetch tries a **multi-host fallback chain** (`candidate_urls`): `cdn.jsdelivr.net@main` → `fastly.jsdelivr.net@main` (warm-cache shortcut, not independent) → `raw.githubusercontent.com/…/main/…` (the real backstop — bypasses jsDelivr's branch resolver entirely). Added after the 2026-06-16 jsDelivr branch-resolution outage 301-looped every `@main` URL and took the whole tab down at once. Sources load progressively — the tab is usable as soon as any source (typically AA) lands. Dev override: set `MODELS_DATA_BASE_URL` to serve `{base}/{id}.json` from a local dir/staging host instead (bypasses the chain)
- Benchmark pipeline (build-time, not in the app): the `transform` bin (`src/bin/transform/`, Cargo feature `pipeline`) converts raw upstream API/CSV dumps into the v2 schema (`transform aa|arena|epoch|llmstats`) → `data/v2/{aa,epoch,arena,llmstats}.json`, served via jsDelivr. The legacy `data/benchmarks.json` lane (jq-only, AA) is **frozen** for already-released binaries that still fetch it — its shape must never change. Sunset criterion: retire the jq lane (leaving the last file in place so old binaries degrade to stale-not-broken) once monthly jsDelivr hits on `data/benchmarks.json` decay — check `https://data.jsdelivr.com/v1/stats/packages/gh/reyamira/models@main/files?period=month` (baseline at v2 launch, 2026-06: ~1,035 hits/month). **Read the per-day baseline trend, not the raw total** — `benchmarks.json` can be inflated by external/scraper spikes isolated to that one file (e.g. 2026-06-13/14 jumped ~40→~410/day while the v2 files stayed flat). Discount any spike where the `data/v2/*` files don't move in lockstep: those are external traffic on the legacy file, not real pre-v2 launches, and must not delay sunset. jsDelivr stats are aggregate-only (no per-IP/referrer/UA), so the source of such spikes is not attributable — judge by the v2-relative shape, see `data/stats/README.md`. `update-benchmarks.yml` is now multi-source: AA + LLM Stats run every trigger; Arena + Epoch are gated to twice daily (UTC hour 06/18 on the `:17` run), with a `workflow_dispatch` `refresh_all` boolean to bypass the gate. Each source step is `continue-on-error` so one transform failure leaves that source's previous file in place
- Agent/GitHub data: disk-cached with ETag conditional fetching (`src/agents/cache.rs`, `src/agents/github.rs`)
- CLI agents: uses `fetch_releases_only` (1 API call, no repo metadata) — TUI uses full `fetch_conditional` (2 calls, includes stars/issues/license)
- Status data: fetched from each provider's official status page (Statuspage, BetterStack, Instatus, etc.) with apistatuscheck.com as fallback (`src/status/fetch.rs`), provider registry and strategy mapping in `src/status/registry.rs`
- Status source contract and normalization rules are documented in code comments within `src/status/` adapters

### Async Pattern
Background fetches use tokio::spawn + mpsc channels. Results arrive as `Message` variants processed in the main loop (`src/tui/mod.rs`). The app never blocks on network calls.

### Mouse Interaction
Mouse capture is enabled at startup (`EnableMouseCapture` in `src/tui/mod.rs`). The event loop (`src/tui/event.rs`) handles `Event::Mouse` in `handle_mouse`: high-frequency `Moved`/`Drag` events are dropped, popups take precedence (the wheel scrolls/navigates whichever popup is open and a left-click selects/toggles the row under the cursor — sort picker applies, checkbox pickers toggle; clicks outside rows are swallowed so they don't leak behind; the help popup closes on click), header tab-bar clicks switch tabs (`ui::tab_at`), then the event routes to the active tab's `handle_<tab>_mouse(app, ev)`. Single-click selects a list row / focuses a panel; the wheel is **focus-then-scroll** (the panel under the cursor gets focus, then the same nav the arrow keys drive). No double-click/activation in v1.
- **Geometry cache pattern:** ratatui discards layout `Rect`s each frame, so each sub-app stores `Option<Rect>` fields for its focusable panels, written at render time and read by its mouse handler on the next event (valid because the loop draws before it handles events). Pure hit-test helpers live in `src/tui/mouse.rs` (`hit`, `row_at`) with unit tests; `handle_<tab>_mouse` mutates sub-app state directly (focus/selection/scroll) and adds **no new `Message` variants**.
- **`ListState::offset()` gotcha:** a list's `offset()` is only correct *after* the widget renders into **that same** state object. Rendering into a *copy* (`let mut s = self.list_state; render(.., &mut s)`) leaves the real state's `offset()` stale and silently breaks click-to-select on a scrolled list — render into `&mut self.list_state` instead.
- Per-tab clickable regions and mouse tests are documented in each `.claude/rules/tui-*-tab.md`; the reference implementation is the Models tab (`handle_models_mouse` + `mouse_tests` in `src/tui/models/`).

### Agents & CLI
See `src/agents/CLAUDE.md` and `src/cli/CLAUDE.md` for detailed module docs.
- Binary aliases: `models agents <cmd>` or `agents <cmd>` via argv[0] symlink detection. Alias names configurable via `[aliases]` in config.toml (defaults: `agents`, `benchmarks`, `mstatus`)
- Commands: `list`, `search`, `show`, `benchmarks`, `completions <shell>`, `link`, full agents suite (`status`, `latest`, `list-sources`, `<tool>`), full status suite (`list`, `show`, `status`, `sources`)
- CLI pickers use shared `PickerTerminal` infrastructure in `src/cli/picker.rs`

### Key Files

Each module has its own `CLAUDE.md` with detailed documentation. Top-level highlights:

- `src/formatting.rs` — shared utilities: `truncate`, `parse_date`, `format_tokens`, `format_stars`, `EM_DASH`, `cmp_opt_f64`
- `src/data.rs` — Provider/Model data structures from models.dev API
- `src/config.rs` — user config file (agents, cache, display, aliases, benchmarks settings). `AliasesConfig` struct + `AliasKind` enum for symlink routing; `BenchmarksConfig.columns` persists the benchmarks tab's visible metric columns per source (metric ids, not indices)
- `src/provider_category.rs` — provider categorization logic
- `src/benchmarks/` — `schema.rs` (v2 `SourceFile`/`MetricDef`/`ModelRow`/`ScoreCell` — shared with the transform bin via `#[path]`), `multi.rs` (`MultiStore`, `SortKey`, registry-driven view helpers: kind formatting, group ordering, radar groups, default sort, reasoning filter), `sources.rs` (compile-time `SourceDescriptor` registry of the 4 sources), `fetch.rs` (per-source CDN fetcher + `MODELS_DATA_BASE_URL` override), `traits.rs` (AA Jaro-Winkler matching + generic `enrich_from_models_dev`/`creator_openness` for non-AA sources). `store.rs`/`BenchmarkStore`/`BenchmarkEntry` are GONE
- `src/bin/transform/` — offline data-pipeline bin (feature `pipeline`): `main.rs` (clap subcommands) + `aa.rs`/`arena.rs`/`epoch.rs`/`llmstats.rs`. See `src/bin/transform/CLAUDE.md`
- `src/status/` — `types.rs`, `registry.rs`, `assessment.rs`, `fetch.rs`, `adapters/` (per-source-family parsers)
- `src/tui/` — `app.rs` (App state, Message enum), `event.rs` (NavAction dedup), `ui.rs` (shared helpers), `markdown.rs`, `widgets/` (ScrollablePanel, SoftCard, ScrollOffset, ComparisonLegend), per-tab subdirs: `models/`, `agents/`, `benchmarks/` (includes `radar.rs`), `status/` — each with `app.rs` (sub-app state) + `render.rs` (tab rendering)
- `src/agents/health.rs` — agent-to-status-provider mapping for service health display in the Agents tab
- `src/cli/` — `picker.rs` (shared PickerTerminal, nav helpers, style constants), `models.rs`/`benchmarks.rs`/`agents_ui.rs`/`status.rs` (inline pickers), `styles.rs`

### GitHub Actions
- `ci.yml` — runs on PR/push: fmt check, clippy, test
- `build-with-nix.yml` — runs on PR/push/manual dispatch: `nix build .` then `nix flake check` across Linux, Linux ARM, and macOS. Caching is **Cachix** (public cache `modelsdev`, `cachix-action@v17`, `CACHIX_AUTH_TOKEN` repo secret; fork PRs degrade to read-only pulls) — one global pool shared by PR runs, main runs, local dev, and end users. `flake.nix` `nixConfig` advertises the substituter + public signing key so `nix run github:reyamira/models` downloads prebuilt binaries. Replaced Magic Nix Cache/GHA cache 2026-06-12 (branch-scoped, CI-only, and macOS uploads blew the 30-min timeout).
- `release.yml` — triggered by `v*` tags: builds 5 targets in parallel with Rust caching, packages .deb/.rpm via cargo-binstall (pinned versions), generates SHA256SUMS, publishes to crates.io, and updates AUR package. Homebrew Core updates are handled in `Homebrew/homebrew-core` by Homebrew automation/maintainers, not from this repo. Pre-release tags (containing `-`) skip publish/AUR and mark the GitHub release as prerelease. Scoop Extras handles Windows updates via its own autoupdate mechanism.
- `flakehub-publish-tagged.yml` — manual-only; dispatch with an existing `v<version>` tag if FlakeHub publishing is intentionally enabled. Do not assume GitHub flake availability requires FlakeHub.
- `update-benchmarks.yml` — `workflow_dispatch`-only; multi-source. Checks out + builds the `transform` bin (Swatinem/rust-cache). Every trigger: AA (curl+jq → legacy `data/benchmarks.json`, plus the raw response → `transform aa` → `data/v2/aa.json`) and LLM Stats (bounded `/v1/rankings` + `/v1/models` fetch loop → `transform llmstats` → `data/v2/llmstats.json`). Twice-daily-gated (UTC hour 06/18 on the `:17` run, or `refresh_all` dispatch input): Arena (oolong-tea `latest.json` → 6 board JSONs → `transform arena`) and Epoch (epoch.ai ZIP → unzip → `transform epoch`). Each source step is `continue-on-error`; commit-if-changed covers `data/benchmarks.json` + `data/v2/`; jsDelivr purge per changed file (**retry/backoff** — jsDelivr's purge endpoint intermittently returns a 200 body of `no available server`, which curl can't auto-retry; without a successful purge an aliased `@main` URL serves stale "up to 7 days", so the app can show hours-old benchmark data even though git HEAD is fresh — the per-file purge loop retries 5× with backoff and is non-fatal, the next run re-purges). Triggered every 30 min (at `:17` and `:47` UTC) by the Cloudflare Worker in `infra/benchmark-trigger/` — the original GH `schedule:` cron was removed after proving unreliable under GitHub's cron throttling. `mise run refresh-sources` runs the same fetch+transform locally (`mise.toml` ↔ workflow stay in sync); `mise run refresh-benchmarks` is the legacy AA-only jq lane
- `snapshot-stats.yml` — daily `schedule:` + `workflow_dispatch`; runs `scripts/snapshot-stats.sh` (`mise run snapshot-stats`) to record jsDelivr per-file daily CDN hits into `data/stats/jsdelivr-history.json` as an app-launch proxy (the app has no telemetry; it fetches the v2 sources fresh per launch). jsDelivr serves only a rolling ~30-day window, so the script **upserts** the trailing 30 days — self-healing across dropped runs (tolerates GH cron throttling; no Cloudflare Worker needed). `fetch-depth: 0` + rebase-before-push so it doesn't race the benchmark bot. Self-traffic is filtered via a `data/stats/self-ping` sentinel fetched only by the maintainer's wrapped `models` launches: `other-users ≈ /data/v2/aa.json hits − self-ping hits`. See `data/stats/README.md`
- Use `mise run <task>` for all CLI operations — never run bare commands
- Keep clippy clean with `-D warnings`
- Enum-based message passing (no callbacks)
- No disk cache — benchmark data fetched fresh from CDN on every launch, sources empty until each CDN response lands
- `src/benchmarks/schema.rs` is the single source of truth for the v2 wire format, compiled both into the app (`crate::benchmarks::schema`) and into the transform bin via `#[path]`. Keep it self-contained — never reference other crate modules from it. Optional/forward-compat fields use `#[serde(default, skip_serializing_if = ...)]` so the JSON stays minimal and old files keep deserializing (`higher_is_better` defaults to `true`, `description`/`last_updated`/`ci`/etc. omitted when absent)
- Status detail semantics use parallel `*_state` metadata on `ProviderStatus`; UI and assessment logic should use helper methods instead of inferring meaning from empty vectors

## Gotchas
- Mouse handlers read a list's scroll position via `ListState::offset()` — this is only valid when the widget rendered into **that same** state object. Several lists historically rendered into a throwaway copy (`let mut s = self.list_state; …`), which leaves `offset()` stale and breaks click-to-select on a scrolled list. Render into the real `&mut self.list_state`. See the Mouse Interaction section above.
- clippy `-D warnings` treats unused enum variant fields as errors — if a Message variant's payload is only passed through (e.g., error strings logged nowhere), use a unit variant instead
- `Cargo.lock` must be committed after `Cargo.toml` version bumps
- GitHub Actions `workflow_dispatch` only works when the workflow file exists on the default branch — cannot test from feature branches
- Adding a new v2 field: add it to `schema.rs` with `#[serde(default, skip_serializing_if = ...)]` — no cache versioning needed since data is fetched fresh every launch, and old `data/v2/*.json` keep deserializing
- Bare `cargo run` is **ambiguous** (two `[[bin]]` targets: `models` + `transform`). `Cargo.toml` sets `default-run = "models"` so bare `cargo run` launches the TUI; run the pipeline bin explicitly with `cargo run --features pipeline --bin transform -- <subcommand> ...`
- Transform `if-changed` semantics: each transform writes the output only when it differs from the existing file **after normalizing `fetched_at` out** (the timestamp changes every run) — so a no-op run leaves the file's mtime/content untouched and commit-if-changed has nothing to stage. Add any other per-run-volatile field to the same normalization
- The AA API uses `0` as a sentinel for missing performance data — the legacy jq lane converts `0` → `null` (e.g., `if . == 0 then null else . end`); the `transform aa` bin treats those `0`s as `None` in typed serde structs
- The legacy `data/benchmarks.json` jq lane uses null-safe access (`?.` / `// null`) for nested objects — `mise.toml` (`refresh-benchmarks`) and `update-benchmarks.yml` must stay byte-identical for that file. The new v2 fetch+transform path is mirrored between `mise.toml` (`refresh-sources`) and the workflow's source steps
- Never use `eprintln!` in TUI mode — stderr output corrupts ratatui's alternate screen buffer, causing rendering glitches. Use `Message` variants or status bar updates instead. (`eprintln!` is fine in CLI-only code paths like `src/cli/agents.rs`)
- Agents `GitHubData` fields `open_issues`, `license`, and `last_commit` are fetched/cached but never displayed in the UI — only `stars` (detail panel + sort) and `releases` are rendered. Kept for potential future use
- `Paragraph::scroll((y, 0))` with `.wrap(Wrap { trim: false })` counts **visual (wrapped) lines**, not logical lines — scroll positions must account for line wrapping when jumping to specific content
- Use `line.width()` (unicode-aware) not `.len()` (byte count) when computing wrapped line heights — ratatui wraps on display width, not byte length. Word-wrapping needs +1 buffer per wrapped line since `div_ceil` underestimates
- TLS uses `rustls-tls-native-roots` (not `rustls-tls`) — loads certificates from the OS trust store to support corporate TLS-inspecting proxies
- Status-source quirks to preserve: Better Stack resources use `public_name`; Status.io `status_code = 400` means degraded; incident.io incidents and Instatus components need second fetches; the Google adapter is currently summary-derived rather than preserving raw incident rows
- Nix builds compile from a **filtered fileset** (`flake.nix` `lib.fileset`), not the full checkout — tests that read committed files (e.g. the `data/v2/*.json` drift guards) pass plain CI but fail in the Nix sandbox unless their paths are in the fileset union
- The benchmarks data bot commits to main every ~30 min — branches that touch `data/benchmarks.json` regrow merge conflicts on that file continuously; the resolution is always to take main's copy (both sides regenerate from the same API)

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
4. `git tag v<version> && git push && git push origin v<version>` — push the release tag **explicitly**; bare `git push --tags` errors benignly here because an old local tag diverges from the remote
5. Release workflow runs automatically: builds binaries, packages .deb/.rpm, publishes to crates.io, and updates AUR package. Homebrew Core bumps happen separately in `Homebrew/homebrew-core`.
6. The GitHub flake is available from the pushed tag automatically (for example, `nix run github:reyamira/models/v<version>`). FlakeHub publishing is manual-only and should only be dispatched after account/org setup is intentionally enabled.
7. Visuals pass: regenerate screenshots/GIFs against the release binary via the VHS tapes in `public/tapes/` with `PATH=$PWD/target/release:$PATH` (screenshots are committed in `public/assets/` and hot-linked by README + wiki; wiki GIFs republish via `vhs publish` + URL swap; website mp4s are ffmpeg conversions; `public/assets/wiki/` is gitignored intermediates)

## Secrets
- `AA_API_KEY` — Artificial Analysis API key (GitHub repo secret, local `.env`)
- `LLM_STATS_API_KEY` — LLM Stats API key for `/v1/rankings` + `/v1/models` (GitHub repo secret, local `.env`; same pattern as `AA_API_KEY`)
- `AUR_SSH_PRIVATE_KEY` — SSH key for pushing to AUR (`~/.ssh/aur`)
- `CARGO_REGISTRY_TOKEN` — crates.io publish token (GitHub repo secret)
