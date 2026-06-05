"""End-to-end tests of the dsp-harness tool functions (no MCP client needed).

These exercise the real tools against the placeholder "bleep" engine: render a
still-bleep voice (`sd`, robust to BD becoming a real voice), analyze it, run the
regime-A invariants + parity gates, and round-trip the regime-B/C plumbing and
the param write side.

The cargo-backed tests (`run_invariants`, `parity_check`) actually compile and
run the dsp_core suite, so they are slower; they're marked `slow`.
"""

from __future__ import annotations

import pytest

from harness import tools
from harness import params


# --- render → analyze --------------------------------------------------------


def test_render_voice_then_analyze_bleep():
    # sd is still a bleep voice, so this stays valid once BD becomes real.
    wav_id = tools.render_voice("sd")
    metrics = tools.analyze(wav_id, feature_set=["fundamental", "dc_offset", "peak"])
    f = metrics["features"]
    # bleep ≈ 880 Hz
    assert 860.0 <= f["fundamental"] <= 900.0
    # DC ≈ 0
    assert abs(f["dc_offset"]) < 1e-2
    # audible
    assert f["peak"] > 0.05


def test_render_voice_isolation_sets_voice_field():
    # Two different still-bleep voices both render; the isolation path works.
    for v in ("sd", "lt", "ht"):
        wav_id = tools.render_voice(v)
        m = tools.analyze(wav_id, feature_set=["fundamental"])
        assert 860.0 <= m["features"]["fundamental"] <= 900.0


def test_render_voice_accent_decays_longer():
    plain = tools.render_voice("sd", accent=False)
    accented = tools.render_voice("sd", accent=True)
    dp = tools.analyze(plain, feature_set=["decay"])["features"]["decay"]["tau_e"]
    da = tools.analyze(accented, feature_set=["decay"])["features"]["decay"]["tau_e"]
    # accent nudges the bleep's tau up (§4.10 direction check, not magnitude).
    assert da >= dp


def test_render_mix_round_trips():
    mix_id = tools.render_mix({"events": [(0, "sd"), (12000, "sd")]})
    m = tools.analyze(mix_id, feature_set=["peak", "dc_offset"])
    assert m["features"]["peak"] > 0.05
    assert abs(m["features"]["dc_offset"]) < 1e-2


def test_analyze_unknown_wav_id_raises():
    with pytest.raises(FileNotFoundError):
        tools.analyze("does_not_exist_zzz")


# --- regime B / C ------------------------------------------------------------


def test_compare_to_targets_structured_deltas():
    wav_id = tools.render_voice("sd")
    metrics = tools.analyze(wav_id, feature_set=["fundamental", "dc_offset", "decay"])
    result = tools.compare_to_targets(metrics, "sd")
    assert result["regime"] == "B"
    keys = {d["feature"] for d in result["deltas"]}
    assert {"fundamental", "dc_offset", "decay"} <= keys
    fund = next(d for d in result["deltas"] if d["feature"] == "fundamental")
    assert fund["within_tol"]  # bleep hits the 880 Hz trivial target


def test_compare_to_reference_shape_distance():
    wav_id = tools.render_voice("sd")
    result = tools.compare_to_reference(wav_id, "sd")
    assert result["regime"] == "C"
    # references/sd.wav ships in the repo, so the regime-C path is available.
    assert result["available"] is True
    assert isinstance(result["shape_distance"], float)
    assert result["shape_distance"] >= 0.0
    # Decomposed per-feature deltas are reported ALONGSIDE the scalar (§7.2.5);
    # voices without a dedicated suite get the generic band-limited centroid row.
    assert isinstance(result["feature_deltas"], list)
    feats = {r["feature"] for r in result["feature_deltas"]}
    assert "centroid_lp_hz" in feats


# --- write side --------------------------------------------------------------


def test_param_edit_round_trip_on_whitelisted_constant():
    params.reset_overrides()
    try:
        prop = tools.propose_param_edit("bleep.freq_hz", 660.0)
        assert prop["accepted"] is True
        applied = tools.apply_param_edit("bleep.freq_hz", 660.0)
        assert applied["applied"] is True
        assert params.value("bleep.freq_hz") == 660.0
    finally:
        params.reset_overrides()


def test_param_edit_refuses_out_of_scope():
    prop = tools.propose_param_edit("mixer.master_level", 2.0)
    assert prop["accepted"] is False
    assert "out-of-scope" in prop["reason"]
    with pytest.raises(params.ParamScopeError):
        tools.apply_param_edit("mixer.master_level", 2.0)


def test_param_edit_refuses_out_of_bounds():
    prop = tools.propose_param_edit("bleep.freq_hz", 999999.0)
    assert prop["accepted"] is False
    assert "out-of-bounds" in prop["reason"]
    with pytest.raises(ValueError):
        tools.apply_param_edit("bleep.freq_hz", 999999.0)


def test_list_params_includes_whitelist():
    keys = {p["key"] for p in tools.list_params()}
    assert "bleep.freq_hz" in keys
    assert "bleep.base_tau" in keys


# --- regime A: invariants + parity (cargo-backed, slower) --------------------


@pytest.mark.slow
def test_run_invariants_regime_a_is_green():
    report = tools.run_invariants("regime-a")
    assert report["green"] is True, report
    assert report["passed"] > 0
    assert report["report"] == "GREEN"


@pytest.mark.slow
def test_parity_check_zero_divergence():
    result = tools.parity_check({"events": [(0, "sd")]})
    assert result["max_divergence"] == 0, result
    assert result["native_streaming_gate"] == "PASS"
