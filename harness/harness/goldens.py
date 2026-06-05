"""Goldens + the `--bless` workflow (`calibration-and-testing-strategy.md` §9).

A **golden** is a *frozen render plus its metric vector*. Goldens guard regimes
**B** and **C** and the full mix against accidental drift; they are regenerated
**only** by an explicit, reviewed `bless_golden`, which emits a before/after diff
report for the one human glance (§9). Regime-A invariants are correctness, not
taste — they are **never** blessed away, and nothing here touches them.

## What a golden stores (committed) vs. what is ignored

Each golden lives in `harness/goldens/<name>/golden.json`, which **is committed**
— the whole point of a golden is that its baseline is in git. `golden.json`
holds:

  * `request` — the exact (sterile, §2.3) render request, so the golden is
    reproducible: a fixed seed, `variance_depth=0`, fixed event placement.
  * `metric_vector` — the `analyze(...)` feature dict (the frozen measurement).
  * `render_hash` — a sha256 over the rendered f32 PCM (a compact signature of
    the WAV itself; a content hash, not the bytes — diffable and tiny).
  * `overrides` — the active calibratable-constant overrides at bless time (so a
    re-render is compared on equal footing; see the shadow note below).
  * `tolerances` / `regime` / `blessed` — per-metric drift tolerances, the regime
    the golden guards (B / C / mix — never A), and a small audit block (reason,
    UTC timestamp, schema version).

The transient working render (`.work/<id>.wav` + sidecar JSON) is **git-ignored**
(`harness/.gitignore`). Only the `golden.json` baselines are tracked. No raw WAV
is committed: the metric vector + render hash is the diffable, git-committable
baseline the task calls for.

## The override shadow (why a perturbed constant moves the render *here*)

The offline `render` binary hard-codes the placeholder "bleep" constants and does
**not** yet read `harness/.work/param_overrides.json`; folding a blessed value
back into `dsp_core` is a later, human-reviewed step (`params.py`). So that the
calibration loop can nonetheless *see* a constant edit before that fold-back, the
golden render path applies the active whitelisted overrides as a **documented
harness-side shadow** of the bleep constants — a `bleep.freq_hz` override
pitch-shifts the rendered tone, a `bleep.base_tau` override re-windows its decay,
a `bleep.base_amp` override scales it. This shadow lives entirely in the harness
(`crates/` and `analysis/` are untouched); it exists only until `render` learns
to read the overrides, after which it collapses to a no-op.
"""

from __future__ import annotations

import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path

import numpy as np

from . import params as _params
from . import tools as _tools
from .paths import HARNESS_DIR

# Golden store: committed baselines (NOT under .work/, which is git-ignored).
GOLDENS_DIR = HARNESS_DIR / "goldens"
GOLDEN_SCHEMA_VERSION = 1

# The metric vector frozen per golden: scalar features `analyze` returns that
# move under a calibratable-constant edit (regime B/C territory). Sub-paths dig
# into a structured feature (e.g. decay.tau_e).
GOLDEN_FEATURES = ("fundamental", "spectral_centroid", "dc_offset", "peak", "rms", "decay")

# Per-metric absolute drift tolerances. A re-render must stay within these of the
# frozen value or the comparator flags drift. Generous enough to absorb the
# analyzer's own numeric noise on an unchanged render, tight enough that a real
# constant edit trips them.
DEFAULT_TOLERANCES: dict[str, float] = {
    "fundamental": 5.0,            # Hz
    "spectral_centroid": 25.0,     # Hz
    "dc_offset": 1e-3,             # linear
    "peak": 5e-3,                  # linear
    "rms": 5e-3,                   # linear
    "decay.tau_e": 5e-3,           # s
}


class GoldenError(RuntimeError):
    pass


# --- the override shadow (see module docstring) ------------------------------


