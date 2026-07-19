//! `quant-sim export-events`: internal binary log → research interchange
//! files (docs/interchange.md). All files LF, scaled-i64, no floats.

use std::io::Write;
use std::path::Path;

use market_sim::log::read_log;
use market_sim::projection::BookProjection;
use sim_core::{EngineEvent, EventRecord};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("log: {0}")]
    Log(#[from] market_sim::log::LogError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum EventFormat {
    Csv,
    Jsonl,
}

pub fn export(run_dir: &Path, format: EventFormat) -> Result<(), ExportError> {
    let (header, records) = read_log(&run_dir.join("events.qsim"))?;
    eprintln!(
        "log: seed={} scenario={} records={}",
        header.seed,
        &header.scenario_hash[..16.min(header.scenario_hash.len())],
        records.len()
    );

    match format {
        EventFormat::Jsonl => write_events_jsonl(run_dir, &records)?,
        EventFormat::Csv => write_events_csv(run_dir, &records)?,
    }
    write_fills_csv(run_dir, &records)?;
    write_trades_csv(run_dir, &records)?;
    write_l1_csv(run_dir, &records)?;
    eprintln!("wrote exports into {}", run_dir.display());
    Ok(())
}

fn create(path: &Path) -> Result<std::io::BufWriter<std::fs::File>, ExportError> {
    Ok(std::io::BufWriter::new(std::fs::File::create(path)?))
}

fn write_events_jsonl(dir: &Path, records: &[EventRecord]) -> Result<(), ExportError> {
    let mut w = create(&dir.join("events.jsonl"))?;
    for record in records {
        serde_json::to_writer(&mut w, record)?;
        w.write_all(b"\n")?;
    }
    Ok(())
}

// One row-shape table per event variant; splitting would obscure the schema.
#[allow(clippy::too_many_lines)]
fn write_events_csv(dir: &Path, records: &[EventRecord]) -> Result<(), ExportError> {
    let mut w = create(&dir.join("events.csv"))?;
    w.write_all(
        b"seq,ts_ns,symbol,type,id,maker_id,taker_id,price_ticks,qty_lots,remaining_lots,reason\n",
    )?;
    for r in records {
        let (ty, id, maker, taker, price, qty, remaining, reason) = match r.event {
            EngineEvent::OrderAccepted { id, qty, .. } => (
                "accepted",
                Some(id.value()),
                None,
                None,
                None,
                Some(qty.lots()),
                None,
                String::new(),
            ),
            EngineEvent::OrderRejected { id, reason } => (
                "rejected",
                Some(id.value()),
                None,
                None,
                None,
                None,
                None,
                format!("{reason:?}"),
            ),
            EngineEvent::Fill {
                maker_id,
                taker_id,
                price,
                qty,
                taker_remaining,
                ..
            } => (
                "fill",
                None,
                Some(maker_id.value()),
                Some(taker_id.value()),
                Some(price.ticks()),
                Some(qty.lots()),
                Some(taker_remaining.lots()),
                String::new(),
            ),
            EngineEvent::OrderRested {
                id,
                price,
                remaining,
            } => (
                "rested",
                Some(id.value()),
                None,
                None,
                Some(price.ticks()),
                None,
                Some(remaining.lots()),
                String::new(),
            ),
            EngineEvent::OrderCancelled {
                id,
                remaining,
                reason,
            } => (
                "cancelled",
                Some(id.value()),
                None,
                None,
                None,
                None,
                Some(remaining.lots()),
                format!("{reason:?}"),
            ),
            EngineEvent::OrderModified {
                id,
                new_price,
                new_remaining,
                ..
            } => (
                "modified",
                Some(id.value()),
                None,
                None,
                Some(new_price.ticks()),
                None,
                Some(new_remaining.lots()),
                String::new(),
            ),
        };
        let fmt_opt_u = |v: Option<u64>| v.map_or_else(String::new, |x| x.to_string());
        let fmt_opt_i = |v: Option<i64>| v.map_or_else(String::new, |x| x.to_string());
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{}",
            r.seq.value(),
            r.ts.nanos(),
            r.symbol.value(),
            ty,
            fmt_opt_u(id),
            fmt_opt_u(maker),
            fmt_opt_u(taker),
            fmt_opt_i(price),
            fmt_opt_u(qty),
            fmt_opt_u(remaining),
            reason
        )?;
    }
    Ok(())
}

fn write_fills_csv(dir: &Path, records: &[EventRecord]) -> Result<(), ExportError> {
    let mut w = create(&dir.join("fills.csv"))?;
    w.write_all(
        b"ts_ns,trade_id,symbol,maker_owner,taker_owner,taker_side,price_ticks,qty_lots\n",
    )?;
    for r in records {
        if let EngineEvent::Fill {
            trade_id,
            maker_owner,
            taker_owner,
            price,
            qty,
            taker_side,
            ..
        } = r.event
        {
            writeln!(
                w,
                "{},{},{},{},{},{},{},{}",
                r.ts.nanos(),
                trade_id.value(),
                r.symbol.value(),
                maker_owner.value(),
                taker_owner.value(),
                match taker_side {
                    sim_core::Side::Buy => 1,
                    sim_core::Side::Sell => -1,
                },
                price.ticks(),
                qty.lots()
            )?;
        }
    }
    Ok(())
}

/// The `trades` interchange schema: one row per fill, aggressor-signed —
/// identical shape to normalized real-market trades so real-vs-sim analysis
/// is symmetric.
fn write_trades_csv(dir: &Path, records: &[EventRecord]) -> Result<(), ExportError> {
    let mut w = create(&dir.join("trades.csv"))?;
    w.write_all(b"ts_ns,side,price_ticks,qty_lots,trade_id\n")?;
    for r in records {
        if let EngineEvent::Fill {
            trade_id,
            price,
            qty,
            taker_side,
            ..
        } = r.event
        {
            writeln!(
                w,
                "{},{},{},{},{}",
                r.ts.nanos(),
                match taker_side {
                    sim_core::Side::Buy => 1,
                    sim_core::Side::Sell => -1,
                },
                price.ticks(),
                qty.lots(),
                trade_id.value()
            )?;
        }
    }
    Ok(())
}

fn write_l1_csv(dir: &Path, records: &[EventRecord]) -> Result<(), ExportError> {
    let mut w = create(&dir.join("l1.csv"))?;
    w.write_all(b"ts_ns,bid_ticks,bid_lots,ask_ticks,ask_lots\n")?;
    let mut projection = BookProjection::new();
    let mut last: Option<(i64, u128, i64, u128)> = None;
    for r in records {
        projection.apply(r);
        let l1 = projection.l1();
        if let (Some((b, bq)), Some((a, aq))) = (l1.bid, l1.ask) {
            let row = (b.ticks(), bq, a.ticks(), aq);
            if last != Some(row) {
                writeln!(
                    w,
                    "{},{},{},{},{}",
                    r.ts.nanos(),
                    row.0,
                    row.1,
                    row.2,
                    row.3
                )?;
                last = Some(row);
            }
        }
    }
    Ok(())
}
