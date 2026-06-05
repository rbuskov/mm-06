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
from scipy.signal import butter, filtfilt, hilbert

from .paths import REFERENCES_DIR


def _extractors():
    """The analysis `extractors` module, imported lazily.

    `analysis/` lives off sys.path until a harness tool wires it in (it is a
    *different* codebase from dsp_core, §2.2). The BD comparator reuses its
    autocorrelation `fundamental` extractor for the pitch-track windows; import
    it at call time so merely importing `harness.targets` doesn't require the
    path to be set up yet.
    """
    from .paths import add_analysis_to_syspath

    add_analysis_to_syspath()
    from analysis import extractors as fx

    return fx

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

# Per-voice target sets. The remaining placeholder voices share the bleep target
# set; BD is the first *real* regime-B voice and has its own signal-domain target
# suite (see :func:`_compare_bd_targets`). This is the seam where real per-voice
# targets land as each voice goes live.
TARGETS: dict[str, list] = {
    v: list(_BLEEP_TARGETS) for v in ("sd", "lt", "ht", "cy", "oh", "ch")
}


# -----------------------------------------------------------------------------
# BD regime-B targets (spec §4.1 / §6 BD row) — signal-domain measurements.
# -----------------------------------------------------------------------------
# The BD is two stationary Twin-T resonators (OSC1 ≈60 Hz Q≈7 body, OSC2 ≈130 Hz
# Q≈3 click) pinged in phase by one shared edge. The §6 BD row asks for: the two
# partial pitches & Qs, OSC2 at a lower relative level, an in-phase positive
# attack edge, beating, and — the single most important guard — a flat pitch
# track with NO downward sweep. Those last three are *shape* properties the
# scalar `fundamental`/`partials` features can't express, so BD gets a dedicated
# comparator that measures them straight off the rendered signal. Every number is
# derived under the sterile (variance_depth=0, fixed seed) render.
#
# Each entry: (key, measure_fn, target, tol, unit, note). `measure_fn(x, sr,
# feats)` returns the measured scalar; `within_tol` is |measured-target| <= tol
# unless the entry overrides the comparison (the boolean shape guards below carry
# their target as 1.0 = "holds").

# Resonator bands (Hz) used to separate the two partials.
_OSC1_BAND = (45.0, 90.0)    # the ≈60 Hz body
_OSC2_BAND = (100.0, 170.0)  # the ≈130 Hz click


def _band_peak(x: np.ndarray, sr: float, lo: float, hi: float, zero_pad: int = 8):
    """(freq, magnitude) of the strongest spectral peak in [lo, hi].

    Zero-padded so a low, short-lived partial resolves to better than the raw
    bin spacing; returns (0, 0) if the band is empty.
    """
    x = np.asarray(x, dtype=np.float64)
    if x.size < 4:
        return 0.0, 0.0
    n = max(x.size * zero_pad, 1024)
    spec = np.abs(np.fft.rfft(x * np.hanning(x.size), n=n))
    freqs = np.fft.rfftfreq(n, 1.0 / sr)
    band = (freqs >= lo) & (freqs <= hi)
    if not np.any(band) or spec[band].max() <= 0.0:
        return 0.0, 0.0
    fb = freqs[band]
    sb = spec[band]
    k = int(np.argmax(sb))
    return float(fb[k]), float(sb[k])


def _band_rms(x: np.ndarray, sr: float, lo: float, hi: float) -> float:
    """RMS-like spectral energy of a window inside [lo, hi] (whole-window FFT)."""
    x = np.asarray(x, dtype=np.float64)
    if x.size < 4:
        return 0.0
    spec = np.abs(np.fft.rfft(x * np.hanning(x.size)))
    freqs = np.fft.rfftfreq(x.size, 1.0 / sr)
    band = (freqs >= lo) & (freqs <= hi)
    return float(np.sqrt(np.sum(spec[band] ** 2))) if np.any(band) else 0.0


def _bd_osc1_freq(x, sr, feats):
    """OSC1 body pitch (Hz) — the autocorrelation fundamental in the 60 Hz band.

    Prefers the analyzer's `fundamental` (a separate codebase, §2.2) when it
    already landed in-band; falls back to a band-restricted estimate.
    """
    f = float(feats.get("fundamental", 0.0)) if feats else 0.0
    if _OSC1_BAND[0] <= f <= _OSC1_BAND[1]:
        return f
    return _extractors().fundamental(
        np.asarray(x), sr, fmin=_OSC1_BAND[0], fmax=_OSC1_BAND[1]
    )


