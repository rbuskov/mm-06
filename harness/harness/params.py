"""The write side: a whitelisted registry of **calibratable constants**.

`propose_param_edit` / `apply_param_edit` are the calibration loop's only way to
mutate the synth, and they are scoped to *calibratable constants only* — never
to shipped control semantics (LEVEL, accent routing, the protocol, mixer/output
topology). The scope is enforced by a hard whitelist: an edit to anything not in
:data:`REGISTRY` is refused.

## Scoping rationale (read before widening the whitelist)

A "calibratable constant" is a numeric tuning value that moves a *measurable
acoustic feature* (a resonator frequency, a Q, a decay τ, an oscillator pitch)
without changing how the engine is *controlled* or *wired*. Those are exactly the
constants regime B's target table maps to
(`calibration-and-testing-strategy.md` §6). Things that are explicitly **out of
scope** and will be refused:

  * control semantics — LEVEL gain law, accent direction, choke behaviour;
  * the message protocol / wire ids;
  * mixer, output saturation, oversample factor (architecture, not taste);
  * `variance_depth`, seed, master level (best-guess / human-gated, §7.1).

## Where edits are written

To keep the harness self-contained and avoid mutating the read-only `crates/`
tree from a worktree, applied edits are persisted to a harness-local JSON
**override file** (`harness/.work/param_overrides.json`, git-ignored). This is a
clearly-scoped registry the harness owns. Each registry entry records the source
constant it shadows (file + symbol) so a later step can fold a blessed value back
into `dsp_core` under human review; the harness never edits `crates/` itself.

Each entry: a logical key -> {default, min, max, source_file, source_symbol,
description}. `value()` returns the active value (override if present, else
default).
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from .paths import WORK_DIR, ensure_work_dir


@dataclass(frozen=True)
class Param:
    key: str
    default: float
    min: float
    max: float
    source_file: str  # relative to repo root, for fold-back under human review
    source_symbol: str
    description: str


# The whitelist. Only these keys may be edited. Today they shadow the
# placeholder "bleep" constants in `crates/dsp_core/src/lib.rs`; as real voices
# land, their resonator freqs / Qs / decay τs (regime-B §6 table) are added here.
REGISTRY: dict[str, Param] = {
    "bleep.freq_hz": Param(
        key="bleep.freq_hz",
        default=880.0,
        min=20.0,
        max=20000.0,
        source_file="crates/dsp_core/src/lib.rs",
        source_symbol="Bleep::FREQ_HZ",
        description="Placeholder bleep pitch (Hz). Stand-in calibratable constant "
        "until real per-voice resonator pitches land.",
    ),
    "bleep.base_tau": Param(
        key="bleep.base_tau",
        default=0.10,
        min=0.001,
        max=5.0,
        source_file="crates/dsp_core/src/lib.rs",
        source_symbol="Bleep::BASE_TAU",
        description="Placeholder bleep decay time-constant (s, to ~1/e).",
    ),
    "bleep.base_amp": Param(
        key="bleep.base_amp",
        default=0.6,
        min=0.0,
        max=1.0,
        source_file="crates/dsp_core/src/lib.rs",
        source_symbol="Bleep::BASE_AMP",
        description="Placeholder bleep pre-VCA amplitude (linear).",
    ),
}


class ParamScopeError(ValueError):
    """Raised when an edit targets a key outside the calibratable whitelist."""


def overrides_path() -> Path:
    return WORK_DIR / "param_overrides.json"


def _load_overrides() -> dict[str, float]:
    p = overrides_path()
    if not p.exists():
        return {}
    try:
        return {k: float(v) for k, v in json.loads(p.read_text()).items()}
    except (json.JSONDecodeError, ValueError, OSError):
        return {}


def _save_overrides(overrides: dict[str, float]) -> None:
    ensure_work_dir()
    overrides_path().write_text(json.dumps(overrides, indent=2, sort_keys=True) + "\n")


def list_params() -> list[dict]:
    """Every calibratable constant with its bounds and active value."""
    overrides = _load_overrides()
    out = []
    for p in REGISTRY.values():
        out.append(
            {
                "key": p.key,
                "default": p.default,
                "value": overrides.get(p.key, p.default),
                "min": p.min,
                "max": p.max,
                "source_file": p.source_file,
                "source_symbol": p.source_symbol,
                "description": p.description,
            }
        )
    return out


def value(key: str) -> float:
    """Active value for a key (override if applied, else the registered default)."""
    if key not in REGISTRY:
        raise ParamScopeError(f"unknown / out-of-scope param {key!r}")
    return _load_overrides().get(key, REGISTRY[key].default)


def propose_param_edit(key: str, new_value: float) -> dict:
    """Validate (but do NOT persist) an edit. Returns a structured proposal.

    Refuses out-of-scope keys and out-of-bounds values so the loop sees the
    refusal before it writes anything (`apply_param_edit` re-checks).
    """
    if key not in REGISTRY:
        return {
            "key": key,
            "accepted": False,
            "reason": "out-of-scope: not a whitelisted calibratable constant",
            "whitelist": sorted(REGISTRY.keys()),
        }
    p = REGISTRY[key]
    new_value = float(new_value)
    in_bounds = p.min <= new_value <= p.max
    current = value(key)
    return {
        "key": key,
        "accepted": in_bounds,
        "reason": None if in_bounds else f"out-of-bounds: must be in [{p.min}, {p.max}]",
        "current_value": current,
        "proposed_value": new_value,
        "delta": new_value - current,
        "min": p.min,
        "max": p.max,
        "source_file": p.source_file,
        "source_symbol": p.source_symbol,
    }


def apply_param_edit(key: str, new_value: float) -> dict:
    """Persist an in-scope, in-bounds edit to the harness override file.

    Raises :class:`ParamScopeError` for an out-of-scope key and ``ValueError``
    for an out-of-bounds value — the loop must `propose_param_edit` first.
    """
    if key not in REGISTRY:
        raise ParamScopeError(
            f"refused: {key!r} is not a whitelisted calibratable constant; "
            f"valid keys: {sorted(REGISTRY.keys())}"
        )
    p = REGISTRY[key]
    new_value = float(new_value)
    if not (p.min <= new_value <= p.max):
        raise ValueError(
            f"refused: {key}={new_value} out of bounds [{p.min}, {p.max}]"
        )
    overrides = _load_overrides()
    previous = overrides.get(key, p.default)
    overrides[key] = new_value
    _save_overrides(overrides)
    return {
        "key": key,
        "applied": True,
        "previous_value": previous,
        "value": new_value,
        "source_file": p.source_file,
        "source_symbol": p.source_symbol,
        "overrides_file": str(overrides_path()),
    }


def reset_overrides() -> None:
    """Drop all applied overrides (back to registered defaults)."""
    p = overrides_path()
    if p.exists():
        p.unlink()
