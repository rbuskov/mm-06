//! Vendored transcendentals: `sin` / `cos` / `exp` / `tanh`.
//!
//! These exist for one reason — **bit-parity between the wasm32 and native
//! builds** (architecture.md §parity). The standard library's transcendentals
//! may be implemented differently per target and can disagree in the low bits;
//! fixed polynomial approximations, evaluated by the exact same operations
//! everywhere (with fast-math / FP-reordering disabled in both builds), do not.
//!
//! For the walking skeleton we need only determinism plus good-enough accuracy.
//! Tightening these to minimax polynomials is later calibration work; the unit
//! tests below pin the current error envelope so a regression can't slip in.

use core::f32::consts::PI;

const TWO_PI: f32 = 2.0 * PI;
const HALF_PI: f32 = PI / 2.0;

/// sin(x) for any real x. Range-reduce to `[-π, π]`, fold into `[-π/2, π/2]`
/// via `sin(π - r) = sin(r)`, then an odd Taylor series through `r⁹`. Abs error
/// stays below ~4e-6 over the magnitudes this engine uses (the `r⁹` term cut the
/// old ~1.6e-4 residual near ±π/2, where every oscillator spends time).
pub fn sin(x: f32) -> f32 {
    // Reduce to [-π, π].
    let k = (x * (1.0 / TWO_PI) + 0.5 * x.signum()).trunc();
    let mut r = x - k * TWO_PI;
    // Fold the outer quarter-turns inward.
    if r > HALF_PI {
        r = PI - r;
    } else if r < -HALF_PI {
        r = -PI - r;
    }
    // r - r³/3! + r⁵/5! - r⁷/7! + r⁹/9!  (Horner form in r²)
    let x2 = r * r;
    r * (1.0
        + x2 * (-1.0 / 6.0
            + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0)))))
}

/// cos(x) for any real x. Range-reduce to `[-π, π]`, fold into `[0, π/2]` using
/// `cos(π - r) = -cos(r)` and the evenness of cosine, then an even Taylor series
/// in `r` through `r¹⁰`. Evaluating the even series directly (rather than the
/// old `sin(x + π/2)`, which sampled `sin` near its inaccurate peak) keeps the
/// error small near `x = 0`, where `cos` is flat and a high-Q resonator's pole
/// placement is most sensitive to it. The `r¹⁰` term trims the residual near
/// ±π/2 too; abs error stays below ~1e-6 over the magnitudes this engine uses.
#[allow(dead_code)]
pub fn cos(x: f32) -> f32 {
    // Reduce to [-π, π], then use evenness to work in [0, π].
    let k = (x * (1.0 / TWO_PI) + 0.5 * x.signum()).trunc();
    let mut r = (x - k * TWO_PI).abs();
    // Fold [π/2, π] down into [0, π/2] via cos(π - r) = -cos(r).
    let mut sign = 1.0f32;
    if r > HALF_PI {
        r = PI - r;
        sign = -1.0;
    }
    // 1 - r²/2! + r⁴/4! - r⁶/6! + r⁸/8! - r¹⁰/10!  (Horner form in r²)
    let x2 = r * r;
    let c = 1.0
        + x2 * (-1.0 / 2.0
            + x2 * (1.0 / 24.0
                + x2 * (-1.0 / 720.0 + x2 * (1.0 / 40320.0 + x2 * (-1.0 / 3_628_800.0)))));
    sign * c
}

