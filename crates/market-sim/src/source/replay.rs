//! Historical replay: real trades drive the engine, faithfully on the tape.
//!
//! Free historical data gives the *trade tape* (aggressor side, price, qty)
//! and L1 quotes — not full order-by-order L2 (see docs/interchange.md). So
//! this source replays honestly at the tape level:
//!
//! - before each historical trade it places a synthetic maker order at
//!   exactly the historical price/qty on the passive side;
//! - the aggressor arrives as an IOC limit at that price — the fill then
//!   reproduces the historical trade **exactly** (price, qty, aggressor
//!   side, time);
//! - a two-sided "frame" quote around the last price keeps L1 defined
//!   between trades.
//!
//! Bias profile (documented, not hidden): strategy orders interact with the
//! synthetic frame, not with real historical queues — fills against it are
//! optimistic. Non-interactive L2 delta replay with queue-position models
//! (Stage 3, fed by the live recorder) is the complementary mode.

use serde::{Deserialize, Serialize};
use sim_core::{
    Instrument, OrderKind, OwnerId, Price, Qty, Request, SimDuration, SimTime, SymbolId,
    TimeInForce,
};

use super::{FlowCtx, OrderFlowSource, OwnOrders, submit};
use crate::data::TradeRow;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ReplayConfig {
    /// Frame half-spread in ticks around the last trade price.
    pub frame_spread_ticks: i64,
    /// Frame depth per side, lots.
    pub frame_lots: u64,
    /// Refresh the frame when the last price moved this many ticks.
    pub frame_refresh_ticks: i64,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            frame_spread_ticks: 5,
            frame_lots: 1_000,
            frame_refresh_ticks: 3,
        }
    }
}

#[derive(Debug)]
enum Step {
    /// Place the passive counterparty for trade `index`.
    Maker { index: usize },
    /// Fire the aggressor for trade `index`.
    Taker { index: usize },
    /// Cancel one of our resting orders (stale frame or residual).
    CancelFrame { id: sim_core::OrderId },
    /// Place a frame quote at (side, price).
    PlaceFrame { side: sim_core::Side, price: i64 },
}

#[derive(Debug)]
pub struct ReplaySource {
    config: ReplayConfig,
    symbol: SymbolId,
    /// Owns frames + synthetic makers. Distinct from `taker_owner`, or
    /// self-match prevention would cancel every replayed trade.
    owner: OwnerId,
    taker_owner: OwnerId,
    /// (virtual time, side ±1, price ticks, qty lots), time-mapped, sorted.
    trades: Vec<(SimTime, i8, i64, u64)>,
    cursor: usize,
    queue: std::collections::VecDeque<Step>,
    frame_bid: Option<(sim_core::OrderId, i64)>,
    frame_ask: Option<(sim_core::OrderId, i64)>,
    last_price: Option<i64>,
    /// Our resting orders (frames + any maker leftovers).
    own: OwnOrders,
}

impl ReplaySource {
    /// Map real epoch-ns rows onto virtual time starting at `offset`.
    pub fn new(
        rows: &[TradeRow],
        instrument: &Instrument,
        config: ReplayConfig,
        symbol: SymbolId,
        owner: OwnerId,
        taker_owner: OwnerId,
        offset: SimDuration,
    ) -> Result<Self, String> {
        if rows.is_empty() {
            return Err("empty replay dataset".into());
        }
        if owner == taker_owner {
            return Err(
                "replay maker and taker owners must differ (STP would kill the tape)".into(),
            );
        }
        let t0 = rows[0].ts_ns;
        let mut trades = Vec::with_capacity(rows.len());
        for row in rows {
            let price = instrument
                .price_from_e8_exact(row.price_e8)
                .ok_or_else(|| format!("price {} not on tick grid", row.price_e8))?;
            let qty = instrument
                .qty_from_e8_exact(row.qty_e8)
                .ok_or_else(|| format!("qty {} not on lot grid", row.qty_e8))?;
            if qty.is_zero() {
                continue;
            }
            let dt = u64::try_from(row.ts_ns - t0).map_err(|_| "unsorted trades".to_string())?;
            let at = SimTime::from_nanos(dt)
                .checked_add(offset)
                .map_err(|e| e.to_string())?;
            trades.push((at, row.side, price.ticks(), qty.lots()));
        }
        Ok(Self {
            config,
            symbol,
            owner,
            taker_owner,
            trades,
            cursor: 0,
            queue: std::collections::VecDeque::new(),
            frame_bid: None,
            frame_ask: None,
            last_price: None,
            own: OwnOrders::default(),
        })
    }

