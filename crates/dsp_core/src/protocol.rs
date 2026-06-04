//! The control message protocol shared between the main thread and the audio
//! engine (architecture.md §"Control & event protocol").
//!
//! Messages are fixed **8-byte little-endian frames** so the audio side parses
//! them with zero allocation and no branching on length:
//!
//! ```text
//! byte 0      tag   (u8)   — which message
//! byte 1      a     (u8)   — first small operand (voice id / global id)
//! byte 2      b     (u8)   — second small operand (param id / accent flag)
//! byte 3      _pad  (u8)
//! bytes 4..8  value (f32)  — continuous value, little-endian
//! ```
//!
//! The schema is defined here once in Rust and mirrored by hand in
//! `web/src/protocol.ts`. The architecture calls for the TS side to be
//! *generated* from this; for the walking skeleton the two are kept in sync
//! manually and guarded by the round-trip notes in each file.

use crate::{Engine, Voice};

pub const FRAME_LEN: usize = 8;

// ---- Main → Worklet (control) -------------------------------------------------

/// `Trigger { voice, accent }` — fire a voice now. a = voice id, b = accent flag.
pub const TAG_TRIGGER: u8 = 0;
/// `ParamSet { voice, param_id, value }` — a per-voice continuous control.
/// a = voice id, b = param id, value = f32.
pub const TAG_PARAM_SET: u8 = 1;
/// `GlobalSet { id, value }` — a global continuous control. a = global id.
pub const TAG_GLOBAL_SET: u8 = 2;

// Per-voice param ids (only LEVEL exists on a 606 — spec §9).
pub const PARAM_LEVEL: u8 = 0;

// Global ids (dev-ui-spec.md §"Global controls").
pub const GLOBAL_ACCENT_LEVEL: u8 = 0;
pub const GLOBAL_MASTER_LEVEL: u8 = 1;
pub const GLOBAL_DRIVE: u8 = 2;
pub const GLOBAL_VARIANCE_DEPTH: u8 = 3;
/// Seed is an integer carried in the f32 value field (exact up to 2^24, plenty
/// for a dev rig). Changing it re-derives the deterministic per-hit variance.
pub const GLOBAL_SEED: u8 = 4;

// ---- Worklet → Main (events) --------------------------------------------------

/// `VoiceFired { voice, accent }` — emitted when a voice actually fires.
pub const TAG_VOICE_FIRED: u8 = 0;
/// `XRun` — a process callback overran its deadline (diagnostic).
pub const TAG_XRUN: u8 = 1;

/// Encode an 8-byte control frame.
#[inline]
pub fn encode(tag: u8, a: u8, b: u8, value: f32) -> [u8; FRAME_LEN] {
    let v = value.to_le_bytes();
    [tag, a, b, 0, v[0], v[1], v[2], v[3]]
}

/// Decode the f32 value field of a frame.
#[inline]
fn frame_value(frame: &[u8]) -> f32 {
    f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]])
}

/// Apply every control frame in `bytes` to `engine`. Silently ignores unknown
/// tags / ids and any trailing partial frame — the audio thread must never
/// panic on a malformed message.
pub fn apply_messages(engine: &mut Engine, bytes: &[u8]) {
    let mut off = 0;
    while off + FRAME_LEN <= bytes.len() {
        let f = &bytes[off..off + FRAME_LEN];
        let (tag, a, b) = (f[0], f[1], f[2]);
        let value = frame_value(f);
        match tag {
            TAG_TRIGGER => {
                if let Some(v) = Voice::from_id(a) {
                    engine.trigger(v, b != 0);
                }
            }
            TAG_PARAM_SET => {
                if b == PARAM_LEVEL {
                    if let Some(v) = Voice::from_id(a) {
                        engine.set_level(v, value);
                    }
                }
            }
            TAG_GLOBAL_SET => match a {
                GLOBAL_ACCENT_LEVEL => engine.set_accent_level(value),
                GLOBAL_MASTER_LEVEL => engine.set_master_level(value),
                GLOBAL_DRIVE => engine.set_drive(value),
                GLOBAL_VARIANCE_DEPTH => engine.set_variance_depth(value),
                GLOBAL_SEED => engine.set_seed(value as u64),
                _ => {}
            },
            _ => {}
        }
        off += FRAME_LEN;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrips_value() {
        let f = encode(TAG_PARAM_SET, Voice::Sd as u8, PARAM_LEVEL, 0.625);
        assert_eq!(f[0], TAG_PARAM_SET);
        assert_eq!(frame_value(&f), 0.625);
    }

    #[test]
    fn apply_sets_level_and_triggers() {
        let mut e = Engine::new(48_000.0);
        let mut msg = Vec::new();
        msg.extend_from_slice(&encode(TAG_PARAM_SET, Voice::Bd as u8, PARAM_LEVEL, 0.9));
        msg.extend_from_slice(&encode(TAG_TRIGGER, Voice::Bd as u8, 1, 0.0));
        apply_messages(&mut e, &msg);
        // Triggers fire during process(); run one block, then a VoiceFired
        // frame should be available to drain.
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        e.process(&mut l, &mut r);
        assert!(!e.drain_events().is_empty());
    }
}
