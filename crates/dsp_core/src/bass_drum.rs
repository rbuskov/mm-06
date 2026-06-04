//! Bass drum voice — two Twin-T resonators on one shared excitation edge
//! (audio-engine-spec.md §4.1).
//!
//! The 606 kick is **not** an 808: there is no VCO and **no downward pitch
//! sweep**. It is two bridged-T (Twin-T) resonators — OSC1 ≈ 60 Hz Q≈7 (the
//! body/sustain) and OSC2 ≈ 130 Hz Q≈3 (lower amplitude and shorter — the
//! click/attack) — pinged by **one shared ~1 ms excitation edge** so they start
//! **in phase**. That brief in-phase moment is the solid, slightly clicky
//! positive front edge (the "thud"); as the two stationary partials diverge they
//! **beat**, giving the short, punchy 606 character. The resonator centres never
//! move — the attack comes from the shared phase-locked ping, not a glide.
//!
//! Accent scales the **excitation amplitude** (spec §2.3): a bigger ping makes
//! both resonators ring with greater initial amplitude *and longer* (the Q-ring
//! stays above the floor longer), and drives the output VCA's grit harder — so
//! accented hits are punchier/grittier, not merely turned up.
//!
//! Decay is governed by resonator Q (§3.3): no external AD is needed. A gentle
//! output [`RcEnvelope`] × [`Vca`] adds a touch of analog taper and accent grit
//! without colouring the pitch. Tails flush to exact zero and the whole voice is
//! deterministic and DC-free (the band-passes carry no DC). Every transcendental
//! goes through [`crate::mathx`], so native and wasm builds ring bit-identically.

use crate::envelope::{RcEnvelope, Vca};
use crate::excitation::Excitation;
use crate::resonator::Resonator;

/// OSC1 — the body/sustain resonator centre (Hz).
const OSC1_FREQ_HZ: f32 = 60.0;
/// OSC1 quality factor (higher Q ⇒ the long body ring).
const OSC1_Q: f32 = 7.0;

/// OSC2 — the click/attack resonator centre (Hz). Higher than OSC1 so the two
/// partials beat once they drift out of the shared in-phase start.
const OSC2_FREQ_HZ: f32 = 130.0;
/// OSC2 quality factor (lower Q ⇒ a shorter ring than OSC1).
const OSC2_Q: f32 = 3.0;

/// OSC2 is summed **lower** than OSC1 (spec §4.1): it is the click/attack, not
/// the body. This scales OSC2's ping amplitude relative to OSC1's so it both
/// starts quieter *and* (with its lower Q) decays away first.
const OSC2_LEVEL: f32 = 0.5;

/// Peak excitation edge for a plain (unaccented) hit. Sized so the summed BD
/// peaks comfortably below 1.0 through the gentle output VCA, leaving headroom
/// for the per-voice LEVEL and the master/bus stages downstream.
const BASE_EDGE: f32 = 0.9;

/// Extra excitation an accent adds on top of [`BASE_EDGE`], at `accent_level`
/// 1.0. Accent raises the ping (louder + longer ring) and the VCA grit — never
/// a pure output-gain change (spec §2.3).
const ACCENT_EDGE: f32 = 0.7;

/// Ring make-up gain into the **VCA input**. A single-impulse ping of a
/// low-frequency constant-skirt band-pass produces a tiny absolute swing (the
/// impulse-response amplitude scales with `sin(w0)/2`, small at 60 Hz), so the
/// summed ring is brought up here. This is sized to keep the VCA input in its
/// near-linear region (so the accent-grit `tanh` adds harmonics without
/// swallowing the accent's extra loudness): a plain hit reaches ≈ 0.05 at the
/// VCA input, an accented one ≈ 0.10. Pure gain, applied equally to both
/// resonators — never pitch or the in-phase/beating relationship.
const RING_GAIN: f32 = 5.0;

/// Post-VCA output make-up so the (near-linear) VCA output reaches a musical
/// level comparable to the placeholder bleep, with headroom for accent and the
/// per-voice LEVEL. Applied *after* the saturation, so accent's larger VCA
/// input survives as extra loudness (the grit lives in the harmonics tanh
/// adds, not in a level cut).
const OUTPUT_GAIN: f32 = 9.0;