    #[must_use]
    pub fn trade_count(&self) -> usize {
        self.trades.len()
    }

    fn schedule_trade(&mut self, index: usize) {
        let (_, _, price, _) = self.trades[index];
        // Tape exactness guard: cancel any maker leftover from earlier trades
        // (created when an aggressor partially filled against residuals at a
        // better price). Without this, stale liquidity could intercept later
        // aggressors and the replayed tape would drift from history.
        let frame_ids = [
            self.frame_bid.map(|(id, _)| id),
            self.frame_ask.map(|(id, _)| id),
        ];
        for id in self.own.ids() {
            if !frame_ids.contains(&Some(id)) {
                self.queue.push_back(Step::CancelFrame { id });
            }
        }
        // Refresh the frame first if price moved enough (or first trade).
        let needs_frame = match self.last_price {
            None => true,
            Some(last) => (price - last).abs() >= self.config.frame_refresh_ticks,
        };
        if needs_frame {
            if let Some((id, _)) = self.frame_bid.take() {
                self.queue.push_back(Step::CancelFrame { id });
            }
            if let Some((id, _)) = self.frame_ask.take() {
                self.queue.push_back(Step::CancelFrame { id });
            }
            let bid = price - self.config.frame_spread_ticks;
            let ask = price + self.config.frame_spread_ticks;
            if bid >= 1 {
                self.queue.push_back(Step::PlaceFrame {
                    side: sim_core::Side::Buy,
                    price: bid,
                });
            }
            self.queue.push_back(Step::PlaceFrame {
                side: sim_core::Side::Sell,
                price: ask,
            });
            self.last_price = Some(price);
        }
        self.queue.push_back(Step::Maker { index });
        self.queue.push_back(Step::Taker { index });
    }
}

impl OrderFlowSource for ReplaySource {
    fn next(&mut self, ctx: &mut FlowCtx<'_>) -> Option<(SimTime, Request)> {
        // Each step returns exactly one request; refill from the next trade
        // when the queue drains. Scheduling always enqueues (Maker + Taker),
        // so a single refill guarantees a non-empty queue below.
        if self.queue.is_empty() {
            if self.cursor >= self.trades.len() {
                return None;
            }
            let index = self.cursor;
            self.cursor += 1;
            self.schedule_trade(index);
        }
        let step = self.queue.pop_front().expect("scheduling enqueues steps");
        match step {
            Step::CancelFrame { id } => {
                let at = self.trades[self.cursor.saturating_sub(1)].0;
                Some((at, Request::Cancel { id }))
            }
            Step::PlaceFrame { side, price } => {
                let at = self.trades[self.cursor.saturating_sub(1)].0;
                let (id, request) = submit(
                    ctx.id_gen,
                    self.owner,
                    self.symbol,
                    side,
                    OrderKind::Limit(Price::new(price)),
                    Qty::new(self.config.frame_lots),
                    TimeInForce::Gtc,
                );
                match side {
                    sim_core::Side::Buy => self.frame_bid = Some((id, price)),
                    sim_core::Side::Sell => self.frame_ask = Some((id, price)),
                }
                Some((at, request))
            }
            Step::Maker { index } => {
                let (at, side, price, lots) = self.trades[index];
                // Passive counterparty on the opposite side of the aggressor.
                let maker_side = if side > 0 {
                    sim_core::Side::Sell
                } else {
                    sim_core::Side::Buy
                };
                let (_, request) = submit(
                    ctx.id_gen,
                    self.owner,
                    self.symbol,
                    maker_side,
                    OrderKind::Limit(Price::new(price)),
                    Qty::new(lots),
                    TimeInForce::Gtc,
                );
                Some((at, request))
            }
            Step::Taker { index } => {
                let (at, side, price, lots) = self.trades[index];
                let taker_side = if side > 0 {
                    sim_core::Side::Buy
                } else {
                    sim_core::Side::Sell
                };
                let (_, request) = submit(
                    ctx.id_gen,
                    self.taker_owner,
                    self.symbol,
                    taker_side,
                    OrderKind::Limit(Price::new(price)),
                    Qty::new(lots),
                    TimeInForce::Ioc,
                );
                Some((at, request))
            }
        }
    }

    fn on_own_record(&mut self, record: &sim_core::EventRecord) {
        self.own.on_record(record);
    }
}
