"""Bass-drum regime-B calibration loop, end to end (§6 BD row, §11).

The BD is the first *real* regime-B voice: two stationary Twin-T resonators
(OSC1 ≈60 Hz Q≈7 body, OSC2 ≈130 Hz Q≈3 click) pinged in phase by one shared
edge. This suite drives the sterile `render_voice("bd") → analyze →
compare_to_targets` loop (variance_depth=0, fixed seed) and asserts every §6 BD
target lands within tolerance:

  * dual-partial pitch ≈ 60 Hz & 130 Hz;
  * Qs ≈ 7 & 3 (OSC1 rings longer than OSC2 — a decay-ordering ratio);
  * OSC2 at a lower relative level than OSC1;
  * in-phase POSITIVE attack-edge polarity;
  * beat presence (the two close partials beat as they diverge);
  * pitch-track flatness — NO downward sweep (the most important BD guard).

It also confirms the BD golden re-renders with no drift. The render shells out to
the built `render` binary, so these are marked `slow`.
"""

from __future__ import annotations

import pytest

from harness import goldens, tools


@pytest.fixture(scope="module")
def bd_render():
    """One sterile BD render reused across the target checks."""
    wav_id = tools.render_voice("bd")
    metrics = tools.analyze(wav_id, feature_set="all")
    return wav_id, metrics


def test_bd_targets_all_within_tolerance(bd_render):
    wav_id, metrics = bd_render
    result = tools.compare_to_targets(metrics, "bd", wav_id=wav_id)
    assert result["voice"] == "bd"
    assert result["regime"] == "B"
    # Every BD §6 target must land within tolerance on the as-built (≈on-target)
    # voice — a readable per-target failure message if any does not.
    failures = [d for d in result["deltas"] if not d["within_tol"]]
    assert not failures, "BD targets out of tolerance: " + "; ".join(
        f"{d['feature']} measured={d['measured']:.4f} target={d['target']} "
        f"({d['kind']})"
        for d in failures
    )
    assert result["all_within_tol"] is True


def test_bd_target_set_covers_every_spec_item(bd_render):
    wav_id, metrics = bd_render
    result = tools.compare_to_targets(metrics, "bd", wav_id=wav_id)
    keys = {d["feature"] for d in result["deltas"]}
    assert {
        "osc1_freq",
        "osc2_freq",
        "osc2_relative_level",
        "decay_ordering",
        "attack_polarity",
        "beat_reswells",
        "pitch_sweep_drift",
    } <= keys


def test_bd_dual_partial_pitches(bd_render):
    wav_id, metrics = bd_render
    deltas = {
        d["feature"]: d
        for d in tools.compare_to_targets(metrics, "bd", wav_id=wav_id)["deltas"]
    }
    # ≈60 Hz body and ≈130 Hz click, both in-band.
    assert abs(deltas["osc1_freq"]["measured"] - 60.0) <= 5.0
    assert abs(deltas["osc2_freq"]["measured"] - 130.0) <= 8.0


def test_bd_osc2_lower_level_and_shorter(bd_render):
    wav_id, metrics = bd_render
    deltas = {
        d["feature"]: d
        for d in tools.compare_to_targets(metrics, "bd", wav_id=wav_id)["deltas"]
    }
    # OSC2 summed lower than OSC1 ...
    assert deltas["osc2_relative_level"]["measured"] < 1.0
    # ... and rings shorter (OSC1 Q≈7 outlasts OSC2 Q≈3).
    assert deltas["decay_ordering"]["measured"] >= 3.0


def test_bd_in_phase_positive_attack(bd_render):
    wav_id, metrics = bd_render
    deltas = {
        d["feature"]: d
        for d in tools.compare_to_targets(metrics, "bd", wav_id=wav_id)["deltas"]
    }
    # Shared-edge ping → in phase → a solid POSITIVE front transient.
    assert deltas["attack_polarity"]["measured"] >= 0.3


def test_bd_beats_and_does_not_sweep(bd_render):
    wav_id, metrics = bd_render
    deltas = {
        d["feature"]: d
        for d in tools.compare_to_targets(metrics, "bd", wav_id=wav_id)["deltas"]
    }
    # The two close partials beat ...
    assert deltas["beat_reswells"]["measured"] >= 1.0
    # ... but the pitch track stays flat — NO downward sweep (the 808 guard).
    assert deltas["pitch_sweep_drift"]["measured"] < 0.10


def test_bd_requires_signal_for_targets(bd_render):
    _, metrics = bd_render
    # BD targets are signal-domain; without the signal the comparator refuses.
    with pytest.raises(ValueError):
        tools.compare_to_targets(metrics, "bd")


def test_bd_golden_no_drift():
    # The committed BD golden re-renders bit-for-bit with the same metrics.
    report = goldens.compare_to_golden("bd")
    assert report["drift"] is False, report
    assert report["hash_match"] is True
    assert report["drifted_metrics"] == []
