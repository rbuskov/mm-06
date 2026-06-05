# `harness/` — the `dsp-harness` MCP server

The MM-06 calibration agent's hands. It wraps the offline `render` binary
(`crates/render`) and the Python `analysis/` layer and exposes the tool surface
from `documents/calibration-and-testing-strategy.md` §2.3, so the agent drives a
tight **render → analyze → invariants → parity** loop instead of edit-and-pray.

Every tool is a plain, typed function in `harness/tools.py` — **unit-testable
without a live MCP client**. `harness/server.py` registers those same functions
with the official MCP Python SDK (`mcp`). The SDK is imported *lazily*, so the
tool functions (and their tests) run even where `mcp` isn't installed.

All synthesis is the deterministic `dsp_core` core; there are no samples
anywhere. Calibration defaults are **sterile and reproducible**:
`variance_depth=0`, a fixed default seed (`0x606`), and fixed default event
placement (one hit at sample 0).

## Layout

```
harness/
  harness/
    tools.py      # the §2.3 tool surface (plain functions)
    server.py     # MCP server entry (lazy `mcp` import) — `dsp-harness` console script
    selftest.py   # `python -m harness.selftest` end-to-end smoke
    params.py     # the write side: whitelisted calibratable-constant registry
    targets.py    # regime-B target table + regime-C reference shape distance
    goldens.py    # the golden store + comparator + `bless_golden` (§9)
    paths.py      # repo-path resolution (relative to this package; worktree-safe)
  goldens/        # COMMITTED golden baselines: <name>/golden.json
  tests/          # pytest: function-level E2E against the bleep
  requirements.txt / pyproject.toml   # pinned deps
  .gitignore      # .venv/, .work/, __pycache__, generated *.wav/*.json (goldens/ tracked)
```

Generated WAV/JSON artifacts and the param-override file live in
`harness/.work/` (a `wav_id` is `.work/<id>.wav`); everything there is
git-ignored. The **golden baselines** under `harness/goldens/` are the deliberate
exception: they **are committed** — see *Goldens & `--bless`* below.

## Setup

```sh
cd harness
python3 -m venv .venv
.venv/bin/pip install -r ../analysis/requirements.txt   # numpy/scipy/soundfile/librosa
.venv/bin/pip install -r requirements.txt               # mcp SDK + pytest
# build the render binary the harness shells out to (also built on demand):
cargo build -p render        # from the repo root
.venv/bin/python -m pytest -q                            # all green
.venv/bin/python -m harness.selftest                     # end-to-end smoke
```

`analysis/` is imported off `sys.path` (left unmodified) — the analysis deps
must therefore be installed into the same venv.

Verified on macOS / CPython 3.14 (arm64): `mcp==1.27.2`, `pytest==9.0.3`, and
the analysis chain (numpy/scipy/soundfile/librosa) all installed cleanly.

## Run the MCP server

```sh
.venv/bin/python -m harness.server     # or the console script: dsp-harness
```

It speaks MCP over **stdio** — point a `claude_desktop_config.json` /
`mcp.json` stdio server entry at that command (with the venv's interpreter and
`harness/` as cwd). `build_server()` constructs a `FastMCP("dsp-harness")` with
all twelve tools registered.

## The tool surface (§2.3)

