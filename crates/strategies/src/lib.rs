//! Reference strategies.
//!
//! Lands at M5 (naive MM baseline) and M8 (Avellaneda-Stoikov with fill-rate
//! calibration, OU pairs with walk-forward AR(1) refits, Almgren-Chriss vs
//! TWAP execution). Strategy math is `f64`; conversion to ticks/lots happens
//! at intent time.
