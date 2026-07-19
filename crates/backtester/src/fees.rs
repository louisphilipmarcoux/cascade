//! Fee model with the system's single rounding rule.

use serde::{Deserialize, Serialize};
use sim_core::{ArithmeticError, Cash};

/// Maker/taker fees as e8-scaled rates (fraction of notional).
/// `2 bps = 0.0002 → 20_000`. Negative maker rate = rebate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FeeModel {
    pub maker_rate_e8: i64,
    pub taker_rate_e8: i64,
}

impl Default for FeeModel {
    fn default() -> Self {
        Self {
            maker_rate_e8: 10_000, // 1 bp
            taker_rate_e8: 50_000, // 5 bps
        }
    }
}

impl FeeModel {
    /// Fee on `notional` (always positive notional); positive result = paid,
    /// negative = rebate received. Rounds toward +∞ — the adverse direction
    /// (pay ≥ true fee, receive ≤ true rebate) — via
    /// [`Cash::checked_mul_rate_e8_ceil`].
    pub fn fee(&self, notional: Cash, is_maker: bool) -> Result<Cash, ArithmeticError> {
        let rate = if is_maker {
            self.maker_rate_e8
        } else {
            self.taker_rate_e8
        };
        notional.checked_mul_rate_e8_ceil(i128::from(rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_rounding_is_adverse_in_both_directions() {
        let fees = FeeModel {
            maker_rate_e8: -10_000,
            taker_rate_e8: 50_000,
        };
        // Taker pays: 33 units * 5bps = 0.0165 units → ceil at e8 scale.
        let notional = Cash::from_e8(33);
        let taker = fees.fee(notional, false).unwrap();
        assert_eq!(taker.e8(), 1, "taker fee rounds up from 0.0000165e8");
        // Maker rebate: -0.0000033e8 → toward +inf = 0 (receives nothing).
        let maker = fees.fee(notional, true).unwrap();
        assert_eq!(maker.e8(), 0, "rebate rounds toward zero magnitude");
    }

    /// P20: |rounded − exact| < 1 e8-unit, and the error is never in the
    /// strategy's favor.
    #[test]
    fn fee_error_bounded_and_adverse() {
        let fees = FeeModel {
            maker_rate_e8: -12_345,
            taker_rate_e8: 67_891,
        };
        for notional_e8 in [1i128, 7, 99, 12_345_678, 987_654_321] {
            for is_maker in [true, false] {
                let rate = if is_maker { -12_345i128 } else { 67_891 };
                let fee = fees.fee(Cash::from_e8(notional_e8), is_maker).unwrap();
                let exact_num = notional_e8 * rate; // exact = exact_num / 1e8
                let rounded_num = fee.e8() * 100_000_000;
                assert!(rounded_num >= exact_num, "must round toward +inf");
                assert!(rounded_num - exact_num < 100_000_000, "within one unit");
            }
        }
    }
}
