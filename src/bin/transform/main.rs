//! `transform` — offline data-pipeline binary (feature `pipeline`).
//!
//! Converts raw upstream benchmark API/data dumps into the v2 `SourceFile`
//! schema that the TUI/CLI deserialize. Built only with `--features pipeline`
//! so the published `models` binary stays lean.
//!
//! The crate has no lib target, so the shared schema is pulled in via a
//! `#[path]` module include of the very same file the app compiles as
//! `crate::benchmarks::schema`. This guarantees the transform output can never
//! drift from what the app reads.

#[path = "../../benchmarks/schema.rs"]
mod schema;

mod aa;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "transform",
    about = "Transform raw benchmark data dumps into the v2 SourceFile schema"
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Aa { input, output } => aa::run(&input, &output),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
