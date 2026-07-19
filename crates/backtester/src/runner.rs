//! The backtest runner: run the simulation, then fold the authoritative
//! event log into exchange-truth accounting, equity curves and metrics.
//!
//! Accounting is deliberately *post-run over the recorded log*: the engine's
//! stream is the single source of truth (fees and cash move at engine time),
//! while strategies only ever knew what their delayed feeds told them. The
//! two views agreeing on fills is a property of the event log, not an
//! assumption.

use std::collections::BTreeMap;

use market_sim::Simulation;
use market_sim::projection::BookProjection;
use sim_core::{EngineEvent, Instrument, OwnerId, SimDuration, SimTime, SymbolId};

use crate::account::Account;
use crate::fees::FeeModel;
use crate::metrics::{
    self, annualization_factor, deflated_sharpe, max_drawdown, moments, probabilistic_sharpe,
};
use crate::report::{MetricsBlock, PnlBlock, RunReport, StrategyReport, StudyReport, TradeStats};

/// Reporting configuration for one run.
#[derive(Debug, Clone)]
pub struct ReportSpec {
    pub scenario_hash: String,
    /// Equity sampling period.
    pub sample_every: SimDuration,
    /// Base capital for return scaling (e8).
    pub capital_base_e8: i128,
    /// (name, owner) per tracked strategy.
    pub strategies: Vec<(String, OwnerId)>,
}

