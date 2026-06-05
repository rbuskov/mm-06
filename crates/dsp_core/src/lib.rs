//! MM-06 — pure, deterministic TR-606 DSP core (architecture.md §"DSP layer").
//!
//! This crate has **no I/O, no globals, no clock reads, and no web/wasm-bindgen
//! dependency**. It compiles unchanged to `wasm32` (the shipped AudioWorklet
//! engine, via `dsp_wasm`) and to native (the offline `render` binary and the
//! test suite). Identical `(seed, params, events, sample_rate, block_size)`
//! produces a byte-identical buffer.
//!
//! **Walking-skeleton scope — placeholder synthesis.** The real per-voice models
//! (bridged-T resonators, the free-running metallic cluster, noise, oversampling
//! — audio-engine-spec.md §§3–6) are not implemented yet. Every voice currently
//! plays the **same short "bleep"** (a decaying sine) purely so the end-to-end
//! path makes an audible, distinguishable-per-hit sound. What is already real is
//! the *architecture*: determinism, the trigger/accent path, per-voice levels +
//! mixer + output saturation, the offline/streaming split, and the message
//! protocol. The bleep slots out for real voices behind these same interfaces.

#![forbid(unsafe_code)]

use bass_drum::BassDrum;

mod mathx;
// The bass drum (§4.1) is real: two Twin-T resonators on one shared excitation
// edge. It uses the excitation / envelope / resonator blocks below, so those no
// longer need a blanket dead-code allow for the parts BD touches.
mod bass_drum;
mod excitation;
// Shared output-stage blocks (RC amp envelope + accent-grit VCA, spec §§3.3, 3.9).
mod envelope;
pub mod protocol;
// Kept for the real engine (seeded noise + per-hit variance, spec §10); unused
// by the placeholder bleep.
#[allow(dead_code)]
mod rng;
mod resonator;

const TAU: f32 = core::f32::consts::TAU;

/// Representative bus program level the Drive makeup is referenced to (spec §7).
/// The auto-makeup preserves loudness at *this* amplitude rather than at zero,
/// so turning Drive up compresses peaks (adds saturation) without dropping the
/// overall level. Sized to a typical mixed-bus peak; `→ 0` would recover the old
/// slope-at-zero `tanh(x·k)/k` that let Drive go quiet.
const BUS_REF: f32 = 0.3;

/// The seven voices, in kit order (dev-ui-spec.md). The discriminant is the
/// wire id used by the message protocol.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Voice {
    Bd = 0, // Bass Drum
    Sd = 1, // Snare Drum
    Lt = 2, // Low Tom
    Ht = 3, // High Tom
    Cy = 4, // Cymbal
    Oh = 5, // Open Hat
    Ch = 6, // Closed Hat
}

pub const VOICE_COUNT: usize = 7;

impl Voice {
    pub fn from_id(id: u8) -> Option<Voice> {
        match id {
            0 => Some(Voice::Bd),
            1 => Some(Voice::Sd),
            2 => Some(Voice::Lt),
            3 => Some(Voice::Ht),
            4 => Some(Voice::Cy),
            5 => Some(Voice::Oh),
            6 => Some(Voice::Ch),
            _ => None,
        }
    }
    #[inline]
    fn idx(self) -> usize {
        self as usize
    }
}

/// One scheduled hit. The offline `render()` gets the whole list up front; the
/// streaming engine enqueues one of these per `Trigger` message.
#[derive(Clone, Copy, Debug)]
pub struct TrigEvent {
    pub sample_index: usize,
    pub voice: Voice,
    pub accent: bool,
}

/// A complete offline render job (architecture.md §"DSP layer").
pub struct RenderRequest {
    pub sample_rate: f64,
    pub seed: u64,
    pub variance_depth: f32,
    pub block_size: usize,
    pub events: Vec<TrigEvent>,
    pub length_samples: usize,
}

// =============================================================================
// Placeholder voice + helpers
// =============================================================================

/// The placeholder voice: a short decaying sine — a "bleep". Every voice uses
/// one of these with identical settings, so all seven sound the same for now.
#[derive(Clone)]
struct Bleep {
    sr: f32,
    phase: f32,
    amp: f32,
    coef: f32, // per-sample amplitude decay multiplier
}

