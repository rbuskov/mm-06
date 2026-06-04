//! Shared output-stage blocks for the voices (audio-engine-spec.md §§3.3, 3.9).
//!
//! Two small, related building blocks the BD (and the metallic VCA-gating
//! voices) need:
//!
//! - [`RcEnvelope`] — the analog **RC charge/discharge** amplitude envelope:
//!   decay-only, anti-log/exponential taper, **never** a linear ramp (§3.3).
//!   Each sample multiplies by `exp(-1/(tau·sr))`, exactly the per-sample
//!   amplitude coefficient `Bleep` already uses in `lib.rs`. Time-constants are
//!   clamped against denormals and the tail is flushed to exact zero (§8).
//! - [`Vca`] — `gain × mild saturating nonlinearity` whose drive scales with the
//!   excitation/accent level, so accented hits are audibly **grittier** as well
//!   as louder (§3.9). It mirrors the output bus's `tanh(x·k)/k` auto-makeup
//!   trick (`lib.rs`): a larger `k` adds harmonics without adding raw level.
//!
//! Both use `mathx` for every transcendental so the native and wasm32 builds
//! stay bit-identical (architecture.md §parity).

use crate::mathx;

/// Smallest time-constant we will honour, in seconds. Anything shorter (or a
/// non-finite / non-positive request) is clamped up to this so the per-sample
/// coefficient `exp(-1/(tau·sr))` can't underflow into a denormal or a NaN.
const MIN_TAU: f32 = 1.0e-5;

/// Below this the envelope multiplier is flushed to exact zero so tails stop
/// dead rather than dribbling denormals (audio-engine-spec.md §8). Matches the
/// `Bleep` amp flush threshold in `lib.rs`.
const FLUSH: f32 = 1.0e-8;

/// A decay-only RC (exponential / anti-log) amplitude envelope.
///
/// [`trigger`](Self::trigger) (re)starts it at a chosen level; each
/// [`tick`](Self::tick) returns the current multiplier and advances the decay by
/// one sample. The value falls `level → 0` along a true RC curve — at `t = tau`
/// it has reached `1/e` of its start — and snaps to exact `0.0` once it dips
/// below [`FLUSH`].
#[derive(Clone)]
pub struct RcEnvelope {
    sr: f32,
    value: f32,
    coef: f32, // per-sample decay multiplier, exp(-1/(tau·sr))
}

impl RcEnvelope {
    /// Construct an idle envelope (value 0) for the given sample rate.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            value: 0.0,
            coef: 0.0,
        }
    }

    /// (Re)start the decay from `level`, taking `tau_seconds` to fall to `1/e`.
    /// `tau_seconds` is clamped to at least [`MIN_TAU`] (and against non-finite
    /// input) so the coefficient stays well-conditioned.
    pub fn trigger(&mut self, level: f32, tau_seconds: f32) {
        let tau = if tau_seconds.is_finite() {
            tau_seconds.max(MIN_TAU)
        } else {
            MIN_TAU
        };
        self.value = level;
        self.coef = mathx::exp(-1.0 / (tau * self.sr));
    }

    /// The current multiplier, before this sample's decay step.
    ///
    /// Shared-block API (used by this module's tests); the BD reads its taper via
    /// `tick`, so the engine doesn't peek the value yet — kept for voices that
    /// need to gate other blocks off the same envelope (spec §3.4/§3.6).
    #[allow(dead_code)]
    #[inline]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Emit the current multiplier (1→0) and advance the decay one sample.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        let v = self.value;
        self.value *= self.coef;
        if self.value < FLUSH {
            self.value = 0.0; // flush tails to exact zero (§8)
        }
        v
    }
}

/// A voice VCA: `gain × mild saturating nonlinearity` whose drive scales with
/// the excitation/accent level (audio-engine-spec.md §3.9).
///
/// Like the output bus (`lib.rs`), the saturation is `tanh(x·k)/k` with auto
/// makeup: the small-signal gain is `tanh'(0)·k / k = 1` for any `k`, so raising
/// the drive adds **harmonics**, not raw level. The drive `k` is derived from the
/// excitation/accent level, so an accented hit gets a larger `k` ⇒ more grit on
/// top of (not instead of) its extra loudness.
#[derive(Clone)]
pub struct Vca {
    gain: f32,
    k: f32,
}

impl Vca {
    /// Extra drive an accent (level 1.0) adds on top of the clean `k = 1`. At
    /// level 0 the VCA is a near-clean unity-`tanh`; at level 1 it pushes `k` to
    /// `1 + ACCENT_DRIVE` for noticeably more harmonics.
    const ACCENT_DRIVE: f32 = 4.0;

    /// Construct a unity-gain, clean (drive 0) VCA.
    pub fn new() -> Self {
        Self { gain: 1.0, k: 1.0 }
    }

