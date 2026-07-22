//! Almgren–Chriss (2000) optimal execution trajectory.
//!
//! Liquidate `X` lots over `[0, T]` in `N` slices, minimizing
//! `E[cost] + λ·Var[cost]` under linear temporary impact `η` and permanent
//! impact `γ_p`, with price volatility `σ`. The optimal holdings follow the
//! sinh trajectory
//!
//! `x_j = X · sinh(κ(T − t_j)) / sinh(κT)`
//!
//! where `κ` solves `2(cosh(κτ) − 1)/τ² = λσ²/η̃`, `η̃ = η − γ_p·τ/2`, and
//! `τ = T/N`. The per-slice trade is `n_j = x_{j−1} − x_j`. As `λ → 0` this
//! collapses to TWAP (uniform); larger `λ` front-loads to cut risk. The
//! schedule is executed against the real engine as marketable child orders
//! and its implementation shortfall is compared to [`super::twap`] over
//! paired seeds.

use backtester::strategy::{OwnFill, Strategy, TradeApi};
use serde::{Deserialize, Serialize};
use sim_core::{EventRecord, OrderKind, Qty, Side, SimDuration, SymbolId, TimeInForce};

use super::ExecutionConfig;

// `flatten` is incompatible with `deny_unknown_fields` in serde, so this
// struct validates unknown fields structurally rather than via the derive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlmgrenChrissParams {
    #[serde(flatten)]
    pub exec: ExecutionConfig,
    /// Risk aversion λ (≥ 0). 0 → TWAP; larger → front-loaded.
    pub risk_aversion: f64,
    /// Temporary impact coefficient η (price move per lot/sec traded).
    pub eta: f64,
    /// Permanent impact coefficient `gamma_p` (price move per lot).
    pub gamma_permanent: f64,
    /// Price volatility σ (ticks/√s), for the risk term.
    pub sigma: f64,
}

impl Default for AlmgrenChrissParams {
    fn default() -> Self {
        Self {
            exec: ExecutionConfig::default(),
            risk_aversion: 1e-6,
            eta: 1e-4,
            gamma_permanent: 1e-6,
            sigma: 1.0,
        }
    }
}

/// The optimal per-slice trade sizes (lots to execute at each step).
#[must_use]
pub fn schedule(params: &AlmgrenChrissParams) -> Vec<i64> {
    let total = params.exec.target_lots;
    let n = params.exec.slices.max(1) as usize;
    let big_t = params.exec.horizon_secs.max(1e-9);
    let tau = big_t / n as f64;
    let x = total.unsigned_abs() as f64;

    // κ from 2(cosh(κτ)−1)/τ² = λσ²/η̃; η̃ = η − γ_p·τ/2.
    let eta_tilde = params.eta - params.gamma_permanent * tau / 2.0;
    let kappa2 = if eta_tilde > 0.0 {
        params.risk_aversion * params.sigma * params.sigma / eta_tilde
    } else {
        0.0
    };
    let holdings: Vec<f64> = if params.risk_aversion <= 0.0 || kappa2 <= 0.0 {
        // TWAP limit: linear liquidation.
        (0..=n).map(|j| x * (1.0 - j as f64 / n as f64)).collect()
    } else {
        let kappa = libm::sqrt(kappa2);
        let kt = kappa * big_t;
        // Numerically stable holdings: multiply the sinh ratio through by
        // e^{−κT} so every exponent is ≤ 0 (no overflow for large κT, which
        // is the strong-risk-aversion / front-loaded regime).
        //   x_j = X · (e^{−κ t_j} − e^{−κ(2T − t_j)}) / (1 − e^{−2κT})
        let denom = 1.0 - libm::exp(-2.0 * kt);
        if denom < 1e-15 {
            // κT ~ 0: degenerate, fall back to linear.
            (0..=n).map(|j| x * (1.0 - j as f64 / n as f64)).collect()
        } else {
            (0..=n)
                .map(|j| {
                    let t_j = tau * j as f64;
                    let num = libm::exp(-kappa * t_j) - libm::exp(-kappa * (2.0 * big_t - t_j));
                    x * num / denom
                })
                .collect()
        }
    };

    // Per-slice trade = holdings[j-1] − holdings[j], rounded, sign of target.
    let sign = if total >= 0 { 1 } else { -1 };
    let mut trades: Vec<i64> = holdings
        .windows(2)
        .map(|w| ((w[0] - w[1]).round() as i64) * sign)
        .collect();
    // Correct rounding drift so the schedule sums to exactly `total`.
    let scheduled: i64 = trades.iter().sum();
    if let Some(last) = trades.last_mut() {
        *last += total - scheduled;
    }
    trades
}

