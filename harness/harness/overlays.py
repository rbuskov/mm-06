"""Regime-C overlay plots: render vs. reference clip (§7 testing, §11.5).

Renders the three overlays the strategy asks for when anchoring a voice to its
reference clip — **spectrogram, decay envelope, and pitch track** — drawing the
sterile render and `references/<voice>.wav` on the same axes after the regime-C
prep (DC-remove, onset-align, amplitude-normalize; §7.2.1) so the comparison is
shape-only, never loudness.

matplotlib is an OPTIONAL analysis dependency. If it cannot be imported, this
module still emits the overlay **data** (the curves/arrays) as JSON and notes the
limitation rather than failing (per the task), so a downstream tool can plot it.

Output goes to ``harness/goldens/<voice>/overlays/``. The PNGs are small
(≈1024×768, well under a few hundred KB) and are committed alongside the golden
so the one human glance has the visual to look at; the raw curve JSON is written
next to them as the matplotlib-free fallback artifact.
"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
from scipy.signal import butter, filtfilt, hilbert

from . import targets as _targets
from . import tools as _tools
from .paths import HARNESS_DIR


def overlays_dir(voice: str) -> Path:
    return HARNESS_DIR / "goldens" / voice.lower() / "overlays"


def _load_reference(voice: str):
    import soundfile as sf

    ref = _targets.reference_path(voice)
    x, sr = sf.read(str(ref), dtype="float64", always_2d=False)
    x = np.asarray(x, dtype=np.float64)
    if x.ndim > 1:
        x = x.mean(axis=1)
    return x, float(sr)


def _spectrogram(x: np.ndarray, sr: float, n_fft: int = 1024):
    """log1p-magnitude spectrogram as (times_s, freqs_hz, S[frames, bins])."""
    hop = n_fft // 4
    win = np.hanning(n_fft)
    if x.size < n_fft:
        x = np.pad(x, (0, n_fft - x.size))
    frames = []
    times = []
    for start in range(0, x.size - n_fft + 1, hop):
        seg = x[start : start + n_fft] * win
        frames.append(np.log1p(np.abs(np.fft.rfft(seg))))
        times.append((start + n_fft / 2) / sr)
    S = np.asarray(frames) if frames else np.zeros((1, n_fft // 2 + 1))
    freqs = np.fft.rfftfreq(n_fft, 1.0 / sr)
    return np.asarray(times), freqs, S


def _decay_env(x: np.ndarray, sr: float):
    """Smoothed Hilbert amplitude envelope vs. time (s)."""
    env = np.abs(hilbert(x))
    w = max(1, int(0.005 * sr))
    env = np.convolve(env, np.ones(w) / w, mode="same")
    t = np.arange(env.size) / sr
    return t, env


def _pitch_track(x: np.ndarray, sr: float):
    """Windowed autocorrelation pitch (Hz) over the high-SNR head (the no-sweep
    guard, §7.3): body-band (40-400 Hz) fundamental per 120 ms window, 50% hop."""
    fx = _targets._extractors()
    win = int(0.12 * sr)
    hop = win // 2
    head_end = int(0.35 * sr)
    ts, fs = [], []
    i = 0
    while i + win <= head_end and i + win <= x.size:
        f = fx.fundamental(x[i : i + win], sr, fmin=40.0, fmax=400.0)
        if f > 0.0:
            ts.append((i + win / 2) / sr)
            fs.append(f)
        i += hop
    return np.asarray(ts), np.asarray(fs)


def render_overlays(voice: str = "bd", wav_id: str | None = None) -> dict:
    """Render (or emit data for) the three regime-C overlays for `voice`.

    Returns a report with the output paths and, always, the decomposed curve
    arrays — so even with matplotlib absent the overlay DATA is captured (the
    task's fallback). The render is the sterile `render_voice(voice)` and the
    reference is `references/<voice>.wav`; both are prepped (§7.2.1).
    """
    voice = voice.lower()
    if wav_id is None:
        wav_id = _tools.render_voice(voice)
    rx, rsr = _tools.load_signal(wav_id)
    ref_x, ref_sr = _load_reference(voice)

    a = _targets._prep(np.asarray(rx, dtype=np.float64))
    b = _targets._prep(ref_x)

    # Curves (the matplotlib-free fallback data).
    ta, ea = _decay_env(a, rsr)
    tb, eb = _decay_env(b, ref_sr)
    pta, pfa = _pitch_track(a, rsr)
    ptb, pfb = _pitch_track(b, ref_sr)

    out_dir = overlays_dir(voice)
    out_dir.mkdir(parents=True, exist_ok=True)

    # Always write the curve data as the portable fallback artifact.
    def _ds(arr, n=512):
        arr = np.asarray(arr)
        if arr.size <= n:
            return [float(v) for v in arr]
        step = max(1, arr.size // n)
        return [float(v) for v in arr[::step]]

    data = {
        "voice": voice,
        "decay_envelope": {
            "rendered": {"t_s": _ds(ta), "env": _ds(ea)},
            "reference": {"t_s": _ds(tb), "env": _ds(eb)},
        },
        "pitch_track": {
            "rendered": {"t_s": [float(v) for v in pta], "f_hz": [float(v) for v in pfa]},
            "reference": {"t_s": [float(v) for v in ptb], "f_hz": [float(v) for v in pfb]},
        },
        "note": "render vs references/<voice>.wav, prepped (DC-removed, onset-aligned, normalized); shape-only",
    }
    data_path = out_dir / "overlay_data.json"
    data_path.write_text(json.dumps(data, indent=2) + "\n")

    report = {
        "voice": voice,
        "overlays_dir": str(out_dir),
        "data_path": str(data_path),
        "matplotlib": False,
        "plots": [],
    }

    try:
        import matplotlib

        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except Exception as exc:  # matplotlib optional — emit data + note, don't fail
        report["note"] = (
            f"matplotlib unavailable ({exc}); wrote overlay DATA to {data_path} "
            "instead of PNGs (curves are plot-ready)"
        )
        return report

    report["matplotlib"] = True
    plots = []

    # 1) Spectrogram overlay (render | reference side by side, shared bands).
    sta, sfa, Sa = _spectrogram(a, rsr)
    stb, sfb, Sb = _spectrogram(b, ref_sr)
    fig, axes = plt.subplots(1, 2, figsize=(10, 4), sharey=True)
    for ax, (t, f, S, title) in zip(
        axes,
        [(sta, sfa, Sa, "render (bd)"), (stb, sfb, Sb, "reference bd.wav")],
    ):
        ax.pcolormesh(t, f, S.T, shading="auto", cmap="magma")
        ax.set_ylim(0, 1500)  # body + click + low harmonics; hiss above is excluded
        ax.set_title(title)
        ax.set_xlabel("time (s)")
    axes[0].set_ylabel("freq (Hz)")
    fig.suptitle("BD regime-C spectrogram — render vs bd.wav (shape-only)")
    fig.tight_layout()
    p = out_dir / "spectrogram.png"
    fig.savefig(p, dpi=96)
    plt.close(fig)
    plots.append(str(p))

    # 2) Decay-envelope overlay.
    fig, ax = plt.subplots(figsize=(8, 4))
    ax.plot(ta, ea, label="render (bd)", lw=1.5)
    ax.plot(tb, eb, label="reference bd.wav", lw=1.5, alpha=0.8)
    ax.set_xlim(0, 0.35)
    ax.set_xlabel("time (s)")
    ax.set_ylabel("normalized amplitude")
    ax.set_title("BD decay envelope — render vs bd.wav (two-resonator ring/beat)")
    ax.legend()
    fig.tight_layout()
    p = out_dir / "decay_envelope.png"
    fig.savefig(p, dpi=96)
    plt.close(fig)
    plots.append(str(p))

    # 3) Pitch-track overlay (the no-sweep guard against the clip).
    fig, ax = plt.subplots(figsize=(8, 4))
    ax.plot(pta, pfa, "o-", label="render (bd)", lw=1.5)
    ax.plot(ptb, pfb, "s-", label="reference bd.wav", lw=1.5, alpha=0.8)
    ax.axhline(60.0, color="gray", ls=":", lw=1, label="OSC1 target 60 Hz")
    ax.set_xlabel("time (s)")
    ax.set_ylabel("fundamental (Hz)")
    ax.set_ylim(0, 200)
    ax.set_title("BD pitch track — render vs bd.wav (stationary, NO sweep)")
    ax.legend()
    fig.tight_layout()
    p = out_dir / "pitch_track.png"
    fig.savefig(p, dpi=96)
    plt.close(fig)
    plots.append(str(p))

    report["plots"] = plots
    return report