| tool | what it does |
|------|--------------|
| `render_voice(voice, sample_rate, block_size, seed, variance_depth, accent, events, length_samples) -> wav_id` | render ONE voice in isolation (pre-mix, no bus coloration); sets the request's `voice` field. `events=None` places one (optionally accented) hit at sample 0. |
| `render_mix(request) -> wav_id` | render the full main output (mixer + output stage). `request` is a RenderRequest dict; missing fields fall back to sterile defaults. |
| `analyze(wav_id, feature_set) -> metrics_json` | wraps `analysis/` — centroid, decay envelope, fundamental/partials, alias floor, DC, etc. |
| `run_invariants(scope) -> report` | regime-A suite: `regime-a`/`cheap` → `cargo test -p dsp_core`; `full`/`heavy` → `--features full`. Parsed into `{green, passed, failed, ignored, ...}`. |
| `parity_check(request) -> {max_divergence, ...}` | WASM↔native parity (§5): runs the native streaming-vs-offline gate (`cargo test --test parity`); also runs `scripts/parity.sh` if a wasm runtime (wasmtime/wasmer) is present. `max_divergence` is **0** when bit-identical, `-1` on failure. |
| `compare_to_targets(metrics, voice) -> deltas` | regime B: signed deltas from measured features to the spec targets. |
| `compare_to_reference(wav_id, voice) -> shape_distance` | regime C: normalized, onset-aligned, multi-resolution log-STFT **shape distance** to `references/<voice>.wav` (never a null test). |
| `list_params() -> [...]` | the whitelist of calibratable constants + their active values. |
| `propose_param_edit(key, new_value) -> proposal` | validate (don't persist) an edit; refuses out-of-scope keys and out-of-bounds values. |
| `apply_param_edit(key, new_value) -> result` | persist an in-scope, in-bounds edit to `.work/param_overrides.json`. |
| `compare_to_golden(name) -> drift_report` | re-render a golden's request, recompute its metric vector, **flag drift** (per-metric deltas vs. tolerance + render-hash match). |
| `bless_golden(voice, reason, name?) -> diff_report` | the **only** writer of a golden: re-render, compute the new metric vector + hash, emit a readable **before/after diff**, refresh the stored golden (§9). |

### A typical loop

```python
from harness import tools
wav = tools.render_voice("sd")                       # sterile bleep, isolated
m   = tools.analyze(wav, ["fundamental", "decay", "dc_offset"])
print(tools.compare_to_targets(m, "sd"))             # regime B deltas
print(tools.compare_to_reference(wav, "sd"))         # regime C shape distance
print(tools.run_invariants("regime-a"))              # gate on regime A
print(tools.parity_check())                          # §5 parity (0 = bit-identical)
```

## The write side — what `*_param_edit` may and may not touch

`propose_param_edit` / `apply_param_edit` are scoped to **calibratable constants
only** — never shipped control semantics. Scope is enforced by a hard whitelist
(`harness/params.py :: REGISTRY`); any key outside it is **refused**, as is any
value outside the registered `[min, max]`.

A *calibratable constant* is a numeric tuning value that moves a measurable
acoustic feature (a resonator frequency, a Q, a decay τ, an oscillator pitch)
without changing how the engine is controlled or wired — exactly the constants
regime B's target table maps to (§6). Explicitly **out of scope** (refused):
LEVEL gain law, accent direction, choke behaviour, the message protocol, the
mixer / output saturation / oversample factor, and `variance_depth` / seed /
master level (best-guess, human-gated — §7.1).

Today the whitelist shadows the placeholder "bleep" constants in
`crates/dsp_core/src/lib.rs` (`Bleep::FREQ_HZ`, `BASE_TAU`, `BASE_AMP`); each
registry entry records the source file + symbol it shadows. As real voices land,
their resonator freqs/Qs/decay-τs (the §6 table) are added to the registry.

**Where edits are written:** to keep the harness self-contained and avoid
mutating the read-only `crates/` tree from a worktree, applied edits are
persisted to `harness/.work/param_overrides.json` (git-ignored). A blessed value
is later folded back into `dsp_core` under human review (`--bless`, §9) — the
harness never edits `crates/` itself.

## Regime-B / regime-C status

These are wired end to end with a **trivial/placeholder target set** per the task
scope; the structures they return are stable as real per-voice targets/clips fill
in. `compare_to_targets` anchors the current 880 Hz bleep (fundamental, DC, decay
τ). `compare_to_reference` already works against the shipped `references/<voice>.wav`
clips, returning a shape distance plus decomposed feature deltas.

## Goldens & the `--bless` workflow (§9)

A **golden** is a *frozen render plus its metric vector*. It guards regimes **B**
and **C** and the full mix against accidental drift; regime-A invariants are
correctness, not taste, and are **never** guarded or blessed here (`run_invariants`
gates those). `bless_golden` is the **only** writer of a golden and always emits a
before/after diff for the one human glance.

**What is committed vs. ignored.** Each golden is `harness/goldens/<name>/golden.json`,
which **is committed** — the whole point of a golden is a baseline in git. It holds
the sterile render `request`, the frozen `metric_vector`, a `render_hash` (sha256
over the rendered f32 PCM — a compact signature, not the WAV bytes), the active
`overrides` at bless time, per-metric `tolerances`, and a `blessed` audit block. No
raw WAV is committed; the metric vector + hash is the diffable baseline. Transient
working renders stay in the git-ignored `.work/`. `harness/.gitignore` keeps
`goldens/**/golden.json` tracked despite the blanket `*.json` ignore.

```python
from harness import tools
tools.bless_golden("sd", "establish baseline")  # writes goldens/sd/golden.json + diff
tools.compare_to_golden("sd")                    # {drift: False, ...} on an unchanged render
tools.apply_param_edit("bleep.freq_hz", 990.0)   # perturb a calibratable constant
tools.compare_to_golden("sd")                    # {drift: True, drifted_metrics: ["fundamental", ...]}
tools.bless_golden("sd", "accept the new pitch") # before/after diff + refresh → compare clean again
```

**The override shadow.** The offline `render` binary hard-codes the bleep
constants and does not yet read `param_overrides.json` (fold-back is a later,
human-reviewed step). So a constant edit is *visible* before that, the golden
render path applies the active whitelisted overrides as a **documented
harness-side shadow** (pitch-shift for `bleep.freq_hz`, decay re-window for
`bleep.base_tau`, gain for `bleep.base_amp`); `crates/` and `analysis/` are
untouched. It collapses to a no-op once `render` reads the overrides. See
`goldens.py`.

## Testing

`tests/` exercises the tool functions directly (no MCP client needed):
render a **still-bleep** voice (`sd`, robust to BD becoming a real voice) →
analyze (≈ 880 Hz, DC ≈ 0) → `render_mix` round-trip → regime-B/C plumbing →
param-edit round-trip + out-of-scope/out-of-bounds refusals → the **golden
round-trip** (`test_goldens.py`: committed baseline re-renders clean → a perturbed
constant flags drift and names the moved metric → `bless` diffs + refreshes →
clean again) → and the cargo-backed `run_invariants` (GREEN) and `parity_check`
(0 divergence), marked `slow`. Run a subset with `-m "not slow"`.