def _apply_override_shadow(x: np.ndarray, sr: float, overrides: dict[str, float]) -> np.ndarray:
    """Shadow the active bleep overrides onto a rendered signal.

    A harness-side stand-in until the `render` binary reads
    `param_overrides.json` (then this collapses to a no-op). Touches only the
    whitelisted bleep constants; anything else is ignored here.
    """
    x = np.asarray(x, dtype=np.float64)
    if x.size == 0:
        return x

    # bleep.freq_hz → resample (pitch shift) by override/default, holding length.
    freq = overrides.get("bleep.freq_hz")
    if freq is not None:
        default = _params.REGISTRY["bleep.freq_hz"].default
        if default > 0 and abs(freq - default) > 1e-9:
            ratio = freq / default
            n = x.size
            # Read the source at `ratio`× speed (higher pitch), zero-padded past
            # the end so the buffer length — and thus the analysis window — holds.
            src_idx = np.arange(n) * ratio
            lo = np.floor(src_idx).astype(np.int64)
            frac = src_idx - lo
            x_pad = np.concatenate([x, np.zeros(1)])
            in_range = lo < n
            hi = np.where(lo + 1 <= n, lo + 1, n)
            x = np.where(
                in_range,
                (1.0 - frac) * x_pad[np.clip(lo, 0, n)] + frac * x_pad[np.clip(hi, 0, n)],
                0.0,
            )

    # bleep.base_tau → re-window the exponential decay by exp(-t (1/new - 1/old)).
    tau = overrides.get("bleep.base_tau")
    if tau is not None:
        default = _params.REGISTRY["bleep.base_tau"].default
        if default > 0 and tau > 0 and abs(tau - default) > 1e-12:
            t = np.arange(x.size) / sr
            x = x * np.exp(-t * (1.0 / tau - 1.0 / default))

    # bleep.base_amp → linear gain by override/default.
    amp = overrides.get("bleep.base_amp")
    if amp is not None:
        default = _params.REGISTRY["bleep.base_amp"].default
        if default > 0 and abs(amp - default) > 1e-12:
            x = x * (amp / default)

    return x


# --- render + measure a golden -----------------------------------------------


def _render_and_measure(request: dict) -> tuple[dict, str, int]:
    """Render `request`, apply the override shadow, return (metrics, hash, n).

    The metric vector is computed from the (possibly shadowed) signal; the hash
    is a sha256 over its f32 PCM — a compact, diffable signature of the render.
    """
    voice = request.get("voice")
    if voice is not None:
        wav_id = _tools.render_voice(
            voice,
            sample_rate=request["sample_rate"],
            block_size=request["block_size"],
            seed=request["seed"],
            variance_depth=request["variance_depth"],
            events=request["events"],
            length_samples=request["length_samples"],
        )
    else:
        wav_id = _tools.render_mix(request)

    x, sr = _tools.load_signal(wav_id)
    overrides = _params._load_overrides()
    x = _apply_override_shadow(np.asarray(x, dtype=np.float64), sr, overrides)

    _tools.add_analysis_to_syspath()
    from analysis import analyze_array  # analysis/ unmodified

    metrics = analyze_array(x, sr, feature_set=list(GOLDEN_FEATURES))
    pcm = x.astype("<f4").tobytes()
    render_hash = hashlib.sha256(pcm).hexdigest()
    return metrics, render_hash, int(x.size)


def _flatten_metric_vector(metrics: dict) -> dict[str, float]:
    """Pull the frozen scalar metric vector out of an `analyze` result."""
    feats = metrics.get("features", {})
    out: dict[str, float] = {}
    for name in GOLDEN_FEATURES:
        if name not in feats:
            continue
        val = feats[name]
        if name == "decay" and isinstance(val, dict):
            out["decay.tau_e"] = float(val.get("tau_e", 0.0))
        elif isinstance(val, (int, float)):
            out[name] = float(val)
    return out


def _default_request(voice: str) -> dict:
    """The sterile (§2.3) render request a voice golden freezes."""
    voice = voice.lower()
    return {
        "sample_rate": _tools.DEFAULT_SAMPLE_RATE,
        "seed": _tools.DEFAULT_SEED,
        "variance_depth": _tools.DEFAULT_VARIANCE_DEPTH,
        "block_size": _tools.DEFAULT_BLOCK_SIZE,
        "length_samples": _tools.DEFAULT_LENGTH_SAMPLES,
        "voice": voice,
        "events": [{"sample_index": _tools.DEFAULT_EVENT_SAMPLE, "voice": voice}],
    }


# --- store I/O ---------------------------------------------------------------


def golden_dir(name: str) -> Path:
    return GOLDENS_DIR / name


def golden_path(name: str) -> Path:
    return golden_dir(name) / "golden.json"


def golden_exists(name: str) -> bool:
    return golden_path(name).exists()


def load_golden(name: str) -> dict:
    p = golden_path(name)
    if not p.exists():
        raise GoldenError(f"no golden named {name!r} ({p}); bless it first")
    return json.loads(p.read_text())


def _write_golden(name: str, doc: dict) -> Path:
    d = golden_dir(name)
    d.mkdir(parents=True, exist_ok=True)
    p = golden_path(name)
    p.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    return p


