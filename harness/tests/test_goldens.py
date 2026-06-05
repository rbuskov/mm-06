"""Golden store + `--bless` round-trip against the bleep (§9).

Exercises the full workflow on a still-bleep voice (`sd`, robust to BD becoming a
real voice):

  * the committed `sd` golden re-renders with **no drift**;
  * perturbing a calibratable constant (`bleep.freq_hz`, via the params override
    mechanism) makes the comparator **flag drift** and name the moved metric;
  * `bless_golden` emits a readable **before/after diff** and refreshes the
    golden so a subsequent compare is clean again.

The create/refresh assertions run against a **temporary** golden name so the
committed `harness/goldens/sd/golden.json` baseline is never rewritten by the
test; overrides are always restored, so the suite stays hermetic.
"""

from __future__ import annotations

import shutil

import pytest

from harness import goldens, params, tools


@pytest.fixture(autouse=True)
def _hermetic_overrides():
    """Every test starts and ends with no param overrides applied."""
    params.reset_overrides()
    try:
        yield
    finally:
        params.reset_overrides()


@pytest.fixture
def temp_golden():
    """A throwaway golden name; its dir is removed before and after the test."""
    name = "_test_sd_golden"
    shutil.rmtree(goldens.golden_dir(name), ignore_errors=True)
    try:
        yield name
    finally:
        shutil.rmtree(goldens.golden_dir(name), ignore_errors=True)


# --- the committed baseline re-renders clean ---------------------------------


def test_committed_sd_golden_exists():
    assert goldens.golden_exists("sd"), "sd golden baseline must be committed"
    assert "sd" in goldens.list_goldens()


def test_committed_golden_no_drift_on_unchanged_render():
    report = tools.compare_to_golden("sd")
    assert report["drift"] is False, report
    assert report["hash_match"] is True
    assert report["drifted_metrics"] == []
    # the metric vector round-trips exactly within tolerance
    assert all(row["within_tol"] for row in report["deltas"])


# --- perturbing a calibratable constant flags drift --------------------------


def test_perturbed_constant_flags_drift_and_names_metric():
    # baseline is clean
    assert tools.compare_to_golden("sd")["drift"] is False
    # shadow the bleep pitch via the whitelisted param-override mechanism
    applied = tools.apply_param_edit("bleep.freq_hz", 990.0)
    assert applied["applied"] is True

    report = tools.compare_to_golden("sd")
    assert report["drift"] is True, report
    # the comparator identifies which metric moved: pitch shows up in fundamental
    assert "fundamental" in report["drifted_metrics"]
    # and the render-hash mismatch is flagged too
    assert report["hash_match"] is False
    fund = next(r for r in report["deltas"] if r["metric"] == "fundamental")
    assert fund["delta"] is not None and abs(fund["delta"]) > fund["tol"]
    # (override restored by the autouse fixture → committed baseline stays clean)


def test_baseline_clean_again_after_override_restored():
    tools.apply_param_edit("bleep.freq_hz", 990.0)
    assert tools.compare_to_golden("sd")["drift"] is True
    params.reset_overrides()
    assert tools.compare_to_golden("sd")["drift"] is False


# --- bless: create, diff, refresh (round-trip) -------------------------------


def test_bless_creates_then_refreshes_with_diff(temp_golden):
    name = temp_golden

    # 1. create the baseline (first bless)
    created = tools.bless_golden("sd", "establish baseline", name=name)
    assert created["created"] is True
    assert created["refreshed"] is False
    assert goldens.golden_exists(name)
    # re-render is clean immediately after blessing
    assert tools.compare_to_golden(name)["drift"] is False

    # 2. perturb a constant → drift
    tools.apply_param_edit("bleep.freq_hz", 990.0)
    drifted = tools.compare_to_golden(name)
    assert drifted["drift"] is True
    assert "fundamental" in drifted["drifted_metrics"]

    # 3. bless again → readable before/after diff + refresh
    refreshed = tools.bless_golden("sd", "accept the new pitch", name=name)
    assert refreshed["refreshed"] is True
    assert refreshed["hash_changed"] is True
    # the diff shows the metric vector moving (one human glance, §9)
    fund = next(r for r in refreshed["diff"] if r["metric"] == "fundamental")
    assert fund["before"] is not None and fund["after"] is not None
    assert abs(fund["after"] - fund["before"]) > 50.0  # ~880 → ~990 Hz

    # 4. after the refresh a compare is clean again
    assert tools.compare_to_golden(name)["drift"] is False


def test_bless_records_reason_and_regime_never_a(temp_golden):
    name = temp_golden
    result = tools.bless_golden("sd", "freeze for B/C drift guard", name=name)
    assert result["regime"] == "B/C"  # never regime A (§9)
    doc = goldens.load_golden(name)
    assert doc["blessed"]["reason"] == "freeze for B/C drift guard"
    assert doc["regime"] != "A"
    assert "metric_vector" in doc and "render_hash" in doc


# --- the comparator never claims to guard regime A ---------------------------


def test_compare_report_disclaims_regime_a():
    report = tools.compare_to_golden("sd")
    assert "regime-A" in report["note"]
