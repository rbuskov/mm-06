/// <reference path="./worklet-env.d.ts" />
//
// The AudioWorklet processor (architecture.md §"Audio runtime"). It is
// deliberately thin: it owns one `dsp_wasm` `Engine`, forwards queued control
// frames into it, runs `engine.process()` for the 128-sample quantum, copies the
// result into the Web Audio output, and posts drained diagnostic frames back.
// It never blocks, never `await`s, and does no fetching (the compiled WASM
// module is handed in via `processorOptions`).
//
// This file is bundled by esbuild into `public/worklet.js` (see package.json
// `build:worklet`) so it loads as a single self-contained script — no ESM /
// fetch in the worklet scope.

// Must run before the wasm-bindgen glue (it touches TextDecoder at load time).
import "./worklet-polyfill.js";
import { initSync, Engine } from "../../wasm/dsp_wasm.js";

interface MM06Options {
  processorOptions: { module: WebAssembly.Module };
}

class MM06Processor extends AudioWorkletProcessor {
  private engine: Engine;

  constructor(options?: unknown) {
    super();
    const opts = options as MM06Options;
    // Instantiate the pre-compiled module synchronously (no fetch in worklet).
    initSync({ module: opts.processorOptions.module });
    this.engine = Engine.new(sampleRate);

    // Main → Worklet control frames arrive as transferred ArrayBuffers.
    this.port.onmessage = (e: MessageEvent) => {
      this.engine.handle_message(new Uint8Array(e.data as ArrayBuffer));
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    if (!out || out.length === 0) return true;
    const left = out[0];
    const right = out[1] ?? out[0];

    this.engine.process(left, right);

    // Post any diagnostic / VoiceFired frames back to the main thread.
    const ev = this.engine.drain_events();
    if (ev.length > 0) {
      this.port.postMessage(ev.buffer, [ev.buffer]);
    }
    return true; // keep the processor alive
  }
}

registerProcessor("mm06", MM06Processor);
