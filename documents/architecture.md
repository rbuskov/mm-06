# MM-06 (TR-606 Emulation) — Architecture

> "MM-06" is the project/product name. It deliberately avoids "Roland" / "TR-606"

This is a browser-only application. There is no backend: the drum engine, MIDI handling, and UI all run client-side. The system is split into two languages crossing a single boundary:

- **Rust**, compiled to **WebAssembly**, implements all real-time audio code (the per-voice DSP and mixer) and runs inside an **AudioWorklet**.
- **TypeScript** implements the UI, control surface, and MIDI I/O, running on the main thread.

The two halves communicate through a small, typed message protocol over the worklet's `MessagePort`.

One decision shapes everything below and is **not** inherited from a typical web-synth layout: the DSP is a **pure Rust library (`dsp_core`) with no web/`wasm-bindgen` dependency**, compiled from identical source to *two* targets — the `wasm32` AudioWorklet build, and a *native* build that drives an offline render binary and the test suite. This done to enable automated testing and calibration by AI using MCP tools. The worklet and the offline renderer are two thin front-ends over the same deterministic core.

> **No sampled voices.** MM-06 is fully analog — all seven voices are synthesized — so there
> is no PCM asset loading, no resampler, and no captured-sample playback anywhere. The
> reference clips (one per voice) exist only to *calibrate* the synthesis (regime C), never to
> be played back.

## High-level layout

```
┌────────────────────────────── Main thread (TypeScript) ──────────────────────────────┐
│                                                                                      │
│   ┌──────────────┐    ┌──────────────┐    ┌────────────────┐    ┌────────────────┐   │
│   │  Panel UI    │    │  Trigger     │    │  MIDI I/O      │    │  Kit state     │   │
│   │  per-voice   │    │  pads        │    │  (Web MIDI,    │    │  (control vals,│   │
│   │  controls    │    │  (audition)  │    │   drum map)    │    │   persistence) │   │
│   └──────┬───────┘    └──────┬───────┘    └───────┬────────┘    └────────┬───────┘   │
│          │                   │                    │                      │           │
│          └───────────────────┴────────┬───────────┴──────────────────────┘           │
│                                       │                                              │
│                              ┌────────▼─────────┐                                    │
│                              │  Worklet bridge  │  ← typed binary messages           │
│                              │  (MessagePort)   │                                    │
│                              └────────┬─────────┘                                    │
└───────────────────────────────────────┼──────────────────────────────────────────────┘
                                        │
┌──────────────────────── Audio thread (AudioWorklet) ─────────────────────────────────┐
│                                                                                      │
│                          ┌───────────────▼────────────────┐                          │
│                          │  dsp_wasm (thin Engine shell)   │                          │
│                          │  ┌───────────────────────────┐  │                          │
│                          │  │ dsp_core (pure Rust)       │  │                          │
│                          │  │ ┌───────────────────────┐ │  │                          │
│                          │  │ │ Voices (ALL analog):  │ │  │                          │
│                          │  │ │  tonal: BD, SD,       │ │  │                          │
│                          │  │ │   LT, HT              │ │  │                          │
│                          │  │ │  metallic: CY,        │ │  │                          │
│                          │  │ │   OH, CH              │ │  │                          │
│                          │  │ ├───────────────────────┤ │  │                          │
│                          │  │ │ Mixer + output sat,   │ │  │                          │
│                          │  │ │ oversampling, variance│ │  │                          │
│                          │  │ └───────────┬───────────┘ │  │                          │
│                          │  └─────────────┼─────────────┘  │                          │
│                          └────────────────┼────────────────┘                          │
│                                           │ audio buffer (f32)                         │
│                                ┌──────────▼───────────┐                               │
│                                │ AudioContext output  │ → speakers                    │
│                                └──────────────────────┘                               │
└──────────────────────────────────────────────────────────────────────────────────────┘

         the SAME dsp_core, driven offline for tests (no browser, native build):

         bin/render ──RenderRequest{events,…}──► dsp_core::render() ──► WAV ──► analysis/ (Python) ──► metrics
```

The two front-ends in that picture — the live worklet and the offline `render` binary — feed `dsp_core` through the *same* trigger interface and must produce identical output for identical input (the WASM↔native parity requirement). That equivalence is the whole reason the fidelity work can be automated.

## Why this split

