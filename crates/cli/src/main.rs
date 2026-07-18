//! `quant-sim`: scenario runner, model fitting, exports, reports.
//!
//! Subcommands land at M6 (`run`, `scenario validate`, `export-events`,
//! `--verify-hash`) and M7 (`fit hawkes`, `fit ou`, `data check`).

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "quant-sim", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!(
        "quant-sim {}: scenario runner arrives at milestone M6 (see docs/spec.md build order)",
        env!("CARGO_PKG_VERSION")
    );
}