impl Bleep {
    /// Fixed pitch and decay of the placeholder tone.
    const FREQ_HZ: f32 = 880.0;
    const BASE_AMP: f32 = 0.6;
    const BASE_TAU: f32 = 0.10; // seconds to ~1/e

    fn new(sr: f32) -> Self {
        Self {
            sr,
            phase: 0.0,
            amp: 0.0,
            coef: 0.0,
        }
    }

    /// (Re)start the bleep. `energy` scales loudness and `tau` sets the decay —
    /// the only things accent nudges, just so the controls aren't dead.
    fn trigger(&mut self, energy: f32, tau: f32) {
        self.phase = 0.0;
        self.amp = Self::BASE_AMP * energy;
        self.coef = mathx::exp(-1.0 / (tau * self.sr));
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        let v = self.amp * mathx::sin(TAU * self.phase);
        self.phase += Self::FREQ_HZ / self.sr;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.amp *= self.coef;
        if self.amp < 1.0e-8 {
            self.amp = 0.0; // flush tails to exact zero
        }
        v
    }
}

/// A one-pole parameter smoother — avoids zipper noise on level changes
/// (architecture.md §"Continuous panel controls").
#[derive(Clone)]
struct Smoother {
    target: f32,
    value: f32,
    a: f32,
}

impl Smoother {
    fn new(initial: f32, sr: f32) -> Self {
        let rc = 0.005; // ~5 ms
        let dt = 1.0 / sr;
        Self {
            target: initial,
            value: initial,
            a: dt / (rc + dt),
        }
    }
    #[inline]
    fn set(&mut self, t: f32) {
        self.target = t;
    }
    #[inline]
    fn tick(&mut self) -> f32 {
        self.value += self.a * (self.target - self.value);
        self.value
    }
}

// =============================================================================
// Engine
// =============================================================================

const EVENT_QUEUE_CAP: usize = 256;
const OUT_EVENT_CAP: usize = 256;

/// The pure streaming engine. Driven per render quantum by `dsp_wasm` (live) and
/// in one shot by `render()` (offline) — both through the same per-sample path.
pub struct Engine {
    global_sample: u64,

    // BD (index 0) is the real two-resonator kick (spec §4.1); the other six
    // voices are still the placeholder bleep. `voices[0]` is left unused and
    // silent — the mix loop drives `bd` for slot 0 and `voices[1..]` for the
    // rest, behind the same trigger/level/accent interface.
    bd: BassDrum,
    voices: [Bleep; VOICE_COUNT],
    levels: [Smoother; VOICE_COUNT],
    master: Smoother,
    accent_level: f32,
    drive: Smoother,
    bus_enabled: bool, // false in render_voice (pre-mix, no bus coloration)

