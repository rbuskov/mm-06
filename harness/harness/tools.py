"""The dsp-harness tool surface (`calibration-and-testing-strategy.md` §2.3).

Each tool is a plain, typed Python function so it is unit-testable WITHOUT a live
MCP client; `harness.server` registers these same functions with an MCP server.

A `wav_id` is an opaque handle into the harness working dir
(`harness/.work/<id>.wav`, git-ignored). The render tools shell out to the built
`render` binary; `analyze` imports the unmodified `analysis/` package.

Calibration defaults are **sterile and reproducible** (§2.3): `variance_depth=0`,
a fixed default seed, and fixed default event placement.
"""

from __future__ import annotations

import json
import re
import subprocess
import uuid
from pathlib import Path

from . import params as _params
from . import targets as _targets
from .paths import (
    REPO_ROOT,
    add_analysis_to_syspath,
    ensure_work_dir,
    render_binary,
)

# --- sterile calibration defaults (§2.3) -------------------------------------
DEFAULT_SEED = 0x606
DEFAULT_SAMPLE_RATE = 48000.0
DEFAULT_BLOCK_SIZE = 128
DEFAULT_VARIANCE_DEPTH = 0.0
DEFAULT_LENGTH_SAMPLES = 24000  # 0.5 s @ 48 kHz — bleep tail completes
# Fixed default event placement: a single hit at t=0 of the chosen voice.
DEFAULT_EVENT_SAMPLE = 0


class RenderError(RuntimeError):
    pass


