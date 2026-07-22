//! Reference strategies.
//!
//! - [`naive_mm`]: symmetric quote-around-mid baseline (M5);
//! - M8 adds Avellaneda-Stoikov (with fill-rate calibration), OU pairs
//!   (walk-forward AR(1) refits) and Almgren-Chriss vs TWAP execution.
//!
//! Strategy math is `f64`; conversion to ticks/lots happens at intent time.

pub mod avellaneda_stoikov;
pub mod estimators;
pub mod execution;
pub mod naive_mm;
pub mod ou_pairs;

pub use avellaneda_stoikov::{AvellanedaStoikov, AvellanedaStoikovConfig};
pub use estimators::{EwmaVol, OuFit, fit_ou};
pub use execution::almgren_chriss::{AlmgrenChriss, AlmgrenChrissParams};
pub use execution::twap::{Twap, TwapParams};
pub use naive_mm::{NaiveMm, NaiveMmConfig};
pub use ou_pairs::{OuPairs, OuPairsConfig};
