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
    paths.py      # repo-path resolution (relative to this package; worktree-safe)
  tests/          # pytest: function-level E2E against the bleep
  requirements.txt / pyproject.toml   # pinned deps
  .gitignore      # .venv/, .work/, __pycache__, generated *.wav/*.json
```

Generated WAV/JSON artifacts and the param-override file live in
`harness/.work/` (a `wav_id` is `.work/<id>.wav`); everything there is
git-ignored.

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
all ten tools registered.

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

## Testing

`tests/` exercises the tool functions directly (no MCP client needed):
render a **still-bleep** voice (`sd`, robust to BD becoming a real voice) →
analyze (≈ 880 Hz, DC ≈ 0) → `render_mix` round-trip → regime-B/C plumbing →
param-edit round-trip + out-of-scope/out-of-bounds refusals → and the
cargo-backed `run_invariants` (GREEN) and `parity_check` (0 divergence), marked
`slow`. Run a subset with `-m "not slow"`.
