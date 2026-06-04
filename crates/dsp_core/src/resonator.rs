//! Bridged-T (Twin-T) resonator — the core tonal block (audio-engine-spec.md
//! §3.1).
//!
//! A real bridged-T network is a resonant band-pass; pinged by a short pulse it
//! rings as a naturally decaying sine whose decay rate is set by its Q
//! (`Q = sqrt(R1/R2) / (sqrt(C1/C2) + sqrt(C2/C1))`). This is the Tier-A model:
//! a high-Q RBJ band-pass biquad, *excited on trigger* so the ring is
//! phase-locked to the hit (the tonal-voice analog of "phase reset", §2.3). The
//! bass drum (§4.1) instances two of these sharing one excitation edge; the
//! snare and toms instance one each.
//!
//! Parity note: every transcendental goes through [`mathx`] — the same vendored
//! polynomials native and wasm — so two builds ring bit-identically.

use crate::mathx;

const TAU: f32 = core::f32::consts::TAU;

/// A high-Q band-pass biquad pinged on trigger — the decaying-sine "ring" that
/// every tonal voice is built from.
///
/// Constant-skirt-gain RBJ band-pass (peak gain = Q), evaluated in transposed
/// direct-form II so a single excitation impulse becomes the filter's impulse
/// response: a sine at `freq_hz` decaying at a rate set by `q`. Higher Q ⇒
/// narrower band ⇒ longer ring.
#[derive(Clone)]
pub struct Resonator {
    // Feed-forward / feedback coefficients (b0, b2 only; b1 = 0 for a BPF).
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    // Transposed-DF-II state.
    z1: f32,
    z2: f32,
}

impl Resonator {
    /// Build a resonator centred at `freq_hz` with quality factor `q`, designed
    /// for `sample_rate` Hz. `freq_hz` is clamped below Nyquist and `q` to a
    /// sane positive floor so the design stays stable for any caller input.
    pub fn new(sample_rate: f32, freq_hz: f32, q: f32) -> Self {
        let mut r = Self {
            b0: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        };
        r.set(sample_rate, freq_hz, q);
        r
    }

    /// (Re)design the coefficients. Keeps the filter inside the stable region:
    /// `freq_hz` is held within `(0, Nyquist)` and `q` to `>= 0.5`.
    fn set(&mut self, sample_rate: f32, freq_hz: f32, q: f32) {
        let sr = sample_rate.max(1.0);
        // Keep the centre strictly below Nyquist; the upper guard avoids the
        // cot/tan blow-up of the bilinear transform near fs/2. At absurdly low
        // sample rates `sr*0.49` can fall below the 1 Hz floor, so take the
        // upper bound as the larger of the two to keep `clamp` well-ordered
        // (a degenerate-rate guard; the design stays finite, just detuned).
        let hi = (sr * 0.49).max(1.0);
        let f = freq_hz.clamp(1.0, hi);
        let q = q.max(0.5);

        // RBJ band-pass (constant skirt gain, peak gain = Q).
        let w0 = TAU * f / sr;
        let cos_w0 = mathx::cos(w0);
        let sin_w0 = mathx::sin(w0);
        let alpha = sin_w0 / (2.0 * q);

        // Unnormalised biquad terms.
        //   b0 =  sin(w0)/2 ,  b1 = 0 ,  b2 = -sin(w0)/2
        //   a0 =  1 + alpha ,  a1 = -2 cos(w0) ,  a2 = 1 - alpha
        let a0 = 1.0 + alpha;
        let inv_a0 = 1.0 / a0;
        self.b0 = (sin_w0 * 0.5) * inv_a0;
        self.b2 = -(sin_w0 * 0.5) * inv_a0;
        self.a1 = (-2.0 * cos_w0) * inv_a0;
        self.a2 = (1.0 - alpha) * inv_a0;
    }

    /// Ping the resonator: inject an excitation impulse of size `amplitude` and
    /// reset the ring state so the response is deterministic and phase-locked to
    /// the hit. A larger `amplitude` ⇒ a larger initial swing ⇒ a longer audible
    /// ring — the accent mechanism (§2.3, §3.1).
    pub fn trigger(&mut self, amplitude: f32) {
        // Reset state, then run one impulse `amplitude` through transposed DF-II.
        // For an impulse input x[0] = amplitude (subsequent x = 0), the standard
        // TDF-II update with zero initial state gives:
        //   y[0]  = b0 * amplitude
        //   z1    = b1*x - a1*y = -a1*y0        (b1 = 0)
        //   z2    = b2*x - a2*y =  b2*amplitude - a2*y0
        // Seeding the state this way (rather than feeding a sample) keeps the
        // very next `tick()` producing the impulse response, phase-locked.
        let y0 = self.b0 * amplitude;
        self.z1 = -self.a1 * y0;
        self.z2 = self.b2 * amplitude - self.a2 * y0;
    }