- **AudioWorklet for DSP** is the only mechanism in the browser that runs audio code on a dedicated real-time-priority thread with a fixed 128-sample render quantum. Anything that must not glitch (the voices and their envelopes) lives here.
- **WebAssembly for the DSP language** gives deterministic, allocation-free code at near-native speed with no GC pauses in the audio callback. The 606 is a relatively light engine, but it still wants up-to-8× oversampling around the six square oscillators and the nonlinear stages, which would not meet the latency budget in JS at full polyphony.
- **Rust specifically**, and as a *host-agnostic library*: memory safety without a runtime, and — critically — the ability to compile one source to both `wasm32` (ship) and native (test). The native build lets ~99% of DSP tests run as fast `cargo test` and lets an offline `render` binary dump WAVs that a Python analysis layer measures. This is what makes the spec's numeric targets (kick 60/130 Hz resonators, metallic oscillators ≈ 245–625 Hz, 7100/3440 Hz band-pass, …) machine-checkable without a browser in the loop.
- **TypeScript on the main thread** for everything that touches the DOM, Web MIDI, or persistent state. MIDI I/O on the main thread is not a choice — `navigator.requestMIDIAccess()` is only available there.
- **The DSP core has no I/O, no globals, no time, and is deterministic.** Identical `(seed, params, events, sample_rate, block_size)` produces a byte-identical buffer. This is a hard requirement for effective testing. Per-hit "analog variance" — including the **free-running metallic oscillator phase** — is driven by a seeded PRNG, never wall-clock or thread state.

## Project layout

```
mm06/
├── Cargo.toml                  # workspace (dsp_core + dsp_wasm + render)
├── crates/
│   ├── dsp_core/               # PURE Rust DSP library — NO web/wasm-bindgen deps
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          # RenderRequest, render(), render_voice(), Engine (pure)
│   │       ├── blocks/         # shared §3 primitives, instanced per voice:
│   │       │                   #   bridged-T resonator (the core tonal block),
│   │       │                   #   trigger-pulse/excitation generator, RC envelopes,
│   │       │                   #   shared analog noise, six-osc Schmitt metallic bank,
│   │       │                   #   bridged-T band-pass (7100/3440), HPF stages, VCA+grit
│   │       ├── voices/         # §4 tonal: BD (2 resonators), SD, tom (LT/HT = one model,
│   │       │                   #   2 tunings).  §6 metallic: CY, hat (CH+OH from one
│   │       │                   #   shared free-running cluster, choke group)
│   │       ├── mix/            # §7 summing mixer + bus/output saturation, DC blockers
│   │       ├── variance/       # §10 per-hit jitter + per-instance tolerance + metallic
│   │       │                   #   free-run phase (seeded PRNG)
│   │       ├── oversample/     # §2.2 host-rate-scaled factor, polyphase FIR up/down, BLEP/BLAMP edge correction
│   │       └── mathx/          # vendored fixed-poly sin/exp/tanh for WASM↔native bit-parity (§7.1)
│   ├── dsp_wasm/               # THIN wasm-bindgen wrapper around dsp_core (ship target)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs          # Engine: process / handle_message / drain_events + binary protocol
│   └── render/                 # offline render binary (native target, depends on dsp_core)
│       └── src/main.rs         # JSON/CLI RenderRequest → WAV (+ sidecar JSON). Deterministic.
├── web/                        # TypeScript app (Vite)
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   ├── main.ts                 # entry, AudioContext setup, worklet load
│   ├── audio/
│   │   ├── worklet.ts          # AudioWorkletProcessor (imports dsp_wasm)
│   │   └── bridge.ts           # main-thread side of the message protocol
│   ├── midi/                   # Web MIDI in/out, GM-drum-map ↔ voice triggers
│   ├── ui/                     # per-voice panel, trigger pads (sequencer/transport UI out of scope)
│   ├── state/                  # kit state (control values), persistence
│   └── wasm/                   # wasm-pack output (gitignored)
├── analysis/                   # Python: WAV → metrics JSON (numpy / scipy / soundfile / librosa)
├── harness/                    # `dsp-harness` MCP server wrapping render + analysis
├── references/                 # human-supplied reference clips, ONE flat .wav per voice
│   ├── bd.wav                  # Bass Drum
│   ├── sd.wav                  # Snare Drum
│   ├── lt.wav                  # Low Tom
│   ├── ht.wav                  # High Tom
│   ├── cy.wav                  # Cymbal
│   ├── oh.wav                  # Open Hat
│   └── ch.wav                  # Closed Hat
└── documents/
    ├── architecture.md
    ├── audio-engine-spec.md
    ├── calibration-and-testing-strategy.md
    └── dev-ui-spec.md
```

