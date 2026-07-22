//! Avellaneda–Stoikov optimal market making (2008), finite-horizon variant.
//!
//! The inventory-risk-aware quoter from the actual paper. Given mid `s`,
//! inventory `q`, risk aversion `γ`, volatility `σ`, order-arrival intensity
//! parameter `k`, and time-to-horizon `T − t`:
//!
//! - reservation price   `r = s − q·γ·σ²·(T − t)`   (skews quotes against inventory)
//! - optimal total spread `δ = γ·σ²·(T − t) + (2/γ)·ln(1 + γ/k)`
//! - quotes               `bid = r − δ/2`, `ask = r + δ/2`
//!
//! σ is estimated online from mid returns (EWMA); `k` may be supplied or set
//! to "auto" and estimated from realized fill intensity (a `ln λ(δ) = ln A −
//! k·δ` regression over the strategy's own quote-distance/fill history).
//!
//! It is measured against [`crate::NaiveMm`] under identical seeds, flow and
//! frictions — the paper's edge has to *show up in `PnL`* to count, which is
//! exactly what the paired-seed backtests test.

use backtester::strategy::{OwnFill, Strategy, TradeApi};
use serde::{Deserialize, Serialize};
use sim_core::{EventRecord, Price, Qty, Side, SimDuration, SymbolId};

use crate::estimators::EwmaVol;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AvellanedaStoikovConfig {
    pub symbol_index: u16,
    /// Inventory risk aversion γ (> 0). Higher → tighter inventory control.
    pub gamma: f64,
    /// Order-book liquidity parameter k. If `None` (TOML: omit / null), it is
    /// estimated online from fill intensity.
    pub k: Option<f64>,
    /// EWMA decay for the volatility estimate (per mid-return sample).
    pub sigma_lambda: f64,
    /// Session horizon in seconds; drives the `(T − t)` urgency term.
    pub horizon_secs: f64,
    /// Quote size in lots.
    pub size_lots: u64,
    /// Requote cadence.
    pub requote_ms: i64,
    /// Hard inventory cap (lots); beyond it we quote one-sided to unwind.
    pub inventory_cap_lots: i64,
    /// Force-liquidate remaining inventory at the horizon (keeps the
    /// vs-naive comparison inventory-neutral).
    pub liquidate_at_end: bool,
    /// Fallback k when auto-estimation has too few fills yet.
    pub k_fallback: f64,
    /// Floor on σ (ticks/√s) so early quotes aren't degenerate.
    pub sigma_floor: f64,
    /// Practical guard: quotes and inventory skew are clamped to this
    /// fraction of mid (± band). Not part of the A-S theory; prevents a σ
    /// spike from producing nonsensical thousand-tick quotes.
    pub max_band_frac: f64,
}

impl Default for AvellanedaStoikovConfig {
    fn default() -> Self {
        Self {
            symbol_index: 0,
            gamma: 0.1,
            k: None,
            sigma_lambda: 0.98,
            horizon_secs: 600.0,
            size_lots: 5,
            requote_ms: 250,
            inventory_cap_lots: 50,
            liquidate_at_end: true,
            k_fallback: 1.5,
            sigma_floor: 0.2,
            max_band_frac: 0.05,
        }
    }
}

const TIMER_REQUOTE: u64 = 1;

/// Online estimator of the fill-intensity parameter `k` in `λ(δ) = A·e^{−kδ}`.
///
/// We bucket each realized fill by the quote distance δ (ticks from mid at
/// the time it was posted) and, per bucket, track fills and total posted
/// time. `ln(fill_rate)` is then linear in δ with slope `−k`; a running
/// least-squares over the buckets recovers `k`. Deliberately simple and
/// robust — the point is a *data-driven* k, not a perfect one.
#[derive(Debug, Default)]
struct KEstimator {
    /// `(delta_ticks -> (fills, exposure_seconds))` accumulators.
    buckets: std::collections::BTreeMap<i64, (f64, f64)>,
    total_fills: u64,
}

impl KEstimator {
    fn observe_post(&mut self, delta_ticks: i64, exposure_secs: f64) {
        let entry = self.buckets.entry(delta_ticks).or_insert((0.0, 0.0));
        entry.1 += exposure_secs;
    }

