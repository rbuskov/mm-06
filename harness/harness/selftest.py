"""`python -m harness.selftest` — end-to-end smoke of the tool surface.

Exercises the bleep round-trip (render → analyze → invariants → parity) plus the
regime-B/C plumbing and the param write side, printing a short report. This is
the no-pytest path; the same checks live as assertions in `tests/`.
"""

from __future__ import annotations

import sys

from . import tools


def run() -> int:
    print("dsp-harness selftest")
    print("=" * 60)

    # 1. render a still-bleep voice in isolation (sd: robust to BD becoming real)
    wav_id = tools.render_voice("sd")
    print(f"[render_voice]  sd → {wav_id}")

    # 2. analyze it — expect ~880 Hz bleep, DC ≈ 0
    metrics = tools.analyze(wav_id, feature_set=["fundamental", "dc_offset", "peak"])
    f = metrics["features"]
    print(
        f"[analyze]       fundamental={f['fundamental']:.1f} Hz  "
        f"dc={f['dc_offset']:.2e}  peak={f['peak']:.3f}"
    )

    # 3. render_mix round-trip
    mix_id = tools.render_mix({"events": [(0, "sd")]})
    print(f"[render_mix]    → {mix_id}")

    # 4. regime-A invariants (cheap)
    inv = tools.run_invariants("regime-a")
    print(f"[invariants]    {inv['report']}  ({inv['passed']} passed)")

    # 5. parity
    par = tools.parity_check({"events": [(0, "sd")]})
    print(f"[parity]        max_divergence={par['max_divergence']}")

    # 6. regime B / C
    deltas = tools.compare_to_targets(
        tools.analyze(wav_id, feature_set=["fundamental", "dc_offset", "decay"]), "sd"
    )
    print(f"[compare_B]     all_within_tol={deltas['all_within_tol']}")
    ref = tools.compare_to_reference(wav_id, "sd")
    print(
        f"[compare_C]     available={ref.get('available')}  "
        f"shape_distance={ref.get('shape_distance')}"
    )

    # 7. param write side
    prop = tools.propose_param_edit("bleep.freq_hz", 660.0)
    print(f"[propose]       accepted={prop['accepted']}")
    refused = tools.propose_param_edit("mixer.master_level", 2.0)
    print(f"[propose oob]   accepted={refused['accepted']} (expected False)")

    # 8. golden round-trip (§9): bless a throwaway → compare clean → perturb a
    #    constant → drift flagged → restore. Committed goldens/ are untouched.
    import shutil

    from . import goldens, params

    g_name = "_selftest_sd"
    params.reset_overrides()
    try:
        goldens.bless_golden("sd", "selftest baseline", name=g_name)
        clean = tools.compare_to_golden(g_name)
        tools.apply_param_edit("bleep.freq_hz", 990.0)
        drifted = tools.compare_to_golden(g_name)
        print(
            f"[golden]        clean.drift={clean['drift']}  "
            f"perturbed.drift={drifted['drift']}  moved={drifted['drifted_metrics']}"
        )
        golden_ok = (
            (not clean["drift"])
            and drifted["drift"]
            and "fundamental" in drifted["drifted_metrics"]
        )
    finally:
        params.reset_overrides()
        shutil.rmtree(goldens.golden_dir(g_name), ignore_errors=True)

    ok = (
        870.0 <= f["fundamental"] <= 890.0
        and abs(f["dc_offset"]) < 1e-2
        and inv["green"]
        and par["max_divergence"] == 0
        and prop["accepted"]
        and not refused["accepted"]
        and golden_ok
    )
    print("=" * 60)
    print("RESULT:", "GREEN" if ok else "RED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(run())