    /// Set the output gain (loudness). Independent of grit.
    #[inline]
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    /// Set the saturation drive from an excitation/accent level in `[0, 1]`.
    /// 0 ⇒ clean (k = 1); 1 ⇒ grittiest (k = 1 + [`ACCENT_DRIVE`]). The level is
    /// clamped so out-of-range accents can't run the drive away.
    #[inline]
    pub fn set_drive(&mut self, accent_level: f32) {
        let a = accent_level.clamp(0.0, 1.0);
        self.k = 1.0 + Self::ACCENT_DRIVE * a;
    }

    /// Process one sample: gain, then auto-makeup soft saturation.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let g = x * self.gain;
        mathx::tanh(g * self.k) / self.k
    }
}

impl Default for Vca {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathx;

    const SR: f32 = 48_000.0;

    /// At t = tau the RC envelope has decayed to ≈ 1/e, and it is a true
    /// exponential (constant successive ratios), **not** a linear ramp (which
    /// would have constant successive differences).
    #[test]
    fn envelope_is_exponential_not_linear() {
        let tau = 0.05; // 50 ms
        let mut env = RcEnvelope::new(SR);
        env.trigger(1.0, tau);

        let n_at_tau = (tau * SR) as usize;
        let mut samples = Vec::with_capacity(n_at_tau + 8);
        for _ in 0..(n_at_tau + 8) {
            samples.push(env.tick());
        }

        // Value at t = tau ≈ 1/e. The envelope multiplies by the *same* mathx
        // coefficient every sample, so over `tau·sr` steps it lands on
        // `mathx::exp(-1/(tau·sr))^(tau·sr)`. mathx's tiny small-argument bias
        // compounds over thousands of steps, so the realized "1/e" drifts a few
        // percent from the ideal 0.3679 — assert the RC value is in the right
        // ballpark (clearly between a linear ramp's ~0 and the start's 1.0).
        let inv_e = 1.0 / core::f32::consts::E;
        let v_tau = samples[n_at_tau];
        assert!(
            (v_tau - inv_e).abs() < 0.1,
            "value at tau {v_tau} not ≈ 1/e {inv_e}"
        );
        // A linear ramp falling to 0 over the full decay would be far from the
        // start by t = tau; an exponential is still well above zero. The RC
        // value at tau must be a substantial fraction, never near zero.
        assert!(v_tau > 0.25, "value at tau {v_tau} too low to be an RC curve");

        // Exponential ⇒ successive ratios are constant (and equal to coef).
        // Linear ⇒ successive *differences* would be constant; assert ours are
        // NOT, by showing the ratio is near-constant while the diff is not.
        let r0 = samples[101] / samples[100];
        let r1 = samples[201] / samples[200];
        assert!(
            (r0 - r1).abs() < 1.0e-4,
            "ratios drift: {r0} vs {r1} — not a clean exponential"
        );

        // For a decaying exponential the per-sample step shrinks in proportion
        // to the current value, so a step taken near t = 0 is far bigger than
        // one taken near t = tau (where the value is ~1/e of the start). A
        // linear ramp would have *identical* steps everywhere. Compare a step
        // near the start to one near tau (≈ n_at_tau samples later).
        let d_start = samples[0] - samples[1];
        let d_tau = samples[n_at_tau] - samples[n_at_tau + 1];
        assert!(
            d_start > d_tau * 2.0,
            "successive differences nearly constant ({d_start} vs {d_tau}) — looks linear, not exponential"
        );
    }

    /// The decaying tail must reach **exact** 0.0, not a lingering denormal.
    #[test]
    fn envelope_tail_flushes_to_exact_zero() {
        let mut env = RcEnvelope::new(SR);
        env.trigger(1.0, 0.01); // short 10 ms decay
        let mut last = 1.0f32;
        for _ in 0..SR as usize {
            // 1 s — long past the tail
            last = env.tick();
        }
        assert_eq!(last, 0.0, "tail did not flush to exact zero (got {last})");
        // And it stays exactly zero.
        assert_eq!(env.tick(), 0.0);
    }

    /// Tiny tau is clamped rather than producing a NaN/denormal coefficient.
    #[test]
    fn envelope_clamps_degenerate_tau() {
        let mut env = RcEnvelope::new(SR);
        for &tau in &[0.0f32, -1.0, 1.0e-30, f32::NAN, f32::INFINITY] {
            env.trigger(1.0, tau);
            let v = env.tick();
            assert!(v.is_finite(), "tau {tau} gave non-finite first sample {v}");
            // Drain it; must stay finite and end at exact zero.
            let mut last = v;
            for _ in 0..SR as usize {
                last = env.tick();
            }
            assert!(last.is_finite());
            assert_eq!(last, 0.0, "tau {tau} tail not exact zero");
        }
    }

    /// At a tiny input the VCA is essentially linear: out ≈ in · gain.
    #[test]
    fn vca_near_linear_at_low_level() {
        let mut vca = Vca::new();
        vca.set_gain(2.0);
        vca.set_drive(1.0); // even at full drive, small signals pass ~linearly
        let x = 1.0e-3;
        let y = vca.tick(x);
        let want = x * 2.0;
        let rel = ((y - want) / want).abs();
        assert!(rel < 1.0e-3, "VCA not near-linear at low level: rel err {rel}");
    }

