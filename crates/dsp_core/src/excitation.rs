//! Trigger / excitation generator (audio-engine-spec.md §3.2, §2.3).
//!
//! Shapes the raw step trigger into the short, band-limited pulse (~1 ms in
//! hardware) that pings the bridged-T resonators and opens the noise/metallic
//! gates. On [`Excitation::trigger`] it emits a single pulse — successive
//! [`Excitation::tick`] calls walk through the pulse and then return exact zero.
//!
//! **Band-limited by construction.** Rather than a raw step (whose discontinuity
//! aliases — spec §2.2/§8), the pulse is a **raised-cosine / Hann window**:
//! `0.5·(1 − cos(2π·t/T))` over one period `T ≈ 1 ms`. Both its endpoints and
//! their first derivatives are zero, so there is no hard edge to alias — the
//! equivalent of BLEP/BLAMP edge handling, but baked into the shape. The
//! spectral energy near Nyquist sits far below a naive hard step (see tests).
//!
//! **Accent.** [`Excitation::trigger`] takes an `accent_amplitude` that scales
//! the pulse peak linearly: larger accent ⇒ larger pulse ⇒ (downstream) larger
//! initial resonator amplitude and longer ring (spec §2.3, §3.1). Monotonic.
//!
//! **Deterministic.** No internal RNG — the same trigger at the same point
//! yields byte-identical output. Hit-to-hit variance is §10's job, applied
//! explicitly upstream by scaling `accent_amplitude` / timing, never modelled
//! here as trigger sloppiness (spec §3.2).
//!
//! Uses [`crate::mathx`] for every transcendental, preserving native↔wasm
//! bit-parity (architecture.md §parity).

use crate::mathx;

/// Nominal pulse width, in seconds. Hardware trigger ≈ 1 ms (spec §2.3). The
/// width in *samples* scales with sample rate so the pulse holds ≈1 ms in time
/// at any host rate.
const PULSE_WIDTH_S: f32 = 0.001;

const TAU: f32 = core::f32::consts::TAU;

/// The ~1 ms band-limited excitation pulse generator (spec §3.2).
///
/// One instance per excitation site (e.g. shared across the two BD resonators so
/// they ping from the *same* edge — spec §3.1/§4.1; the caller fans the single
/// output out to its consumers). Construct with [`Excitation::new`], fire with
/// [`Excitation::trigger`], pull samples with [`Excitation::tick`].
#[derive(Clone)]
pub struct Excitation {
    /// Per-sample phase increment, `1 / width_samples`. Pure function of the
    /// host rate, so the width-in-time is rate-independent and deterministic.
    phase_inc: f32,
    /// Pulse position in `[0, 1)`; `>= 1.0` means idle (no pulse in flight).
    phase: f32,
    /// Peak amplitude of the pulse in flight (set from `accent_amplitude`).
    amp: f32,
}

impl Excitation {
    /// Build an excitation generator for `sample_rate` (Hz). Idle until
    /// triggered. The pulse width in samples is `sample_rate · PULSE_WIDTH_S`,
    /// so the pulse spans ≈1 ms in time at any rate.
    pub fn new(sample_rate: f32) -> Self {
        // Guard against degenerate rates so phase_inc stays finite and the pulse
        // spans at least one sample.
        let width_samples = (sample_rate * PULSE_WIDTH_S).max(1.0);
        Self {
            phase_inc: 1.0 / width_samples,
            phase: 1.0, // idle
            amp: 0.0,
        }
    }

    /// Start a pulse. `accent_amplitude` scales the peak linearly (monotonic):
    /// larger accent ⇒ larger pulse (spec §2.3). Restarts cleanly if a pulse is
    /// already in flight, so retriggers are phase-locked to the hit (spec §3.1).
    /// Negative amplitudes are clamped to zero.
    pub fn trigger(&mut self, accent_amplitude: f32) {
        self.phase = 0.0;
        self.amp = accent_amplitude.max(0.0);
    }

