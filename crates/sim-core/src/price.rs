//! Fixed-point market quantities.
//!
//! The engine never sees decimals: a [`Price`] is a signed count of *ticks*, a
//! [`Qty`] an unsigned count of *lots*, and [`Cash`] an `i128` amount of quote
//! currency at 1e-8 scale (`_e8`). Conversion between decimals and ticks/lots
//! happens exactly once, at the data/scenario boundary
//! ([`crate::Instrument`]), which makes tick alignment correct by
//! construction.

use serde::{Deserialize, Serialize};

use crate::error::ArithmeticError;

/// Scale factor for [`Cash`]: 1 quote-currency unit == `1e8` cash units.
pub const E8: i128 = 100_000_000;

/// A price expressed as a signed number of ticks.
///
/// Signed so that spreads, deltas and signed depth arithmetic are
/// representable; *order* prices are validated strictly positive at the
/// engine boundary.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Default,
    bincode::Encode,
    bincode::Decode,
)]
#[serde(transparent)]
pub struct Price(i64);

impl Price {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(ticks: i64) -> Self {
        Self(ticks)
    }

    #[must_use]
    pub const fn ticks(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub const fn checked_add_ticks(self, delta: i64) -> Result<Self, ArithmeticError> {
        match self.0.checked_add(delta) {
            Some(v) => Ok(Self(v)),
            None => Err(ArithmeticError::Overflow {
                op: "Price::checked_add_ticks",
            }),
        }
    }

    pub const fn checked_sub(self, other: Self) -> Result<i64, ArithmeticError> {
        match self.0.checked_sub(other.0) {
            Some(v) => Ok(v),
            None => Err(ArithmeticError::Underflow {
                op: "Price::checked_sub",
            }),
        }
    }
}

impl core::fmt::Display for Price {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}t", self.0)
    }
}

/// A quantity expressed as an unsigned number of lots. Zero is representable
/// (e.g. "remaining after full fill") but *order* quantities are validated
/// strictly positive at the engine boundary.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Default,
    bincode::Encode,
    bincode::Decode,
)]
#[serde(transparent)]
pub struct Qty(u64);

impl Qty {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(lots: u64) -> Self {
        Self(lots)
    }

    #[must_use]
    pub const fn lots(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    pub const fn checked_add(self, other: Self) -> Result<Self, ArithmeticError> {
        match self.0.checked_add(other.0) {
            Some(v) => Ok(Self(v)),
            None => Err(ArithmeticError::Overflow {
                op: "Qty::checked_add",
            }),
        }
    }

    pub const fn checked_sub(self, other: Self) -> Result<Self, ArithmeticError> {
        match self.0.checked_sub(other.0) {
            Some(v) => Ok(Self(v)),
            None => Err(ArithmeticError::Underflow {
                op: "Qty::checked_sub",
            }),
        }
    }
}

impl core::fmt::Display for Qty {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}l", self.0)
    }
}

/// Quote-currency cash at 1e-8 scale, stored as `i128`.
///
/// All notional/fee/PnL arithmetic happens in this type; with instrument
/// construction validating that `tick_size × lot_size` is exactly
/// representable at e8 scale, every such computation is exact integer math.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Default,
    bincode::Encode,
    bincode::Decode,
)]
#[serde(transparent)]
pub struct Cash(i128);

impl Cash {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn from_e8(units_e8: i128) -> Self {
        Self(units_e8)
    }

    #[must_use]
    pub const fn e8(self) -> i128 {
        self.0
    }

    pub const fn checked_add(self, other: Self) -> Result<Self, ArithmeticError> {
        match self.0.checked_add(other.0) {
            Some(v) => Ok(Self(v)),
            None => Err(ArithmeticError::Overflow {
                op: "Cash::checked_add",
            }),
        }
    }

    pub const fn checked_sub(self, other: Self) -> Result<Self, ArithmeticError> {
        match self.0.checked_sub(other.0) {
            Some(v) => Ok(Self(v)),
            None => Err(ArithmeticError::Underflow {
                op: "Cash::checked_sub",
            }),
        }
    }

    pub const fn checked_neg(self) -> Result<Self, ArithmeticError> {
        match self.0.checked_neg() {
            Some(v) => Ok(Self(v)),
            None => Err(ArithmeticError::Overflow {
                op: "Cash::checked_neg",
            }),
        }
    }

    /// Multiply by a rate expressed at e8 scale (`rate_e8 / 1e8`), rounding
    /// the result toward **positive infinity**.
    ///
    /// This is the single fee-rounding rule of the whole system, chosen to be
    /// adverse to the strategy: fees paid round up, rebates received round
    /// down (toward zero magnitude). See `docs/ARCHITECTURE.md`.
    pub const fn checked_mul_rate_e8_ceil(self, rate_e8: i128) -> Result<Self, ArithmeticError> {
        match self.0.checked_mul(rate_e8) {
            Some(product) => Ok(Self(div_ceil_i128(product, E8))),
            None => Err(ArithmeticError::Overflow {
                op: "Cash::checked_mul_rate_e8_ceil",
            }),
        }
    }
}

impl core::fmt::Display for Cash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let whole = self.0 / E8;
        let frac = (self.0 % E8).unsigned_abs();
        if frac == 0 {
            write!(f, "{whole}")
        } else {
            let mut frac_str = format!("{frac:08}");
            while frac_str.ends_with('0') {
                frac_str.pop();
            }
            if self.0 < 0 && whole == 0 {
                write!(f, "-0.{frac_str}")
            } else {
                write!(f, "{whole}.{frac_str}")
            }
        }
    }
}

/// Ceiling division for a positive divisor: rounds the quotient toward `+∞`.
const fn div_ceil_i128(a: i128, b: i128) -> i128 {
    debug_assert!(b > 0);
    let q = a / b;
    let r = a % b;
    if r > 0 { q + 1 } else { q }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cash_display_formats_e8() {
        assert_eq!(Cash::from_e8(150_000_000).to_string(), "1.5");
        assert_eq!(Cash::from_e8(-50_000_000).to_string(), "-0.5");
        assert_eq!(Cash::from_e8(200_000_000).to_string(), "2");
        assert_eq!(Cash::from_e8(1).to_string(), "0.00000001");
    }

    #[test]
    fn mul_rate_rounds_toward_positive_infinity() {
        // 3.5 e8-units of fee -> pays 4 (adverse).
        let notional = Cash::from_e8(35);
        assert_eq!(notional.checked_mul_rate_e8_ceil(E8 / 10).unwrap().e8(), 4);
        // Rebate -3.5 -> receives 3 (adverse: toward +inf is -3).
        let rebate = Cash::from_e8(-35);
        assert_eq!(rebate.checked_mul_rate_e8_ceil(E8 / 10).unwrap().e8(), -3);
        // Exact division has no rounding.
        assert_eq!(
            Cash::from_e8(40)
                .checked_mul_rate_e8_ceil(E8 / 10)
                .unwrap()
                .e8(),
            4
        );
    }

    #[test]
    fn qty_checked_sub_underflows() {
        assert!(Qty::new(1).checked_sub(Qty::new(2)).is_err());
        assert_eq!(Qty::new(2).checked_sub(Qty::new(2)).unwrap(), Qty::ZERO);
    }

    #[test]
    fn price_ordering_is_tick_ordering() {
        assert!(Price::new(100) < Price::new(101));
        assert!(Price::new(-1) < Price::new(0));
    }
}