def ensure_render_built() -> Path:
    """Return the render binary path, building it on demand if missing."""
    exe = render_binary()
    if exe.exists():
        return exe
    proc = subprocess.run(
        ["cargo", "build", "-p", "render"],
        cwd=str(REPO_ROOT),
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0 or not exe.exists():
        raise RenderError(
            "failed to build render binary (cargo build -p render):\n" + proc.stderr
        )
    return exe


def _wav_path(wav_id: str) -> Path:
    return ensure_work_dir() / f"{wav_id}.wav"


def _run_render(request: dict, wav_id: str) -> str:
    """Write the JSON request, invoke `render`, return the wav_id."""
    exe = ensure_render_built()
    work = ensure_work_dir()
    wav = _wav_path(wav_id)
    req_path = work / f"{wav_id}.request.json"
    req_path.write_text(json.dumps(request))
    proc = subprocess.run(
        [str(exe), str(req_path), "-o", str(wav)],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0 or not wav.exists():
        raise RenderError(f"render failed (exit {proc.returncode}): {proc.stderr}")
    return wav_id


def _normalize_events(events, default_voice: str | None):
    """Coerce the events list to render's JSON event schema.

    `None` => the fixed default placement (one hit at sample 0). Each event may
    be a dict (passed through) or a 2/3-tuple `(sample_index, voice[, accent])`.
    """
    if events is None:
        if default_voice is None:
            return []
        return [{"sample_index": DEFAULT_EVENT_SAMPLE, "voice": default_voice}]
    out = []
    for e in events:
        if isinstance(e, dict):
            ev = {"sample_index": int(e["sample_index"]), "voice": str(e["voice"])}
            if e.get("accent"):
                ev["accent"] = True
            out.append(ev)
        else:
            sample_index, voice = e[0], e[1]
            ev = {"sample_index": int(sample_index), "voice": str(voice)}
            if len(e) > 2 and e[2]:
                ev["accent"] = True
            out.append(ev)
    return out


def render_voice(
    voice: str,
    sample_rate: float = DEFAULT_SAMPLE_RATE,
    block_size: int = DEFAULT_BLOCK_SIZE,
    seed: int = DEFAULT_SEED,
    variance_depth: float = DEFAULT_VARIANCE_DEPTH,
    accent: bool = False,
    events=None,
    length_samples: int = DEFAULT_LENGTH_SAMPLES,
) -> str:
    """Render ONE voice in isolation (pre-mix, no bus coloration) → wav_id.

    Sets the render request's `voice` field (the `render_voice` isolation path).
    With `events=None` a single (optionally accented) hit is placed at sample 0;
    pass `events` to override placement. Sterile defaults (§2.3).
    """
    voice = voice.lower()
    evs = _normalize_events(events, default_voice=voice)
    if events is None and accent:
        evs[0]["accent"] = True
    request = {
        "sample_rate": float(sample_rate),
        "seed": int(seed),
        "variance_depth": float(variance_depth),
        "block_size": int(block_size),
        "length_samples": int(length_samples),
        "voice": voice,
        "events": evs,
    }
    wav_id = f"voice_{voice}_{uuid.uuid4().hex[:8]}"
    return _run_render(request, wav_id)


def render_mix(request: dict) -> str:
    """Render the full main output (mixer + output stage) → wav_id.

    `request` is a RenderRequest dict (no `voice` field = full mix). Missing
    fields fall back to the sterile defaults. `events` may use the tuple or
    dict form accepted by :func:`_normalize_events`.
    """
    req = {
        "sample_rate": float(request.get("sample_rate", DEFAULT_SAMPLE_RATE)),
        "seed": int(request.get("seed", DEFAULT_SEED)),
        "variance_depth": float(request.get("variance_depth", DEFAULT_VARIANCE_DEPTH)),
        "block_size": int(request.get("block_size", DEFAULT_BLOCK_SIZE)),
        "length_samples": int(request.get("length_samples", DEFAULT_LENGTH_SAMPLES)),
        "events": _normalize_events(request.get("events"), default_voice=None),
    }
    wav_id = f"mix_{uuid.uuid4().hex[:8]}"
    return _run_render(req, wav_id)


def analyze(wav_id: str, feature_set=None) -> dict:
    """Wrap `analysis/`: extract a metrics JSON from a rendered wav_id.

    `feature_set` is `None`/`"all"` (every feature), a name, or a list of names
    (see `analysis.schema.ALL_FEATURES`).
    """
    add_analysis_to_syspath()
    from analysis import analyze as _analyze  # imported lazily; analysis/ unmodified

    wav = _wav_path(wav_id)
    if not wav.exists():
        raise FileNotFoundError(f"no such wav_id {wav_id!r} ({wav})")
    return _analyze(str(wav), feature_set=feature_set)


def load_signal(wav_id: str):
    """Load a rendered wav_id as `(mono_float64, sample_rate)` (regime-C helper)."""
    add_analysis_to_syspath()
    from analysis.core import load_wav

    return load_wav(str(_wav_path(wav_id)))


# --- regime A: invariants ----------------------------------------------------

_TEST_RESULT_RE = re.compile(
    r"test result:\s*(?P<status>ok|FAILED)\.\s*"
    r"(?P<passed>\d+)\s+passed;\s*(?P<failed>\d+)\s+failed;\s*"
    r"(?P<ignored>\d+)\s+ignored"
)


def run_invariants(scope: str = "regime-a") -> dict:
    """Run the regime-A invariant suite and parse it into a structured report.

    `scope`:
      * `"regime-a"` / `"cheap"` → `cargo test -p dsp_core` (the commit gate);
      * `"full"` / `"heavy"` → adds `--features full` (block-size / sample-rate
        sweeps, long-decay tails).

    Returns `{green, passed, failed, ignored, command, ...}`; `green` is True
    only when cargo reports `ok` and zero failures.
    """
    scope = (scope or "regime-a").lower()
    cmd = ["cargo", "test", "-p", "dsp_core"]
    if scope in ("full", "heavy"):
        cmd += ["--features", "full"]
    elif scope not in ("regime-a", "cheap", "a"):
        raise ValueError(f"unknown invariant scope {scope!r}")

    proc = subprocess.run(
        cmd, cwd=str(REPO_ROOT), capture_output=True, text=True
    )
    output = proc.stdout + proc.stderr

    passed = failed = ignored = 0
    any_result = False
    for m in _TEST_RESULT_RE.finditer(output):
        any_result = True
        passed += int(m.group("passed"))
        failed += int(m.group("failed"))
        ignored += int(m.group("ignored"))

    green = proc.returncode == 0 and failed == 0 and any_result
    return {
        "scope": scope,
        "regime": "A",
        "green": green,
        "passed": passed,
        "failed": failed,
        "ignored": ignored,
        "command": " ".join(cmd),
        "returncode": proc.returncode,
        "report": "GREEN" if green else "RED",
    }


# --- parity (regime A / §5) --------------------------------------------------


def _have_wasm_runtime() -> bool:
    import shutil

    return shutil.which("wasmtime") is not None or shutil.which("wasmer") is not None


def parity_check(request: dict | None = None) -> dict:
    """WASM↔native parity gate (§5). Returns the max bit divergence.

    Runs the native streaming-vs-offline parity gate
    (`cargo test --test parity`). If a wasm runtime (wasmtime/wasmer) is present
    it additionally runs `scripts/parity.sh` (cross-architecture). `max_divergence`
    is **0** when the gate passes bit-identical; non-zero / -1 signals a failure.

    `request` is accepted for interface symmetry with the strategy's signature;
    the parity gate exercises its own canonical request.
    """
    cmd = ["cargo", "test", "--test", "parity", "-p", "dsp_core"]
    proc = subprocess.run(
        cmd, cwd=str(REPO_ROOT), capture_output=True, text=True
    )
    output = proc.stdout + proc.stderr
    failed = 0
    passed = 0
    for m in _TEST_RESULT_RE.finditer(output):
        passed += int(m.group("passed"))
        failed += int(m.group("failed"))
    native_pass = proc.returncode == 0 and failed == 0

    result = {
        "regime": "A/parity",
        "native_streaming_gate": "PASS" if native_pass else "FAIL",
        "native_command": " ".join(cmd),
        "native_passed": passed,
        "native_failed": failed,
        # max_divergence is 0 when the bit-identical gate passes; -1 on failure.
        "max_divergence": 0 if native_pass else -1,
        "wasm_runtime": False,
        "cross_arch": "skipped (no wasm runtime)",
    }

    if _have_wasm_runtime():
        result["wasm_runtime"] = True
        sh = subprocess.run(
            ["bash", "scripts/parity.sh"],
            cwd=str(REPO_ROOT),
            capture_output=True,
            text=True,
        )
        cross_pass = sh.returncode == 0
        result["cross_arch"] = "PASS" if cross_pass else "FAIL"
        result["cross_arch_output"] = (sh.stdout + sh.stderr).strip()
        if not cross_pass:
            result["max_divergence"] = -1
    return result


# --- regime B / C ------------------------------------------------------------


def compare_to_targets(metrics: dict, voice: str) -> dict:
    """Regime B: structured deltas from measured metrics to the spec targets."""
    return _targets.compare_to_targets(metrics, voice)


def compare_to_reference(wav_id: str, voice: str) -> dict:
    """Regime C: shape distance from a rendered wav_id to references/<voice>.wav."""
    x, sr = load_signal(wav_id)
    return _targets.compare_to_reference(x, sr, voice)


# --- write side --------------------------------------------------------------


def list_params() -> list:
    """The whitelist of calibratable constants and their active values."""
    return _params.list_params()


def propose_param_edit(key: str, new_value: float) -> dict:
    """Validate (don't persist) a calibratable-constant edit. See `harness.params`."""
    return _params.propose_param_edit(key, new_value)


def apply_param_edit(key: str, new_value: float) -> dict:
    """Persist an in-scope, in-bounds calibratable-constant edit. See `harness.params`."""
    return _params.apply_param_edit(key, new_value)
