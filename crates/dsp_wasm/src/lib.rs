//! `dsp_wasm` — the thin `wasm-bindgen` shell the AudioWorklet drives
//! (architecture.md §"Audio runtime"). It owns a `dsp_core::Engine` and does
//! nothing but marshal the streaming render quantum and the binary control
//! protocol across the JS↔WASM boundary. All DSP lives in `dsp_core`.
//!
//! There is deliberately **no `load_sample` method** — MM-06 has no samples.

use dsp_core::{protocol, Engine as CoreEngine};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Engine {
    core: CoreEngine,
}

#[wasm_bindgen]
impl Engine {
    /// Construct an engine for the host sample rate (the worklet's `sampleRate`).
    pub fn new(sample_rate: f32) -> Engine {
        Engine {
            core: CoreEngine::new(sample_rate),
        }
    }

    /// Render one quantum into the Web Audio output channels (dual-mono).
    pub fn process(&mut self, out_left: &mut [f32], out_right: &mut [f32]) {
        self.core.process(out_left, out_right);
    }

    /// Apply a batch of control frames (Main → Worklet).
    pub fn handle_message(&mut self, bytes: &[u8]) {
        protocol::apply_messages(&mut self.core, bytes);
    }

    /// Drain queued diagnostic/event frames (Worklet → Main).
    pub fn drain_events(&mut self) -> Vec<u8> {
        self.core.drain_events()
    }
}
