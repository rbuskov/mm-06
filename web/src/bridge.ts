// Main-thread side of the worklet message protocol (architecture.md
// §"Worklet bridge"). Owns the AudioContext + AudioWorkletNode and exposes a
// small typed API the UI calls. The compiled WASM module is handed to the
// worklet via `processorOptions`; the worklet itself does no fetching.

import wasmUrl from "../wasm/dsp_wasm_bg.wasm?url";
import {
  encodeFrame,
  decodeFrames,
  TAG_TRIGGER,
  TAG_PARAM_SET,
  TAG_GLOBAL_SET,
  TAG_VOICE_FIRED,
  TAG_XRUN,
  PARAM_LEVEL,
  type VoiceId,
} from "./protocol";

export interface BridgeEvents {
  onVoiceFired?: (voice: number, accent: boolean) => void;
  onXRun?: () => void;
}

export class Bridge {
  private ctx: AudioContext;
  private node: AudioWorkletNode;

  private constructor(ctx: AudioContext, node: AudioWorkletNode) {
    this.ctx = ctx;
    this.node = node;
  }

  get sampleRate(): number {
    return this.ctx.sampleRate;
  }

  /** Create the audio graph. Must be called from a user gesture. */
  static async create(events: BridgeEvents = {}): Promise<Bridge> {
    const ctx = new AudioContext();

    // Compile the WASM module on the main thread, then hand it to the worklet.
    const resp = await fetch(wasmUrl);
    const module = await WebAssembly.compile(await resp.arrayBuffer());

    await ctx.audioWorklet.addModule("/worklet.js");

    const node = new AudioWorkletNode(ctx, "mm06", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { module },
    });
    node.connect(ctx.destination);

    node.port.onmessage = (e: MessageEvent) => {
      const frames = decodeFrames(new Uint8Array(e.data as ArrayBuffer));
      for (const f of frames) {
        if (f.tag === TAG_VOICE_FIRED) {
          events.onVoiceFired?.(f.a, f.b !== 0);
        } else if (f.tag === TAG_XRUN) {
          events.onXRun?.();
        }
      }
    };

    await ctx.resume();
    return new Bridge(ctx, node);
  }

  private send(frame: Uint8Array): void {
    // Transfer the backing buffer for a zero-copy hand-off.
    this.node.port.postMessage(frame.buffer, [frame.buffer]);
  }

  trigger(voice: VoiceId, accent: boolean): void {
    this.send(encodeFrame(TAG_TRIGGER, voice, accent ? 1 : 0, 0));
  }

  setLevel(voice: VoiceId, value: number): void {
    this.send(encodeFrame(TAG_PARAM_SET, voice, PARAM_LEVEL, value));
  }

  setGlobal(id: number, value: number): void {
    this.send(encodeFrame(TAG_GLOBAL_SET, id, 0, value));
  }
}