The Rust side is a **Cargo workspace** with three members: `dsp_core` (pure, dual-target), `dsp_wasm` (the `wasm-bindgen` shell), and `render` (the offline binary). The split keeps `dsp_core`'s dependency surface free of anything browser-specific so it stays compilable to native; `dsp_wasm` and `render` are the only crates that pull in target-specific deps. Build output (`.wasm` + JS glue) is emitted into `web/wasm/` and imported from `worklet.ts`. The Vite `root` is `web/`.

## DSP layer (Rust → WASM)

`dsp_core` exposes a pure, deterministic function shape with no I/O, globals, or clock reads:

```rust
// dsp_core — host-agnostic. Compiled to BOTH wasm32 and native from this source.
pub struct RenderRequest {
    pub sample_rate: f64,        // host rate: 44100, 48000, 88200, 96000…
    pub seed: u64,               // seeds ALL per-hit variance incl. metallic phase (spec §10)
    pub variance_depth: f32,     // 0.0 = sterile / fully repeatable (spec §10)
    pub block_size: usize,       // process() quantum; 128 for the AudioWorklet
    pub events: Vec<TrigEvent>,  // (sample_index, voice, params, accent)
    pub length_samples: usize,
}

pub fn render(req: &RenderRequest) -> Vec<f32>;             // main mix
pub fn render_voice(req: &RenderRequest, v: Voice) -> Vec<f32>; // offline: one voice in isolation, pre-mix — calibration only, NOT a shipped output (spec §2.1)
```

`dsp_wasm` wraps that core in the streaming `Engine` the worklet drives per render quantum:

```rust
#[wasm_bindgen]
pub struct Engine { /* owns a dsp_core engine: voices, mixer */ }

#[wasm_bindgen]
impl Engine {
    pub fn new(sample_rate: f32) -> Engine;
    pub fn process(&mut self, out_left: &mut [f32], out_right: &mut [f32]);
    pub fn handle_message(&mut self, bytes: &[u8]); // control → DSP
    pub fn drain_events(&mut self) -> Vec<u8>;      // DSP → main (diagnostics, voice-fired)
}
```

Note the **absence of any `load_sample` method** — there are no samples in this engine.

The streaming `Engine` and the offline `render()` are the same DSP scheduled two ways: the worklet pushes `TrigEvent`s as messages arrive; `render()` takes the whole `events` list up front. Both must agree to the WASM↔native parity tolerance.

Rules the core must follow:

- **Deterministic given inputs.** No reads from `Date`, no unseeded RNG. The variance layer — including the free-running metallic cluster's phase — draws only from the seeded PRNG. Treat any nondeterminism as a bug.
- **No allocation in `process()` / steady-state render.** All buffers are pre-sized at construction. Events emitted from `process()` go into a fixed-capacity ring that `drain_events` consumes between callbacks. A native test runs under an allocator that flags heap traffic inside the render scope.
- **No panics in `process()`.** Checked construction at setup, proven-bounds `get_unchecked` in hot loops. No `NaN`/`Inf` ever reaches the output.
- **`f32` audio buffers**; `sample_rate` carried as `f64`. The square-oscillator bank and nonlinear stages run **oversampled with a host-rate-scaled factor** (8× at 44.1/48 kHz, 4× at 88.2/96 kHz; noise paths half that) and decimate back to host rate with a polyphase FIR. The factor is a pure function of the host rate, so it stays deterministic and identical native↔WASM.
- **Vendored transcendentals.** `sin`/`exp`/`tanh` use fixed polynomial approximations in `mathx` so `wasm32` and native compute the *same bits*; fast-math / FP reordering disabled in both builds.

Continuous panel controls (the per-voice levels and global accent) route through a per-parameter smoother (one-pole or linear ramp) to avoid zipper noise.

## Audio runtime (AudioWorklet)

The worklet processor is thin. Its job is to:

1. Receive the compiled `dsp_wasm` module (via `processorOptions`) and instantiate one `Engine`.
2. Allocate the WASM-side output buffers once at construction.
3. On each `process()` call: forward queued messages into the engine, run `engine.process()`, copy WASM memory into the `Float32Array` outputs Web Audio gives us, and post any drained events back to the main thread.
4. Never block, never allocate, never `await`.

(No sample-handoff step — there is nothing to load.) The worklet feeds a single `AudioWorkletNode` connected directly to `audioContext.destination`. No intermediate Web Audio nodes — the summing mixer and output saturation are internal to WASM, and the engine exposes a **single stereo main output** with no per-voice channels.

