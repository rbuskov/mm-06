// Control/event protocol — the TypeScript mirror of `crates/dsp_core/src/protocol.rs`.
//
// Architecture.md calls for this to be *generated* from the Rust `#[repr(C)]`
// schema; for the walking skeleton it is hand-mirrored. Keep the constants and
// the 8-byte little-endian frame layout in lockstep with the Rust file:
//
//   byte 0   tag
//   byte 1   a      (voice id / global id)
//   byte 2   b      (param id / accent flag)
//   byte 3   pad
//   bytes 4..8  value (f32 LE)

export const FRAME_LEN = 8;

// Voice ids (kit order — dev-ui-spec.md).
export const Voice = {
  Bd: 0,
  Sd: 1,
  Lt: 2,
  Ht: 3,
  Cy: 4,
  Oh: 5,
  Ch: 6,
} as const;
export type VoiceId = (typeof Voice)[keyof typeof Voice];

// Main → Worklet tags.
export const TAG_TRIGGER = 0;
export const TAG_PARAM_SET = 1;
export const TAG_GLOBAL_SET = 2;

// Per-voice param ids.
export const PARAM_LEVEL = 0;

// Global ids.
export const GLOBAL_ACCENT_LEVEL = 0;
export const GLOBAL_MASTER_LEVEL = 1;
export const GLOBAL_DRIVE = 2;
export const GLOBAL_VARIANCE_DEPTH = 3;
export const GLOBAL_SEED = 4;

// Worklet → Main tags.
export const TAG_VOICE_FIRED = 0;
export const TAG_XRUN = 1;

/** Build one 8-byte control frame. */
export function encodeFrame(tag: number, a: number, b: number, value: number): Uint8Array {
  const buf = new Uint8Array(FRAME_LEN);
  const dv = new DataView(buf.buffer);
  dv.setUint8(0, tag);
  dv.setUint8(1, a);
  dv.setUint8(2, b);
  dv.setUint8(3, 0);
  dv.setFloat32(4, value, true);
  return buf;
}

export interface DecodedEvent {
  tag: number;
  a: number;
  b: number;
  value: number;
}

/** Decode a buffer of concatenated 8-byte event frames (Worklet → Main). */
export function decodeFrames(bytes: Uint8Array): DecodedEvent[] {
  const out: DecodedEvent[] = [];
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let off = 0; off + FRAME_LEN <= bytes.byteLength; off += FRAME_LEN) {
    out.push({
      tag: dv.getUint8(off),
      a: dv.getUint8(off + 1),
      b: dv.getUint8(off + 2),
      value: dv.getFloat32(off + 4, true),
    });
  }
  return out;
}
