//! `transform` — offline data-pipeline binary (feature `pipeline`).
//!
//! Converts raw upstream benchmark API/data dumps into the v2 `SourceFile`
//! schema and extracts models.dev's provider-TOML canonical identity edge.
//! Built only with `--features pipeline` so the published `models` binary
//! stays lean.
//!
//! The crate has no lib target, so the shared schema is pulled in via a
//! `#[path]` module include of the very same file the app compiles as
//! `crate::benchmarks::schema`. This guarantees the transform output can never
//! drift from what the app reads.

#[path = "../../model_refs.rs"]
mod model_refs;
#[path = "../../benchmarks/schema.rs"]
mod schema;

mod aa;
mod arena;
mod epoch;
mod llmstats;
mod models_dev;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "transform",
    about = "Transform benchmark data and models.dev canonical references"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Transform a raw Artificial Analysis API response (`{"data": [...]}`).
    Aa {
        /// Path to the raw AA API JSON response.
        input: PathBuf,
        /// Output path for the generated `SourceFile` JSON.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Transform an Arena leaderboard snapshot directory (6 board JSONs).
    Arena {
        /// Directory holding the per-board JSON files (text/vision/code/...).
        input: PathBuf,
        /// Output path for the generated `SourceFile` JSON.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Transform an unzipped Epoch AI benchmark CSV directory.
    Epoch {
        /// Directory holding the per-benchmark Epoch CSVs.
        input: PathBuf,
        /// Output path for the generated `SourceFile` JSON.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Transform collected LLM Stats `/v1/rankings` (+ optional `/v1/models`).
    Llmstats {
        /// Assembled `/v1/rankings` responses (`{"rankings": [...]}`).
        rankings: PathBuf,
        /// Optional `/v1/models` list (`{"models": [...]}`) for metadata enrichment.
        #[arg(short, long)]
        models: Option<PathBuf>,
        /// Output path for the generated `SourceFile` JSON.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Extract models.dev's provider-offering → canonical-model references.
    ModelsDevRefs {
        /// Path to a checkout of anomalyco/models.dev.
        input: PathBuf,
        /// Exact upstream Git commit represented by the checkout.
        #[arg(long)]
        upstream_commit: String,
        /// Output path for the generated reference artifact.
        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Aa { input, output } => aa::run(&input, &output),
        Command::Arena { input, output } => arena::run(&input, &output),
        Command::Epoch { input, output } => epoch::run(&input, &output),
        Command::Llmstats {
            rankings,
            models,
            output,
        } => llmstats::run(&rankings, models.as_deref(), &output),
        Command::ModelsDevRefs {
            input,
            upstream_commit,
            output,
        } => models_dev::run(&input, &upstream_commit, &output),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
