"""Synthetic-truth unit tests for the core extractors.

Every signal is generated in-test with numpy so the ground truth is known
exactly — the whole point of keeping analysis in a different codebase from the
DSP. No render binary is needed for these.
"""

import numpy as np
import pytest

from analysis import analyze_array
from analysis import extractors as fx
from analysis.schema import ALL_FEATURES, SCHEMA_VERSION

SR = 48000.0


def sine(freq, dur, sr=SR, amp=1.0, phase=0.0):
    t = np.arange(int(dur * sr)) / sr
    return amp * np.sin(2 * np.pi * freq * t + phase)


def decaying_sine(freq, tau, dur, sr=SR, amp=1.0):
    t = np.arange(int(dur * sr)) / sr
    return amp * np.exp(-t / tau) * np.sin(2 * np.pi * freq * t)


# ---------------------------------------------------------------------------
# pure sine: centroid and pitch ~ 880 Hz
# ---------------------------------------------------------------------------

def test_pure_sine_centroid():
    x = sine(880.0, 1.0)
    c = fx.spectral_centroid(x, SR)
    assert c == pytest.approx(880.0, abs=5.0)


def test_pure_sine_pitch():
    x = sine(880.0, 1.0)
    f0 = fx.fundamental(x, SR)
    assert f0 == pytest.approx(880.0, rel=0.01)


def test_pure_sine_amplitude_features():
    x = sine(880.0, 0.5, amp=0.6)
    assert fx.peak(x) == pytest.approx(0.6, abs=1e-3)
    assert fx.rms(x) == pytest.approx(0.6 / np.sqrt(2), abs=1e-3)
    assert abs(fx.dc_offset(x)) < 1e-3


def test_pure_sine_top_partial():
    x = sine(880.0, 1.0)
    parts = fx.partial_track(x, SR)
    assert parts, "expected at least one partial"
    assert parts[0]["freq"] == pytest.approx(880.0, abs=5.0)


# ---------------------------------------------------------------------------
# exponentially decaying sine: measured tau ~ known tau
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("tau", [0.05, 0.10, 0.20])
def test_decay_tau_e(tau):
    x = decaying_sine(440.0, tau, dur=max(1.0, 8 * tau))
    d = fx.decay_time(x, SR)
    # 1/e crossing should land near the true tau (envelope is e^{-t/tau}).
    assert d["tau_e"] == pytest.approx(tau, rel=0.15)


@pytest.mark.parametrize("tau", [0.05, 0.10, 0.20])
def test_decay_slope_fit(tau):
    x = decaying_sine(440.0, tau, dur=max(1.0, 8 * tau))
    d = fx.decay_time(x, SR)
    # log-envelope fit: expected slope -8.686/tau dB/s.
    expected_slope = -8.685889638065035 / tau
    assert d["slope_db_per_s"] == pytest.approx(expected_slope, rel=0.15)
    assert d["tau_fit"] == pytest.approx(tau, rel=0.15)


def test_envelope_peak_near_start():
    tau = 0.1
    x = decaying_sine(440.0, tau, dur=1.0)
    env = fx.energy_envelope(x, SR)
    # a decaying tone peaks within the first few ms.
    assert env["peak_time"] < 0.02
    # The analytic (Hilbert) envelope overshoots a little at the sharp onset,
    # so allow a modest band around the true peak amplitude of 1.0.
    assert env["peak"] == pytest.approx(1.0, abs=0.12)


# ---------------------------------------------------------------------------
# edge cases: DC and Nyquist (alternating +/-1)
# ---------------------------------------------------------------------------

def test_dc_signal():
    x = np.full(int(0.5 * SR), 0.7)
    assert fx.dc_offset(x) == pytest.approx(0.7, abs=1e-6)
    assert fx.peak(x) == pytest.approx(0.7, abs=1e-6)
    # Windowed FFT of a constant -> energy at/near DC; centroid stays low/sane.
    c = fx.spectral_centroid(x, SR)
    assert 0.0 <= c < 50.0
    # no crash on pitch / partials / alias for a pure DC signal
    fx.fundamental(x, SR)
    fx.partial_track(x, SR)
    a = fx.alias_floor(x, SR)
    assert 0.0 <= a["alias_energy_ratio"] <= 1.0
    d = fx.decay_time(x, SR)
    assert np.isfinite(d["tau_e"])


def test_nyquist_alternating():
    n = int(0.5 * SR)
    x = np.empty(n)
    x[0::2] = 1.0
    x[1::2] = -1.0
    # All energy is at Nyquist -> centroid near sr/2, alias band catches it.
    c = fx.spectral_centroid(x, SR)
    assert c == pytest.approx(SR / 2.0, rel=0.02)
    a = fx.alias_floor(x, SR)
    assert a["alias_energy_ratio"] > 0.5
    assert abs(fx.dc_offset(x)) < 1e-6
    # no crash / no NaN
    f0 = fx.fundamental(x, SR)
    assert np.isfinite(f0)
    parts = fx.partial_track(x, SR)
    for p in parts:
        assert np.isfinite(p["freq"]) and np.isfinite(p["mag"])


def test_empty_and_silence_no_crash():
    for x in (np.zeros(0), np.zeros(1000)):
        m = analyze_array(x, SR)
        for v in _flatten_floats(m["features"]):
            assert np.isfinite(v)


# ---------------------------------------------------------------------------
# analyze() dispatch + schema
# ---------------------------------------------------------------------------

def test_analyze_array_schema_shape():
    x = decaying_sine(880.0, 0.1, dur=0.5)
    m = analyze_array(x, SR)
    assert m["schema_version"] == SCHEMA_VERSION
    assert m["sample_rate"] == SR
    assert m["n_samples"] == x.size
    assert m["duration_s"] == pytest.approx(x.size / SR)
    assert set(m["features"].keys()) == set(ALL_FEATURES)


def test_analyze_array_feature_subset():
    x = sine(880.0, 0.2)
    m = analyze_array(x, SR, feature_set=["spectral_centroid", "decay"])
    assert set(m["features"].keys()) == {"spectral_centroid", "decay"}


def test_analyze_array_unknown_feature_raises():
    x = sine(880.0, 0.1)
    with pytest.raises(ValueError):
        analyze_array(x, SR, feature_set=["bogus"])


def test_metrics_json_serializable():
    import json

    x = decaying_sine(880.0, 0.1, dur=0.5)
    m = analyze_array(x, SR)
    # must round-trip through JSON without custom encoders (no numpy scalars).
    s = json.dumps(m)
    assert json.loads(s)["schema_version"] == SCHEMA_VERSION


def _flatten_floats(obj):
    if isinstance(obj, dict):
        for v in obj.values():
            yield from _flatten_floats(v)
    elif isinstance(obj, (list, tuple)):
        for v in obj:
            yield from _flatten_floats(v)
    elif isinstance(obj, (int, float)):
        yield float(obj)
