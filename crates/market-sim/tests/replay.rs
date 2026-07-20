//! Replay faithfulness on the committed fixture: the simulated fill tape
//! must reproduce the historical trade tape **exactly** — every trade, at
//! its historical price, quantity, aggressor side and mapped time.

use std::path::PathBuf;

use market_sim::data::read_trades_csv;
use market_sim::source::replay::{ReplayConfig, ReplaySource};
use market_sim::{LatencySpec, SimConfig, Simulation};
use matching_engine::EngineConfig;
use sim_core::{EngineEvent, Instrument, OwnerId, SimDuration, SimTime, SymbolId};

const SYM: SymbolId = SymbolId::new(0);
const REPLAY_OWNER: OwnerId = OwnerId::new(0);

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/binance-um/trades/BTCUSDT-2024-01-07-h00.csv")
}

#[test]
fn replay_reproduces_the_historical_tape_exactly() {
    let rows = read_trades_csv(&fixture_path()).expect("fixture readable");
    // BTCUSDT um: tick 0.01, lot 0.001 (data/instruments.toml).
    let instrument = Instrument::new("BTCUSDT", SYM, 1_000_000, 100_000).unwrap();
    let offset = SimDuration::from_secs(1);
    let source = ReplaySource::new(
        &rows,
        &instrument,
        ReplayConfig::default(),
        SYM,
        REPLAY_OWNER,
        OwnerId::new(1),
        offset,
    )
    .expect("replay source");
    let expected_trades = source.trade_count();
    assert!(
        expected_trades > 40_000,
        "fixture has {expected_trades} trades"
    );

    let last_dt = u64::try_from(rows.last().unwrap().ts_ns - rows[0].ts_ns).unwrap();
    let mut sim = Simulation::new(SimConfig {
        seed: 42,
        t_end: SimTime::from_nanos(last_dt + 2_000_000_000),
    });
    sim.add_engine(SYM, EngineConfig::default());
    sim.add_source(SYM, REPLAY_OWNER, Box::new(source), &LatencySpec::Zero);
    sim.run();

    // Collect the simulated tape: aggressor fills only (frame churn is
    // cancels/rests, and maker legs share the taker's trade rows).
    let mut tape: Vec<(u64, i8, i64, u64)> = Vec::new();
    for record in &sim.log.records {
        if let EngineEvent::Fill {
            price,
            qty,
            taker_side,
            ..
        } = record.event
        {
            let side = match taker_side {
                sim_core::Side::Buy => 1i8,
                sim_core::Side::Sell => -1,
            };
            match tape.last_mut() {
                // Coalesce partial fills of one aggressor at one price/time
                // (cannot happen with the exactness guard, but keeps the
                // comparison well-defined if semantics evolve).
                Some((ts, s, p, q))
                    if *ts == record.ts.nanos() && *s == side && *p == price.ticks() =>
                {
                    *q += qty.lots();
                }
                _ => tape.push((record.ts.nanos(), side, price.ticks(), qty.lots())),
            }
        }
    }

    assert_eq!(
        tape.len(),
        expected_trades,
        "tape length differs from history"
    );
    let base = rows[0].ts_ns;
    for (i, (row, sim_row)) in rows.iter().zip(&tape).enumerate() {
        let expected_ts = u64::try_from(row.ts_ns - base).unwrap() + offset.nanos() as u64;
        let expected_price = row.price_e8 / 1_000_000; // ticks of 0.01
        let expected_lots = u64::try_from(row.qty_e8 / 100_000).unwrap();
        assert_eq!(
            sim_row,
            &(expected_ts, row.side, expected_price, expected_lots),
            "trade {i} diverged from history"
        );
    }
}
