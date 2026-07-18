//! Virtual time.
//!
//! The simulation clock is a plain `u64` count of nanoseconds since the sim
//! epoch (t = 0). There is no wall clock anywhere in the simulation
//! (`clippy::disallowed_methods` bans it); historical replay maps real epoch
//! timestamps to sim offsets at the data boundary and records the mapping in
//! the run report.

use serde::{Deserialize, Serialize};

use crate::error::ArithmeticError;

pub const NANOS_PER_MICRO: u64 = 1_000;
pub const NANOS_PER_MILLI: u64 = 1_000_000;
pub const NANOS_PER_SEC: u64 = 1_000_000_000;

/// A point in virtual time: nanoseconds since the sim epoch.
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
pub struct SimTime(u64);

impl SimTime {
    pub const EPOCH: Self = Self(0);

    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros * NANOS_PER_MICRO)
    }

    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis * NANOS_PER_MILLI)
    }

    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs * NANOS_PER_SEC)
    }

    #[must_use]
    pub const fn nanos(self) -> u64 {
        self.0
    }

    pub const fn checked_add(self, d: SimDuration) -> Result<Self, ArithmeticError> {
        let nanos = d.nanos();
        if nanos >= 0 {
            match self.0.checked_add(nanos.unsigned_abs()) {
                Some(v) => Ok(Self(v)),
                None => Err(ArithmeticError::Overflow {
                    op: "SimTime::checked_add",
                }),
            }
        } else {
            match self.0.checked_sub(nanos.unsigned_abs()) {
                Some(v) => Ok(Self(v)),
                None => Err(ArithmeticError::Underflow {
                    op: "SimTime::checked_add",
                }),
            }
        }
    }

    /// Time elapsed since `earlier`.
    pub const fn checked_since(self, earlier: Self) -> Result<SimDuration, ArithmeticError> {
        match self.0.checked_sub(earlier.0) {
            Some(v) if v <= i64::MAX as u64 => Ok(SimDuration::from_nanos(v as i64)),
            _ => Err(ArithmeticError::Underflow {
                op: "SimTime::checked_since",
            }),
        }
    }
}

impl core::fmt::Display for SimTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let secs = self.0 / NANOS_PER_SEC;
        let frac = self.0 % NANOS_PER_SEC;
        write!(f, "{secs}.{frac:09}s")
    }
}

/// A signed span of virtual time in nanoseconds.
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
pub struct SimDuration(i64);

impl SimDuration {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn from_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    #[must_use]
    pub const fn from_micros(micros: i64) -> Self {
        Self(micros * NANOS_PER_MICRO as i64)
    }

    #[must_use]
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis * NANOS_PER_MILLI as i64)
    }

    #[must_use]
    pub const fn from_secs(secs: i64) -> Self {
        Self(secs * NANOS_PER_SEC as i64)
    }

    #[must_use]
    pub const fn nanos(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub const fn checked_add(self, other: Self) -> Result<Self, ArithmeticError> {
        match self.0.checked_add(other.0) {
            Some(v) => Ok(Self(v)),
            None => Err(ArithmeticError::Overflow {
                op: "SimDuration::checked_add",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_since_round_trip() {
        let t0 = SimTime::from_secs(5);
        let t1 = t0.checked_add(SimDuration::from_millis(250)).unwrap();
        assert_eq!(t1.nanos(), 5_250_000_000);
        assert_eq!(t1.checked_since(t0).unwrap(), SimDuration::from_millis(250));
        assert!(t0.checked_since(t1).is_err());
    }

    #[test]
    fn negative_duration_moves_backwards() {
        let t = SimTime::from_secs(1);
        let back = t.checked_add(SimDuration::from_millis(-100)).unwrap();
        assert_eq!(back.nanos(), 900_000_000);
        assert!(
            SimTime::EPOCH
                .checked_add(SimDuration::from_nanos(-1))
                .is_err()
        );
    }

    #[test]
    fn display_is_stable() {
        assert_eq!(SimTime::from_millis(1500).to_string(), "1.500000000s");
    }
}