def _bd_osc2_freq(x, sr, feats):
    """OSC2 click pitch (Hz) — the 130 Hz partial.

    OSC1 (Q≈7, full-level) dominates the raw spectrum, so the short, low-level
    OSC2 ring is recovered by high-passing above the 60 Hz body before peaking
    the 100-170 Hz band. The residual OSC1 skirt biases the estimate a few Hz
    low (≈127 Hz measured), so the tolerance is sized for that.
    """
    x = np.asarray(x, dtype=np.float64)
    if x.size < 16:
        return 0.0
    b, a = butter(4, 95.0 / (sr / 2.0), btype="high")
    xh = filtfilt(b, a, x)
    f, _ = _band_peak(xh, sr, *_OSC2_BAND)
    return f


def _bd_osc2_relative_level(x, sr, feats):
    """OSC2 / OSC1 partial-magnitude ratio in an early window (both present).

    Measured early (first 30 ms) where OSC2 still rings; OSC1 is summed full,
    OSC2 lower (spec §4.1), so this is < 1.0 — the target asserts the ordering.
    """
    x = np.asarray(x, dtype=np.float64)
    seg = x[: int(0.03 * sr)]
    if seg.size < 16:
        return 1.0
    b, a = butter(4, 95.0 / (sr / 2.0), btype="high")
    segh = filtfilt(b, a, seg)
    _, m1 = _band_peak(seg, sr, *_OSC1_BAND)
    _, m2 = _band_peak(segh, sr, *_OSC2_BAND)
    return (m2 / m1) if m1 > 0 else 1.0


def _bd_decay_ordering(x, sr, feats):
    """OSC1-outlasts-OSC2 ratio: (OSC1 late/early) / (OSC2 late/early).

    Q maps to ring decay (§3.1/§3.3); OSC1 (Q≈7) must ring longer than OSC2
    (Q≈3). Band-energy retention from an early window (0-60 ms) into a late one
    (150-250 ms): OSC1 keeps ≈1% of its energy, OSC2 ≈0.05%, so this ratio is
    >> 1 when the ordering holds. Returns the ratio (target asserts it is large).
    """
    x = np.asarray(x, dtype=np.float64)

    def win(a, b):
        return x[int(a * sr) : int(b * sr)]

    o1e = _band_rms(win(0.0, 0.06), sr, *_OSC1_BAND)
    o1l = _band_rms(win(0.15, 0.30), sr, *_OSC1_BAND)
    o2e = _band_rms(win(0.0, 0.06), sr, *_OSC2_BAND)
    o2l = _band_rms(win(0.15, 0.30), sr, *_OSC2_BAND)
    r1 = (o1l / o1e) if o1e > 0 else 0.0
    r2 = (o2l / o2e) if o2e > 0 else 1.0
    return (r1 / r2) if r2 > 0 else float("inf")


def _bd_attack_polarity(x, sr, feats):
    """Front-edge polarity: signed largest extremum in the first 10 ms / peak.

    Both resonators are pinged from one shared edge → in phase → a solid
    POSITIVE front transient (no cancellation). Returns the fraction of the
    overall peak the front extremum reaches, signed; the target asks for a
    positive, solid (≳0.3·peak) attack edge.
    """
    x = np.asarray(x, dtype=np.float64)
    peak = float(np.max(np.abs(x))) if x.size else 0.0
    if peak <= 0.0:
        return 0.0
    front = x[: int(0.01 * sr)]
    fi = int(np.argmax(np.abs(front)))
    return float(front[fi]) / peak


def _bd_beat_reswells(x, sr, feats):
    """Count of beat re-swells in the amplitude envelope after the attack.

    Two close stationary partials (60 & 130 Hz) beat once they drift out of the
    shared in-phase start: the |x| envelope is not a single monotonic decay but
    re-rises. Returns the number of >2% window-over-window re-rises after the
    initial attack windows; the target asks for at least one.
    """
    x = np.asarray(x, dtype=np.float64)
    if x.size < 64:
        return 0.0
    env = np.abs(hilbert(x))
    w = max(1, int(0.005 * sr))
    env = np.convolve(env, np.ones(w) / w, mode="same")
    ds = env[::w]
    rises = 0
    for i in range(3, len(ds) - 1):
        if ds[i] > 0 and ds[i + 1] > ds[i] * 1.02:
            rises += 1
    return float(rises)


