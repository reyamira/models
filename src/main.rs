mod agents;
mod api;
mod benchmarks;
mod cli;
mod config;
mod data;
mod formatting;
mod labs;
mod model_refs;
mod provider_category;
mod status;
mod tui;

use std::path::PathBuf;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(name = "models")]
#[command(about = "CLI/TUI for browsing AI models, benchmarks, and coding agents")]
#[command(version)]
#[command(disable_help_subcommand = true)]
#[command(help_template = "\
{about}

\x1b[1;4mUsage:\x1b[0m {usage}

\x1b[1;4mCommands:\x1b[0m
  list           List models, optionally filtered by provider
  providers      List providers
  show           Show detailed information about a model
  search         Search models by name or provider

\x1b[1;4mSetup:\x1b[0m
  completions    Generate shell completions
  link           Create shell symlinks for `agents`, `benchmarks`, and `status` commands

\x1b[1;4mAdditional:\x1b[0m
  agents         Track AI coding agent releases and changelogs
  benchmarks     Query benchmark data from the command line
  status         Check AI provider service health

\x1b[1;4mOptions:\x1b[0m
{options}

\x1b[1;4mExamples:\x1b[0m
  models                              Launch the interactive TUI
  models list                         Open the inline model picker
  models benchmarks list              Open the inline benchmark picker
  models agents claude                Browse Claude Code releases
  models link                         Create `agents`, `benchmarks`, and `mstatus` symlinks
")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List models, optionally filtered by provider
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  models list                         Open the interactive model picker
  models list openai                  Picker prefiltered to a provider
  models list --json                  Dump model rows as JSON")]
    List {
        /// Filter by provider ID or exact provider name
        provider: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List providers
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  models providers
  models providers --json")]
    Providers {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show detailed information about a model
    Show {
        /// Model ID (e.g., claude-opus-4-1, gpt-4o)
        model_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Search models by name or provider
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  models search claude
  models search gpt-4o --json

\x1b[1;4mNote:\x1b[0m
  Search now uses the same matcher and interactive picker flow as `models list`.")]
    Search {
        /// Search query
        query: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
    /// Track AI coding agent releases and changelogs
    #[command(after_help = "\
\x1b[1;4mTool Commands:\x1b[0m
  agents <tool>                 Browse releases for a tool
  agents <tool> --latest        Show latest changelog directly
  agents <tool> --list, -l      List all versions
  agents <tool> --version <v>   Show changelog for a specific version
  agents <tool> --web, -w       Open releases page in browser

\x1b[1;4mExamples:\x1b[0m
  agents claude                 Browse Claude Code releases
  agents claude --latest        Latest Claude Code changelog
  agents cursor --list          All Cursor versions
  agents cursor --version 1.0.0 Show a specific Cursor version")]
    Agents {
        #[command(subcommand)]
        command: Option<cli::agents::AgentsCommand>,
    },
    /// Query benchmark data from the command line
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  models benchmarks list                     Open the interactive benchmark picker
  models benchmarks list --sort speed --limit 10
  models benchmarks list --creator openai --reasoning
  models benchmarks list --json
  models benchmarks show gpt-4o              Show benchmark details by slug
  models benchmarks show \"Claude Sonnet 4\"   Show by display name
  models benchmarks show gpt-4o --json       Output details as JSON")]
    Benchmarks {
        #[command(subcommand)]
        command: Option<cli::benchmarks::BenchmarksCommand>,
    },
    /// Check AI provider service health
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  models status status            Quick dashboard table
  models status list              Interactive provider health picker
  models status list --json       All provider statuses as JSON
  models status show openai       Detailed OpenAI status
  models status show anthropic --json
  models status sources           Manage tracked providers
  models status sources --json    List all available sources")]
    Status {
        #[command(subcommand)]
        command: Option<cli::status::StatusCommand>,
    },
    /// Create shell symlinks for `agents`, `benchmarks`, and `status` commands
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  models link                     Create symlinks next to the binary
  models link --dir ~/.local/bin  Create symlinks in a specific directory
  models link --status            Check symlink status
  models link --remove            Remove previously created symlinks")]
    Link {
        /// Target directory for symlinks (defaults to binary's directory)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Remove symlinks instead of creating them
        #[arg(long)]
        remove: bool,
        /// Show current symlink status
        #[arg(long)]
        status: bool,
    },
}

fn main() -> Result<()> {
    // Check if invoked via a symlink alias (e.g. "agents", "benchmarks", "mstatus")
    let binary_name = std::env::args()
        .next()
        .and_then(|s| {
            std::path::Path::new(&s)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
        })
        .unwrap_or_default();

    let config = config::Config::load().unwrap_or_default();
    if let Some(kind) = config.match_alias(&binary_name) {
        return match kind {
            config::AliasKind::Agents => cli::agents::run(),
            config::AliasKind::Benchmarks => cli::benchmarks::run(),
            config::AliasKind::Status => cli::status::run(),
        };
    }

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::List { provider, json }) => cli::list::models(provider, json)?,
        Some(Commands::Providers { json }) => cli::list::providers(json)?,
        Some(Commands::Show { model_id, json }) => cli::show::model(&model_id, json)?,
        Some(Commands::Search { query, json }) => cli::search::search(&query, json)?,
        Some(Commands::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "models", &mut std::io::stdout());
        }
        Some(Commands::Agents { command }) => cli::agents::run_with_command(command)?,
        Some(Commands::Benchmarks { command }) => cli::benchmarks::run_with_command(command)?,
        Some(Commands::Status { command }) => cli::status::run_with_command(command)?,
        Some(Commands::Link {
            dir,
            remove,
            status,
        }) => cli::link::run(dir, remove, status, &config)?,
        None => {
            // Fetch providers + the canonical model registry in one coherent
            // catalog snapshot before entering the async runtime.
            let catalog = api::fetch_catalog()?;

            // Create and run the async runtime only for the TUI
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(tui::run(catalog))?;
        }
    }

    Ok(())
}