def list_goldens() -> list[str]:
    if not GOLDENS_DIR.exists():
        return []
    return sorted(p.name for p in GOLDENS_DIR.iterdir() if (p / "golden.json").exists())


# --- comparator --------------------------------------------------------------


def _diff_vectors(old: dict[str, float], new: dict[str, float], tolerances: dict[str, float]) -> list[dict]:
    """Per-metric before/after deltas with within-tolerance flags."""
    rows = []
    for key in sorted(set(old) | set(new)):
        before = old.get(key)
        after = new.get(key)
        tol = tolerances.get(key, 0.0)
        delta = None if (before is None or after is None) else (after - before)
        within = (delta is not None) and (abs(delta) <= tol)
        rows.append(
            {
                "metric": key,
                "before": before,
                "after": after,
                "delta": delta,
                "tol": tol,
                "within_tol": within,
            }
        )
    return rows


def compare_to_golden(name: str) -> dict:
    """Re-render a golden's request, recompute its metrics, flag drift.

    Returns a structured drift report: per-metric before/after deltas (vs. the
    frozen metric vector and its tolerance) plus a render-hash match flag.
    `drift` is True if any metric exceeds tolerance OR the render hash changed.
    """
    golden = load_golden(name)
    new_metrics, new_hash, _ = _render_and_measure(golden["request"])
    new_vec = _flatten_metric_vector(new_metrics)
    old_vec = golden["metric_vector"]
    tolerances = golden.get("tolerances", DEFAULT_TOLERANCES)

    rows = _diff_vectors(old_vec, new_vec, tolerances)
    metric_drift = [r["metric"] for r in rows if not r["within_tol"]]
    hash_match = new_hash == golden["render_hash"]
    drift = bool(metric_drift) or not hash_match

    return {
        "name": name,
        "regime": golden.get("regime"),
        "drift": drift,
        "hash_match": hash_match,
        "old_hash": golden["render_hash"],
        "new_hash": new_hash,
        "drifted_metrics": metric_drift,
        "deltas": rows,
        "note": "regime-A invariants are never guarded by a golden — run_invariants gates those (§9)",
    }


# --- bless (the only writer of a golden) -------------------------------------


def bless_golden(voice: str, reason: str, name: str | None = None) -> dict:
    """Freeze (or refresh) a golden and emit a before/after diff report (§9).

    THE ONLY writer of a golden. Re-renders the sterile request for `voice`
    (regime B / C / mix — **never** a regime-A invariant), computes the new
    metric vector + render hash, emits a readable before/after diff of the old
    vs. new metric vector (per-field deltas), then refreshes the stored golden.
    Always returns the diff for the one human glance.

    `voice` selects the default per-voice sterile render and is the golden name
    unless `name` is given. `reason` is recorded in the golden's audit block.
    """
    voice = voice.lower()
    key = name or voice
    request = _default_request(voice)

    new_metrics, new_hash, _ = _render_and_measure(request)
    new_vec = _flatten_metric_vector(new_metrics)

    had_prior = golden_exists(key)
    old_vec: dict[str, float] = {}
    old_hash: str | None = None
    if had_prior:
        prior = load_golden(key)
        old_vec = prior.get("metric_vector", {})
        old_hash = prior.get("render_hash")

    diff_rows = _diff_vectors(old_vec, new_vec, DEFAULT_TOLERANCES)

    doc = {
        "schema_version": GOLDEN_SCHEMA_VERSION,
        "name": key,
        "voice": voice,
        "regime": "B/C",  # a golden freezes B/C/mix renders, never regime-A (§9)
        "request": request,
        "metric_vector": new_vec,
        "render_hash": new_hash,
        "overrides": _params._load_overrides(),
        "tolerances": DEFAULT_TOLERANCES,
        "blessed": {
            "reason": reason,
            "at_utc": datetime.now(timezone.utc).isoformat(),
            "previously_blessed": had_prior,
        },
    }
    path = _write_golden(key, doc)

    return {
        "name": key,
        "voice": voice,
        "reason": reason,
        "regime": "B/C",
        "created": not had_prior,
        "refreshed": had_prior,
        "old_hash": old_hash,
        "new_hash": new_hash,
        "hash_changed": old_hash != new_hash,
        "diff": diff_rows,           # the readable before/after, per metric
        "golden_path": str(path),
        "note": "regime-A invariants are never blessed — bless freezes B/C/mix renders only (§9)",
    }