const TIMER_SLICE: u64 = 1;

#[derive(Debug)]
pub struct AlmgrenChriss {
    params: AlmgrenChrissParams,
    symbol: SymbolId,
    trades: Vec<i64>,
    next_slice: usize,
    slice_dt_ms: i64,
}

impl AlmgrenChriss {
    #[must_use]
    pub fn new(params: AlmgrenChrissParams) -> Self {
        let trades = schedule(&params);
        let slice_dt_ms =
            ((params.exec.horizon_secs / f64::from(params.exec.slices.max(1))) * 1000.0) as i64;
        Self {
            symbol: SymbolId::new(params.exec.symbol_index),
            params,
            trades,
            next_slice: 0,
            slice_dt_ms: slice_dt_ms.max(1),
        }
    }

    fn fire_slice(&mut self, api: &mut TradeApi<'_, '_>) {
        if self.next_slice >= self.trades.len() {
            return;
        }
        let lots = self.trades[self.next_slice];
        self.next_slice += 1;
        if lots == 0 {
            return;
        }
        // Positive target = sell down; negative = buy back.
        let side = if self.params.exec.target_lots >= 0 {
            Side::Sell
        } else {
            Side::Buy
        };
        api.submit(
            self.symbol,
            side,
            OrderKind::Market,
            Qty::new(lots.unsigned_abs()),
            TimeInForce::Ioc,
        );
    }
}

impl Strategy for AlmgrenChriss {
    fn on_start(&mut self, api: &mut TradeApi<'_, '_>) {
        // Wait out the warmup, then fire the first slice on the slice clock.
        let warmup_ms = (self.params.exec.warmup_secs * 1000.0) as i64;
        api.set_timer(SimDuration::from_millis(warmup_ms.max(0)), TIMER_SLICE);
    }

    fn on_market_data(&mut self, _record: &EventRecord, _api: &mut TradeApi<'_, '_>) {}

    fn on_own_fill(&mut self, _fill: &OwnFill, _api: &mut TradeApi<'_, '_>) {}

    fn on_timer(&mut self, token: u64, api: &mut TradeApi<'_, '_>) {
        if token == TIMER_SLICE && self.next_slice < self.trades.len() {
            self.fire_slice(api);
            api.set_timer(SimDuration::from_millis(self.slice_dt_ms), TIMER_SLICE);
        }
    }

    fn on_end(&mut self, _api: &mut TradeApi<'_, '_>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_conserves_total_and_front_loads_with_risk() {
        let base = ExecutionConfig {
            target_lots: 1000,
            slices: 10,
            horizon_secs: 100.0,
            symbol_index: 0,
            warmup_secs: 0.0,
        };
        // λ ≈ 0 → uniform (TWAP-like).
        let twap_like = schedule(&AlmgrenChrissParams {
            exec: base,
            risk_aversion: 0.0,
            ..AlmgrenChrissParams::default()
        });
        assert_eq!(twap_like.iter().sum::<i64>(), 1000);
        assert!(
            twap_like.iter().all(|&t| (t - 100).abs() <= 1),
            "λ=0 must be ~uniform"
        );

        // λ large → front-loaded: first slice > last slice.
        let front = schedule(&AlmgrenChrissParams {
            exec: base,
            risk_aversion: 1e-3,
            eta: 1e-4,
            sigma: 5.0,
            ..AlmgrenChrissParams::default()
        });
        assert_eq!(front.iter().sum::<i64>(), 1000);
        assert!(
            front[0] > front[9],
            "risk aversion must front-load: {} !> {}",
            front[0],
            front[9]
        );
    }

    #[test]
    fn handles_short_liquidation() {
        let trades = schedule(&AlmgrenChrissParams {
            exec: ExecutionConfig {
                target_lots: -500,
                slices: 5,
                horizon_secs: 50.0,
                symbol_index: 0,
                warmup_secs: 0.0,
            },
            ..AlmgrenChrissParams::default()
        });
        assert_eq!(trades.iter().sum::<i64>(), -500);
        assert!(trades.iter().all(|&t| t <= 0));
    }
}
