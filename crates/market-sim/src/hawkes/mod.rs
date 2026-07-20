//! Multivariate exponential-kernel Hawkes processes: simulation (Ogata
//! thinning) and point-MLE fitting (recursive log-likelihood, deterministic
//! Nelder–Mead).
//!
//! `lambda_m(t) = mu_m + sum_n alpha_{mn} sum_{t_i^n < t} exp(-beta (t - t_i^n))`
//!
//! Stability requires the spectral radius of the branching matrix
//! `G = alpha/beta` to be < 1; construction and fitting both enforce it. The
//! Python research layer fits the same likelihood independently — a
//! cross-implementation agreement test holds the two to the same numbers.

pub mod fit;
pub mod params_io;
pub mod simulate;

pub use fit::{FitResult, fit_exp_mle};
pub use params_io::{HawkesParamsFile, read_params, write_params};
pub use simulate::HawkesProcess;

/// Spectral radius of a small square matrix (power iteration; exact closed
/// form for 2×2). Elements are nonnegative in a branching matrix.
#[must_use]
pub fn spectral_radius(matrix: &[Vec<f64>]) -> f64 {
    let dim = matrix.len();
    if dim == 2 {
        // Largest |eigenvalue| of [[a,b],[c,d]].
        let (a, b) = (matrix[0][0], matrix[0][1]);
        let (c, d) = (matrix[1][0], matrix[1][1]);
        let trace = a + d;
        let det = a * d - b * c;
        let disc = trace * trace - 4.0 * det;
        if disc >= 0.0 {
            let root = libm::sqrt(disc);
            f64::midpoint(trace, root)
                .abs()
                .max(f64::midpoint(trace, -root).abs())
        } else {
            libm::sqrt(det.abs())
        }
    } else {
        // Deterministic power iteration.
        let mut vector = vec![1.0; dim];
        let mut radius = 0.0;
        for _ in 0..200 {
            let mut next = vec![0.0; dim];
            for (i, row) in matrix.iter().enumerate() {
                next[i] = row.iter().zip(&vector).map(|(entry, x)| entry * x).sum();
            }
            radius = next.iter().fold(0.0f64, |acc, x| acc.max(x.abs()));
            if radius == 0.0 {
                return 0.0;
            }
            for x in &mut next {
                *x /= radius;
            }
            vector = next;
        }
        radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectral_radius_2x2_matches_known() {
        // [[0.5, 0.2], [0.1, 0.4]] → eigenvalues (0.9 ± sqrt(0.01+0.08))/2.
        let matrix = vec![vec![0.5, 0.2], vec![0.1, 0.4]];
        let expected = f64::midpoint(0.9, libm::sqrt(0.09));
        assert!((spectral_radius(&matrix) - expected).abs() < 1e-12);
    }
}
