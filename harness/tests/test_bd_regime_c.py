"""Bass-drum regime-C reference-clip shape match (§7, §11; spec §11).

Regime C anchors the BD default operating point to `references/bd.wav` on
**shape only** — never absolute level, accent, or per-hit variance (§7.1), and
never a sample-level null test (§11.8). The BD is already regime-B-on-target, so
the expectation is small/zero shape moves: this suite asserts the decomposed
per-feature shape deltas land within tolerance on the sterile (variance_depth=0,
fixed seed) BD render, and that the aggregate multi-resolution log-STFT shape
distance — the §7.2.5 *guide*, reported alongside the deltas — stays under its
sanity ceiling.

The §7.3 BD decomposition checked here:
  * OSC1 ≈60 Hz & OSC2 ≈130 Hz partial centres;
  * the two-partial relative level (OSC2/OSC1 early-window magnitude ratio);
  * band-limited (<2 kHz) brightness — recording hiss above is excluded (§7.1);
  * body-ring 1/e decay shape;
  * the two-resonator beat rate confirmed against the clip;
  * the in-phase POSITIVE attack edge present in both;
  * NO downward pitch sweep in either (the 808 guard, against the clip).

The render shells out to the built `render` binary, so these are `slow`.
"""

from __future__ import annotations

import pytest

from harness import tools

pytestmark = pytest.mark.slow


@pytest.fixture(scope="module")
def bd_reference():
    """One sterile BD render compared to references/bd.wav on shape."""
    wav_id = tools.render_voice("bd")
    return tools.compare_to_reference(wav_id, "bd")


def test_bd_reference_available(bd_reference):
    # references/bd.wav ships in the repo — regime C fires.
    assert bd_reference["voice"] == "bd"
    assert bd_reference["regime"] == "C"
    assert bd_reference["available"] is True


def test_bd_shape_distance_under_ceiling(bd_reference):
    # The aggregate log-STFT distance is a GUIDE (not an objective to minimise
    # blindly, §7.2.5); assert it stays under the sanity ceiling and report the
    # per-resolution split so a single off resolution is visible.
    dist = bd_reference["shape_distance"]
    ceiling = bd_reference["shape_distance_ceiling"]
    assert isinstance(dist, float) and dist >= 0.0
    assert ceiling is not None
    assert dist <= ceiling, (
        f"BD shape distance {dist:.4f} exceeds ceiling {ceiling} "
        f"(per-resolution {bd_reference['shape_distance_per_resolution']})"
    )


def test_bd_decomposed_shape_deltas_within_tolerance(bd_reference):
    # Every §7.3 decomposed shape delta must land within tolerance on the
    # as-built (≈on-target) BD — a readable per-feature failure if any does not.
    rows = bd_reference["feature_deltas"]
    failures = [r for r in rows if not r["within_tol"]]
    assert not failures, "BD regime-C shape deltas out of tolerance: " + "; ".join(
        f"{r['feature']} rendered={r['rendered']:.4f} reference={r['reference']:.4f} "
        f"(tol {r['tol']}, {r['kind']})"
        for r in failures
    )
    assert bd_reference["all_within_tol"] is True


def test_bd_reference_covers_every_7_3_feature(bd_reference):
    keys = {r["feature"] for r in bd_reference["feature_deltas"]}
    assert {
        "osc1_freq",
        "osc2_freq",
        "osc2_relative_level",
        "centroid_lp_hz",
        "decay_tau_e_s",
        "beat_rate_hz",
        "attack_polarity",
        "pitch_sweep_drift",
    } <= keys


def test_bd_two_partials_match_clip(bd_reference):
    rows = {r["feature"]: r for r in bd_reference["feature_deltas"]}
    # ≈60 Hz body and ≈130 Hz click resolved in both render and clip.
    assert abs(rows["osc1_freq"]["delta"]) <= rows["osc1_freq"]["tol"]
    assert abs(rows["osc2_freq"]["delta"]) <= rows["osc2_freq"]["tol"]
    # OSC2 summed lower than OSC1 in BOTH (the two-partial balance, not loudness).
    assert rows["osc2_relative_level"]["rendered"] < 1.0
    assert rows["osc2_relative_level"]["reference"] < 1.0


def test_bd_in_phase_attack_and_beat_against_clip(bd_reference):
    rows = {r["feature"]: r for r in bd_reference["feature_deltas"]}
    # Shared-edge ping → in-phase POSITIVE front edge in both.
    assert rows["attack_polarity"]["rendered"] > 0.0
    assert rows["attack_polarity"]["reference"] > 0.0
    assert rows["attack_polarity"]["within_tol"] is True
    # The two close partials beat — and at the SAME rate as the clip.
    assert rows["beat_rate_hz"]["within_tol"] is True
    assert rows["beat_rate_hz"]["rendered"] > 0.0


def test_bd_no_downward_sweep_in_either(bd_reference):
    # THE 808 guard, against the clip: neither render nor reference glides.
    sweep = {r["feature"]: r for r in bd_reference["feature_deltas"]}["pitch_sweep_drift"]
    assert sweep["rendered"] < sweep["tol"]
    assert sweep["reference"] < sweep["tol"]
    assert sweep["within_tol"] is True


def test_bd_shape_match_does_not_compare_loudness(bd_reference):
    # Regime C is shape-only: nothing in the decomposition is an absolute-level,
    # accent, or variance delta (§7.1 — those stay best-guess). The reported
    # features are all relative shape / structure.
    keys = {r["feature"] for r in bd_reference["feature_deltas"]}
    forbidden = {"peak", "rms", "absolute_level", "accent", "variance_depth"}
    assert not (keys & forbidden)
