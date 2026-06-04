//! Cross-architecture parity probe (calibration-and-testing-strategy.md §5).
//!
//! Renders a single **canonical** `RenderRequest` through `dsp_core::render`
//! and writes the raw IEEE-754 bit pattern of every output sample to stdout,
//! one `%08x` per line. The point: build this same example for **two targets**
//! and diff the two streams. If a single bit differs, the "test native, ship
//! wasm" assumption is broken.
//!
//! It uses only `std` I/O to stdout (no allocation tricks, no platform calls),
//! so it compiles unchanged to native AND to `wasm32-wasip1`, the second of
//! which runs under `wasmtime`/`wasmer`. See `scripts/parity.sh` for the driver
//! and `docs/parity.md` for the exact commands and the current status on this
//! machine.
//!
//! The fixture here MUST stay identical for every target — it is the shared
//! input both sides render. Keep it in lockstep with any change to the engine's
//! public scheduling behaviour.

use dsp_core::{render, RenderRequest, TrigEvent, Voice};
use std::io::Write;

/// The canonical parity fixture. Touches every voice, both accent states, and a
/// spread of onsets (block edges, shared samples, mid-buffer) so the diff
/// exercises the whole per-sample path, not just a single bleep.
fn fixture() -> RenderRequest {
    RenderRequest {
        sample_rate: 48_000.0,
        seed: 0x6060_6060,
        variance_depth: 0.0,
        block_size: 128,
        events: vec![
            TrigEvent { sample_index: 0, voice: Voice::Bd, accent: true },
            TrigEvent { sample_index: 0, voice: Voice::Ch, accent: false },
            TrigEvent { sample_index: 127, voice: Voice::Sd, accent: true },
            TrigEvent { sample_index: 128, voice: Voice::Lt, accent: false },
            TrigEvent { sample_index: 1500, voice: Voice::Ht, accent: true },
            TrigEvent { sample_index: 9001, voice: Voice::Cy, accent: false },
            TrigEvent { sample_index: 9001, voice: Voice::Oh, accent: true },
            TrigEvent { sample_index: 20_000, voice: Voice::Bd, accent: true },
        ],
        length_samples: 24_000,
    }
}

fn main() {
    let buf = render(&fixture());

    // Stream hex bits to stdout. A `BufWriter` keeps it fast under WASI too.
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for s in &buf {
        // One canonical line per sample: zero-padded 32-bit hex of the bits.
        writeln!(out, "{:08x}", s.to_bits()).expect("write");
    }
    out.flush().expect("flush");
}
