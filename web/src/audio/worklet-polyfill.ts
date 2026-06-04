// Polyfill TextDecoder/TextEncoder for AudioWorkletGlobalScope.
//
// The wasm-bindgen glue (`dsp_wasm.js`) instantiates `new TextDecoder(...)` at
// module top level. That global exists on Window and in Workers, but NOT in
// AudioWorkletGlobalScope — so without this the glue throws on load and
// `registerProcessor('mm06', ...)` never runs (the "node name 'mm06' is not
// defined" error).
//
// This module is imported FIRST by worklet.ts, so by ES module evaluation order
// it runs before the glue. MM-06's control protocol carries no strings across
// the boundary, so these are exercised only by wasm-bindgen's internal helpers
// (an empty priming `.decode()`); the implementations are nonetheless correct
// UTF-8 so anything that does cross — e.g. a panic message — survives.

const g = globalThis as unknown as {
  TextDecoder?: unknown;
  TextEncoder?: unknown;
};

if (typeof g.TextDecoder === "undefined") {
  class TextDecoderPolyfill {
    readonly encoding = "utf-8";
    constructor(_label?: string, _options?: unknown) {}
    decode(input?: ArrayBufferView | ArrayBuffer): string {
      if (!input) return "";
      const bytes =
        input instanceof ArrayBuffer
          ? new Uint8Array(input)
          : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      let out = "";
      let i = 0;
      while (i < bytes.length) {
        const b0 = bytes[i++];
        let cp: number;
        if (b0 < 0x80) {
          cp = b0;
        } else if (b0 < 0xe0) {
          cp = ((b0 & 0x1f) << 6) | (bytes[i++] & 0x3f);
        } else if (b0 < 0xf0) {
          cp = ((b0 & 0x0f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
        } else {
          cp =
            ((b0 & 0x07) << 18) |
            ((bytes[i++] & 0x3f) << 12) |
            ((bytes[i++] & 0x3f) << 6) |
            (bytes[i++] & 0x3f);
        }
        out += String.fromCodePoint(cp);
      }
      return out;
    }
  }
  g.TextDecoder = TextDecoderPolyfill;
}

if (typeof g.TextEncoder === "undefined") {
  class TextEncoderPolyfill {
    readonly encoding = "utf-8";
    encode(input = ""): Uint8Array {
      const bytes: number[] = [];
      for (const ch of input) {
        let cp = ch.codePointAt(0)!;
        if (cp < 0x80) {
          bytes.push(cp);
        } else if (cp < 0x800) {
          bytes.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
        } else if (cp < 0x10000) {
          bytes.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
        } else {
          bytes.push(
            0xf0 | (cp >> 18),
            0x80 | ((cp >> 12) & 0x3f),
            0x80 | ((cp >> 6) & 0x3f),
            0x80 | (cp & 0x3f),
          );
        }
      }
      return new Uint8Array(bytes);
    }
  }
  g.TextEncoder = TextEncoderPolyfill;
}