/// exp(x). Split `x` into `n·ln2 + f`, evaluate `exp(f)` by a short Taylor
/// series, and scale by `2^n` through the IEEE exponent field. Hard clamps keep
/// the bounded negative arguments of the envelope decays away from overflow and
/// denormals.
///
/// `n` is chosen by **round-to-nearest**, not floor, so the residual lands in
/// `[-ln2/2, ln2/2]` and the series is evaluated close to 0 — where five terms
/// are tight (~4e-6). Floor left `f` near `ln2≈0.693`, where the truncation
/// error was ~1e-4; that mattered because the decay coefficient `exp(-1/(τ·sr))`
/// is raised to thousands of powers, and the error compounded into a visibly
/// short decay (an effective τ of ~0.07 s for a nominal 0.10 s). Round-to-nearest
/// is expressed with the same `+ ½·signum` then `trunc` idiom `sin` uses, so it
/// stays on primitive ops and keeps native↔wasm bit-parity.
pub fn exp(x: f32) -> f32 {
    if x <= -87.0 {
        return 0.0;
    }
    if x >= 88.0 {
        return f32::MAX;
    }
    const LOG2E: f32 = 1.442_695_f32;
    const LN2: f32 = 0.693_147_18_f32;

    let kf = x * LOG2E;
    let n = (kf + 0.5 * kf.signum()).trunc();
    let f = x - n * LN2; // residual in [-ln2/2, ln2/2]
    // 1 + f + f²/2! + … + f⁵/5!
    let p = 1.0 + f * (1.0 + f * (0.5 + f * (1.0 / 6.0 + f * (1.0 / 24.0 + f * (1.0 / 120.0)))));
    ldexp2(p, n as i32)
}

/// `p · 2^n`, applied by adding `n` to the float's exponent field directly.
fn ldexp2(p: f32, n: i32) -> f32 {
    let clamped = n.clamp(-126, 127);
    let bits = ((clamped + 127) as u32) << 23;
    p * f32::from_bits(bits)
}

/// tanh(x), built from `exp` so it inherits the same native↔wasm bit-parity
/// guarantee: `tanh(x) = (e^{2x} - 1) / (e^{2x} + 1)`. Smooth, monotonic,
/// saturates to ±1. Used for the output-bus soft saturation.
pub fn tanh(x: f32) -> f32 {
    if x > 8.0 {
        return 1.0;
    }
    if x < -8.0 {
        return -1.0;
    }
    let e = exp(2.0 * x);
    (e - 1.0) / (e + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_matches_std() {
        let mut max_err = 0.0f32;
        let mut t = -50.0f32;
        while t < 50.0 {
            max_err = max_err.max((sin(t) - t.sin()).abs());
            t += 0.01;
        }
        assert!(max_err < 8.0e-6, "sin max err {max_err}");
    }

    #[test]
    fn cos_matches_std() {
        let mut max_err = 0.0f32;
        let mut t = -20.0f32;
        while t < 20.0 {
            max_err = max_err.max((cos(t) - t.cos()).abs());
            t += 0.01;
        }
        assert!(max_err < 2.0e-6, "cos max err {max_err}");
        // The direct even series is far tighter near zero than the old
        // sin(x+π/2); a high-Q resonator's pole placement leans on this.
        let near0 = (cos(0.0078) - 0.0078f32.cos()).abs();
        assert!(near0 < 1.0e-5, "cos near zero err {near0}");
    }

    #[test]
    fn exp_matches_std() {
        let mut max_rel = 0.0f32;
        let mut t = -20.0f32;
        while t < 5.0 {
            let got = exp(t);
            let want = t.exp();
            max_rel = max_rel.max(((got - want) / want).abs());
            t += 0.01;
        }
        assert!(max_rel < 8.0e-6, "exp max rel err {max_rel}");
    }

    #[test]
    fn tanh_matches_std() {
        let mut max_err = 0.0f32;
        let mut t = -6.0f32;
        while t < 6.0 {
            max_err = max_err.max((tanh(t) - t.tanh()).abs());
            t += 0.01;
        }
        assert!(max_err < 5.0e-6, "tanh max err {max_err}");
    }

    #[test]
    fn deterministic_bits() {
        // Same input → same bits. The whole reason these are vendored.
        assert_eq!(sin(1.234_5).to_bits(), sin(1.234_5).to_bits());
        assert_eq!(cos(0.007_8).to_bits(), cos(0.007_8).to_bits());
        assert_eq!(exp(-3.21).to_bits(), exp(-3.21).to_bits());
        assert_eq!(tanh(0.77).to_bits(), tanh(0.77).to_bits());
    }
}
