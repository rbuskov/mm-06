//! WASM↔native parity gate — native half (calibration-and-testing-strategy.md §5).
//!
//! The whole "test natively, ship wasm" approach rests on the two builds of
//! `dsp_core` agreeing bit-for-bit. This file pins the two halves of that
//! assumption that CAN be checked on a single architecture, with **zero**
//! tolerance (bit-identical via [`f32::to_bits`]):
//!
//! 1. **Streaming vs. offline equivalence.** The shipped worklet pushes
//!    `TrigEvent`s one at a time as control frames arrive (`dsp_wasm::Engine`
//!    → `protocol::apply_messages`/`Engine::trigger`); the offline `render()`
//!    gets the whole `events` list up front. Feeding the *same* events both
//!    ways must yield a byte-identical buffer. This also exercises the
//!    marshalling layer (the 8-byte protocol frames) end to end.
//!
//! 2. **Determinism of the streaming path itself** under blockwise driving.
//!
//! The remaining half — the same `render()` run on a *second* architecture
//! (`wasm32`) and diffed against native — is scaffolded in `scripts/parity.sh`
//! and documented in `docs/parity.md`. It is NOT executed by `cargo test`
//! because this machine has the `wasm32-unknown-unknown` target but no WASI
//! runtime (`wasmtime`/`wasmer`) and no `wasm-bindgen-test-runner`; see
//! `docs/parity.md` for the exact command to run it once a runtime is present.

use dsp_core::protocol::{
    apply_messages, encode, PARAM_LEVEL, TAG_GLOBAL_SET, TAG_PARAM_SET, TAG_TRIGGER,
    GLOBAL_DRIVE, GLOBAL_MASTER_LEVEL, GLOBAL_ACCENT_LEVEL,
};
use dsp_core::{render, Engine, RenderRequest, TrigEvent, Voice};

/// Bit-exact comparison: any single differing bit is a parity failure.
fn assert_bit_identical(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: length mismatch");
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x.to_bits() != y.to_bits() {
            panic!(
                "{what}: divergence at sample {i}: {x:?} (0x{:08x}) != {y:?} (0x{:08x})",
                x.to_bits(),
                y.to_bits()
            );
        }
    }
}

