//! `quant-sim`: scenario runner, exports, reports.
//!
//! Machine output (JSON) goes to stdout; tables, logs and progress go to
//! stderr — pipelines stay clean.

mod export;
mod fit;
mod run;
mod scenario;
mod table;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "quant-sim", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a scenario (or its study sweep when `[study]` is present).
    Run {
        scenario: PathBuf,
        /// Override the scenario seed.
        #[arg(long)]
        seed: Option<u64>,
        /// Write events.qsim + report.json here.
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "table")]
        format: run::OutputFormat,
        /// Re-run determinism check: fail unless the event-stream BLAKE3
        /// equals this hex value.
        #[arg(long)]
        verify_hash: Option<String>,
    },
    /// Validate a scenario file and print its hash.
    Scenario {
        #[command(subcommand)]
        command: ScenarioCommand,
    },
    /// Convert a saved run (events.qsim) into research interchange files.
    ExportEvents {
        run_dir: PathBuf,
        #[arg(long, value_enum, default_value = "csv")]
        format: export::EventFormat,
    },
    /// Fit models from interchange data.
    Fit {
        #[command(subcommand)]
        command: FitCommand,
    },
}

#[derive(Debug, Subcommand)]
enum FitCommand {
    /// 2-dim (buy/sell) exponential Hawkes MLE from a trades CSV.
    Hawkes {
        trades_csv: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "BTCUSDT")]
        symbol: String,
        #[arg(long, default_value = "binance-um")]
        exchange: String,
    },
    /// Ornstein-Uhlenbeck fit from a `(ts_ns, value)` CSV; JSON on stdout.
    Ou { series_csv: PathBuf },
}

#[derive(Debug, Subcommand)]
enum ScenarioCommand {
    Validate { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result: Result<(), String> = match cli.command {
        Command::Run {
            scenario,
            seed,
            out,
            format,
            verify_hash,
        } => run::run(&run::RunArgs {
            scenario,
            seed,
            out,
            format,
            verify_hash,
        })
        .map_err(|e| e.to_string()),
        Command::Scenario { command } => match command {
            ScenarioCommand::Validate { path } => run::validate(&path).map_err(|e| e.to_string()),
        },
        Command::ExportEvents { run_dir, format } => {
            export::export(&run_dir, format).map_err(|e| e.to_string())
        }
        Command::Fit { command } => match command {
            FitCommand::Hawkes {
                trades_csv,
                out,
                symbol,
                exchange,
            } => fit::hawkes(&trades_csv, &out, &symbol, &exchange).map_err(|e| e.to_string()),
            FitCommand::Ou { series_csv } => fit::ou(&series_csv).map_err(|e| e.to_string()),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
