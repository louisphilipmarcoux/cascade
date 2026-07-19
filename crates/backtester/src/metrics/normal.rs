//! Standard normal CDF and inverse CDF, implemented in-tree.
//!
//! Φ uses Hart/West's double-precision rational approximation (~1e-15 abs
//! error); Φ⁻¹ uses Acklam's rational approximation refined by one Halley
//! step against our Φ (machine precision in the bulk). Both are unit-tested
//! against reference values and the Python research layer cross-checks the
//! metrics built on top (≤1e-7 tolerance). No stats-crate dependency: the
//! formulas the honesty claims rest on are readable in this file.

// Coefficients are transcribed verbatim from the published algorithms (West
// 2005; Acklam). Digit separators would invite transcription errors and
// break direct comparison against the papers.
#![allow(clippy::unreadable_literal)]

/// Standard normal CDF Φ(x) (West 2005 double-precision algorithm).
#[must_use]
pub fn norm_cdf(x: f64) -> f64 {
    let z = x.abs();
    let cumulative = if z > 37.0 {
        0.0
    } else {
        let e = libm::exp(-z * z / 2.0);
        if z < 7.071_067_811_865_475 {
            // |x| < 5√2: high-accuracy rational.
            let build =
                (((((3.52624965998911e-02 * z + 0.700383064443688) * z + 6.37396220353165) * z
                    + 33.912866078383)
                    * z
                    + 112.079291497871)
                    * z
                    + 221.213596169931)
                    * z
                    + 220.206867912376;
            let build_d =
                ((((((8.83883476483184e-02 * z + 1.75566716318264) * z + 16.064177579207) * z
                    + 86.7807322029461)
                    * z
                    + 296.564248779674)
                    * z
                    + 637.333633378831)
                    * z
                    + 793.826512519948)
                    * z
                    + 440.413735824752;
            e * build / build_d
        } else {
            // Tail continued fraction.
            let b = z + 0.65;
            let b = z + 4.0 / b;
            let b = z + 3.0 / b;
            let b = z + 2.0 / b;
            let b = z + 1.0 / b;
            e / (b * 2.506628274631)
        }
    };
    if x > 0.0 {
        1.0 - cumulative
    } else {
        cumulative
    }
}

// Acklam's rational approximation coefficients.
const A: [f64; 6] = [
    -3.969683028665376e+01,
    2.209460984245205e+02,
    -2.759285104469687e+02,
    1.38357751867269e+02,
    -3.066479806614716e+01,
    2.506628277459239e+00,
];
const B: [f64; 5] = [
    -5.447609879822406e+01,
    1.615858368580409e+02,
    -1.556989798598866e+02,
    6.680131188771972e+01,
    -1.328068155288572e+01,
];
const C: [f64; 6] = [
    -7.784894002430293e-03,
    -3.223964580411365e-01,
    -2.400758277161838e+00,
    -2.549732539343734e+00,
    4.374664141464968e+00,
    2.938163982698783e+00,
];
const D: [f64; 4] = [
    7.784695709041462e-03,
    3.224671290700398e-01,
    2.445134137142996e+00,
    3.754408661907416e+00,
];
const P_LOW: f64 = 0.02425;

/// Inverse standard normal CDF Φ⁻¹(p) for p ∈ (0, 1).
#[must_use]
pub fn norm_inv_cdf(p: f64) -> f64 {
    assert!(p > 0.0 && p < 1.0, "norm_inv_cdf domain is (0,1), got {p}");

    let x = if p < P_LOW {
        let q = libm::sqrt(-2.0 * libm::log(p));
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = libm::sqrt(-2.0 * libm::log(1.0 - p));
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };

    // One Halley refinement against the accurate CDF.
    let e = norm_cdf(x) - p;
    let u = e * libm::sqrt(2.0 * core::f64::consts::PI) * libm::exp(x * x / 2.0);
    x - u / (1.0 + x * u / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdf_matches_reference_values() {
        // Reference values (scipy.stats.norm.cdf).
        let cases = [
            (0.0, 0.5),
            (1.0, 0.841_344_746_068_542_9),
            (-1.0, 0.158_655_253_931_457_05),
            (1.959_963_984_540_054, 0.975),
            (-2.575_829_303_548_901, 0.005),
            (3.0, 0.998_650_101_968_369_9),
        ];
        for (x, expected) in cases {
            assert!(
                (norm_cdf(x) - expected).abs() < 1e-12,
                "cdf({x}) = {} != {expected}",
                norm_cdf(x)
            );
        }
    }

    #[test]
    fn inverse_round_trips() {
        for p in [0.001, 0.01, 0.05, 0.25, 0.5, 0.75, 0.95, 0.99, 0.999] {
            let x = norm_inv_cdf(p);
            assert!((norm_cdf(x) - p).abs() < 1e-12, "round trip failed at {p}");
        }
        assert!((norm_inv_cdf(0.975) - 1.959_963_984_540_054).abs() < 1e-9);
    }
}
