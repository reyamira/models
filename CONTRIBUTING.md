# Contributing

Thanks for your interest in contributing to `models`! This guide will help you get started.

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable) — install via [rustup](https://rustup.rs/)
- [mise](https://mise.jdx.dev/) (optional, recommended) — task runner that wraps cargo commands
- [Nix](https://nixos.org/download/) (optional) — for validating the flake build locally
- Git

## Getting Started

```bash
git clone https://github.com/reyamira/models
cd models
cargo build
cargo run
```

If you have mise installed, `mise run build` and `mise run run` work as well.

## Running Checks

Run these before every PR:

```bash
# With mise
mise run fmt && mise run clippy && mise run test

# Without mise
cargo fmt && cargo clippy -- -D warnings && cargo test
```

All three must pass. CI enforces the same checks on pull requests.

If your change touches `flake.nix`, `flake.lock`, packaging, or release/install behavior, also run:

```bash
nix build .
nix flake check
```

## Code Conventions

- **Clippy** runs with `-D warnings` — all warnings are errors
- **No `eprintln!` in TUI code** — stderr output corrupts ratatui's alternate screen buffer. Use `Message` variants or status bar updates instead. (`eprintln!` is fine in CLI-only code paths.)
- **Enum-based message passing** — the TUI uses an Elm-architecture pattern with a `Message` enum. No callbacks.
- **New `BenchmarkEntry` fields** must use `#[serde(default)]`
- **Commit `Cargo.lock`** alongside `Cargo.toml` when changing dependencies or version

## Data Directory

- **`data/agents.json`** — curated catalog of AI coding agents. Contributions welcome! Adding a new agent here requires no Rust knowledge or build tools.
- **`data/benchmarks.json`** — auto-generated from the Artificial Analysis API every 30 minutes. Do not edit manually.

See [Custom Agents](docs/custom-agents.md) for the agent entry format.

## Architecture

For detailed architecture documentation, key file locations, async patterns, and gotchas, see [CLAUDE.md](CLAUDE.md).

## Pull Requests

- Branch from `main` and keep changes focused
- Reference related issues in your PR description
- CI runs format checking, clippy, tests, and Nix build/flake checks on every PR. Rust CI skips doc-only changes; the Nix workflow still validates pushes and pull requests.

## Reporting Issues

When opening an issue, please include:

- Steps to reproduce the problem
- Expected vs actual behavior
- Your environment (OS, terminal, Rust version)

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.