/// Drive `dsp_core::Engine` in the *streaming* manner: walk the timeline one
/// render quantum at a time and, at the exact sample boundary each event is due,
/// push it as an 8-byte protocol frame (the same bytes the worklet receives
/// from the main thread). The engine timestamps each trigger with its own
/// `global_sample`, so injecting at the right block reproduces the offline
/// schedule.
///
/// `setup` frames (levels/master/drive) are applied once up front, matching how
/// `render()` configures the engine before any audio runs.
fn render_streaming(
    sample_rate: f32,
    block_size: usize,
    length_samples: usize,
    setup: &[[u8; 8]],
    mut events: Vec<TrigEvent>,
) -> Vec<f32> {
    let mut e = Engine::new(sample_rate);

    // Apply the static control surface once, before the clock advances.
    let mut setup_bytes = Vec::new();
    for f in setup {
        setup_bytes.extend_from_slice(f);
    }
    apply_messages(&mut e, &setup_bytes);

    // Stable sort by onset so events at the same sample fire in list order,
    // exactly as the offline queue does.
    events.sort_by_key(|ev| ev.sample_index);

    let block = block_size.max(1);
    let mut out = vec![0.0f32; length_samples];
    let mut scratch_l = vec![0.0f32; block];
    let mut scratch_r = vec![0.0f32; block];

    let mut next = 0usize; // index into `events`
    let mut pos = 0usize; // current sample position
    while pos < length_samples {
        // Push every event whose onset falls within (or before) this quantum's
        // start sample. The engine stamps the trigger with the *current*
        // global_sample, so we must drain each event precisely when the clock
        // reaches its sample_index — i.e. one frame per sample boundary.
        let this = block.min(length_samples - pos);

        // Walk this quantum sample-by-sample only where an event lands, so the
        // trigger's stamped global_sample matches the offline sample_index.
        let mut local = 0usize;
        while local < this {
            // Fire any events due exactly at sample (pos + local).
            let mut msg = Vec::new();
            while next < events.len() && events[next].sample_index == pos + local {
                let ev = events[next];
                msg.extend_from_slice(&encode(
                    TAG_TRIGGER,
                    ev.voice as u8,
                    ev.accent as u8,
                    0.0,
                ));
                next += 1;
            }
            if !msg.is_empty() {
                apply_messages(&mut e, &msg);
            }
            // Advance exactly one sample so the next event's stamp is correct.
            e.process(
                &mut scratch_l[..1],
                &mut scratch_r[..1],
            );
            out[pos + local] = scratch_l[0];
            local += 1;
        }
        pos += this;
    }
    // Anything past the last event is irrelevant to scheduling but we already
    // rendered the whole buffer above.
    out
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// 1. Streaming (one frame per event, pushed at its onset) == offline
///    (`render()` with the whole list). Bit-identical, zero tolerance.
#[test]
fn streaming_matches_offline_bit_identical() {
    let sample_rate = 48_000.0f64;
    let block_size = 128;
    let length = 24_000;

    // A spread of voices, accents, and onsets — some sharing a sample boundary,
    // some mid-block, some on a block edge.
    let events = vec![
        TrigEvent { sample_index: 0, voice: Voice::Bd, accent: false },
        TrigEvent { sample_index: 0, voice: Voice::Ch, accent: true }, // same sample
        TrigEvent { sample_index: 127, voice: Voice::Sd, accent: true }, // block edge-1
        TrigEvent { sample_index: 128, voice: Voice::Lt, accent: false }, // block edge
        TrigEvent { sample_index: 1500, voice: Voice::Ht, accent: true },
        TrigEvent { sample_index: 9001, voice: Voice::Cy, accent: false },
        TrigEvent { sample_index: 9001, voice: Voice::Oh, accent: true }, // same sample
        TrigEvent { sample_index: 20_000, voice: Voice::Bd, accent: true },
    ];

    let offline = render(&RenderRequest {
        sample_rate,
        seed: 1,
        variance_depth: 0.0,
        block_size,
        events: events.clone(),
        length_samples: length,
    });

    let streaming = render_streaming(
        sample_rate as f32,
        block_size,
        length,
        &[],
        events,
    );

    assert_bit_identical(&offline, &streaming, "streaming-vs-offline (default params)");
}

/// 1b. The protocol marshalling layer is transparent: triggering a voice by
///     pushing an 8-byte `TAG_TRIGGER` frame through `apply_messages` must
///     produce a buffer byte-identical to calling `Engine::trigger` directly,
///     even with a non-default control surface (levels/master/drive/accent)
///     applied — also via protocol frames vs. direct setters. This proves the
///     wire encoding/decoding does not perturb a single bit and that the
///     streaming control path is exact.
#[test]
fn protocol_path_matches_direct_api_bit_identical() {
    let sr = 48_000.0f32;
    let block = 128usize;
    let length = 16_000usize;

    let events = vec![
        (10usize, Voice::Bd, false),
        (4000, Voice::Sd, true),
        (4000, Voice::Cy, false),
        (12_000, Voice::Bd, true),
    ];

    // --- Path A: drive entirely through protocol frames. ---
    let via_protocol = {
        let mut e = Engine::new(sr);
        let setup = [
            encode(TAG_GLOBAL_SET, GLOBAL_MASTER_LEVEL, 0, 0.7),
            encode(TAG_GLOBAL_SET, GLOBAL_ACCENT_LEVEL, 0, 0.9),
            encode(TAG_GLOBAL_SET, GLOBAL_DRIVE, 0, 0.4),
            encode(TAG_PARAM_SET, Voice::Bd as u8, PARAM_LEVEL, 0.6),
            encode(TAG_PARAM_SET, Voice::Sd as u8, PARAM_LEVEL, 0.8),
            encode(TAG_PARAM_SET, Voice::Cy as u8, PARAM_LEVEL, 0.3),
        ];
        let mut bytes = Vec::new();
        for f in &setup {
            bytes.extend_from_slice(f);
        }
        apply_messages(&mut e, &bytes);
        drive_with(&mut e, sr, block, length, &events, |e, v, a| {
            let f = encode(TAG_TRIGGER, v as u8, a as u8, 0.0);
            apply_messages(e, &f);
        })
    };

    // --- Path B: drive through the direct Rust API (no wire). ---
    let via_api = {
        let mut e = Engine::new(sr);
        e.set_master_level(0.7);
        e.set_accent_level(0.9);
        e.set_drive(0.4);
        e.set_level(Voice::Bd, 0.6);
        e.set_level(Voice::Sd, 0.8);
        e.set_level(Voice::Cy, 0.3);
        drive_with(&mut e, sr, block, length, &events, |e, v, a| {
            e.trigger(v, a);
        })
    };

    assert_bit_identical(&via_protocol, &via_api, "protocol-vs-direct-api");
}

/// Render `length` samples, advancing one sample at a time so each trigger is
/// stamped with the correct `global_sample`, firing every event due at the
/// current sample via `fire` (the closure decides protocol-frame vs. direct).
fn drive_with<F: FnMut(&mut Engine, Voice, bool)>(
    e: &mut Engine,
    _sr: f32,
    _block: usize,
    length: usize,
    events: &[(usize, Voice, bool)],
    mut fire: F,
) -> Vec<f32> {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(|(s, _, _)| *s);
    let mut out = vec![0.0f32; length];
    let mut l = [0.0f32; 1];
    let mut r = [0.0f32; 1];
    let mut next = 0usize;
    for (pos, slot) in out.iter_mut().enumerate() {
        while next < sorted.len() && sorted[next].0 == pos {
            let (_, v, a) = sorted[next];
            fire(e, v, a);
            next += 1;
        }
        e.process(&mut l, &mut r);
        *slot = l[0];
    }
    out
}

/// 2. The streaming path is itself deterministic across block sizes: same
///    events, different quantum, byte-identical (a precondition for parity).
#[test]
fn streaming_block_size_invariant_bit_identical() {
    let sr = 48_000.0f32;
    let length = 20_000;
    let events = vec![
        TrigEvent { sample_index: 100, voice: Voice::Sd, accent: true },
        TrigEvent { sample_index: 5000, voice: Voice::Ch, accent: false },
        TrigEvent { sample_index: 5000, voice: Voice::Oh, accent: true },
    ];
    let a = render_streaming(sr, 64, length, &[], events.clone());
    let b = render_streaming(sr, 512, length, &[], events);
    assert_eq!(bits(&a), bits(&b), "streaming not block-size invariant");
}