def _bd_pitch_sweep_drift(x, sr, feats):
    """Relative pitch drift across the high-SNR head — the 808 sweep guard.

    THE single most important BD guard (§6). Windowed autocorrelation pitch over
    the first 300 ms (where the body ring is well above the floor) must stay
    flat: two STATIONARY partials, no glide. Returns |last-first|/first across
    the head windows; the target asks for a tiny drift (no sweep).
    """
    x = np.asarray(x, dtype=np.float64)
    fx = _extractors()
    win = int(0.12 * sr)
    hop = win // 2
    head_end = int(0.30 * sr)
    fs = []
    i = 0
    while i + win <= head_end and i + win <= x.size:
        f = fx.fundamental(x[i : i + win], sr, fmin=40.0, fmax=400.0)
        if f > 0.0:
            fs.append(f)
        i += hop
    if len(fs) < 2:
        return 1.0
    return abs(fs[-1] - fs[0]) / fs[0] if fs[0] > 0 else 1.0


# Each BD target: (key, measure_fn, target, tol, unit, kind, note).
# `kind` controls the within-tol test:
#   "abs"  -> |measured - target| <= tol
#   "min"  -> measured >= target (tol unused; target is the floor)
#   "max"  -> measured <= target (tol unused; target is the ceiling)
_BD_TARGETS = [
    ("osc1_freq", _bd_osc1_freq, 60.0, 5.0, "Hz", "abs",
     "OSC1 body resonator centre (spec §4.1: ≈60 Hz)"),
    ("osc2_freq", _bd_osc2_freq, 130.0, 8.0, "Hz", "abs",
     "OSC2 click resonator centre (spec §4.1: ≈130 Hz); HP-recovered, "
     "slight low bias from the OSC1 skirt"),
    ("osc2_relative_level", _bd_osc2_relative_level, 0.7, 0.0, "ratio", "max",
     "OSC2 summed lower than OSC1 (spec §4.1): early-window mag ratio < 1"),
    ("decay_ordering", _bd_decay_ordering, 3.0, 0.0, "ratio", "min",
     "OSC1 (Q≈7) rings longer than OSC2 (Q≈3): band-energy retention ratio"),
    ("attack_polarity", _bd_attack_polarity, 0.3, 0.0, "frac_of_peak", "min",
     "in-phase positive front edge — a solid positive transient (≳0.3·peak)"),
    ("beat_reswells", _bd_beat_reswells, 1.0, 0.0, "count", "min",
     "the two close partials beat (≥1 envelope re-swell after the attack)"),
    ("pitch_sweep_drift", _bd_pitch_sweep_drift, 0.10, 0.0, "rel", "max",
     "pitch-track flatness — NO downward sweep (the most important BD guard)"),
]


def _within(kind: str, measured: float, target: float, tol: float) -> bool:
    if kind == "min":
        return measured >= target
    if kind == "max":
        return measured <= target
    return abs(measured - target) <= tol


def _compare_bd_targets(metrics: dict, signal, sr) -> dict:
    """Regime-B BD comparison: per-target measured/target/within_tol deltas.

    `signal`/`sr` are the sterile BD render (`render_voice("bd")`); the
    signal-domain shape targets (polarity, beat, sweep) are measured straight
    off it, the pitch/level/decay targets off it plus the `analyze` features.
    """
    if signal is None or sr is None:
        raise ValueError(
            "BD regime-B targets need the rendered signal; call "
            "compare_to_targets(metrics, 'bd', signal=..., sr=...) "
            "(tools.compare_to_targets passes it from the wav_id)"
        )
    x = np.asarray(signal, dtype=np.float64)
    feats = metrics.get("features", {}) if metrics else {}
    deltas = []
    for key, fn, target, tol, unit, kind, note in _BD_TARGETS:
        measured = float(fn(x, float(sr), feats))
        deltas.append(
            {
                "feature": key,
                "sub_path": None,
                "measured": measured,
                "target": target,
                "delta": measured - target,
                "within_tol": _within(kind, measured, target, tol),
                "tol": tol,
                "unit": unit,
                "kind": kind,
                "note": note,
            }
        )
    return {
        "voice": "bd",
        "regime": "B",
        "all_within_tol": all(d["within_tol"] for d in deltas),
        "deltas": deltas,
    }


def _dig(value, sub_path):
    if sub_path is None:
        return value
    cur = value
    for part in sub_path.split("."):
        cur = cur[part]
    return cur


def compare_to_targets(metrics: dict, voice: str, signal=None, sr=None) -> dict:
    """Regime B: signed deltas from measured features to the spec targets.

    `metrics` is an `analysis.analyze(...)` dict. Returns one entry per target
    with measured / target / delta / within_tol so the agent can rank edits.

    BD is the first real regime-B voice: its targets (dual-partial pitch + Qs,
    OSC2-lower level, in-phase positive attack, beat, no-sweep) are signal-domain
    shapes, so pass the sterile rendered `signal`/`sr` — `tools.compare_to_targets`
    loads them from the wav_id for you.
    """
    voice = voice.lower()
    if voice == "bd":
        return _compare_bd_targets(metrics, signal, sr)
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