    /// Advance one sample of free ring (no input) and return the output.
    /// Denormal-clamped; flushes the state to exact zero once the ring has
    /// decayed into the noise floor, mirroring the `Bleep` tail flush.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        // Transposed direct-form II with x = 0 (free ringing after the ping):
        //   y  = b0 * x + z1                 = z1
        //   z1 = b1 * x - a1 * y + z2        = -a1*y + z2
        //   z2 = b2 * x - a2 * y             = -a2*y
        let y = self.z1;
        self.z1 = -self.a1 * y + self.z2;
        self.z2 = -self.a2 * y;

        // Flush a decayed-out ring to exact zero so tails don't dribble
        // denormals forever (§8). The band-pass has no DC term, so once both
        // state words are sub-threshold the ring is effectively over.
        if self.z1.abs() < 1.0e-8 && self.z2.abs() < 1.0e-8 {
            self.z1 = 0.0;
            self.z2 = 0.0;
        }
        y
    }

    /// Clear the ring state (silence). Coefficients are preserved, so the next
    /// `trigger` rings identically to a fresh instance of the same design.
    ///
    /// Shared-block API (exercised by this module's tests); the BD always
    /// retriggers (which re-seeds the state), so the engine never silences a
    /// live ring yet — kept for the choke paths of later voices (spec §9).
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Render `n` samples of the ring after a unit ping.
    fn ring(freq: f32, q: f32, n: usize) -> Vec<f32> {
        let mut r = Resonator::new(SR, freq, q);
        r.trigger(1.0);
        (0..n).map(|_| r.tick()).collect()
    }

    /// Estimate the dominant frequency of a buffer by counting positive-going
    /// zero crossings over its (non-silent) span.
    fn dominant_freq_zc(buf: &[f32]) -> f32 {
        // Only look while the ring is meaningfully above the floor, so the
        // flushed-to-zero tail doesn't dilute the estimate.
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let thresh = peak * 1.0e-3;
        let mut last = None;
        let mut first_cross = None;
        let mut last_cross = None;
        let mut crossings = 0u32;
        for (i, w) in buf.windows(2).enumerate() {
            if w[1].abs() < thresh && w[0].abs() < thresh {
                continue;
            }
            // positive-going zero crossing
            if w[0] <= 0.0 && w[1] > 0.0 {
                if first_cross.is_none() {
                    first_cross = Some(i);
                }
                last_cross = Some(i);
                crossings += 1;
                last = Some(i);
            }
        }
        let _ = last;
        let (Some(a), Some(b)) = (first_cross, last_cross) else {
            return 0.0;
        };
        if crossings < 2 || b == a {
            return 0.0;
        }
        // (crossings - 1) full periods span (b - a) samples.
        let periods = (crossings - 1) as f32;
        let span = (b - a) as f32;
        periods * SR / span
    }

    /// Sample index where the energy envelope first falls to 1/e of its peak.
    fn decay_samples_to_1_over_e(buf: &[f32]) -> usize {
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let target = peak / core::f32::consts::E;
        // Walk a short sliding-max envelope so we ignore the per-sample sine
        // zero-crossings and track the decay of the amplitude envelope.
        let win = 64usize;
        let mut peak_idx = 0usize;
        for i in 0..buf.len() {
            let lo = i.saturating_sub(win);
            let hi = (i + win).min(buf.len());
            let env = buf[lo..hi].iter().fold(0.0f32, |m, s| m.max(s.abs()));
            if (buf[i].abs() - peak).abs() < 1e-12 {
                peak_idx = i;
            }
            if i > peak_idx && env <= target {
                return i;
            }
            let _ = env;
        }
        buf.len()
    }

    #[test]
    fn centre_frequency_matches_design() {
        for &(f, q) in &[(60.0f32, 7.0f32), (130.0, 3.0), (440.0, 20.0), (1000.0, 10.0)] {
            let buf = ring(f, q, (SR as usize) / 2);
            let est = dominant_freq_zc(&buf);
            let rel = (est - f).abs() / f;
            assert!(rel < 0.05, "freq {f}Hz Q{q}: estimated {est}Hz (rel err {rel})");
        }
    }

    #[test]
    fn decay_time_tracks_q() {
        // Same centre, increasing Q ⇒ strictly longer ring.
        let f = 200.0;
        let n = SR as usize; // 1 s
        let d_lo = decay_samples_to_1_over_e(&ring(f, 4.0, n));
        let d_mid = decay_samples_to_1_over_e(&ring(f, 16.0, n));
        let d_hi = decay_samples_to_1_over_e(&ring(f, 64.0, n));
        assert!(d_lo < d_mid, "decay did not grow with Q: {d_lo} !< {d_mid}");
        assert!(d_mid < d_hi, "decay did not grow with Q: {d_mid} !< {d_hi}");
    }

    #[test]
    fn bandwidth_narrows_with_q() {
        // −3 dB bandwidth of a band-pass is roughly f0/Q, so a higher Q gives a
        // narrower band. Measure the magnitude response by driving a sine at a
        // few offsets from centre and comparing the on-centre vs. off-centre
        // steady-state amplitude. We assert the relative width shrinks with Q by
        // proxy of the decay time (longer ring = narrower band), which the
        // physics ties together; here we cross-check directly via the impulse
        // response energy spread.
        let f = 500.0;
        let n = SR as usize;
        // Ringing energy concentration: a narrower band rings longer, so the
        // fraction of total energy in the first 50 ms is *smaller* for high Q.
        let early_fraction = |q: f32| -> f32 {
            let buf = ring(f, q, n);
            let early = (SR as usize) / 20; // 50 ms
            let e_all: f32 = buf.iter().map(|s| s * s).sum();
            let e_early: f32 = buf[..early.min(buf.len())].iter().map(|s| s * s).sum();
            e_early / e_all.max(1e-30)
        };
        let lo = early_fraction(3.0);
        let hi = early_fraction(40.0);
        // High Q spreads its energy over a longer tail ⇒ a *lower* early
        // fraction. (Equivalently, narrower bandwidth.)
        assert!(hi < lo, "higher Q did not narrow the band: early frac {lo} -> {hi}");
    }

    #[test]
    fn high_q_is_stable() {
        // Very high Q over a long render must never blow up.
        let mut r = Resonator::new(SR, 80.0, 80.0);
        r.trigger(4.0); // a hard, accented ping
        for i in 0..(SR as usize * 4) {
            let s = r.tick();
            assert!(s.is_finite(), "non-finite sample at {i}: {s}");
            assert!(s.abs() < 100.0, "ring blew up at {i}: {s}");
        }
    }

    #[test]
    fn tail_flushes_to_exact_zero() {
        // After a long enough free ring the state reaches exact zero (no
        // denormal dribble).
        let mut r = Resonator::new(SR, 300.0, 6.0);
        r.trigger(1.0);
        let mut last = 1.0f32;
        for _ in 0..(SR as usize * 5) {
            last = r.tick();
        }
        assert_eq!(last, 0.0, "tail did not flush to exact zero: {last}");
    }

    #[test]
    fn reset_matches_fresh_instance() {
        let design = (SR, 130.0f32, 3.0f32);
        let mut used = Resonator::new(design.0, design.1, design.2);
        used.trigger(1.0);
        for _ in 0..5000 {
            used.tick();
        }
        used.reset();
        used.trigger(0.7);

        let mut fresh = Resonator::new(design.0, design.1, design.2);
        fresh.trigger(0.7);

        for i in 0..10_000 {
            let a = used.tick().to_bits();
            let b = fresh.tick().to_bits();
            assert_eq!(a, b, "reset != fresh at sample {i}");
        }
    }

    #[test]
    fn ping_is_deterministic_bit_exact() {
        // Two identical pings on two instances are byte-identical (the
        // chunk-independence guard: driving one sample at a time can't depend on
        // any block boundary).
        let mut a = Resonator::new(SR, 220.0, 12.0);
        let mut b = Resonator::new(SR, 220.0, 12.0);
        a.trigger(0.9);
        b.trigger(0.9);
        for i in 0..20_000 {
            assert_eq!(a.tick().to_bits(), b.tick().to_bits(), "diverged at {i}");
        }
    }

    #[test]
    fn larger_ping_rings_longer() {
        // The accent mechanism: a bigger excitation ⇒ a longer audible ring
        // (same decay rate, higher starting point, so it stays above a fixed
        // floor longer).
        let n = SR as usize;
        let audible = |amp: f32| -> usize {
            let mut r = Resonator::new(SR, 90.0, 10.0);
            r.trigger(amp);
            let floor = 1.0e-3;
            let mut last = 0;
            for i in 0..n {
                if r.tick().abs() > floor {
                    last = i;
                }
            }
            last
        };
        assert!(audible(2.0) > audible(0.5), "louder ping did not ring longer");
    }
}
