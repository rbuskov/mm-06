"""Stable, documented metrics-JSON schema for the MM-06 analysis layer.

Bump :data:`SCHEMA_VERSION` whenever a field's *name*, *units*, or *meaning*
changes (adding a new optional field under an existing feature key is backward
compatible and does not require a bump). The full field reference lives in
``analysis/README.md``; this docstring is the machine-readable companion.

Top-level metrics dict shape::

    {
      "schema_version": 1,            # int, this schema's version
      "source": "path/to/file.wav",  # str, the analyzed file (or "<array>")
      "sample_rate": 48000.0,        # float, Hz
      "n_samples": 24000,            # int, number of mono samples analyzed
      "duration_s": 0.5,             # float, seconds
      "features": { ... }            # one key per requested feature set (below)
    }

Feature keys (all frequencies in Hz, all levels linear full-scale unless the
field name ends in ``_db``):

    peak            -> float
    rms             -> float
    dc_offset       -> float                      (signed mean)
    spectral_centroid -> float                     (power-weighted, Hz)
    energy_envelope -> {envelope:[float], hop:int, peak:float, peak_time:float}
    decay           -> {tau_e:float, slope_db_per_s:float, tau_fit:float}
    fundamental     -> float                       (Hz, 0.0 if none found)
    partials        -> [{freq:float, mag:float, mag_db:float}, ...]
    alias           -> {alias_band_hz:[lo,hi], alias_energy_ratio:float,
                        alias_floor_db:float}
"""

from __future__ import annotations

SCHEMA_VERSION = 1

# Canonical ordered set of feature names. "all" expands to this list.
ALL_FEATURES = (
    "peak",
    "rms",
    "dc_offset",
    "spectral_centroid",
    "energy_envelope",
    "decay",
    "fundamental",
    "partials",
    "alias",
)
