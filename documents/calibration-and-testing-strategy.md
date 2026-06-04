# MM-06 — Calibration & Testing Strategy

> Companion to `architecture.md` and `audio-engine-spec.md`. This document defines *how*
> fidelity and correctness are verified, *what* runs unattended vs. gates on a human, and the
> tool surface the calibration agent drives. It uses the **regime A/B/C** vocabulary from
> `architecture.md §Testing & calibration` — do not introduce competing terms.

---

## 0. The linchpin: one deterministic core, two front-ends

Every strategy below rests on one architectural fact (`architecture.md`): `dsp_core` is a
**pure, deterministic Rust library** compiled from identical source to **wasm32 (ship)** and
**native (test)**. Identical `(seed, params, events, sample_rate, block_size)` produces a
**byte-identical** buffer. The live worklet and the offline `render` binary are thin
front-ends over the same scheduler.

Because of this, almost all fidelity work happens **without a browser**: the `render` binary
dumps WAVs, the Python `analysis/` layer measures them, and an agent adjusts constants in
`dsp_core` and re-renders. The browser is needed only for the integration layer (§8) and final
listening (§11). If determinism ever breaks, treat it as a P0 bug — it invalidates the entire
test approach, not just one case.

---

## 1. What gets verified, and under which regime

Five distinct things, three of them the spec's named regimes plus two cross-cutting test
layers. Keep them separate.

| # | What | Regime / layer | Deterministic? | Autonomous? | Hard CI gate? | Arbiter |
|---|---|---|---|---|---|---|
| 1 | Shared DSP blocks (`§3`) | Block unit tests | Yes (bit-exact) | Yes | Yes | the math |
| 2 | Engine invariants | **Regime A** | Yes | Yes | Yes | the contract |
| 3 | WASM↔native equivalence | Parity (part of A) | Yes | Yes | Yes | byte/tolerance |
| 4 | Explicit spec numbers (`§4`,`§6`) | **Regime B** | Yes | Yes (numeric opt) | as goldens | the spec value |
| 5 | One reference clip per voice | **Regime C** | Yes | Yes after clip lands | as goldens | clip *shape* + human ear |
| 6 | Engine↔UI (TS) wiring | Integration | Mostly | Yes | Yes | the protocol |

The human is reduced to **four irreducible inputs** (`architecture.md`): supplying the one
reference clip per voice, one glance at each `--bless` before/after diff, final blind-listening
sign-off, and licensing/trademark review. Everything else runs unattended.

### Why voices are automatable here (and weren't in the generic case)
Autonomous voice calibration is normally a bad idea — there's no reproducible ground truth and
the target is perceptual. MM-06 removes the first objection by construction (deterministic
core) and tames the second by **not** using null/difference tests as the oracle. Per
`audio-engine-spec.md §11.8`, null tests are explicitly *not* primary here: they're meaningless
against a **normalized** clip and a **free-running** metallic cluster. Regime C therefore fits
**relative spectral and temporal shape**, not sample-level equality — and a human still signs
off by ear at the end.

---

## 2. Foundations the strategy rests on

### 2.1 The offline `render` binary (`crates/render`)
Native CLI: JSON/CLI `RenderRequest → WAV (+ sidecar JSON)`, deterministic. It and the worklet
must agree to the WASM↔native parity tolerance (§5). Use `render_voice()` for calibration: it
renders **one voice in isolation, pre-mix, without bus coloration** (`audio-engine-spec.md
§2.1`). This is a measurement aid and **not a shipped output** — never let regime B/C results
depend on the mixer.

### 2.2 Python analysis (`analysis/`)
WAV → metrics JSON via numpy / scipy / soundfile / librosa, plus matplotlib for overlays. Kept
in a **different language from the DSP on purpose** so a bug isn't mirrored in both the synth
and its own test. The analysis layer is the home of every feature extractor referenced in
regimes B and C.

