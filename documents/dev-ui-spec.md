# Dev UI Spec (temporary)

A throwaway control surface for auditioning the engine during development. **Not** the shipping UI — no styling polish, persistence, patterns, or theming required.

## Voices & key map

Seven voices, all analog/synthesized (no samples). One key per voice along the QWERTY row
in kit order. Each voice also has an on-screen **Trigger** button doing the same thing.

| Key | Voice      | Engine voice id | Notes                                                      |
| --- | ---------- | --------------- | ---------------------------------------------------------- |
| `Q` | Bass Drum  | `bd`            | two bridged-T resonators (spec §4.1)                       |
| `W` | Snare Drum | `sd`            | bridged-T + gated noise (spec §4.2)                        |
| `E` | Low Tom    | `lt`            | bridged-T + shared gated noise (spec §4.3)                 |
| `R` | High Tom   | `ht`            | bridged-T + shared gated noise (spec §4.3)                 |
| `T` | Cymbal     | `cy`            | metallic cluster, both bandpass paths (spec §6.2)          |
| `Y` | Open Hat   | `oh`            | shares metallic source + 7100 Hz path with CH              |
| `U` | Closed Hat | `ch`            | triggering CH chokes a ringing OH (engine-side, spec §6.3) |

The cymbal, open hat, and closed hat **all share one six-oscillator metallic source** and
its band-pass paths (spec §3.6, §6.1); they differ in which paths they use and in their
envelopes. The CH→OH choke is handled inside the engine — the UI just fires the triggers.

## Per-voice controls

Each voice gets exactly three things: a **Trigger** (key + button), an **Accent** toggle
(off by default; sets the binary `accent` flag on that voice's trigger), and a **LEVEL**
slider. The LEVEL slider is **normalized 0.0–1.0** (min → max), default **0.5**; the engine
maps position to its internal range.

| Voice      | Slider (`param_id`) | Accent | Trigger      |
| ---------- | ------------------- | ------ | ------------ |
| Bass Drum  | `LEVEL`             | toggle | `Q` / button |
| Snare Drum | `LEVEL`             | toggle | `W` / button |
| Low Tom    | `LEVEL`             | toggle | `E` / button |
| High Tom   | `LEVEL`             | toggle | `R` / button |
| Cymbal     | `LEVEL`             | toggle | `T` / button |
| Open Hat   | `LEVEL`             | toggle | `Y` / button |
| Closed Hat | `LEVEL`             | toggle | `U` / button |

All seven voices have **independent** LEVEL ids. This is the engine's actual
mixer (spec §7): MM-06 splits the stock 606's two shared pots (Toms L+H, Hi-Hat O+C) so
every voice gets its own level. There is nothing to emulate or recombine — the seven sliders
_are_ the panel.

There are **no** per-voice timbral controls (TUNE / DECAY / TONE / SNAPPY / metal-tune / HPF)
in the engine, so there are none on this rig (spec §9). Each voice's pitch, decay, and tone
are fixed at the calibrated default and are tuned offline via the render binary and analysis
lib, where a numeric target decides correctness instead of an ear.

## Global controls

| Control        | Engine message             | Range / default                   | Why                                                                                                                                                                                   |
| -------------- | -------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Master Level   | `GlobalSet level`          | 0.0–1.0, default 0.8              | the requested global level (the stock VOLUME pot)                                                                                                                                     |
| Accent Level   | `GlobalSet accent_level`   | 0.0–1.0, default 0.8 (~3 o'clock) | depth applied when a voice's accent toggle is on; lets you A/B "brighter/longer, not just louder". This is a **real** stock-panel control.                                 |
| Drive          | `GlobalSet drive`          | 0.0–1.0, default 0.0 (clean)      | output-amp drive into the bus soft-saturation stage (spec §7); not a stock-panel control, but the engine exposes it as a global, so the rig surfaces it for A/B of the saturation.    |
| Variance Depth | `GlobalSet variance_depth` | 0.0–1.0, default 0.0              | 0 = sterile/deterministic; matches offline renders bit-for-bit for debugging (spec §10). At >0 the dominant audible effect is the free-running metallic phase scatter on hats/cymbal. |
| Seed           | `GlobalSet seed`           | integer field, default 0          | reproducible per-hit variance — including the metallic cluster phase — so a live audition matches an offline render (spec §10)                                                       |

Variance/seed are dev conveniences but cheap and high-value given the determinism
requirements — keep them. Note that with `variance_depth = 0` the metallic voices still
sound correct, but every hit is identical; turning it up is what makes repeated hats/cymbals
stop machine-gunning.

## Interaction rules

- **Trigger on `keydown`**, and **suppress auto-repeat** (ignore repeated keydown while a
  key is held) so a held key doesn't machine-gun the voice. Trigger fires immediately.
- **Don't fire voice keys while a text input is focused** (the Seed field), so typing a
  seed doesn't trigger drums.
- A voice's trigger sends `Trigger { voice, accent }` with `accent` taken from that voice's
  toggle.
- LEVEL sliders send `ParamSet { voice, param_id, value }` on input, **throttled to
  animation-frame rate** (architecture.md). Globals send `GlobalSet { id, value }`.
- Mouse and keyboard are equivalent — the button and the key both emit the same `Trigger`.

## Suggested layout

```
┌─ Globals ───────────────────────────────────────────────┐
│  Master [====|====]   Accent [======|==]   Drive [|=====] │
│  Variance [|========]   Seed [ 0        ]                 │
└──────────────────────────────────────────────────────────┘
┌ BD (Q) ──┐ ┌ SD (W) ──┐ ┌ LT (E) ──┐ ┌ HT (R) ──┐
│ LEVEL    │ │ LEVEL    │ │ LEVEL    │ │ LEVEL    │
│ [==|==]  │ │ [==|==]  │ │ [==|==]  │ │ [==|==]  │
│ [Accent] │ │ [Accent] │ │ [Accent] │ │ [Accent] │
└──────────┘ └──────────┘ └──────────┘ └──────────┘
┌ CY (T) ──┐ ┌ OH (Y) ──┐ ┌ CH (U) ──┐
│ LEVEL    │ │ LEVEL    │ │ LEVEL    │
│ [==|==]  │ │ [==|==]  │ │ [==|==]  │
│ [Accent] │ │ [Accent] │ │ [Accent] │
└──────────┘ └──────────┘ └──────────┘
              (U=CH chokes a ringing OH — engine-side)
```

One small panel per voice (key label, LEVEL slider, Accent toggle), global strip on top.
Exact arrangement doesn't matter — this is scaffolding.