    queue: Vec<TrigEvent>, // incoming events, sorted by sample_index
    out_events: Vec<u8>,   // outgoing diagnostics drained between callbacks
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        Self {
            global_sample: 0,
            bd: BassDrum::new(sr),
            voices: core::array::from_fn(|_| Bleep::new(sr)),
            levels: core::array::from_fn(|_| Smoother::new(0.5, sr)),
            master: Smoother::new(0.8, sr),
            accent_level: 0.8,
            drive: Smoother::new(0.0, sr),
            bus_enabled: true,
            queue: Vec::with_capacity(EVENT_QUEUE_CAP),
            out_events: Vec::with_capacity(OUT_EVENT_CAP * protocol::FRAME_LEN),
        }
    }

    // ---- control surface (called by protocol::apply_messages) ----------------

    pub fn trigger(&mut self, voice: Voice, accent: bool) {
        self.enqueue(TrigEvent {
            sample_index: self.global_sample as usize,
            voice,
            accent,
        });
    }

    pub fn set_level(&mut self, voice: Voice, value: f32) {
        self.levels[voice.idx()].set(value.clamp(0.0, 1.0));
    }
    pub fn set_master_level(&mut self, value: f32) {
        self.master.set(value.clamp(0.0, 1.0));
    }
    pub fn set_accent_level(&mut self, value: f32) {
        self.accent_level = value.clamp(0.0, 1.0);
    }
    pub fn set_drive(&mut self, value: f32) {
        self.drive.set(value.clamp(0.0, 1.0));
    }
    /// No-op for the placeholder bleep (it draws no randomness). Kept so the
    /// protocol surface is complete; the real variance layer (spec §10) will use
    /// it.
    pub fn set_variance_depth(&mut self, _value: f32) {}
    /// No-op for the placeholder bleep. See [`Self::set_variance_depth`].
    pub fn set_seed(&mut self, _seed: u64) {}

    /// Drain queued diagnostic events as protocol frames (Worklet → Main).
    pub fn drain_events(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out_events)
    }

    // ---- scheduling ----------------------------------------------------------

    fn enqueue(&mut self, ev: TrigEvent) {
        if self.queue.len() >= EVENT_QUEUE_CAP {
            return; // drop rather than grow inside the audio path
        }
        let pos = self
            .queue
            .iter()
            .position(|e| e.sample_index > ev.sample_index)
            .unwrap_or(self.queue.len());
        self.queue.insert(pos, ev);
    }

    fn fire(&mut self, voice: Voice, accent: bool) {
        if voice == Voice::Bd {
            // The real kick (spec §4.1): accent scales the shared excitation edge
            // → louder + longer ring on both resonators (+ VCA grit), not a pure
            // gain bump. The per-voice LEVEL still mixes it like any other voice.
            self.bd.trigger(accent, self.accent_level);
        } else {
            // Placeholder bleep for the other six voices: accent only nudges
            // loudness + decay, identically for each.
            let energy = if accent { 1.0 + 0.5 * self.accent_level } else { 1.0 };
            let tau = if accent {
                Bleep::BASE_TAU * (1.0 + 0.5 * self.accent_level)
            } else {
                Bleep::BASE_TAU
            };
            self.voices[voice.idx()].trigger(energy, tau);
        }

        if self.out_events.len() + protocol::FRAME_LEN <= OUT_EVENT_CAP * protocol::FRAME_LEN {
            let f = protocol::encode(protocol::TAG_VOICE_FIRED, voice as u8, accent as u8, 0.0);
            self.out_events.extend_from_slice(&f);
        }
    }

    // ---- the per-quantum render path -----------------------------------------

    /// Process exactly `out_left.len()` samples (the render quantum). Writes
    /// dual-mono into both channels. Allocation-free.
    pub fn process(&mut self, out_left: &mut [f32], out_right: &mut [f32]) {
        let n = out_left.len().min(out_right.len());
        for i in 0..n {
            // fire any events scheduled at or before this sample
            while let Some(front) = self.queue.first().copied() {
                if front.sample_index as u64 <= self.global_sample {
                    self.queue.remove(0);
                    self.fire(front.voice, front.accent);
                } else {
                    break;
                }
            }

            // Slot 0 is the real BD; slots 1..7 are the placeholder bleep. Every
            // voice's per-sample level smoother is ticked exactly once so the
            // mixer/level path is unchanged from before.
            let mut mix = 0.0;
            for v in 0..VOICE_COUNT {
                let sample = if v == Voice::Bd.idx() {
                    self.bd.tick()
                } else {
                    self.voices[v].tick()
                };
                mix += sample * self.levels[v].tick();
            }

            let master = self.master.tick();
            let drive = self.drive.tick();
            let mut out = mix * master;
            if self.bus_enabled {
                // Bus soft-saturation (spec §7) with auto makeup gain: scale the
                // input into tanh by k, then renormalise so a *representative
                // program level* `BUS_REF` passes at the same loudness for any
                // drive. The naive `tanh(x·k)/k` only normalises the slope at
                // zero (the `BUS_REF → 0` limit), so it holds unity for
                // infinitesimal signals while real peaks — which sit well up the
                // tanh curve — collapse: Drive got audibly *quieter*. Referencing
                // a realistic level keeps RMS roughly constant as drive rises;
                // peaks still compress (that *is* the saturation), loudness holds.
                // drive 0 ⇒ k = 1 ⇒ makeup = 1 ⇒ the same gentle unity tanh.
                let k = 1.0 + 4.0 * drive;
                let makeup = mathx::tanh(BUS_REF) / mathx::tanh(k * BUS_REF);
                out = mathx::tanh(out * k) * makeup;
            }

            out_left[i] = out;
            out_right[i] = out;
            self.global_sample = self.global_sample.wrapping_add(1);
        }
    }
}

// =============================================================================
// Offline entry points (architecture.md §"DSP layer")
// =============================================================================

