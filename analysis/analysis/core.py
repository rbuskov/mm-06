"""The :func:`analyze` entry point: WAV (or array) -> metrics dict.

This is the single home of every feature extractor regimes B and C consume
(calibration-and-testing-strategy.md §2.2). It loads a render WAV with
``soundfile`` (the offline ``render`` binary writes a mono 32-bit-float WAV,
format tag 3, which soundfile reads directly) and dispatches to the pure
extractors in :mod:`analysis.extractors`.
"""

from __future__ import annotations

from typing import Iterable, Sequence

import numpy as np

from . import extractors as fx
from .schema import ALL_FEATURES, SCHEMA_VERSION

__all__ = ["analyze", "analyze_array", "load_wav", "ALL_FEATURES", "SCHEMA_VERSION"]


def load_wav(wav_path: str):
    """Load a WAV into ``(mono_float64, sample_rate)``.

    Uses ``soundfile`` (libsndfile) which reads the f32 format-tag-3 WAV the
    ``render`` binary emits. Stereo/multi-channel input is averaged to mono.
    """
    import soundfile as sf

    data, sr = sf.read(wav_path, dtype="float64", always_2d=False)
    data = np.asarray(data, dtype=np.float64)
    if data.ndim > 1:
        data = data.mean(axis=1)
    return data, float(sr)


def _resolve_features(feature_set) -> list:
    """Normalize the requested feature set to a concrete ordered list of names."""
    if feature_set is None or feature_set == "all":
        return list(ALL_FEATURES)
    if isinstance(feature_set, str):
        feature_set = [feature_set]
    names = []
    for f in feature_set:
        if f == "all":
            names.extend(ALL_FEATURES)
        elif f in ALL_FEATURES:
            names.append(f)
        else:
            raise ValueError(
                f"unknown feature {f!r}; valid: {', '.join(ALL_FEATURES)} (or 'all')"
            )
    # de-dup, preserve order
    seen = set()
    ordered = []
    for n in names:
        if n not in seen:
            seen.add(n)
            ordered.append(n)
    return ordered


def _extract(name: str, x: np.ndarray, sr: float):
    if name == "peak":
        return fx.peak(x)
    if name == "rms":
        return fx.rms(x)
    if name == "dc_offset":
        return fx.dc_offset(x)
    if name == "spectral_centroid":
        return fx.spectral_centroid(x, sr)
    if name == "energy_envelope":
        return fx.energy_envelope(x, sr)
    if name == "decay":
        return fx.decay_time(x, sr)
    if name == "fundamental":
        return fx.fundamental(x, sr)
    if name == "partials":
        return fx.partial_track(x, sr)
    if name == "alias":
        return fx.alias_floor(x, sr)
    raise ValueError(f"unhandled feature {name!r}")


def analyze_array(x, sr: float, feature_set=None, source: str = "<array>") -> dict:
    """Analyze an in-memory signal array. Same contract as :func:`analyze`."""
    x = np.asarray(x, dtype=np.float64)
    if x.ndim > 1:
        x = x.mean(axis=1)
    names = _resolve_features(feature_set)
    features = {name: _extract(name, x, sr) for name in names}
    return {
        "schema_version": SCHEMA_VERSION,
        "source": source,
        "sample_rate": float(sr),
        "n_samples": int(x.size),
        "duration_s": float(x.size / sr) if sr > 0 else 0.0,
        "features": features,
    }


def analyze(wav_path: str, feature_set=None) -> dict:
    """Analyze a render WAV and return a JSON-serializable metrics dict.

    Parameters
    ----------
    wav_path:
        Path to a mono float WAV (e.g. produced by ``crates/render``).
    feature_set:
        ``None`` / ``"all"`` for every feature, a single feature name, or an
        iterable of names (see :data:`analysis.schema.ALL_FEATURES`).

    Returns the metrics dict described in :mod:`analysis.schema`.
    """
    x, sr = load_wav(wav_path)
    return analyze_array(x, sr, feature_set=feature_set, source=str(wav_path))