    /// Next sample of the pulse, then exact `0.0` once it has elapsed.
    ///
    /// The shape is a single raised-cosine (Hann) lobe
    /// `amp · 0.5·(1 − cos(2π·phase))` over `phase ∈ [0, 1)`: zero-valued and
    /// zero-sloped at both ends, hence band-limited (no aliasing edge). Tails
    /// flush to exact zero (spec §8).
    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.phase >= 1.0 {
            return 0.0; // idle: exact zero tail
        }
        let v = self.amp * 0.5 * (1.0 - mathx::cos(TAU * self.phase));
        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            // Pulse just finished — clamp to idle and drop the amplitude so the
            // next tick returns exact zero.
            self.phase = 1.0;
            self.amp = 0.0;
        }
        v
    }

    /// Return to the idle state (no pulse in flight); preserves the configured
    /// sample rate. A ticked-from-reset instance matches a freshly constructed
    /// one (spec §3.2 cross-cutting "Reset").
    ///
    /// Part of the shared-block API (and exercised by this module's tests); the
    /// BD assembly retriggers rather than resetting, so the engine never calls
    /// it yet — kept for the choke / re-arm paths of later voices (spec §9).
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.phase = 1.0;
        self.amp = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pull a whole pulse (plus a little tail) into a buffer.
    fn render_pulse(sr: f32, accent: f32, n: usize) -> Vec<f32> {
        let mut e = Excitation::new(sr);
        e.trigger(accent);
        (0..n).map(|_| e.tick()).collect()
    }

    /// Peak absolute value of a buffer.
    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    #[test]
    fn pulse_width_is_about_one_ms_across_rates() {
        // The non-negligible span (samples above 1% of peak) should correspond
        // to ≈1 ms in time at every host rate.
        for &sr in &[44_100.0f32, 48_000.0, 96_000.0] {
            let buf = render_pulse(sr, 1.0, sr as usize); // 1 s of room
            let pk = peak(&buf);
            assert!(pk > 0.0, "no pulse at {sr} Hz");
            let thresh = 0.01 * pk;
            let span = buf.iter().filter(|&&s| s.abs() > thresh).count();
            let span_ms = 1000.0 * span as f32 / sr;
            assert!(
                (span_ms - 1.0).abs() < 0.2,
                "pulse span {span_ms:.3} ms at {sr} Hz (want ≈1 ms)"
            );
        }
    }

    #[test]
    fn pulse_returns_to_exact_zero() {
        // After the lobe elapses, every subsequent tick is bit-exact zero.
        let sr = 48_000.0;
        let width = (sr * PULSE_WIDTH_S) as usize;
        let buf = render_pulse(sr, 0.7, width + 200);
        for (i, &s) in buf.iter().enumerate().skip(width + 5) {
            assert_eq!(s.to_bits(), 0.0f32.to_bits(), "non-zero tail at sample {i}: {s}");
        }
    }

    #[test]
    fn amplitude_scales_monotonically_with_accent() {
        let sr = 48_000.0;
        let n = (sr * PULSE_WIDTH_S) as usize + 50;
        let mut last = -1.0f32;
        for &a in &[0.0f32, 0.25, 0.5, 0.75, 1.0, 2.0] {
            let pk = peak(&render_pulse(sr, a, n));
            assert!(pk > last, "peak not increasing at accent {a}: {pk} <= {last}");
            last = pk;
        }
        // Linear in accent: peak of the Hann lobe is the amplitude itself.
        let pk1 = peak(&render_pulse(sr, 1.0, n));
        let pk2 = peak(&render_pulse(sr, 2.0, n));
        assert!((pk2 - 2.0 * pk1).abs() < 1e-3, "accent not linear: {pk1} -> {pk2}");
    }

    #[test]
    fn negative_accent_clamps_to_silence() {
        let sr = 48_000.0;
        let n = (sr * PULSE_WIDTH_S) as usize + 20;
        let buf = render_pulse(sr, -1.0, n);
        assert_eq!(peak(&buf), 0.0, "negative accent produced output");
    }

    #[test]
    fn deterministic_hit_to_hit() {
        // Two identical triggers → byte-identical sample streams.
        let sr = 48_000.0;
        let n = (sr * PULSE_WIDTH_S) as usize + 100;
        let a: Vec<u32> = render_pulse(sr, 0.83, n).iter().map(|x| x.to_bits()).collect();
        let b: Vec<u32> = render_pulse(sr, 0.83, n).iter().map(|x| x.to_bits()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn retrigger_restarts_pulse() {
        // Firing mid-pulse restarts cleanly from the top of the lobe.
        let sr = 48_000.0;
        let mut e = Excitation::new(sr);
        e.trigger(1.0);
        let _ = e.tick();
        let _ = e.tick();
        e.trigger(1.0);
        // First sample after a fresh trigger is the lobe start (≈0).
        let first = e.tick();
        assert!(first.abs() < 1e-2, "retrigger did not restart at lobe base: {first}");
    }

    #[test]
    fn reset_matches_fresh_instance() {
        let sr = 48_000.0;
        let n = (sr * PULSE_WIDTH_S) as usize + 50;
        let mut e = Excitation::new(sr);
        e.trigger(1.0);
        for _ in 0..10 {
            let _ = e.tick(); // dirty the state mid-pulse
        }
        e.reset();
        e.trigger(0.5);
        let after: Vec<u32> = (0..n).map(|_| e.tick().to_bits()).collect();

        let mut fresh = Excitation::new(sr);
        fresh.trigger(0.5);
        let reference: Vec<u32> = (0..n).map(|_| fresh.tick().to_bits()).collect();
        assert_eq!(after, reference, "reset state diverged from a fresh instance");
    }

    /// Naive DFT magnitude at integer bin `k` of `buf` (length `n`). Slow but
    /// dependency-free; the buffers here are short. Uses `mathx` so it stays
    /// deterministic, though only relative magnitudes matter for the assertion.
    fn dft_mag(buf: &[f32], k: usize) -> f32 {
        let n = buf.len();
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (i, &s) in buf.iter().enumerate() {
            let ang = -TAU * (k as f32) * (i as f32) / (n as f32);
            re += s * mathx::cos(ang);
            im += s * mathx::sin(ang);
        }
        (re * re + im * im).sqrt()
    }

    /// Total spectral energy in the high band `[from_bin, n/2]`.
    fn high_band_energy(buf: &[f32], from_bin: usize) -> f32 {
        let half = buf.len() / 2;
        (from_bin..=half).map(|k| dft_mag(buf, k).powi(2)).sum()
    }

    #[test]
    fn band_limited_alias_floor() {
        // FFT the pulse and assert the energy near Nyquist is far below a naive
        // hard step of equivalent width/area — the edge is band-limited, not a
        // raw step (spec §2.2/§8).
        let sr = 48_000.0;
        let n = 1024usize;
        let width = (sr * PULSE_WIDTH_S) as usize;

        // Our band-limited Hann pulse.
        let mut e = Excitation::new(sr);
        e.trigger(1.0);
        let pulse: Vec<f32> = (0..n).map(|_| e.tick()).collect();

        // A naive hard rectangular pulse of the same width and unit height —
        // the thing we are NOT (sharp edges, rich high-frequency content).
        let mut step = vec![0.0f32; n];
        for s in step.iter_mut().take(width) {
            *s = 1.0;
        }

        // High band = upper quarter of the spectrum, up toward Nyquist.
        let from = n * 3 / 8;
        let total = |b: &[f32]| (0..=b.len() / 2).map(|k| dft_mag(b, k).powi(2)).sum::<f32>();

        let pulse_high = high_band_energy(&pulse, from);
        let step_high = high_band_energy(&step, from);

        let pulse_frac = pulse_high / total(&pulse);
        let step_frac = step_high / total(&step);

        // Absolute floor: almost no energy lives up near Nyquist.
        assert!(
            pulse_frac < 1e-4,
            "high-band energy fraction {pulse_frac:e} exceeds alias floor"
        );
        // And we are markedly cleaner than the naive hard step.
        assert!(
            pulse_frac < step_frac * 1e-2,
            "pulse not appreciably cleaner than a hard step ({pulse_frac:e} vs {step_frac:e})"
        );
    }
}
