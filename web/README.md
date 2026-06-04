# MM-06 web (dev rig)

Walking-skeleton dev UI for the TR-606 emulation. Plain DOM, no framework.

## Run

```bash
cd web
npm install          # first time only
npm run dev          # builds wasm + worklet, then starts Vite
```

Open the printed URL, click **▶ Start audio** (browsers require a user gesture),
then play with keys **Q W E R T Y U** → BD SD LT HT CY OH CH, or the on-screen
Trigger buttons. Toggle **Accent** per voice; drag **LEVEL**. The global strip
has Master / Accent / Drive / Variance / Seed. Turn **Variance** up so repeated
hats/cymbals stop machine-gunning (the free-running metallic cluster scatters
its gate phase). **U** (Closed Hat) chokes a ringing Open Hat.

## How it fits together

```
main.ts ─ DOM UI ─► bridge.ts ─(8-byte frames, MessagePort)─► audio/worklet.js
                         │                                          │
                  compiles dsp_wasm_bg.wasm                  dsp_wasm Engine
                  hands module to worklet                    └► dsp_core (pure)
```

- `src/protocol.ts` mirrors `crates/dsp_core/src/protocol.rs` (the binary frame
  schema). Keep them in lockstep.
- `src/audio/worklet.ts` is bundled by esbuild into `public/worklet.js`
  (`npm run build:worklet`) so it loads as one self-contained script — the
  compiled WASM module is passed in via `processorOptions`, no fetch in the
  worklet scope.
- `web/wasm/` is the wasm-pack output (gitignored); `npm run wasm` regenerates it.

## Build

```bash
npm run build        # typecheck + wasm + worklet + vite build → dist/
```

## Offline render (no browser)

The same `dsp_core` drives a native binary for tests/calibration:

```bash
cargo run -p render -- req.json -o mix.wav        # full mix
echo '{"sample_rate":48000,"length_samples":48000,"voice":"bd",
       "events":[{"sample_index":0,"voice":"bd"}]}' | cargo run -p render -- -o bd.wav
```

Native `render()` and the WASM streaming `Engine` are bit-exact for identical
input (the architecture's parity requirement) — verified in the skeleton.