### 2.3 The `dsp-harness` MCP server (`harness/`) — the agent's hands
The calibration agent never edits-and-prays; it drives a tight loop through the MCP harness,
which wraps `render` + `analysis`. Recommended tool surface (name them clearly so the agent
reasons about them):

- `render_voice(voice, sample_rate, block_size, seed, variance_depth, accent, events) → wav_id`
- `render_mix(request) → wav_id` — full main output, for integration/golden renders.
- `analyze(wav_id, feature_set) → metrics_json` — centroid, decay envelope, pitch/peak track,
  alias floor, DC, etc.
- `compare_to_targets(metrics, voice) → deltas` — regime B: distance to the spec numbers.
- `compare_to_reference(wav_id, voice) → shape_distance` — regime C: shape distance to
  `references/<voice>.wav` (normalized, onset-aligned).
- `run_invariants(scope) → pass/fail report` — regime A gates (§4).
- `parity_check(request) → max_divergence` — WASM↔native (§5).
- `propose_param_edit / apply_param_edit` — the loop's write side, scoped to calibratable
  constants only (never to shipped control semantics).
- `bless_golden(voice, reason) → diff_report` — gated; freezes a render + its metric vector.

**Calibration is always run sterile and reproducible:** `variance_depth = 0`, a fixed `seed`,
and fixed event placement, so the loop fits the default operating point and isn't fighting
jitter or one-off metallic phase.

---

## 3. Layer 1 — Shared DSP building-block unit tests (`cargo test`, native)

The `§3` primitives are instanced per voice; test them once, in isolation, bit-exact. These are
fast `cargo test` cases and gate every commit.

### 3.1 Per-block patterns
- **Bridged-T / Twin-T resonator (`§3.1`)** — pinged by the ~1 ms excitation, produces a
  decaying sine at the designed centre frequency with the Q-derived decay
  (`Q = sqrt(R1/R2)/(sqrt(C1/C2)+sqrt(C2/C1))`). Verify centre freq, −3 dB bandwidth, and that
  decay-time tracks Q. High-Q stability: no blow-up, no NaN/Inf.
- **Two-resonator BD assembly (`§4.1`)** — both resonators (≈60 Hz Q≈7, ≈130 Hz Q≈3) pinged by
  **one shared edge start in phase**, then beat. Assert the in-phase positive front edge and
  the subsequent beating; assert OSC2 is lower amplitude and shorter. **Assert there is no
  downward pitch sweep** (pitch-track the sum; the partials are stationary). This is the single
  most important "don't accidentally build an 808" guard.
- **Trigger / excitation generator (`§3.2`)** — ~1 ms band-limited pulse; amplitude scales with
  accent; deterministic hit-to-hit (variance is added explicitly in §10, never as trigger
  sloppiness).
- **RC exponential envelopes (`§3.3`)** — decay-only, anti-log/exponential taper (never linear
  ramps); time-constants clamped against denormals; tails flush to exact zero.
- **Shared analog noise (`§3.5`, `§5`)** — one wideband white stream per engine instance, seeded
  from the global seed; flat spectrum; **coincident snare+tom hits draw correlated noise from
  the same stream** (assert correlation — independent per-voice RNGs are a fidelity bug).
- **Six-osc Schmitt metallic bank (`§3.6`)** — six squares at ≈245/308/367/417/438/625 Hz
  (calibration targets), mixed equally. **Free-running: the bank is never reset on trigger** —
  assert that phase at gate time is a function of elapsed samples, *and* that it's deterministic
  given (seed, elapsed). Oversampled (this is the biggest aliasing source) — check the alias
  floor (§4).
- **Bridged-T band-pass (`§3.7`)** — high path ≈7100 Hz, low path ≈3440 Hz; verify centres and
  that the **hats use only the high path, the cymbal uses both**.
- **HPF stages (`§3.8`)** — resonant HPF (sizzle); single-transistor HPF on the cymbal low path
  (Tier A: linear HPF + slight saturation).