## Main-thread layer (TypeScript)

The main thread owns:

- **AudioContext lifecycle.** Created on first user gesture. The worklet module and WASM binary are fetched and compiled here, then handed off.
- **UI.** Per-voice control panels (one LEVEL per voice, plus global accent — spec §9) and a trigger surface for auditioning voices. Knob drag throttled to animation-frame rate; triggers forwarded immediately.
- **MIDI.** `navigator.requestMIDIAccess()`, port selection, and the mapping between MIDI and engine messages via a configurable GM-drum-style map. MIDI clock/transport depend on the sequencer/transport layer, out of scope here.
- **Kit state.** The current value of every control plus MIDI routing. The main thread is the source of truth; the worklet holds the DSP-side mirror only as a smoother target. On boot or tab-restore, the main thread replays the full kit into the worklet.

UI framework choice (React, Svelte, Solid, or plain DOM) is deliberately not pinned here.

## Control & event protocol

Messages between main thread and worklet are encoded as compact binary (a tagged-union over a small `ArrayBuffer`) rather than JSON: zero parsing cost on the audio side, no per-message GC pressure, and a stable schema defined once in Rust (`#[repr(C)]` enums) and mirrored in a generated TS file. Two channels, both over the same `MessagePort`.

**Main → Worklet (control):**
- `Trigger { voice, accent }` — fire a drum voice now (or at a scheduled offset). `accent` is the binary 606 accent. There is no per-trigger pitch.
- `ParamSet { voice, param_id, value }` — any continuous control. The `param_id`s are sparse: per-voice levels plus global accent. Each voice has its **own** LEVEL (seven independent per-voice levels — MM-06 splits the stock 606's shared Toms[L+H] and Hi-Hat[O+C] pots; spec §7). Valid ids per voice: BD LEVEL; SD LEVEL; LT LEVEL, HT LEVEL; CY LEVEL; CH LEVEL, OH LEVEL.
- `GlobalSet { id, value }` — `accent_level`, master `level`, output `drive`, **`variance_depth`**, **`seed`** (the last two make live output match a deterministic offline render when needed).

This is the in-scope control surface. Transport, clock, and pattern messages belong to the sequencer/transport layer and are deliberately not defined here.

**Worklet → Main (events):**
- `VoiceFired { voice, accent }` — emitted when a voice fires, for optional UI feedback.
- `XRun` — diagnostic, emitted if a process callback overran.

Note the **absence** of melodic messages — no pitch bend, modulation, glide, or per-note pitch; those concepts don't exist on a 606.

For high-frequency display-only state, a `SharedArrayBuffer` ring is a viable optimisation later (it's why cross-origin isolation is configured up front). Not needed initially — `postMessage` at event rate is well under any pressure point.

## Sequencer & clock

Out of scope for this document. No sequencer, transport, or clock behaviour is specified or assumed at this stage. The engine is driven purely by discrete triggers: the `Trigger` message at runtime, and the `events` list offline.

## Build & tooling

- **Rust → WASM (ship):** `wasm-pack build crates/dsp_wasm --target web --release --out-dir ../../web/wasm`, invoked from a `web/` npm script. Output lands in `web/wasm/` (gitignored).
- **Web app:** Vite for dev server, build, and TS compilation. The AudioWorklet file is a separate entry so it can be loaded via `audioWorklet.addModule()`.
- **Analysis:** a Python package (`analysis/`, numpy / scipy / soundfile / librosa) consuming WAVs and emitting metrics JSON. DSP and analysis are kept in *different languages on purpose* so a bug isn't mirrored in both the synth and its own test.
- **CI matrix:** run the suite across sample rates {44.1, 48, 88.2, 96 kHz} and block sizes {64, 128, 256, 512}, native + WASM. Cheap invariant tests run in `cargo test`; heavier tests sit behind a `--full` flag (nightly / on calibration PRs).

## References

The DSP is **not** ported from an upstream library and there are **no repos to clone**. All seven voices are modeled from published circuit analyses (First Principles / Robin Whittle; Baratatronix / Peter Barata; Michaux's hyperreal bass-drum analysis; the TR-606 service notes), as documented and cited in [audio-engine-spec.md](audio-engine-spec.md).

Clips live as **flat files under `references/`** (e.g. `references/bd.wav`, `references/cy.wav`). There is exactly **one clip per voice**: a clean isolated hit, level-normalized (so it carries no reliable absolute level — fit shape only) and unaccented.