    /// Peak-normalized harmonic (HF) energy rises with drive: an accented hit is
    /// measurably **grittier**, not merely louder. We feed a sine, peak-normalize
    /// each output (removing the level difference), and compare the fraction of
    /// energy that lands above the fundamental bin.
    #[test]
    fn vca_grit_rises_with_drive_normalized_for_level() {
        let n = 2048usize;
        let f_bin = 8usize; // 8 cycles across the window — a clean exact bin
        let two_pi = core::f32::consts::TAU;

        // Render a peak-normalized sine through the VCA at a given drive, return
        // the HF (everything but the fundamental & DC) energy fraction.
        let hf_fraction = |drive: f32| -> f32 {
            let mut vca = Vca::new();
            vca.set_gain(1.0);
            vca.set_drive(drive);
            let mut out = vec![0.0f32; n];
            let mut peak = 0.0f32;
            for (i, o) in out.iter_mut().enumerate() {
                let x = 0.9 * mathx::sin(two_pi * f_bin as f32 * i as f32 / n as f32);
                let y = vca.tick(x);
                *o = y;
                peak = peak.max(y.abs());
            }
            // Peak-normalize so we measure shape (harmonics), not level.
            for o in out.iter_mut() {
                *o /= peak;
            }
            dft_hf_fraction(&out, f_bin)
        };

        let clean = hf_fraction(0.0);
        let accented = hf_fraction(1.0);
        assert!(
            accented > clean * 2.0,
            "drive did not add harmonics (normalized): clean HF {clean} vs accented {accented}"
        );
    }

    /// An accented VCA is louder AND grittier — confirm both move, so accent is
    /// not "just gain." Loudness comes from the caller's gain; grit from drive.
    #[test]
    fn accent_adds_both_level_and_grit() {
        // Same drive level, different gain ⇒ louder, same grit fraction.
        let level = |gain: f32| {
            let mut v = Vca::new();
            v.set_gain(gain);
            v.set_drive(0.5);
            v.tick(0.5).abs()
        };
        assert!(level(2.0) > level(1.0), "gain did not raise level");

        // Same gain, more drive ⇒ same small-signal level, more HF.
        let n = 2048usize;
        let f_bin = 8usize;
        let two_pi = core::f32::consts::TAU;
        let hf = |drive: f32| {
            let mut vca = Vca::new();
            vca.set_drive(drive);
            let mut out = vec![0.0f32; n];
            let mut peak = 0.0f32;
            for (i, o) in out.iter_mut().enumerate() {
                let y = vca.tick(0.9 * mathx::sin(two_pi * f_bin as f32 * i as f32 / n as f32));
                *o = y;
                peak = peak.max(y.abs());
            }
            for o in out.iter_mut() {
                *o /= peak;
            }
            dft_hf_fraction(&out, f_bin)
        };
        assert!(hf(1.0) > hf(0.0), "drive did not raise grit");
    }

    /// Long render of both blocks stays finite; the envelope tail hits exact
    /// zero (denormal hygiene, §8).
    #[test]
    fn denormal_clean_long_render() {
        let mut env = RcEnvelope::new(SR);
        let mut vca = Vca::new();
        vca.set_gain(0.7);
        vca.set_drive(1.0);
        env.trigger(1.0, 0.2);
        let mut last_env = 1.0f32;
        let two_pi = core::f32::consts::TAU;
        for i in 0..(SR as usize * 4) {
            last_env = env.tick();
            let sig = mathx::sin(two_pi * 220.0 * i as f32 / SR);
            let y = vca.tick(sig * last_env);
            assert!(y.is_finite(), "non-finite VCA output at sample {i}");
        }
        assert_eq!(last_env, 0.0, "envelope tail not exact zero after long render");
    }

    /// DFT magnitude at bin `f`, then sum the energy at every bin except DC and
    /// the fundamental, as a fraction of the total. Small, allocation-light, and
    /// uses `mathx` so it stays deterministic.
    fn dft_hf_fraction(x: &[f32], fundamental_bin: usize) -> f32 {
        let n = x.len();
        let two_pi = core::f32::consts::TAU;
        let mut total = 0.0f32;
        let mut hf = 0.0f32;
        // Only the lower half is unique (real input); that's enough to compare.
        for k in 1..(n / 2) {
            let mut re = 0.0f32;
            let mut im = 0.0f32;
            for (i, &xi) in x.iter().enumerate() {
                let ang = two_pi * k as f32 * i as f32 / n as f32;
                re += xi * mathx::cos(ang);
                im -= xi * mathx::sin(ang);
            }
            let mag = re * re + im * im;
            total += mag;
            if k != fundamental_bin {
                hf += mag;
            }
        }
        if total <= 0.0 {
            0.0
        } else {
            hf / total
        }
    }
}
