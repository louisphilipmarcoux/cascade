//! Terminal rendering of run/study reports.

use backtester::{RunReport, StudyReport};
use comfy_table::{Cell, Table, presets::UTF8_FULL_CONDENSED};

fn cash(e8: i128) -> String {
    format!("{}", sim_core::Cash::from_e8(e8))
}

fn opt(v: Option<f64>) -> String {
    v.map_or_else(|| "—".into(), |x| format!("{x:.4}"))
}

#[must_use]
pub fn run_table(report: &RunReport) -> String {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header([
        "strategy",
        "fills",
        "maker%",
        "realized",
        "unrealized",
        "fees",
        "equity",
        "SR/period",
        "SR/ann",
        "sortino",
        "PSR",
        "DSR",
        "maxDD",
    ]);
    for s in &report.strategies {
        let maker_pct = if s.trade_stats.fills == 0 {
            0.0
        } else {
            100.0 * s.trade_stats.maker_fills as f64 / s.trade_stats.fills as f64
        };
        table.add_row([
            Cell::new(&s.name),
            Cell::new(s.trade_stats.fills),
            Cell::new(format!("{maker_pct:.0}%")),
            Cell::new(cash(s.pnl.realized_e8)),
            Cell::new(cash(s.pnl.unrealized_e8)),
            Cell::new(cash(s.pnl.fees_e8)),
            Cell::new(cash(s.pnl.equity_end_e8)),
            Cell::new(opt(s.metrics.sharpe)),
            Cell::new(opt(s.metrics.sharpe_annualized)),
            Cell::new(opt(s.metrics.sortino)),
            Cell::new(opt(s.metrics.psr)),
            Cell::new(s.metrics.dsr.map_or_else(
                || s.metrics.dsr_reason.clone().unwrap_or_else(|| "—".into()),
                |d| format!("{d:.4}"),
            )),
            Cell::new(format!("{:.2}%", s.metrics.max_drawdown * 100.0)),
        ]);
    }
    format!(
        "run seed={} events={} span={:.1}s hash={}\n{table}",
        report.seed,
        report.event_count,
        report.sim_span_ns as f64 / 1e9,
        &report.events_hash[..16],
    )
}

#[must_use]
pub fn study_table(study: &StudyReport) -> String {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["trial", "SR/period", "selected"]);
    for (i, (name, sr)) in study
        .trial_names
        .iter()
        .zip(&study.trial_sharpes)
        .enumerate()
    {
        table.add_row([
            Cell::new(name),
            Cell::new(format!("{sr:.4}")),
            Cell::new(if i == study.best_index {
                "◀ best"
            } else {
                ""
            }),
        ]);
    }
    format!(
        "study: K={} real trials — DSR(best) = {}\n{table}\n\
         The deflated Sharpe above is computed against these {} actual trials \
         (Bailey & López de Prado); it answers \"is the best variant's Sharpe \
         explainable by selection luck alone?\"",
        study.n_trials,
        study
            .dsr_best
            .map_or_else(|| "—".into(), |d| format!("{d:.4}")),
        study.n_trials,
    )
}