/// Fold a completed simulation into a [`RunReport`].
// One linear pass over the log; splitting it would scatter the accounting
// sequence the report's correctness argument follows.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn build_report(
    sim: &Simulation,
    instruments: &BTreeMap<SymbolId, Instrument>,
    fee_model: &FeeModel,
    spec: &ReportSpec,
) -> RunReport {
    let tracked: BTreeMap<OwnerId, usize> = spec
        .strategies
        .iter()
        .enumerate()
        .map(|(i, (_, owner))| (*owner, i))
        .collect();

    // Per (strategy, symbol) accounts; equity aggregates across symbols.
    let mut accounts: BTreeMap<(usize, SymbolId), Account> = BTreeMap::new();
    let mut projections: BTreeMap<SymbolId, BookProjection> = BTreeMap::new();
    let mut last_mark_per_lot_e8: BTreeMap<SymbolId, i128> = BTreeMap::new();

    let mut equity_series: Vec<Vec<i128>> = vec![Vec::new(); spec.strategies.len()];
    let period = spec.sample_every.nanos().max(1) as u64;
    let mut next_sample = SimTime::from_nanos(period);

    let mark_of = |projection: &BookProjection,
                   last: &BTreeMap<SymbolId, i128>,
                   instrument: &Instrument,
                   symbol: SymbolId| {
        let l1 = projection.l1();
        match (l1.bid, l1.ask) {
            (Some((b, _)), Some((a, _))) => {
                (i128::from(b.ticks()) + i128::from(a.ticks()))
                    * i128::from(instrument.tick_lot_value_e8())
                    / 2
            }
            _ => last.get(&symbol).copied().unwrap_or(0),
        }
    };

    let sample_all = |accounts: &BTreeMap<(usize, SymbolId), Account>,
                      marks: &BTreeMap<SymbolId, i128>,
                      equity_series: &mut Vec<Vec<i128>>| {
        for (strategy_index, series) in equity_series.iter_mut().enumerate() {
            let mut equity: i128 = 0;
            for ((s, symbol), account) in accounts {
                if *s == strategy_index {
                    equity += account.equity(marks.get(symbol).copied().unwrap_or(0)).e8();
                }
            }
            series.push(equity);
        }
    };

    for record in &sim.log.records {
        // Sample the grid up to this record's time.
        while record.ts >= next_sample {
            sample_all(&accounts, &last_mark_per_lot_e8, &mut equity_series);
            next_sample = SimTime::from_nanos(next_sample.nanos() + period);
        }

        let symbol = record.symbol;
        if let Some(instrument) = instruments.get(&symbol) {
            let projection = projections.entry(symbol).or_default();
            projection.apply(record);
            let mark = mark_of(projection, &last_mark_per_lot_e8, instrument, symbol);
            if mark != 0 {
                last_mark_per_lot_e8.insert(symbol, mark);
            }

            if let EngineEvent::Fill {
                maker_owner,
                taker_owner,
                price,
                qty,
                taker_side,
                ..
            } = record.event
            {
                for (owner, side, is_maker) in [
                    (maker_owner, taker_side.opposite(), true),
                    (taker_owner, taker_side, false),
                ] {
                    if let Some(&strategy_index) = tracked.get(&owner) {
                        let notional = instrument
                            .notional(price, qty)
                            .expect("fill notional in range");
                        let fee = fee_model.fee(notional, is_maker).expect("fee in range");
                        accounts
                            .entry((strategy_index, symbol))
                            .or_default()
                            .apply_fill(instrument, side, price, qty, fee, is_maker)
                            .expect("accounting in range");
                    }
                }
            }
        }
    }
    // Final sample.
    sample_all(&accounts, &last_mark_per_lot_e8, &mut equity_series);

    let strategies = spec
        .strategies
        .iter()
        .enumerate()
        .map(|(strategy_index, (name, owner))| {
            let equity = &equity_series[strategy_index];
            let returns: Vec<f64> = equity
                .windows(2)
                .map(|w| (w[1] - w[0]) as f64 / spec.capital_base_e8 as f64)
                .collect();

            let mut realized = 0i128;
            let mut unrealized = 0i128;
            let mut fees = 0i128;
            let mut fills = 0u64;
            let mut maker_fills = 0u64;
            let mut gross = 0i128;
            let mut position_end = 0i64;
            for ((s, symbol), account) in &accounts {
                if *s != strategy_index {
                    continue;
                }
                let mark = last_mark_per_lot_e8.get(symbol).copied().unwrap_or(0);
                realized += account.realized().e8();
                unrealized += account.unrealized(mark).e8();
                fees += account.fees.e8();
                fills += account.fills;
                maker_fills += account.maker_fills;
                gross += account.gross_traded.e8();
                position_end += account.position_lots;
            }
            let equity_end = equity.last().copied().unwrap_or(0);

            let m = moments(&returns);
            let sharpe = metrics::sharpe(&returns);
            let annualize = annualization_factor(period as f64 / 1e9);
            let (dd, dd_len) = max_drawdown(equity, spec.capital_base_e8);
            StrategyReport {
                name: name.clone(),
                owner: owner.value(),
                pnl: PnlBlock {
                    realized_e8: realized,
                    unrealized_e8: unrealized,
                    fees_e8: fees,
                    equity_end_e8: equity_end,
                },
                metrics: MetricsBlock {
                    sharpe,
                    sharpe_annualized: sharpe.map(|s| s * annualize),
                    sortino: metrics::sortino(&returns),
                    psr: m.as_ref().and_then(|m| probabilistic_sharpe(m, 0.0)),
                    dsr: None,
                    dsr_reason: Some("K=1".to_string()),
                    max_drawdown: dd,
                    drawdown_len_samples: dd_len,
                    n_periods: returns.len(),
                },
                trade_stats: TradeStats {
                    fills,
                    maker_fills,
                    gross_traded_e8: gross,
                    position_end_lots: position_end,
                },
                equity_e8: equity.clone(),
                returns,
            }
        })
        .collect();

    RunReport {
        schema_version: 1,
        seed: sim.seed(),
        scenario_hash: spec.scenario_hash.clone(),
        events_hash: sim.log.hash_hex(),
        event_count: sim.log.len(),
        sim_span_ns: sim.now().nanos(),
        period_ns: period,
        capital_base_e8: spec.capital_base_e8,
        strategies,
    }
}

/// Assemble a study from completed runs: the honest-K DSR of the best trial.
///
/// `pick` selects which strategy block within each run is "the trial"
/// (usually index 0; baselines ride along as other blocks).
#[must_use]
pub fn build_study(runs: Vec<RunReport>, pick: usize) -> StudyReport {
    let trial_names: Vec<String> = runs
        .iter()
        .map(|r| {
            r.strategies
                .get(pick)
                .map_or_else(String::new, |s| s.name.clone())
        })
        .collect();
    let trial_sharpes: Vec<f64> = runs
        .iter()
        .map(|r| {
            r.strategies
                .get(pick)
                .and_then(|s| s.metrics.sharpe)
                .unwrap_or(0.0)
        })
        .collect();
    let best_index = trial_sharpes
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i);
    let dsr_best = runs.get(best_index).and_then(|run| {
        let strategy = run.strategies.get(pick)?;
        let m = moments(&strategy.returns)?;
        deflated_sharpe(&m, &trial_sharpes)
    });
    StudyReport {
        schema_version: 1,
        n_trials: runs.len(),
        trial_names,
        trial_sharpes,
        best_index,
        dsr_best,
        runs,
    }
}