/// Gentle output-VCA decay time-constant (seconds). The audible decay is the
/// resonator Q; this only adds a soft analog taper, so it is long enough not to
/// truncate the body ring.
const OUTPUT_TAU_S: f32 = 0.5;

/// The bass-drum voice: two stationary Twin-T resonators pinged in phase by one
/// shared excitation edge, summed (OSC2 lower) through a gentle accent-grit VCA.
#[derive(Clone)]
pub struct BassDrum {
    /// Body/sustain resonator (≈60 Hz, Q≈7).
    osc1: Resonator,
    /// Click/attack resonator (≈130 Hz, Q≈3) — lower amplitude, shorter ring.
    osc2: Resonator,
    /// The shared ~1 ms band-limited excitation edge. One generator for both
    /// resonators so they ping from the *same* edge (spec §3.1/§4.1).
    excite: Excitation,
    /// Gentle output amplitude taper (decay-only RC, §3.3). The real decay is Q.
    env: RcEnvelope,
    /// Output VCA: gain × accent-scaled soft saturation (grit, §3.9).
    vca: Vca,
    /// True while the shared excitation edge is still feeding the resonators.
    /// After the edge elapses the resonators ring freely.
    exciting: bool,
}

impl BassDrum {
    /// Build the BD voice for `sample_rate` (Hz). Idle until [`trigger`]ed; all
    /// state is fixed-size and constructed here — nothing allocates later.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            osc1: Resonator::new(sample_rate, OSC1_FREQ_HZ, OSC1_Q),
            osc2: Resonator::new(sample_rate, OSC2_FREQ_HZ, OSC2_Q),
            excite: Excitation::new(sample_rate),
            env: RcEnvelope::new(sample_rate),
            vca: Vca::new(),
            exciting: false,
        }
    }

    /// Fire the kick. `accent` (with the engine-global `accent_level`) scales the
    /// shared excitation edge: a bigger edge ⇒ both resonators ring louder *and*
    /// longer, and the VCA is driven grittier (spec §2.3). The two resonators are
    /// pinged from the **same** edge so they start in phase (the front edge),
    /// then beat as they diverge. No pitch sweep — centres are fixed.
    pub fn trigger(&mut self, accent: bool, accent_level: f32) {
        let a = if accent { accent_level.clamp(0.0, 1.0) } else { 0.0 };
        let edge = BASE_EDGE + ACCENT_EDGE * a;

        // One shared edge → in-phase start → beating as the partials diverge.
        // OSC2 is summed lower (the click/attack, not the body).
        self.excite.trigger(edge);
        self.osc1.trigger(edge);
        self.osc2.trigger(edge * OSC2_LEVEL);
        self.exciting = true;

        // Gentle output taper + accent grit. The VCA runs at unity gain (the
        // musical make-up is OUTPUT_GAIN, applied post-saturation in `tick`) so
        // accent's larger ring survives as extra loudness; `set_drive` adds the
        // accent grit (harmonics) on top.
        self.env.trigger(1.0, OUTPUT_TAU_S);
        self.vca.set_gain(1.0);
        self.vca.set_drive(a);
    }

    /// Advance one sample. The shared excitation edge re-pings both resonators
    /// in phase for its ~1 ms span (keeping the front edge solid and the start
    /// phase-locked), after which they ring freely. Sum (OSC2 lower) → gentle
    /// envelope → accent-grit VCA. Flushes to exact zero once everything decays.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        // The shared edge: while a pulse is in flight, keep both resonators
        // locked to the *same* drive so they stay in phase through the attack.
        if self.exciting {
            let e = self.excite.tick();
            if e > 0.0 {
                self.osc1.trigger(e);
                self.osc2.trigger(e * OSC2_LEVEL);
            } else {
                self.exciting = false;
            }
        }

        let body = self.osc1.tick();
        let click = self.osc2.tick() * OSC2_LEVEL;
        let env = self.env.tick();
        self.vca.tick((body + click) * RING_GAIN * env) * OUTPUT_GAIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Render `n` samples of a fresh BD hit.
    fn render(accent: bool, accent_level: f32, n: usize) -> Vec<f32> {
        let mut bd = BassDrum::new(SR);
        bd.trigger(accent, accent_level);
        (0..n).map(|_| bd.tick()).collect()
    }

    fn peak(b: &[f32]) -> f32 {
        b.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// Sample index of the last value above `floor` — a crude "ring length".
    fn audible_len(b: &[f32], floor: f32) -> usize {
        let mut last = 0;
        for (i, &s) in b.iter().enumerate() {
            if s.abs() > floor {
                last = i;
            }
        }
        last
    }

    /// Dominant frequency in a window via positive-going zero-crossings.
    fn dominant_freq_zc(buf: &[f32]) -> f32 {
        let pk = peak(buf);
        if pk <= 0.0 {
            return 0.0;
        }
        let thresh = pk * 1.0e-3;
        let mut first = None;
        let mut last = None;
        let mut crossings = 0u32;
        for (i, w) in buf.windows(2).enumerate() {
            if w[1].abs() < thresh && w[0].abs() < thresh {
                continue;
            }
            if w[0] <= 0.0 && w[1] > 0.0 {
                if first.is_none() {
                    first = Some(i);
                }
                last = Some(i);
                crossings += 1;
            }
        }
        let (Some(a), Some(b)) = (first, last) else {
            return 0.0;
        };
        if crossings < 2 || b == a {
            return 0.0;
        }
        (crossings - 1) as f32 * SR / (b - a) as f32
    }

    #[test]
    fn front_edge_opens_positive_and_in_phase() {
        // Both resonators are pinged in phase by the shared edge: the sum opens
        // with a solid POSITIVE transient (no cancellation at the attack).
        let buf = render(false, 0.0, 4_000);
        // Find the very first non-trivial extremum; it must be positive and a
        // meaningful fraction of the overall peak (a real front edge, not noise).
        let pk = peak(&buf);
        let mut front = 0.0f32;
        for &s in &buf[..(0.01 * SR) as usize] {
            if s.abs() > front.abs() {
                front = s;
            }
        }
        assert!(front > 0.0, "front edge is not positive: {front}");
        assert!(front > 0.3 * pk, "front edge {front} not a solid transient (peak {pk})");
    }

    #[test]
    fn beating_is_present_after_the_in_phase_start() {
        // Two close-ish stationary partials (60 & 130 Hz) beat: the amplitude
        // envelope is NOT a single clean monotonic decay — it has at least one
        // local rise (a beat re-swell) after the initial attack.
        let buf = render(false, 0.0, (0.6 * SR) as usize);
        // Coarse |x| envelope over ~5 ms windows.
        let win = (0.005 * SR) as usize;
        let mut env = Vec::new();
        let mut i = 0;
        while i + win <= buf.len() {
            env.push(peak(&buf[i..i + win]));
            i += win;
        }
        // Skip the very first windows (the attack), then look for a re-rise:
        // a window larger than its predecessor after a prior decay — the beat.
        let mut saw_rise = false;
        for w in env.windows(2).skip(3) {
            if w[1] > w[0] * 1.02 {
                saw_rise = true;
                break;
            }
        }
        assert!(saw_rise, "no beating re-swell in the BD envelope: {env:?}");
    }

    #[test]
    fn osc2_is_lower_amplitude_and_shorter_than_osc1() {
        // Compare each resonator's contribution in isolation, pinged exactly as
        // the assembly does (shared base edge, OSC2 scaled by OSC2_LEVEL twice:
        // once at the ping, once at the sum).
        let n = SR as usize;
        let edge = BASE_EDGE;

        let mut o1 = Resonator::new(SR, OSC1_FREQ_HZ, OSC1_Q);
        o1.trigger(edge);
        let body: Vec<f32> = (0..n).map(|_| o1.tick()).collect();

        let mut o2 = Resonator::new(SR, OSC2_FREQ_HZ, OSC2_Q);
        o2.trigger(edge * OSC2_LEVEL);
        let click: Vec<f32> = (0..n).map(|_| o2.tick() * OSC2_LEVEL).collect();

        assert!(
            peak(&click) < peak(&body),
            "OSC2 not lower amplitude: {} !< {}",
            peak(&click),
            peak(&body)
        );
        let floor = 1.0e-3;
        assert!(
            audible_len(&click, floor) < audible_len(&body, floor),
            "OSC2 not shorter: {} !< {}",
            audible_len(&click, floor),
            audible_len(&body, floor)
        );
    }

    #[test]
    fn no_downward_pitch_sweep() {
        // THE 808 guard. Pitch-track the BD sum across successive windows; the
        // dominant frequency must NOT glide monotonically downward. (An 808 kick
        // would start ~well above its body pitch and slide down; the 606 has two
        // STATIONARY partials, so the windowed estimate stays within a band — it
        // may wobble from the beating, but it never trends down.)
        let buf = render(false, 0.0, (0.4 * SR) as usize);
        let win = (0.06 * SR) as usize; // ~3-4 cycles of OSC1 per window
        let mut freqs = Vec::new();
        let mut i = 0;
        while i + win <= buf.len() {
            let f = dominant_freq_zc(&buf[i..i + win]);
            if f > 0.0 {
                freqs.push(f);
            }
            i += win / 2; // 50% overlap
        }
        assert!(freqs.len() >= 4, "too few pitch windows: {freqs:?}");

        // A pitch SWEEP would be a long monotonic-down run. Assert no such run
        // spans the whole track: the first and last estimates are within a tight
        // band of each other (stationary partials, no glide), and the maximum
        // monotonic-decreasing streak is short.
        let first = freqs[0];
        let last = *freqs.last().unwrap();
        let rel = (last - first).abs() / first;
        assert!(
            rel < 0.25,
            "BD pitch drifted {rel:.2} from {first:.1} to {last:.1} Hz — looks like a sweep"
        );

        let mut streak = 1usize;
        let mut max_streak = 1usize;
        for w in freqs.windows(2) {
            if w[1] < w[0] * 0.98 {
                streak += 1;
                max_streak = max_streak.max(streak);
            } else {
                streak = 1;
            }
        }
        assert!(
            max_streak < freqs.len(),
            "BD pitch monotonically descends the whole track ({max_streak}/{}) — an 808 sweep",
            freqs.len()
        );
    }

    #[test]
    fn deterministic_bit_exact() {
        let a: Vec<u32> = render(true, 0.8, 20_000).iter().map(|x| x.to_bits()).collect();
        let b: Vec<u32> = render(true, 0.8, 20_000).iter().map(|x| x.to_bits()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn dc_free() {
        let buf = render(false, 0.0, SR as usize);
        let mean: f64 = buf.iter().map(|&x| x as f64).sum::<f64>() / buf.len() as f64;
        assert!(mean.abs() < 1.0e-3, "BD has DC offset {mean}");
    }

    #[test]
    fn tail_flushes_to_exact_zero() {
        // Long render past the body ring: the tail must reach EXACT zero.
        let mut bd = BassDrum::new(SR);
        bd.trigger(true, 1.0); // loudest/longest hit
        let mut last = 1.0f32;
        for _ in 0..(SR as usize * 8) {
            last = bd.tick();
        }
        assert_eq!(last, 0.0, "BD tail did not flush to exact zero: {last}");
    }

    #[test]
    fn accent_is_louder_and_rings_longer() {
        // Accent scales the excitation edge → louder AND longer ring on both
        // resonators (spec §2.3), not a pure output-gain change.
        let plain = render(false, 0.0, SR as usize);
        let acc = render(true, 1.0, SR as usize);
        assert!(peak(&acc) > peak(&plain), "accent not louder");
        let floor = 1.0e-3;
        assert!(
            audible_len(&acc, floor) > audible_len(&plain, floor),
            "accent did not ring longer"
        );
    }

    #[test]
    fn produces_sound_and_stays_finite() {
        let buf = render(false, 0.0, SR as usize);
        assert!(peak(&buf) > 0.01, "BD silent");
        for &s in &buf {
            assert!(s.is_finite(), "BD non-finite sample: {s}");
        }
    }
}