- **VCA + accent grit (`§3.9`)** — gain × mild saturation whose drive scales with accent, so
  accented hits are grittier as well as louder.
- **Oversample (`§2.2`, `§8`)** — host-rate-scaled factor is the *smallest power of two* meeting
  `host_rate × factor ≥ ≈352 kHz` (osc/nonlinear) or `≥ ≈176 kHz` (noise); verify the factor is
  a pure function of host rate (deterministic, identical native↔WASM); polyphase decimation FIR
  meets its alias-rejection spec; BLEP/BLAMP edge correction reduces edge aliasing.
- **`mathx` vendored transcendentals (`§7.1`)** — fixed-poly `sin`/`exp`/`tanh`. Two tests:
  (a) approximation error within bound vs. a reference implementation, and (b) **bit-identical
  output on wasm32 vs. native** (these functions exist *specifically* to guarantee that — if
  parity (§5) ever fails, suspect a non-`mathx` transcendental leaking in).

### 3.2 Cross-cutting block properties
- **Block-size invariance (bit-exact):** processing one large buffer equals the same input
  split across `{64,128,256,512}` — **byte-identical**, not "within tolerance," because the core
  is deterministic. Catches oversampler/decimator state-carry bugs.
- **Reset:** after construction/reset, a block matches a fresh instance.
- **Numerical hygiene:** sustained zero/DC/Nyquist/decaying input never yields NaN, Inf, or
  runaway denormals; tails reach exact zero.

---

## 4. Regime A — engine invariants (hard CI gates, never blessed away)

These need no hardware and no spec numbers. They are pass/fail and run in `cargo test` (cheap
ones) or behind `--full` (heavier ones). Each maps to a `run_invariants` check in the harness.

1. **Determinism (bit-exact).** Identical `RenderRequest` → byte-identical buffer, native; and
   stable across runs. Compare via `f32::to_bits` so `-0.0`/NaN can't sneak through (NaN must
   never appear regardless).
2. **Tonal excitation phase-lock.** Tonal resonators are pinged **on trigger**, so the ring is
   phase-locked to the hit — the same event at the same sample index alway pings identically
   (the tonal analog of "phase reset").
3. **Free-running-but-deterministic metallic phase.** Two assertions at once: the cluster is
   **not** reset on trigger (two hits at different elapsed times gate at *different* phases →
   genuine hit-to-hit variation), **and** the phase is a deterministic function of
   `(seed, elapsed_samples)` → fully reproducible. Both must hold.
4. **DC-free outputs.** Every voice output and the master have mean ≈ 0 within tolerance (DC
   blockers present, `§7`).
5. **Aliasing floor under oversampling.** Spectral energy at the square-oscillator image
   frequencies stays below the defined floor. This is the "no cheap-clone inharmonic junk" gate.
6. **Buffer-size independence (bit-exact).** Byte-identical output across
   `{64,128,256,512}` at a fixed sample rate.
7. **Sample-rate correctness.** Across `{44.1,48,88.2,96 kHz}` the *features* land correctly
   (e.g. BD resonators still at ≈60/130 Hz; metallic peaks in place; oversample factor scales as
   specified). Cross-rate output is **not** byte-comparable — assert feature-correctness, not
   equality.
8. **Denormal-free tails.** Tails flush to exact zero; no denormal CPU spike; no NaN/Inf ever
   reaches output.
9. **Choke behaviour (`§6.3`).** A CH trigger (or new hat trigger) collapses a sounding OH within
   a few ms.
10. **Parameter monotonicity & semantics.** LEVEL is monotonic gain. `variance_depth = 0` is
    fully sterile/repeatable. **Accent is direction-checked, not fit:** accented hits ring
    **longer and brighter** (resonators pinged harder, more HP snap) — assert
    longer-and-brighter, **not** merely louder (`§2.3`, `§11.7`). Accent magnitude itself is a
    best guess (the clips are unaccented), so it is *only* a monotonicity invariant here.