    fn observe_fill(&mut self, delta_ticks: i64) {
        let entry = self.buckets.entry(delta_ticks).or_insert((0.0, 0.0));
        entry.0 += 1.0;
        self.total_fills += 1;
    }

    /// Least-squares slope of `ln(rate)` on δ → `k = −slope`. `None` until
    /// enough distinct-δ buckets with fills exist.
    fn estimate(&self) -> Option<f64> {
        let points: Vec<(f64, f64)> = self
            .buckets
            .iter()
            .filter(|(_, (fills, exposure))| *fills > 0.0 && *exposure > 0.0)
            .map(|(&d, (fills, exposure))| (d as f64, libm::log(fills / exposure)))
            .collect();
        if points.len() < 3 {
            return None;
        }
        let n = points.len() as f64;
        let mean_x = points.iter().map(|p| p.0).sum::<f64>() / n;
        let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
        let mut cov = 0.0;
        let mut var = 0.0;
        for (x, y) in &points {
            cov += (x - mean_x) * (y - mean_y);
            var += (x - mean_x) * (x - mean_x);
        }
        if var <= 0.0 {
            return None;
        }
        let slope = cov / var;
        let k = -slope;
        (k > 1e-3).then_some(k)
    }
}

#[derive(Debug)]
pub struct AvellanedaStoikov {
    config: AvellanedaStoikovConfig,
    symbol: SymbolId,
    vol: EwmaVol,
    last_mid: Option<i64>,
    last_sample_ns: Option<u64>,
    start_ns: Option<u64>,
    k_est: KEstimator,
    /// δ (ticks from mid) of the currently resting bid/ask, for k-estimation.
    resting_bid_delta: Option<(i64, u64)>,
    resting_ask_delta: Option<(i64, u64)>,
    liquidated: bool,
}

impl AvellanedaStoikov {
    #[must_use]
    pub fn new(config: AvellanedaStoikovConfig) -> Self {
        Self {
            symbol: SymbolId::new(config.symbol_index),
            vol: EwmaVol::new(config.sigma_lambda),
            config,
            last_mid: None,
            last_sample_ns: None,
            start_ns: None,
            k_est: KEstimator::default(),
            resting_bid_delta: None,
            resting_ask_delta: None,
            liquidated: false,
        }
    }

    /// Current volatility estimate in ticks per √second (floored).
    fn sigma(&self) -> f64 {
        // EwmaVol tracks per-sample return variance in ticks²; the requote
        // cadence sets the sample dt, folded into the σ²·(T−t) term below by
        // treating σ as ticks/√s via the sample spacing.
        self.vol.sigma().max(self.config.sigma_floor)
    }

    fn effective_k(&self) -> f64 {
        self.config
            .k
            .or_else(|| self.k_est.estimate())
            .unwrap_or(self.config.k_fallback)
    }

    fn time_remaining(&self, now_ns: u64) -> f64 {
        let start = self.start_ns.unwrap_or(now_ns);
        let elapsed = (now_ns - start) as f64 / 1e9;
        (self.config.horizon_secs - elapsed).max(0.0)
    }

    fn update_vol(&mut self, mid: i64, now_ns: u64) {
        if let (Some(prev), Some(prev_ns)) = (self.last_mid, self.last_sample_ns)
            && now_ns > prev_ns
        {
            // Return in ticks; EWMA of squared tick-returns.
            let r = (mid - prev) as f64;
            self.vol.update(r);
        }
        self.last_mid = Some(mid);
        self.last_sample_ns = Some(now_ns);
    }

