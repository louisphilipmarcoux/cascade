use thiserror::Error;

/// Checked fixed-point arithmetic failure.
///
/// Public arithmetic on [`crate::Price`], [`crate::Qty`] and [`crate::Cash`]
/// returns `Result<_, ArithmeticError>` instead of panicking; release builds
/// additionally keep `overflow-checks = true` as a backstop for any internal
/// arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ArithmeticError {
    #[error("integer overflow in {op}")]
    Overflow { op: &'static str },
    #[error("integer underflow in {op}")]
    Underflow { op: &'static str },
    #[error("division by zero in {op}")]
    DivideByZero { op: &'static str },
}

/// Errors constructing core domain values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoreError {
    #[error("arithmetic: {0}")]
    Arithmetic(#[from] ArithmeticError),
    #[error("invalid instrument {symbol:?}: {reason}")]
    InvalidInstrument { symbol: String, reason: String },
}