11. **Realtime budget.** `process()` for a 128-frame quantum at host rate completes well inside
    `128 / sample_rate` seconds with margin, at full polyphony. Benchmark with `criterion`
    (native) and a node perf harness (wasm). Track it; an `XRun` in the field is a regression.
12. **No allocation in `process()` / steady-state render.** Buffers pre-sized at construction;
    `process()`-emitted events go into a **fixed-capacity ring** drained by `drain_events`
    between callbacks. Run native render-scope tests under an **allocation-flagging global
    allocator** (e.g. the `assert_no_alloc` pattern, or a custom counting allocator — verify the
    crate/toolchain) that fails on any heap traffic inside the render scope. Setup/teardown may
    allocate; the per-quantum path may not.
13. **No panics in `process()`.** Checked construction at setup; proven-bounds `get_unchecked` in
    hot loops; the render path is panic-free.

> Negative guards to keep honest (cheap, high value): **no `load_sample` path anywhere** (there
> are no PCM assets — `architecture.md`); **no BD pitch sweep** (#2 of `§1`); **metallic
> oscillators never reset** (#3 above); **accent ≠ pure gain** (#10).

---

## 5. WASM↔native parity (the assumption everything else trusts)

The entire "test natively, ship wasm" approach is only valid if the two builds agree. The core
is engineered for **bit-parity**: vendored `mathx` polynomials, fast-math and FP reordering
disabled in both builds (`§7.1`). Parity is therefore its own gate:

- **What to compare:** render the same `RenderRequest` through native `dsp_core::render()` and
  through the wasm build, and diff the buffers.
- **How to run the wasm side in CI:** either compile `dsp_core` to a `wasm32` test target and run
  under `wasmtime`/`wasmer`, or exercise the streaming `dsp_wasm` `Engine` via
  `wasm-bindgen-test` / `wasm-pack test --node`. Pick whichever the toolchain makes cheapest; do
  both if feasible (core parity + streaming-engine parity).
- **Tolerance:** target **zero** divergence (bit-identical). Define a tight parity tolerance and
  treat any exceedance as a bug — the usual culprit is a transcendental that bypassed `mathx` or
  an FP-reordering flag leaking into one build.
- **Streaming vs. offline equivalence:** the worklet pushes `TrigEvent`s as messages arrive;
  `render()` takes the whole `events` list up front. Both schedulings of the same events must
  agree to the parity tolerance (`architecture.md`).

---

## 6. Regime B — explicit spec targets (autonomous numeric optimisation)

Every flagged number in `audio-engine-spec.md` maps to a feature the analysis lib measures and a
constant in `dsp_core` that moves it, closed in a `render_voice → analyze → compare_to_targets →
apply_param_edit` loop at `variance_depth = 0`, fixed seed.

| Target (spec) | Voice | Measured feature | Constant that moves it |
|---|---|---|---|
| Resonators ≈ 60 Hz (Q≈7) & 130 Hz (Q≈3); in-phase attack + beat; **no sweep** | BD | dual-partial pitch + Qs; attack-edge polarity; beat rate; pitch-track flatness | the two resonator freqs/Qs, relative level, excitation |
| Body resonator pitch (low-hundreds Hz, target) + HP-gated snap | SD | body fundamental + decay; snap HP corner + gate decay | resonator freq/Q; noise-gate HPF corner; gate decay |
| Two tunings of one model (Low lower, High higher — targets) + LP noise burst | LT/HT | per-tom fundamental; LP burst presence at attack | per-tom resonator freq; tom-noise LP corner + gate |
| Six metallic oscillators ≈ 245/308/367/417/438/625 Hz | CY/OH/CH | inharmonic peak set | the six oscillator frequencies |
| Band-pass 7100 Hz (high) & 3440 Hz (low); hats high-only, cymbal both | CY/OH/CH | peak structure of each path; which paths are present per voice | the two band-pass centres + routing |
| CH short / OH long decay; cymbal long | CH/OH/CY | decay-envelope time | the per-voice RC decay constants |
| Accent ⇒ longer + brighter (not louder) | all | decay length & centroid vs. unaccented | excitation-energy scaling, gate depth, VCA drive |

Regime B is where the agent does most of its autonomous turning. When a target can be hit by
moving a constant, that's a constant edit; when the *shape* is structurally wrong (missing
inharmonic partials, wrong envelope topology, wrong attack character), that's a **Tier A vs.
Tier B / topology** question (`audio-engine-spec.md §1`) — flag it rather than chasing it with
constants.

---

## 7. Regime C — reference-clip match (autonomous, shape-only)

Fires automatically once `references/<voice>.wav` lands. It anchors each voice's **default
operating point** to a **single normalized, clean, unaccented clip** — as *relative* spectral
and temporal shape only.

### 7.1 What regime C can and cannot fit
- **Can:** resonator pitch(es), decay slope, the BD two-partial beat, snare body+snap balance,
  metallic peak/path structure — all as relative shape.
- **Cannot (these stay best-guess, anchored to A/B):** **absolute level / inter-voice balance**
  (the clip is normalized — fit shape, never loudness), **accent** (clips are unaccented — see
  §4.10), and **per-hit variance depth** (`§10`). Do not let the loop "succeed" by drifting these.

### 7.2 Method (per `audio-engine-spec.md §11`; null tests are *not* the oracle)
Render the voice in isolation (`render_voice`, sterile, fixed seed), then compare to the clip
on **shape**, not samples:
1. Onset-align and DC-remove both; normalize for shape comparison.
2. **Per-voice spectrogram:** match spectral centroid, decay slope, and (metallic) the
   inharmonic peak structure of cluster + band-pass paths.
3. **Decay-envelope (Hilbert/energy):** match ring/decay shape — BD two-resonator beat; SD body;
   tom rings; CH short / OH long / cymbal long.
4. **Metallic peak structure only, not phase:** the clip is *one* free-running phase realization
   (`§11.5`) — match the peak set within tolerance, never the phase.
5. Aggregate with a **multi-resolution log-STFT shape distance**, but always report it alongside
   the decomposed per-feature deltas — the scalar is a guide, not an objective to minimise
   blindly, and a lower number is not automatically "more 606."

### 7.3 Per-voice emphasis
- **BD** — two-resonator fit (60/130 Hz, Qs, relative level) **and** confirm the in-phase attack
  edge + beating against the clip; no sweep.
- **SD** — body resonator pitch/decay + HP-gated noise snap (corner + gate decay).
- **LT/HT** — per-tom fundamental + the small LP tom-noise attack burst; two tunings of one model.
- **CY** — both paths (7100 + 3440 Hz), dual-VCA/dual-HPF, long clangy inharmonic decay.
- **OH/CH** — high path only, shared free-running cluster, choke group; decays as fixed internal
  defaults (note: on hardware these are tempo-derived, not panel controls — documented best guess).

---

## 8. Engine ↔ UI integration (TypeScript side)

Mostly `vitest` on the main-thread code, plus one cross-language test.

- **Binary protocol parity (Rust ↔ TS).** The `#[repr(C)]` tagged-union schema is defined once in
  Rust and mirrored in generated TS. Test that the generated codec round-trips every message
  (`Trigger`, `ParamSet`, `GlobalSet`, `VoiceFired`, `XRun`) byte-for-byte against the Rust
  definition. A schema drift here is silent and nasty.
- **Worklet contract.** The processor must **never block, allocate, or `await`** in `process()`;
  it forwards queued messages, runs `engine.process()`, copies WASM memory into the Web Audio
  `Float32Array`s, and posts drained events. Assert these properties (and that `XRun` is emitted
  on overrun).
- **Parameter smoothing.** A `ParamSet` produces a one-pole/linear ramp, not a step (no zipper
  noise). Test the smoother in `dsp_core` (deterministic) and assert the TS side only forwards
  targets — the worklet holds the DSP mirror only as a smoother target; the main thread is the
  source of truth.
- **Kit-state replay.** On boot/tab-restore the main thread replays the full kit into the
  worklet; assert the DSP-side mirror equals the replayed state.
- **MIDI map.** GM-drum-style map → engine `Trigger`s resolves to the correct voice; note there
  is **no per-trigger pitch / pitch-bend / glide** (those concepts don't exist on a 606).
- **Cross-origin isolation** is configured up front (for a future `SharedArrayBuffer` display
  ring) — a config check, not a behavioural test.

---

## 9. Goldens & the `--bless` workflow

A golden is a **frozen render plus its metric vector**. Goldens guard regimes B and C and the
full mix against accidental drift; they are **regenerated only** by an explicit, reviewed
`--bless` that emits a **before/after diff report**.

- The **single human gate per intended sonic change** is one glance at that diff. Everything else
  runs unattended.
- Regime A invariants are **never** blessed away — they are correctness, not taste.
- `bless_golden` is the only harness tool that writes a golden, and it must always produce the
  diff for review.

---

## 10. CI matrix

Per `architecture.md`:

- **Axes:** sample rate `{44.1, 48, 88.2, 96 kHz}` × block size `{64, 128, 256, 512}` ×
  `{native, wasm}`.
- **Tiers:** cheap invariants (determinism, block-size byte-equality, DC, no-alloc, monotonicity)
  in `cargo test` on every push; heavier work (full regime B/C optimisation, parity sweeps,
  spectrogram analysis, benchmarks) behind a **`--full`** flag on nightly / calibration PRs.
- Wasm-side runs use `wasm-bindgen-test` / `wasm-pack test --node` and/or a `wasmtime` core
  target (§5).

---

## 11. The calibration agent's operating loop

For each voice, through the `dsp-harness` MCP tools, at `variance_depth = 0` and fixed seed:

1. **Gate on regime A** (`run_invariants`). If any invariant fails, fix that first — never tune
   fidelity on top of a broken invariant.
2. **Regime B:** `render_voice → analyze → compare_to_targets`. Move the mapped constants
   (§6) until the spec numbers land within tolerance. Distinguish constant edits from
   topology/Tier-B problems and flag the latter.
3. **Regime C (if `references/<voice>.wav` exists):** `compare_to_reference` on shape; close the
   loop on spectral-shape + envelope distance (not null tests). Respect what C *can't* fit
   (§7.1) — leave level/accent/variance as best-guess.
4. **`parity_check`** the result (§5).
5. **Report** per voice: regime-A pass/fail, regime-B deltas vs. each target, regime-C shape
   distance + decomposed feature deltas, overlay plots (spectrogram, decay envelope, pitch/peak
   track), parity max-divergence, and a ranked list of proposed edits each tagged
   *constant* / *topology* with confidence.
6. **Stop at the human gates:** propose a `bless_golden` with a before/after diff for the one
   human glance, then leave **accent character, variance depth, absolute level, and final voicing
   to blind-listening sign-off.** Those four are best-guess by design and are the human's call.

### Deliverables / order of work
1. `crates/render` (offline binary) + `render_voice` isolation path.
2. `analysis/` feature extractors for every regime-B target and regime-C feature.
3. `harness/` `dsp-harness` MCP server exposing the tool surface in §2.3.
4. Block unit tests (§3) and regime-A invariants (§4) wired into `cargo test` / `--full`.
5. WASM↔native parity gate (§5).
6. TS integration + protocol-parity tests (§8).
7. Per-voice regime-B then regime-C loops (§6, §7), goldens via `--bless` (§9).
8. **Do not** turn regime C into a bit-level null test; **do not** let any loop close by drifting
   level/accent/variance; **do not** bless away a regime-A invariant.