    fn requote(&mut self, api: &mut TradeApi<'_, '_>) {
        let now_ns = api.now().nanos();
        let Some(mid) = api.mid_ticks(self.symbol) else {
            return;
        };
        self.update_vol(mid, now_ns);

        // Account posted exposure to k-estimation before cancelling.
        self.account_exposure(now_ns);
        api.cancel_all(self.symbol);
        self.resting_bid_delta = None;
        self.resting_ask_delta = None;

        let position = api.position(self.symbol);
        let time_left = self.time_remaining(now_ns);

        // Horizon liquidation: unwind inventory with a marketable order.
        if self.config.liquidate_at_end && time_left <= 0.0 {
            if position != 0 && !self.liquidated {
                let side = if position > 0 { Side::Sell } else { Side::Buy };
                let qty = Qty::new(position.unsigned_abs());
                api.submit(
                    self.symbol,
                    side,
                    sim_core::OrderKind::Market,
                    qty,
                    sim_core::TimeInForce::Ioc,
                );
                self.liquidated = true;
            }
            return;
        }

        let gamma = self.config.gamma;
        let sigma = self.sigma();
        let sigma2 = sigma * sigma;
        let k = self.effective_k();
        let q = position as f64;
        let mid_f = mid as f64;

        // Reservation price and optimal half-spread (in ticks). The model can
        // produce arbitrarily wide quotes when σ spikes; a market maker never
        // quotes thousands of ticks off mid, so both the inventory skew and
        // the half-spread are clamped to a sane band relative to mid. The cap
        // is documented as a practical guard, not part of the A-S theory.
        let band = (mid_f * self.config.max_band_frac).max(2.0);
        let skew = (q * gamma * sigma2 * time_left).clamp(-band, band);
        let reservation = mid_f - skew;
        let spread = gamma * sigma2 * time_left + (2.0 / gamma) * libm::log(1.0 + gamma / k);
        let half = (spread / 2.0).clamp(1.0, band);

        let bid_price = (reservation - half).round().clamp(1.0, mid_f + band) as i64;
        let ask_price = (reservation + half).round().clamp(1.0, mid_f + band) as i64;

        let quote_bid = position < self.config.inventory_cap_lots && bid_price >= 1;
        let quote_ask = position > -self.config.inventory_cap_lots && ask_price > bid_price;
        if quote_bid {
            api.submit_limit(
                self.symbol,
                Side::Buy,
                Price::new(bid_price),
                Qty::new(self.config.size_lots),
            );
            self.resting_bid_delta = Some((mid - bid_price, now_ns));
        }
        if quote_ask {
            api.submit_limit(
                self.symbol,
                Side::Sell,
                Price::new(ask_price),
                Qty::new(self.config.size_lots),
            );
            self.resting_ask_delta = Some((ask_price - mid, now_ns));
        }
    }

    /// Credit currently-resting quotes' exposure time to their δ buckets.
    fn account_exposure(&mut self, now_ns: u64) {
        for (delta, since_ns) in [self.resting_bid_delta, self.resting_ask_delta]
            .into_iter()
            .flatten()
        {
            let exposure = (now_ns.saturating_sub(since_ns)) as f64 / 1e9;
            if exposure > 0.0 {
                self.k_est.observe_post(delta, exposure);
            }
        }
    }
}

impl Strategy for AvellanedaStoikov {
    fn on_start(&mut self, api: &mut TradeApi<'_, '_>) {
        self.start_ns = Some(api.now().nanos());
        api.set_timer(
            SimDuration::from_millis(self.config.requote_ms),
            TIMER_REQUOTE,
        );
    }

    fn on_market_data(&mut self, _record: &EventRecord, api: &mut TradeApi<'_, '_>) {
        // Volatility tracks every observed mid move; requoting stays on the
        // timer to bound quote churn.
        if let Some(mid) = api.mid_ticks(self.symbol) {
            let now_ns = api.now().nanos();
            self.update_vol(mid, now_ns);
        }
    }

    fn on_own_fill(&mut self, fill: &OwnFill, api: &mut TradeApi<'_, '_>) {
        // Attribute the fill to its quote distance for k-estimation.
        let delta = match fill.side {
            Side::Buy => self.resting_bid_delta.map(|(d, _)| d),
            Side::Sell => self.resting_ask_delta.map(|(d, _)| d),
        };
        if let Some(d) = delta {
            self.k_est.observe_fill(d);
        }
        self.requote(api);
    }

    fn on_timer(&mut self, token: u64, api: &mut TradeApi<'_, '_>) {
        if token == TIMER_REQUOTE {
            self.requote(api);
            api.set_timer(
                SimDuration::from_millis(self.config.requote_ms),
                TIMER_REQUOTE,
            );
        }
    }

    fn on_end(&mut self, api: &mut TradeApi<'_, '_>) {
        api.cancel_all(self.symbol);
    }
}