/// Render the full main mix for `req`. Deterministic.
pub fn render(req: &RenderRequest) -> Vec<f32> {
    let mut e = Engine::new(req.sample_rate as f32);
    e.set_seed(req.seed);
    e.set_variance_depth(req.variance_depth);
    let events: Vec<TrigEvent> = req.events.clone();
    run_offline_events(&mut e, req, &events)
}

/// Render a single voice in isolation, pre-mix, without bus coloration — a
/// calibration aid, **not** a shipped output (spec §2.1).
pub fn render_voice(req: &RenderRequest, voice: Voice) -> Vec<f32> {
    let mut e = Engine::new(req.sample_rate as f32);
    e.set_seed(req.seed);
    e.set_variance_depth(req.variance_depth);
    e.bus_enabled = false;
    e.master.value = 1.0;
    e.master.target = 1.0;
    for v in 0..VOICE_COUNT {
        let g = if v == voice.idx() { 1.0 } else { 0.0 };
        e.levels[v].value = g;
        e.levels[v].target = g;
    }
    let filtered: Vec<TrigEvent> = req
        .events
        .iter()
        .copied()
        .filter(|ev| ev.voice == voice)
        .collect();
    run_offline_events(&mut e, req, &filtered)
}

fn run_offline_events(e: &mut Engine, req: &RenderRequest, events: &[TrigEvent]) -> Vec<f32> {
    for ev in events {
        e.enqueue(*ev);
    }
    let block = req.block_size.max(1);
    let mut out = vec![0.0f32; req.length_samples];
    let mut scratch_l = vec![0.0f32; block];
    let mut scratch_r = vec![0.0f32; block];
    let mut pos = 0;
    while pos < req.length_samples {
        let this = block.min(req.length_samples - pos);
        e.process(&mut scratch_l[..this], &mut scratch_r[..this]);
        out[pos..pos + this].copy_from_slice(&scratch_l[..this]);
        pos += this;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(events: Vec<TrigEvent>, len: usize) -> RenderRequest {
        RenderRequest {
            sample_rate: 48_000.0,
            seed: 1,
            variance_depth: 0.0,
            block_size: 128,
            events,
            length_samples: len,
        }
    }

    #[test]
    fn deterministic_bit_exact() {
        let ev = vec![TrigEvent { sample_index: 0, voice: Voice::Bd, accent: false }];
        let a = render(&req(ev.clone(), 24_000));
        let b = render(&req(ev, 24_000));
        let ab: Vec<u32> = a.iter().map(|x| x.to_bits()).collect();
        let bb: Vec<u32> = b.iter().map(|x| x.to_bits()).collect();
        assert_eq!(ab, bb);
    }

    #[test]
    fn block_size_invariance_bit_exact() {
        // Same input, different block sizes → byte-identical (spec §3.2).
        let ev = vec![
            TrigEvent { sample_index: 100, voice: Voice::Sd, accent: true },
            TrigEvent { sample_index: 5000, voice: Voice::Ch, accent: false },
        ];
        let mut r1 = req(ev.clone(), 20_000);
        r1.block_size = 64;
        let mut r2 = req(ev, 20_000);
        r2.block_size = 512;
        let a: Vec<u32> = render(&r1).iter().map(|x| x.to_bits()).collect();
        let b: Vec<u32> = render(&r2).iter().map(|x| x.to_bits()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn no_nan_or_inf_and_produces_sound() {
        let mut ev = Vec::new();
        for (i, v) in [Voice::Bd, Voice::Sd, Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch]
            .iter()
            .enumerate()
        {
            ev.push(TrigEvent { sample_index: i * 2000, voice: *v, accent: i % 2 == 0 });
        }
        let out = render(&req(ev, 48_000));
        let mut peak = 0.0f32;
        for s in &out {
            assert!(s.is_finite(), "non-finite sample");
            peak = peak.max(s.abs());
        }
        assert!(peak > 0.01, "engine produced silence (peak {peak})");
    }

    #[test]
    fn render_voice_isolates() {
        // Rendering BD-only should be silent for an SD-only event list.
        let out = render_voice(
            &req(vec![TrigEvent { sample_index: 0, voice: Voice::Sd, accent: false }], 8000),
            Voice::Bd,
        );
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak < 1e-6, "BD isolation leaked SD ({peak})");
    }

    #[test]
    fn six_bleeps_identical_and_bd_differs() {
        // BD (slot 0) is now the real two-resonator kick (spec §4.1). The OTHER
        // six voices are still the same placeholder bleep, so rendered in
        // isolation at the same onset they are byte-identical to each other — and
        // the real BD must DIFFER from them (it's a kick, not an 880 Hz bleep).
        let len = 8000;
        let reference: Vec<u32> = render_voice(
            &req(vec![TrigEvent { sample_index: 0, voice: Voice::Sd, accent: false }], len),
            Voice::Sd,
        )
        .iter()
        .map(|x| x.to_bits())
        .collect();
        for v in [Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch] {
            let got: Vec<u32> = render_voice(
                &req(vec![TrigEvent { sample_index: 0, voice: v, accent: false }], len),
                v,
            )
            .iter()
            .map(|x| x.to_bits())
            .collect();
            assert_eq!(got, reference, "{v:?} differs from the SD bleep");
        }

        // The real kick is a different sound from the bleep.
        let bd: Vec<u32> = render_voice(
            &req(vec![TrigEvent { sample_index: 0, voice: Voice::Bd, accent: false }], len),
            Voice::Bd,
        )
        .iter()
        .map(|x| x.to_bits())
        .collect();
        assert_ne!(bd, reference, "BD should no longer match the placeholder bleep");
    }

    #[test]
    fn drive_does_not_boost_level() {
        // Drive adds saturation, not loudness: peak at full drive must not
        // exceed the clean peak (it may compress a little), and must stay in a
        // sane band — never the old ~2x jump.
        let clean = peak_at_drive(0.0);
        let driven = peak_at_drive(1.0);
        assert!(driven <= clean * 1.05, "drive boosted level: {clean} -> {driven}");
        assert!(driven >= clean * 0.5, "drive over-attenuated: {clean} -> {driven}");
    }

    #[test]
    fn drive_preserves_loudness() {
        // The makeup is referenced to a real program level, so turning Drive up
        // compresses peaks but holds overall loudness (RMS) roughly constant —
        // it must NOT go quiet (the old `/k` makeup dropped RMS ~3 dB and peak
        // ~9 dB at full drive). Allow a modest band either side of unity.
        let clean = rms_at_drive(0.0);
        let driven = rms_at_drive(1.0);
        let ratio = driven / clean;
        assert!(
            (0.85..=1.5).contains(&ratio),
            "drive shifted loudness too far: rms {clean} -> {driven} (ratio {ratio:.2})"
        );
    }

    /// Render a single (unaccented) BD hit through the full bus at drive `d`,
    /// returning the output peak.
    fn peak_at_drive(d: f32) -> f32 {
        drive_stats(d).0
    }
    /// As [`peak_at_drive`] but returns the output RMS over the hit.
    fn rms_at_drive(d: f32) -> f32 {
        drive_stats(d).1
    }
    /// (peak, rms) of one BD hit through the full bus at drive `d`.
    fn drive_stats(d: f32) -> (f32, f32) {
        let mut e = Engine::new(48_000.0);
        e.set_master_level(0.8);
        e.set_level(Voice::Bd, 0.5);
        e.set_drive(d);
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        for _ in 0..40 {
            e.process(&mut l, &mut r); // settle the drive smoother on silence
        }
        e.trigger(Voice::Bd, false);
        let mut peak = 0.0f32;
        let mut sumsq = 0.0f64;
        let mut n = 0u64;
        for _ in 0..200 {
            e.process(&mut l, &mut r);
            for &x in &l {
                peak = peak.max(x.abs());
                sumsq += (x as f64) * (x as f64);
                n += 1;
            }
        }
        (peak, (sumsq / n as f64).sqrt() as f32)
    }

    #[test]
    fn accent_is_louder() {
        // Accent nudges loudness (and decay) on the placeholder — direction only.
        let len = 24_000;
        let plain = render_voice(
            &req(vec![TrigEvent { sample_index: 0, voice: Voice::Bd, accent: false }], len),
            Voice::Bd,
        );
        let acc = render_voice(
            &req(vec![TrigEvent { sample_index: 0, voice: Voice::Bd, accent: true }], len),
            Voice::Bd,
        );
        let peak = |b: &[f32]| b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak(&acc) > peak(&plain), "accent was not louder");
    }
}
