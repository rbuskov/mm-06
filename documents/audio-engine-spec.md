# MM-06 (TR-606 Emulation) — Audio Engine Specification

**Scope:** This document specifies the *audio engine* only — synthesis architecture, per-voice models, parameter ranges, and the DSP techniques required to get materially closer to a real Roland TR-606 than existing software (sample-pack ROMplers and naive analog clones). It deliberately excludes platform, language, framework, sequencer, and UI. Numerical values are grounded in published circuit analyses (Robin Whittle / First Principles; Peter Barata / Baratatronix; Thomas Michaux's hyperreal bass-drum analysis) and the Roland TR-606 service notes; where a value is an estimate it is flagged as a **calibration target** to be tuned against the single reference clip for that voice (regime C) or, where no clip can show it, left as a documented **best guess**. The human will conduct final listening tests and request changes.

> ## Calibration anchor: one reference clip per voice (governing decision)
>
> MM-06 calibrates against **a single normalized, clean, unaccented reference clip per
> voice** — one `<voice>.wav` and nothing else. That clip is the only hardware anchor; everything it
> cannot show is a **best guess** from the spec. Three consequences follow and apply
> throughout:
>
> 1. **The clip pins default shape only.** Each clip anchors that voice's *default operating
>    point* — resonator pitch(es), decay slope, the kick's two-partial beat, the metallic
>    peak/path structure — as *relative* spectral and temporal shape. It is **normalized**, so
>    it carries **no** absolute-level information (fit shape, never loudness or inter-voice
>    balance), and **unaccented**, so accent is not represented in the dataset at all. Treat a
>    fit as "matches this clip," not "the 606."
> 2. **The stock 606 has almost no sound controls** (§9). Its panel is master VOLUME, a
>    global ACCENT level, and five instrument *mix-level* pots (BD, SD, Toms[L+H], Cymbal,
>    Hi-Hat[O+C]) — **no per-voice TUNE or DECAY**. MM-06 keeps that minimal surface but
>    splits the two shared pots into **seven independent per-voice levels** (§7, §9) — the
>    sole level-surface divergence, with no fidelity cost. The engine exposes **nothing
>    beyond level and accent**: every voice's pitch, decay, and tone are fixed at their
>    calibrated default operating point, exactly as on the uneditable hardware. There is no
>    knob to turn and no second clip, so the model is calibrated entirely at that default
>    point — see §9.
> 3. **Accent is modelled, not measured.** On the 606 a single ACCENT level scales every
>    triggered voice at once by raising the **trigger-pulse amplitude** (~+5 V → ~+14 V;
>    output 2 → 6 Vp-p), so accented hits ring longer and brighter — not merely louder (§2.3).
>    The engine models this, but because the reference clips are **unaccented**, accent's
>    magnitude and character are a **best guess** and human judgement.

---

## 1. Design goals and fidelity philosophy

The 606 is a **fully analog** instrument — seven voices, all synthesized, no samples or ROM anywhere:

- **Tonal voices (bridged-T):** bass drum (two resonators), snare drum (one resonator + noise), low tom, high tom (one resonator each + a little noise). Each is a **Twin-T / bridged-T resonant network "pinged" by the trigger pulse**, ringing as a naturally (exponentially) decaying sine.
- **Metallic voices (oscillator-bank):** cymbal, open hi-hat, closed hi-hat — all derived from **one shared cluster of six free-running square-wave oscillators**, band-pass-filtered and gated by per-voice envelopes.

A convincing emulation must treat these two families with different techniques and must reproduce four things that off-the-shelf 606 plug-ins routinely miss:

1. **Resonator genesis, not VCO genesis.** The tonal voices are *not* triangle-VCO-through-a-clipper synths. They are **high-Q bridged-T resonators excited by a short trigger pulse**; the decaying sine and its exponential tail come from the network's Q, not from an oscillator + AD envelope. Model the resonator, ping it, let it ring.
2. **The kick's two-resonator attack edge.** The 606 BD is **two** Twin-T resonators (≈ 60 Hz and ≈ 130 Hz) that *start in phase* (both kicked by the same trigger edge) and then drift apart because they are at different frequencies. The solid, slightly clicky front edge — the 606 kick's signature — is that brief in-phase moment, **not** a pitch sweep. The 606 has *no* pitch-envelope sweep (unlike an 808-style kick); do not add one in authentic mode.
3. **Free-running metallic oscillators (the opposite of phase-reset).** The six square oscillators **run continuously and are *not* reset on trigger**. Each hat/cymbal hit opens a VCA at whatever phase the cluster happens to be in, which is *exactly why* the metallic voices vary hit-to-hit on hardware. Resetting them (the naive approach) makes every hat identical and dead. Keep them free-running; their phase at gate time is the principal source of metallic per-hit variance (§6, §10).
4. **Accent = harder excitation.** Accent raises the trigger-pulse voltage, so resonators are pinged harder (ring longer + louder) and the noise/HP gate passes more snap. Model accent as trigger-energy scaling + envelope/length modulation, **not** a pure output-gain change (§2.3).

**Fidelity tiers.** The spec defines a recommended baseline and an optional max-fidelity path:

- **Tier A — physically-informed behavioral DSP (baseline, recommended for all voices).** Each circuit *block* (bridged-T resonator, Schmitt-trigger oscillator, envelope, VCA, filter) is modeled as a DSP element whose parameters are derived from the schematic and calibrated to measurements. Fast, stable, and — done carefully — indistinguishable from hardware in blind listening for most voices.
- **Tier B — circuit-level modeling (optional, for the blocks that matter most).** Wave Digital Filter (WDF) or state-space/nodal models of the strongly resonant or nonlinear subcircuits: the **bridged-T resonators** (BD/SD/toms), the **Schmitt-trigger relaxation oscillators** (whose threshold hysteresis sets both frequency and waveform), and the **single-transistor high-pass stages** in the cymbal/snare. Recommended only where Tier A measurably diverges from hardware.

**Non-negotiable engine-wide requirements:** oversampling around the square oscillators and any nonlinearity; **deterministic** behavior given inputs; **free-running (un-reset) metallic oscillators** with seeded phase; trigger-synchronous excitation of the tonal resonators; DC blocking on every voice output; and a per-hit analog-variance layer (§3 and §10).

---

## 2. Engine-wide architecture

### 2.1 Signal flow (per trigger)

```
trigger + accent
      │
      ▼
 [voice excitation]  ── raw voice signal ──► [voice envelope/VCA] ──► [voice output filter]
      │  (ping resonator, or gate the                                       │
      │   free-running metallic cluster)                                    │
      └────────── per-hit variance (jitter, drift, metallic phase) ─────────┘
                                                                            │
                  ┌─────────────────────────────────────────────────────────┘
                  ▼
                  [summing mixer w/ analog coloration] ──► [output amp + soft saturation] ──► stereo main out
```

Every voice contributes to the **single main mix**; there are no per-voice output channels (like the stock 606, which is main-out only). For calibration, the offline harness can still render any voice **in isolation** — pre-mix, without bus coloration — via the `render_voice` test helper; that is a measurement aid, not a shipped output.

### 2.2 Sample rate and oversampling

- **Run at the host sample rate.** The engine processes at the host/device rate (44.1, 48, 88.2, 96 kHz…) carried in as `sample_rate`; every coefficient is expressed in Hz/seconds, so each voice is correct at any rate. **No ROM, no resampler** — there is nothing sampled in this engine, so there is no varispeed/resampling stage anywhere.
- **Oversample locally, with a host-rate-scaled factor.** Only the aliasing-prone blocks are oversampled — the **six Schmitt-trigger square oscillators** (hard edges, rich harmonics — the principal aliasing source) and any **nonlinear stage** (VCA saturation, single-transistor HPFs); the band-limited **noise paths** use a lower factor. Rather than a fixed multiplier, **scale the factor down as the host rate rises** so the *effective internal rate* stays roughly constant (and CPU does not balloon at 96 kHz):
  - Oscillators + nonlinear stages: **8× at 44.1/48 kHz, 4× at 88.2/96 kHz** → effective ≈ 353–384 kHz.
  - Noise paths: **4× at 44.1/48 kHz, 2× at 88.2/96 kHz** → effective ≈ 176–192 kHz.
  - **General rule:** pick the smallest power-of-two factor with `host_rate × factor ≥ ≈352 kHz` (oscillators/nonlinear) or `≥ ≈176 kHz` (noise). The factor is a pure function of the host rate, so it is deterministic and identical native↔WASM.
- **Resonators run at host rate.** The bridged-T resonators are near-linear high-Q bandpasses and alias little; only the **trigger-pulse ping** edge within them is worth oversampling.
- **Anti-imaging/anti-aliasing:** polyphase FIR decimation back to host rate, with the filter designed for the chosen factor. **BLEP/BLAMP** edge correction on the square-oscillator transitions and the trigger ping complements the oversampling.

### 2.3 Trigger, accent, and dynamics

- **Trigger:** a short pulse (hardware ≈ **1 ms**) that (a) excites/pings the tonal resonators, (b) launches the per-voice envelopes that gate the metallic cluster, and (c) seeds/advances the per-hit variance state. **It does *not* reset the metallic oscillators** (they free-run, §6).
- **Accent (global):** the 606 has **one** accent level shared by all currently-triggered voices. Physically it raises the trigger-pulse amplitude (~+5 V unaccented → up to ~+14 V accented; main output 2 → 6 Vp-p). Model accent as a scaling of **excitation energy**: pinged resonators ring with greater initial amplitude *and longer* (higher effective Q-ring), the snappy HP-noise gate passes more high content, and any VCA distortion is driven harder. Accented hits must be **brighter / longer / punchier**, not just louder.
- **Per-step vs. per-voice:** the hardware accent is a per-step property applied to whatever fires on that step, scaled by the global ACCENT level. In the engine, model a binary `accent` flag per trigger plus a global `accent_level`.

---

## 3. Shared DSP building blocks

These primitives are reused across voices. Specify them once, instance them per voice. The **bridged-T resonator is the backbone** of every tonal voice — the 606 does not use VCO + diode-clipper synthesis, and there is no kick pitch-sweep block.

### 3.1 Bridged-T (Twin-T) resonator — the core tonal block

Used by the bass drum (×2), snare, low tom, and high tom. A bridged-T network is a resonant bandpass; pinged by an impulse it produces a naturally decaying sine whose decay rate is set by its Q. Implement as a high-Q biquad bandpass in Tier A (impulse/pulse excited), or a WDF / state-space model of the actual RC network in Tier B. Decay relates to Q by the network relationship `Q = sqrt(R1/R2) / (sqrt(C1/C2) + sqrt(C2/C1))`. Because component tolerances dominate these voices, expose tolerance-driven variation (§10).

- **Excitation:** a short pulse (~1 ms) scaled by accent (§2.3). Excite **on trigger** so the ring is phase-locked to the hit (this is the tonal-voice analog of "phase reset"). A larger pulse ⇒ larger initial amplitude ⇒ longer audible ring (the accent mechanism).
- **Multiple resonators, shared excitation (BD):** two resonators pinged by the *same* edge start in phase and beat as they diverge (§4.1). Model them as independent biquads sharing one excitation event so their phase relationship is deterministic.

### 3.2 Trigger-pulse / excitation generator

Shapes the raw step trigger into the ~1 ms pulse that pings the resonators and opens the noise/metallic gates. Model as a short, band-limited excitation (deterministic), with amplitude scaled by accent. For the snare and toms it also drives the **noise gate** (§3.4, §3.5). Keep it deterministic and repeatable hit-to-hit (variance is added explicitly in §10, not as trigger sloppiness).

### 3.3 Amplitude envelopes

The 606's envelopes are simple analog **RC charge/discharge (exponential)** shapes, with no two-stage "punch" contour (there is no such circuit here):

- **Tonal voices:** the resonator's own Q *is* the decay — the audible tail is the ringing network, optionally trimmed by a simple VCA envelope. Model the decay primarily through resonator Q; any VCA envelope is a gentle exponential.
- **Metallic voices (VCA-gating):** per-voice exponential decay envelopes gate the (constant) metallic cluster. Closed hat = short; open hat = long; cymbal = long with its own shape. These are **decay-only** RC envelopes with an anti-log/exponential taper matching the hardware VCA curve.

Envelopes are **analog-shaped** (RC exponential), never linear ramps. Clamp time-constants to avoid denormals; flush tails to exact zero (§8).

### 3.4 Snappy / noise high-pass gate

The trigger pulse drives a **noise gating circuit** with a **high-pass filter** to turn the broadband internal noise into the snare's "snappy" transient. Model as: shared noise (§3.5) → gate (enveloped by the trigger) → HPF → VCA. The HP corner and gate decay are the snare's snap character. Accent opens the gate harder (more snap).

### 3.5 Noise generator (shared, analog)

A single **analog white-noise source** feeds the snare and the toms. In hardware it is a **reverse-biased transistor emitter-base junction in breakdown** — a flat-spectrum ("white") analog noise, amplified by a fixed gain stage. This is **not** a clocked binary/PRBS generator. Model one shared wideband Gaussian/uniform white source per engine instance, then per-voice filtering (snare → HPF snap; toms → LPF "rumble" burst). Because it is *shared*, simultaneous snare+tom hits draw from correlated noise — sample **one** stream rather than independent per-voice RNGs if exact-match fidelity is desired (cheap to do right). Drive the noise PRNG from the global seed for determinism.

### 3.6 Schmitt-trigger metallic oscillator bank (cymbal + hats source)

The defining 606 block, shared by cymbal, open hat, and closed hat. Six **square-wave relaxation oscillators**, built in hardware from a hex Schmitt-trigger IC (HD14584B). Each is an RC oscillator whose frequency is set by its R·C and the trigger's hysteresis thresholds. The six are tuned to an **inharmonic** set and **mixed equally** to a metallic cluster.

- **Frequencies (calibration targets; from circuit simulation + the Schmitt-oscillator formula, Baratatronix):** ≈ **245, 308, 367, 417, 438, 625 Hz**. (These are *ideal* values; real units drift with tolerance and age — fit to the reference clip, §10/§11.)
- **Free-running, never reset (§1.3).** The cluster runs continuously; per-voice envelopes gate it. The oscillator phases at gate time are the main metallic per-hit variance — derive them deterministically from the seed + elapsed time, not wall-clock.
- **Waveform:** true square (with the asymmetric mark/space the Schmitt thresholds impose). Oversample (§2.2) — this block is the engine's biggest aliasing risk.

### 3.7 Bridged-T band-pass filters (metallic shaping)

The summed metallic cluster splits into **two** bridged-T band-pass paths (centre frequencies shared with the 808):

- **High path ≈ 7100 Hz** — feeds the hi-hats and the cymbal "shimmer".
- **Low path ≈ 3440 Hz** — feeds the cymbal "body" *only* (the hats do not use it).

Model each as a resonant bandpass; downstream high-pass stages (§3.8) add the final metallic sheen.

### 3.8 High-pass stages (resonant + single-transistor)

- **Resonant HPF** after the shared hi-hat VCA (and on the cymbal's high path) — emphasizes the top end, the "sizzle".
- **Single-transistor HPF** on the cymbal's low (3440 Hz) path. This stage is mildly nonlinear; Tier B may model it as such, Tier A as a linear HPF plus a touch of saturation.

### 3.9 VCA with accent-dependent grit

The 606 VCAs are not perfectly clean. Model each as gain × a mild saturating nonlinearity whose drive scales with excitation/accent level, so accented hits are audibly grittier as well as louder — part of why 606 accents feel punchy rather than merely loud.

---

## 4. Tonal voices (bridged-T)

### 4.1 Bass Drum

**Circuit basis (Whittle / Michaux):** **two** Twin-T (bridged-T) resonators pinged by the trigger pulse — *not* the 808's single resonator with a pitch-switch. Both are kicked by the same edge so they **start in phase** (a strong positive front edge — the solid attack), then drift out of phase because they sit at different frequencies, producing the characteristic short, punchy 606 thud. There is **no pitch-envelope sweep**.

- OSC 1 ≈ **60 Hz**, higher Q (≈ 7) — the body/sustain.
- OSC 2 ≈ **130 Hz**, lower Q (≈ 3), **lower amplitude**, shorter — adds the click/attack and the early beating against OSC 1.

**Synthesis model:**

- Two high-Q bandpass resonators (60 Hz, 130 Hz) pinged by one shared ~1 ms excitation pulse; sum with OSC 2 at lower level. Decay = resonator Q (no external AD needed, though a gentle output VCA envelope is fine).
- Accent scales the excitation amplitude → louder + longer ring on both resonators (§2.3).
- **Do not** add an 808-style downward pitch sweep — the 606 has none. The "thump" is the two-resonator beat + the in-phase attack edge.

**Parameters (authentic):**

| Param | Meaning | Range (target) | Default |
|---|---|---|---|
| LEVEL | voice output gain (panel mix) | 0 → unity | mix-center |
| ACCENT | global accent depth into excitation | — | global |

**Improvements over typical 606 plug-ins:** real two-resonator beating with the in-phase attack edge (sample packs flatten this to one hit; naive synths use a single oscillator + AD and a fake pitch drop); accent that lengthens the ring rather than just raising gain.

### 4.2 Snare Drum

**Circuit basis (Whittle):** a **single** Twin-T resonator made to oscillate as a decaying sine by the trigger pulse, **summed with** a "snappy" path = the shared white noise → trigger-driven gate → **high-pass filter**. The trigger pulse is ~1 ms; on accented beats its amplitude rises (≈ +5 V → +14 V) so the resonator rings at greater amplitude and for longer, and the noise gate passes more snap.

**Synthesis model:**

- One bridged-T resonator (tonal "body"), pinged on trigger. Body fundamental is a **calibration target** (low-hundreds-of-Hz region — the exact pitch is set by the network and varies per unit; Whittle's pitch mod swaps R107/R110/C52). Decay = resonator Q.
- Snappy: shared noise → gate (trigger-enveloped) → HPF → VCA, summed with the body. The HP corner + gate decay define the snap.
- Body↔snap balance and snap decay are fixed in authentic mode (set internally on the stock unit).

**Parameters (authentic):** LEVEL, ACCENT.

**Improvements over typical 606 plug-ins:** a *pinged resonator* body with a genuine HP-gated noise snap and the accent-as-harder-ping behavior, instead of a single noise burst + static tone.

### 4.3 Toms — Low / High

**Circuit basis (Whittle / burnkit):** each tom is a Twin-T resonator at a fixed pitch, **plus** a shared, gated **low-pass** noise burst (a "short and only just noticeable burst of rumble") summed into the attack. The two toms have a **linked architecture** (they share the gated tom-noise circuit and interact at their control extremes). Pitch is set per tom by a single resistor (R318 low, R333 high); decay is fixed by the resonator's R/C.

**Synthesis model per tom:**

- One bridged-T resonator pinged on trigger; decay = Q.
- A short LP-filtered noise burst (shared tom-noise, gated by the trigger) mixed in at the attack for body realism, gone by the tail.
- Low and High differ **only in resonator tuning** — implement both from one parameterised model. **Per-tom fundamentals are calibration targets** (Low lower, High higher; measure from the unit — exact values are unit-dependent and not published reliably). Treat them as two tunings of one model.

**Parameters (authentic):** LEVEL (**per tom** — MM-06 gives Low and High independent levels, unlike the stock panel's single shared Toms pot; §7), ACCENT.

**Improvements over typical 606 plug-ins:** the small LP tom-noise attack burst and the linked two-tom behavior, not just two pitched sines.

---

## 5. The shared noise source

The snare and both toms tap **one** analog white-noise generator (reverse-biased transistor breakdown — flat spectrum). Implement a single shared white stream and derive each voice's noise by filtering taps of it (snare → HP snap gate; toms → LP rumble gate). Maintain one PRNG stream so coincident hits share correlated noise (matches hardware; subtle but real). Seed from the global seed for determinism. (Note: the **cymbal/hats do not use this noise source** — they use the six-oscillator metallic cluster, §6.)

---

## 6. Metallic voices — Cymbal, Open Hat, Closed Hat (synthesized)

**Design decision:** the cymbal and hi-hats are **synthesized**, not sampled. The 606 generates them entirely from the six-oscillator metallic cluster (§3.6) and the two bridged-T band-pass paths (§3.7) — there is no ROM, no PCM, no capture-as-playback. They are pure synthesis blocks like the rest of the engine.

### 6.1 The metallic source (shared)

Six free-running Schmitt-trigger square oscillators (§3.6, ≈ 245/308/367/417/438/625 Hz) mixed equally → metallic cluster → split into the **7100 Hz** (high) and **3440 Hz** (low) bridged-T band-pass paths (§3.7). The cluster is **continuous and never reset** — every metallic hit gates it at a free-running phase, which is the source of hat/cymbal per-hit variance (§1.3, §10).

### 6.2 Cymbal

**Circuit basis (Baratatronix):** the cymbal uses **both** metallic paths:

- **High path (7100 Hz)** → VCA → **resonant HPF** (extra high-frequency emphasis = shimmer).
- **Low path (3440 Hz)** → separate VCA → **single-transistor HPF** (body).
- A **single envelope** drives both VCAs.

**Synthesis model:** gate both filtered metallic paths with one shared decay envelope; sum the resonant-HP'd high path and the transistor-HP'd low path. Long, inharmonic, clangy. Decay is long; on the stock unit it is fixed.

**Parameters (authentic):** LEVEL, ACCENT.

### 6.3 Hi-Hats (Open + Closed)

**Circuit basis (Baratatronix):** OH and CH **share one path**: the **high (7100 Hz)** metallic noise → a **common VCA** → **resonant HPF**. Two **independent envelopes** (open = long decay, closed = short decay) modulate that shared VCA. The hats do **not** use the 3440 Hz path (that is cymbal-only).

- **Choke (shut-off circuit):** triggering the **closed** hat while the **open** hat is still sounding **shortens the open hat's envelope** — a real mutual-exclusion / decay-cut. Model it as a single choke group: a CH trigger (or a new hat trigger) collapses any sounding OH decay within a few ms.
- **Stock decay is uncontrolled and tempo-linked.** On a stock 606 there is **no** OH decay knob; the open-hat decay is set by tempo (faster tempo → shorter decay), overridable by the CH/OH interaction. Since MM-06 specifies no sequencer/clock (§ scope), the OH and CH decays are **fixed internal defaults** calibrated to the reference clips; document that on hardware these are tempo-derived, not panel controls.

**Synthesis model:** one shared filtered metallic VCA fed by two decay envelopes selected by which hat fired; CH trigger chokes a live OH; both derive from the **same** free-running cluster (so CH and OH are timbrally identical apart from decay, exactly like the hardware).

**Parameters (authentic):** CH LEVEL, OH LEVEL (**independent per-hat levels** — the stock panel shares one Hi-Hat mix pot for both, but MM-06 splits them so Open and Closed each have their own level; §7), ACCENT.

---

## 7. Mixer and output stage

- **Single stereo main output** — the summed mix only, no per-voice output channels. Like the stock hardware (main-out only); MM-06 deliberately does **not** model the individual-outputs mod. The 606 mix is mono, so the two output channels are dual-mono unless per-hit variance (§10) adds slight width.
- **Panel mix levels:** **seven independent per-voice levels** — BD, SD, LT, HT, CY, OH, CH — plus master VOLUME and the global ACCENT level. This is a **deliberate divergence** from the stock 606, whose panel has only five mix pots (Toms L+H share one, Hi-Hat O+C share one); MM-06 splits both shared pots so every voice gets its own level. The split mirrors the "individual volume" 606 mods and is a pure mixer/UI choice with **no fidelity consequence** — the reference clips are level-normalized (they carry no absolute level, §1) and the calibration harness renders each voice in isolation anyway (§2.1), so inter-voice balance is never fit to a recording.
- **Summing coloration:** the output mixer/amp adds slight nonlinearity and bandwidth shaping; model a gentle bus saturation and the output amp's bandwidth.
- **DC blocking** on each voice output and on the master.

---

## 8. Aliasing, stability, and numerical hygiene

- Oversample the six square oscillators and every nonlinear stage with the **host-rate-scaled factor** (§2.2: 8× at 44.1/48 kHz, 4× at 88.2/96 kHz; noise paths half that) and decimate with a steep polyphase FIR. The square-oscillator cluster is the dominant aliasing source — get this right or the metallic voices ring with the tell-tale "cheap clone" inharmonic alias junk.
- Use BLEP/BLAMP corrections on the square-oscillator edges (and the trigger-pulse ping) if not relying purely on oversampling.
- Keep the metallic cluster **free-running** but **deterministic**: its phase is a function of (seed, elapsed samples), never wall-clock.
- Clamp envelope and resonator time-constants to avoid denormals; flush-to-zero on tails.
- Seed per-hit RNG deterministically from a global seed so sessions are reproducible when variance is dialed to zero.

---

## 9. Control set: 606-authentic

The reference unit (a stock 606, §1) defines a **minimal control surface**, and MM-06 ships exactly that surface — nothing richer. **There is no "calibration-target adds controls" tier** — the calibration target *is* the thing being cloned, and it has almost no controls.

- **606-authentic controls** — the real panel: master **VOLUME**, a global **ACCENT** level, and instrument **mix-level** pots. The stock unit has five (BD, SD, Toms[L+H], Cymbal, Hi-Hat[O+C]); **MM-06 splits the two shared pots into seven independent per-voice levels** (BD, SD, LT, HT, CY, OH, CH — §7). This is the one sanctioned divergence on the *level* surface — a mixer/UI convenience with no fidelity cost (levels are normalized away in calibration). Per-voice **TUNE/DECAY do not exist** on the stock unit; every voice's pitch, decay, and tone are fixed at their calibrated default operating point, and the sounds are otherwise uneditable.

**Calibration consequence:** the dataset is **one unaccented clip per voice**, so it fits each voice's **single default operating point** — which is the entire control surface, since there are no per-voice timbral knobs to range over. **Accent** is absent from the dataset — its behaviour is a **best guess**.
---

## 10. Analog-variance layer ("the unit feels alive")

Real 606s differ unit-to-unit and hit-to-hit. Model, with a global depth control (0 = sterile/repeatable):

- **Metallic phase per hit (the big one):** the six oscillators free-run, so each hat/cymbal gates at a different cluster phase — this is the dominant, authentic source of hat/cymbal liveliness. Reproduce it by *not* resetting the cluster and by seeding its phase deterministically from the run state (§3.6, §6.1).
- **Per-hit jitter:** small randomization of excitation timing/amplitude, resonator start energy, and noise seed.
- **Component tolerance (per-instance, fixed at "build"):** spread on the bridged-T resonator freqs/Qs (BD, SD, toms), the six oscillator frequencies, and the band-pass corners — gives each emulated "unit" its own character. Real analog tolerance here is several percent.
- **Drift:** slow random-walk on a few parameters (temperature/PSU) for a long-session "warming" feel (very subtle).
- **VCA grit vs. level:** harder (accented) hits distort a touch more (§3.9).

---

## 11. Validation methodology

Calibrate and prove fidelity against **the single normalized, unaccented reference clip per voice** (§1). The clip anchors *relative* spectral and temporal shape only — it is normalized (no absolute level) and unaccented (accent is best-guess, not measured). Caveats are the ordinary analog ones plus the single-clip ones: unit-to-unit variance, recording-chain coloration, and one phase realization for the metallic voices.

1. **Per-voice spectrograms** of the reference clip; match spectral centroid, decay slope, and (for the metallic voices) the inharmonic peak structure of the cluster + band-pass paths.
2. **Decay-envelope extraction** (Hilbert/energy envelope) for every voice — match the resonator ring times (BD two-resonator beat; SD body; toms) and the metallic decay shapes (CH short, OH long, cymbal long). Shape only — the clip is normalized.
3. **Two-resonator BD fit:** fit the 60 Hz and 130 Hz resonators' frequencies, Qs, and *relative* level, and confirm the **in-phase attack edge** and subsequent beating against the reference kick clip.
4. **Snare body+snap:** fit the body resonator pitch/decay and the HP-gated noise snap (corner + gate decay) against the reference clip.
5. **Metallic peak structure:** match the six-oscillator inharmonic peaks (within tolerance) and the 7100/3440 Hz band-pass shaping; verify the cymbal uses both paths and the hats only the high path. The clip is one phase realization — match peak structure, not phase.
6. **Choke behavior:** confirm a CH trigger cuts a sounding OH cleanly within a few ms. *(Regime A — no clip needed.)*
7. **Accent direction (best guess, regime A):** confirm accented hits ring **longer and brighter** (resonators pinged harder; more HP snap), not merely louder — the spec's modelled accent behaviour. This is asserted by monotonicity, **not** fit to hardware: the reference clips are unaccented, so accent has no regime-C A/B and its magnitude is a best guess.
8. **Null/difference tests** are *not* a primary oracle here — they are poor for analog voices (phase/variance, free-running cluster) *and* meaningless against a normalized clip. Prioritise spectral-shape and envelope distance, and say so.

Track each voice against its targets in a regression suite so future changes don't drift away from the reference clip.

---

### Source basis

Circuit analyses: **Robin Whittle / Real World Interfaces (First Principles)** — bass drum (two Twin-T resonators, in-phase attack, no pitch switch), snare (single Twin-T + HP-gated white noise; trigger pulse 1 ms, accent raises pulse amplitude), the shared reverse-biased-transistor noise source, and the toms' shared gated LP-noise burst. **Peter Barata / Baratatronix** — the cymbal/hi-hat metallic source: six HD14584B Schmitt-trigger square oscillators (≈ 245/308/367/417/438/625 Hz, calibration targets), the two bridged-T band-pass paths (7100 Hz hats+cymbal-shimmer, 3440 Hz cymbal-body-only), the cymbal's dual-VCA/dual-HPF structure, and the shared-VCA hi-hats with independent OH/CH envelopes, choke, and tempo-linked OH decay. **Thomas Michaux (hyperreal)** — bass-drum resonator frequencies/Qs (OSC1 ≈ 60 Hz Q≈7; OSC2 ≈ 130 Hz Q≈3, lower amplitude). **Roland TR-606 service notes** — block diagram and schematic. All numeric pitch/timing values marked "calibration target" are starting estimates to be finalized against the single reference clip per voice (§11); quantities no clip can show (accent magnitude, per-hit variance depth, component-tolerance spread, the OH decay-vs-tempo curve, absolute level) are left as documented **best guesses**. The standing caveat is that the clip is one normalized recording of one unit, which varies with component tolerance and age.
