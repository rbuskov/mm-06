"""MM-06 analysis layer — render WAV -> metrics JSON.

The home of every feature extractor regimes B and C consume, kept in a
different language from the Rust ``dsp_core`` on purpose so a bug isn't mirrored
in both the synth and its own test (calibration-and-testing-strategy.md §2.2).

Public API::

    from analysis import analyze, analyze_array
    metrics = analyze("kick.wav")              # all features
    metrics = analyze("kick.wav", ["decay"])   # a subset

CLI::

    python -m analysis kick.wav --features centroid,decay
    analyze kick.wav            # console script (same thing)
"""

from .core import analyze, analyze_array, load_wav
from .schema import ALL_FEATURES, SCHEMA_VERSION

__all__ = [
    "analyze",
    "analyze_array",
    "load_wav",
    "ALL_FEATURES",
    "SCHEMA_VERSION",
]

__version__ = "0.1.0"
