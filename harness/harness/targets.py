"""Regime-B target table + regime-C reference shape distance (plumbing).

Per the task scope, these are the **trivial/placeholder** ends of the loop: a
small target set and a reference-shape distance, wired end to end so the calling
agent gets structured deltas today. Per-voice targets and clips fill in as real
voices land — the *shapes* of the returned structures stay stable.

* `compare_to_targets(metrics, voice)` — regime B (§6): signed deltas from
  measured features to the spec target numbers.
* `compare_to_reference(wav_id, voice)` — regime C (§7): a normalized,
  onset-aligned, multi-resolution log-STFT **shape distance** to
  `references/<voice>.wav` (peak/phase-independent; never a null test).
"""

from __future__ import annotations

import numpy as np

from .paths import REFERENCES_DIR

# -----------------------------------------------------------------------------
# Regime B — trivial target set.
# -----------------------------------------------------------------------------
# Each target names a feature path inside the analysis metrics dict and the
# expected value. Today every voice is the placeholder 880 Hz bleep, so the
# trivial target set anchors that; the regime-B §6 table (resonator freqs/Qs,
# decay τs, metallic peaks) slots in per voice here as the voices land.
#
# A target is (feature_key, sub_path, target_value, tol, unit). `sub_path` is a
# dotted path into the feature's value (or None for a scalar feature).
_BLEEP_TARGETS = [
    ("fundamental", None, 880.0, 30.0, "Hz"),
    ("dc_offset", None, 0.0, 1e-3, "linear"),
    ("decay", "tau_e", 0.07, 0.05, "s"),
]

# Per-voice target sets. All voices currently share the bleep target set; this
# is the seam where real regime-B targets land per voice.
TARGETS: dict[str, list] = {
    v: list(_BLEEP_TARGETS) for v in ("bd", "sd", "lt", "ht", "cy", "oh", "ch")
}


def _dig(value, sub_path):
    if sub_path is None:
        return value
    cur = value
    for part in sub_path.split("."):
        cur = cur[part]
    return cur


def compare_to_targets(metrics: dict, voice: str) -> dict:
    """Regime B: signed deltas from measured features to the spec targets.

    `metrics` is an `analysis.analyze(...)` dict. Returns one entry per target
    with measured / target / delta / within_tol so the agent can rank edits.
    """
    voice = voice.lower()
    if voice not in TARGETS:
        raise ValueError(f"unknown voice {voice!r}")
    feats = metrics.get("features", {})
    deltas = []
    for feature_key, sub_path, target, tol, unit in TARGETS[voice]:
        if feature_key not in feats:
            deltas.append(
                {
                    "feature": feature_key,
                    "sub_path": sub_path,
                    "measured": None,
                    "target": target,
                    "delta": None,
                    "within_tol": False,
                    "tol": tol,
                    "unit": unit,
                    "note": "feature not in metrics (request it in analyze)",
                }
            )
            continue
        measured = float(_dig(feats[feature_key], sub_path))
        delta = measured - target
        deltas.append(
            {
                "feature": feature_key,
                "sub_path": sub_path,
                "measured": measured,
                "target": target,
                "delta": delta,
                "within_tol": abs(delta) <= tol,
                "tol": tol,
                "unit": unit,
            }
        )
    return {
        "voice": voice,
        "regime": "B",
        "all_within_tol": all(d["within_tol"] for d in deltas),
        "deltas": deltas,
    }


# -----------------------------------------------------------------------------
# Regime C — reference-clip shape distance (plumbing).
# -----------------------------------------------------------------------------


def reference_path(voice: str):
    return REFERENCES_DIR / f"{voice.lower()}.wav"


def _onset_index(x: np.ndarray, thresh_frac: float = 0.05) -> int:
    """First sample whose |amplitude| exceeds a fraction of the peak."""
    peak = float(np.max(np.abs(x))) if x.size else 0.0
    if peak <= 0.0:
        return 0
    idx = np.argmax(np.abs(x) >= thresh_frac * peak)
    return int(idx)


def _prep(x: np.ndarray) -> np.ndarray:
    """DC-remove, onset-align (trim leading silence), normalize for shape."""
    x = np.asarray(x, dtype=np.float64)
    if x.size == 0:
        return x
    x = x - np.mean(x)
    x = x[_onset_index(x) :]
    peak = float(np.max(np.abs(x))) if x.size else 0.0
    if peak > 0.0:
        x = x / peak
    return x


def _log_stft_mag(x: np.ndarray, n_fft: int) -> np.ndarray:
    """Multi-frame log-magnitude STFT (peak/phase-independent)."""
    if x.size < n_fft:
        x = np.pad(x, (0, n_fft - x.size))
    hop = n_fft // 4
    win = np.hanning(n_fft)
    frames = []
    for start in range(0, x.size - n_fft + 1, hop):
        seg = x[start : start + n_fft] * win
        mag = np.abs(np.fft.rfft(seg))
        frames.append(np.log1p(mag))
    if not frames:
        frames.append(np.log1p(np.abs(np.fft.rfft(x[:n_fft] * win))))
    return np.asarray(frames)


def _shape_distance(a: np.ndarray, b: np.ndarray) -> float:
    """Multi-resolution log-STFT shape distance between two prepped signals."""
    dists = []
    for n_fft in (256, 1024, 4096):
        sa = _log_stft_mag(a, n_fft)
        sb = _log_stft_mag(b, n_fft)
        n = min(sa.shape[0], sb.shape[0])
        if n == 0:
            continue
        diff = sa[:n] - sb[:n]
        dists.append(float(np.sqrt(np.mean(diff * diff))))
    return float(np.mean(dists)) if dists else 0.0


def compare_to_reference(rendered_signal, sr_rendered, voice: str) -> dict:
    """Regime C: shape distance between a rendered signal and the reference clip.

    Both are DC-removed, onset-aligned, and amplitude-normalized first
    (`§7.2`) — the distance is shape-only, never a sample-level null test.
    Returns the scalar distance plus the decomposed centroid/RMS-envelope
    deltas the strategy wants reported *alongside* the scalar (§7.2.5).
    """
    voice = voice.lower()
    ref = reference_path(voice)
    if not ref.exists():
        return {
            "voice": voice,
            "regime": "C",
            "available": False,
            "reason": f"no reference clip at {ref}",
        }

    import soundfile as sf

    ref_x, ref_sr = sf.read(str(ref), dtype="float64", always_2d=False)
    ref_x = np.asarray(ref_x, dtype=np.float64)
    if ref_x.ndim > 1:
        ref_x = ref_x.mean(axis=1)

    a = _prep(np.asarray(rendered_signal, dtype=np.float64))
    b = _prep(ref_x)

    distance = _shape_distance(a, b)

    def _centroid(x, sr):
        if x.size == 0:
            return 0.0
        spec = np.abs(np.fft.rfft(x))
        freqs = np.fft.rfftfreq(x.size, 1.0 / sr)
        total = float(np.sum(spec))
        return float(np.sum(freqs * spec) / total) if total > 0 else 0.0

    cen_r = _centroid(a, sr_rendered)
    cen_ref = _centroid(b, ref_sr)

    return {
        "voice": voice,
        "regime": "C",
        "available": True,
        "reference": str(ref),
        "shape_distance": distance,
        "feature_deltas": {
            "spectral_centroid_hz": {
                "rendered": cen_r,
                "reference": cen_ref,
                "delta": cen_r - cen_ref,
            },
        },
        "note": "shape-only (normalized, onset-aligned); not a null test (§7.2)",
    }
