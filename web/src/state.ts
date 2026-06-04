// Kit state — the main thread is the source of truth (architecture.md
// §"Main-thread layer"). The worklet holds only a DSP-side mirror as a smoother
// target; on boot (or audio re-create) we replay the whole kit into it.

import {
  Voice,
  type VoiceId,
  GLOBAL_MASTER_LEVEL,
  GLOBAL_ACCENT_LEVEL,
  GLOBAL_DRIVE,
  GLOBAL_VARIANCE_DEPTH,
  GLOBAL_SEED,
} from "./protocol";
import type { Bridge } from "./bridge";

export const VOICES: { id: VoiceId; key: string; label: string; name: string }[] = [
  { id: Voice.Bd, key: "Q", label: "BD", name: "Bass Drum" },
  { id: Voice.Sd, key: "W", label: "SD", name: "Snare Drum" },
  { id: Voice.Lt, key: "E", label: "LT", name: "Low Tom" },
  { id: Voice.Ht, key: "R", label: "HT", name: "High Tom" },
  { id: Voice.Cy, key: "T", label: "CY", name: "Cymbal" },
  { id: Voice.Oh, key: "Y", label: "OH", name: "Open Hat" },
  { id: Voice.Ch, key: "U", label: "CH", name: "Closed Hat" },
];

export interface KitState {
  levels: Record<number, number>; // voice id -> 0..1
  accents: Record<number, boolean>; // voice id -> accent toggle
  master: number;
  accentLevel: number;
  drive: number;
  variance: number;
  seed: number;
}

export function defaultKit(): KitState {
  const levels: Record<number, number> = {};
  const accents: Record<number, boolean> = {};
  for (const v of VOICES) {
    levels[v.id] = 0.5; // dev-ui-spec default
    accents[v.id] = false;
  }
  return {
    levels,
    accents,
    master: 0.8,
    accentLevel: 0.8,
    drive: 0.0,
    variance: 0.0,
    seed: 0,
  };
}

/** Push the entire kit into the worklet (boot / re-create). */
export function replay(bridge: Bridge, kit: KitState): void {
  for (const v of VOICES) {
    bridge.setLevel(v.id, kit.levels[v.id]);
  }
  bridge.setGlobal(GLOBAL_MASTER_LEVEL, kit.master);
  bridge.setGlobal(GLOBAL_ACCENT_LEVEL, kit.accentLevel);
  bridge.setGlobal(GLOBAL_DRIVE, kit.drive);
  bridge.setGlobal(GLOBAL_VARIANCE_DEPTH, kit.variance);
  bridge.setGlobal(GLOBAL_SEED, kit.seed);
}
