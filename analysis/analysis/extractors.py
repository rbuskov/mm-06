"""Core feature extractors for MM-06 render analysis.

Every function here takes a mono float signal ``x`` (already loaded from a WAV)
plus the sample rate ``sr`` and returns plain Python floats / lists — never numpy
scalars — so the result is JSON-serializable straight out of :func:`analyze`.

The extractors are deliberately written in numpy / scipy (and, where available,
librosa) — a *different language and codebase* from the Rust ``dsp_core`` — so a
bug in the synth is not mirrored in the thing that measures it
(calibration-and-testing-strategy.md §2.2).

Units, meanings and the JSON schema are documented in ``analysis/README.md`` and
mirrored in :data:`analysis.schema.SCHEMA_VERSION` / the field docstrings below.
"""

from __future__ import annotations

import numpy as np
from scipy.signal import hilbert

__all__ = [
    "peak",
    "rms",
    "dc_offset",
    "spectral_centroid",
    "energy_envelope",
    "decay_time",
    "fundamental",
    "partial_track",
    "alias_floor",
]


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

def _as_mono(x: np.ndarray) -> np.ndarray:
    """Coerce to a contiguous 1-D float64 array (average channels if stereo)."""
    x = np.asarray(x, dtype=np.float64)
    if x.ndim > 1:
        x = x.mean(axis=tuple(range(1, x.ndim)))
    return np.ascontiguousarray(x)


def _window(n: int) -> np.ndarray:
    """A Hann window of length ``n`` (handles the n<=1 edge cases)."""
    if n <= 1:
        return np.ones(max(n, 0))
    return np.hanning(n)


# ---------------------------------------------------------------------------
# amplitude-domain features
# ---------------------------------------------------------------------------

def peak(x: np.ndarray) -> float:
    """Peak absolute sample value (linear, dimensionless full-scale)."""
    x = _as_mono(x)
    if x.size == 0:
        return 0.0
    return float(np.max(np.abs(x)))


def rms(x: np.ndarray) -> float:
    """Root-mean-square level over the whole buffer (linear full-scale)."""
    x = _as_mono(x)
    if x.size == 0:
        return 0.0
    return float(np.sqrt(np.mean(x * x)))


def dc_offset(x: np.ndarray) -> float:
    """Mean sample value — the DC component (linear full-scale).

    Regime A requires every voice/master output to be DC-free
    (calibration §4.4); this is the measured mean to compare against tolerance.
    """
    x = _as_mono(x)
    if x.size == 0:
        return 0.0
    return float(np.mean(x))


# ---------------------------------------------------------------------------
# spectral features
# ---------------------------------------------------------------------------

def _magnitude_spectrum(x: np.ndarray, sr: float):
    """One-sided magnitude spectrum of the windowed whole buffer.

    Returns ``(freqs_hz, mags)`` with ``mags`` the linear magnitude (not power).
    """
    x = _as_mono(x)
    n = x.size
    if n == 0:
        return np.zeros(0), np.zeros(0)
    w = _window(n)
    spec = np.fft.rfft(x * w)
    freqs = np.fft.rfftfreq(n, d=1.0 / sr)
    return freqs, np.abs(spec)


def spectral_centroid(x: np.ndarray, sr: float) -> float:
    """Power-weighted spectral centroid in Hz.

    The "centre of mass" of the magnitude spectrum — the primary brightness
    feature for regimes B/C (calibration §6, §7.2). Power weighting (mag**2) is
    used so a single dominant partial pulls the centroid close to its frequency,
    which makes the pure-sine ground-truth test exact to within a bin.
    """
    freqs, mags = _magnitude_spectrum(x, sr)
    if freqs.size == 0:
        return 0.0
    power = mags * mags
    total = power.sum()
    if total <= 0.0:
        return 0.0
    return float(np.sum(freqs * power) / total)


# ---------------------------------------------------------------------------
# envelope / decay
# ---------------------------------------------------------------------------

def energy_envelope(x: np.ndarray, sr: float):
    """Analytic (Hilbert) amplitude envelope of the signal.

    Returns a dict with:

    - ``envelope``: the per-sample analytic amplitude |hilbert(x)| (list of
      floats). Downsampled to at most ``max_points`` entries so the JSON stays
      small; ``hop`` reports the decimation used.
    - ``hop``: samples between successive envelope points.
    - ``peak``: max envelope value.
    - ``peak_time``: time (s) of the envelope peak.
    """
    x = _as_mono(x)
    if x.size == 0:
        return {"envelope": [], "hop": 1, "peak": 0.0, "peak_time": 0.0}
    env = np.abs(hilbert(x))
    peak_idx = int(np.argmax(env))
    max_points = 512
    hop = max(1, env.size // max_points)
    env_ds = env[::hop]
    return {
        "envelope": [float(v) for v in env_ds],
        "hop": int(hop),
        "peak": float(env[peak_idx]),
        "peak_time": float(peak_idx / sr),
    }


def decay_time(x: np.ndarray, sr: float) -> dict:
    """Estimate the decay of a pinged/decaying tone from its Hilbert envelope.

    Two complementary estimates of the same physical decay (calibration §6,
    "CH short / OH long decay … the per-voice RC decay constants"):

    - ``tau_e``: time (s) from the envelope peak for the amplitude to fall to
      1/e of the peak. This matches ``dsp_core``'s ``BASE_TAU`` convention
      (``coef = exp(-1/(tau*sr))`` ⇒ amplitude ≈ ``e^{-t/tau}``).
    - ``slope_db_per_s``: slope of a linear fit to the log-envelope (in dB/s,
      negative for a decay). For a clean exponential ``e^{-t/tau}`` this is
      ``-20/(tau*ln10) ≈ -8.686/tau`` dB/s. ``tau_fit`` is that slope converted
      back to a 1/e time constant.

    Both are measured from the envelope peak onward, ignoring the silent tail
    (below ``floor_db`` of the peak) so flushed-to-zero tails don't bias the fit.
    """
    x = _as_mono(x)
    out = {"tau_e": 0.0, "slope_db_per_s": 0.0, "tau_fit": 0.0}
    if x.size < 4:
        return out
    env = np.abs(hilbert(x))
    # Light smoothing kills the 2f ripple in the analytic envelope of a real
    # (single-sided) tone without distorting the decay slope.
    win = max(1, int(sr * 0.001))
    if win > 1 and env.size > win:
        kernel = np.ones(win) / win
        env = np.convolve(env, kernel, mode="same")
    peak_idx = int(np.argmax(env))
    peak_val = float(env[peak_idx])
    if peak_val <= 0.0:
        return out

    seg = env[peak_idx:]
    t = np.arange(seg.size) / sr

    # --- 1/e crossing time -------------------------------------------------
    target = peak_val / np.e
    below = np.where(seg <= target)[0]
    if below.size > 0:
        i = int(below[0])
        if i == 0:
            out["tau_e"] = 0.0
        else:
            # linear interpolation between sample i-1 and i for a smoother estimate
            y0, y1 = seg[i - 1], seg[i]
            frac = 0.0 if y0 == y1 else (y0 - target) / (y0 - y1)
            out["tau_e"] = float((i - 1 + frac) / sr)

    # --- log-envelope linear fit ------------------------------------------
    # Fit a CONTIGUOUS decay span: from the peak to the FIRST time the envelope
    # drops below the floor. The analytic-envelope ripple dips below and back
    # above the floor, so a plain "env > floor" mask scatters tail/noise-floor
    # points across the whole buffer and flattens the slope — fit the head only.
    floor_db = -40.0
    floor = peak_val * (10.0 ** (floor_db / 20.0))
    first_below = np.where(seg <= floor)[0]
    end = int(first_below[0]) if first_below.size > 0 else seg.size
    if end >= 4:
        head = seg[:end]
        tt = t[:end]
        logamp_db = 20.0 * np.log10(head / peak_val)
        A = np.vstack([tt, np.ones_like(tt)]).T
        slope, _ = np.linalg.lstsq(A, logamp_db, rcond=None)[0]
        out["slope_db_per_s"] = float(slope)
        if slope < 0.0:
            # dB/s -> tau: amp = e^{-t/tau} => dB = -20/(tau ln10) * t
            out["tau_fit"] = float(-8.685889638065035 / slope)
    return out


# ---------------------------------------------------------------------------
# pitch / partials
# ---------------------------------------------------------------------------

def _parabolic_peak(mags: np.ndarray, k: int) -> float:
    """Sub-bin peak position around integer bin ``k`` by parabolic interpolation."""
    if k <= 0 or k >= mags.size - 1:
        return float(k)
    a, b, c = mags[k - 1], mags[k], mags[k + 1]
    denom = a - 2.0 * b + c
    if denom == 0.0:
        return float(k)
    return float(k + 0.5 * (a - c) / denom)


def fundamental(x: np.ndarray, sr: float, fmin: float = 30.0, fmax: float = 5000.0) -> float:
    """Estimate the fundamental frequency (Hz) via autocorrelation.

    Autocorrelation (rather than just the spectral peak) is robust for the
    decaying tones we render and gives a clean ≈ truth on a pure sine. Returns
    0.0 if no clear period is found within ``[fmin, fmax]``.
    """
    x = _as_mono(x)
    if x.size < 4:
        return 0.0
    x = x - np.mean(x)
    if not np.any(x):
        return 0.0
    n = x.size
    w = _window(n)
    xw = x * w
    corr = np.correlate(xw, xw, mode="full")[n - 1:]
    if corr[0] <= 0:
        return 0.0
    # Remove the triangular window-autocorrelation bias: a Hann-windowed tone's
    # autocorrelation envelope falls off ~linearly with lag, which suppresses
    # the true (low-lag) period peak relative to subharmonic peaks at 2x/3x the
    # lag and makes a plain argmax report an octave-too-low pitch. Normalize by
    # the deterministic overlap count so all period peaks are comparable.
    overlap = (n - np.arange(n)).astype(np.float64)
    corr = corr / overlap

    lag_min = max(1, int(sr / fmax))
    lag_max = min(n - 1, int(sr / fmin))
    if lag_max <= lag_min:
        return 0.0
    region = corr[lag_min:lag_max]
    if region.size == 0:
        return 0.0
    peak = float(region.max())
    if peak <= 0.0:
        return 0.0
    # Pick the FIRST (lowest-lag) peak that reaches a high fraction of the
    # global max — the fundamental period, not a later subharmonic peak.
    thresh = 0.85 * peak
    k = None
    for i in range(1, region.size - 1):
        if region[i] >= thresh and region[i] >= region[i - 1] and region[i] >= region[i + 1]:
            k = i + lag_min
            break
    if k is None:
        k = int(np.argmax(region)) + lag_min
    # parabolic interpolation around the autocorrelation peak for sub-sample lag
    if 0 < k < corr.size - 1:
        a, b, c = corr[k - 1], corr[k], corr[k + 1]
        denom = a - 2.0 * b + c
        lag = k + (0.5 * (a - c) / denom if denom != 0.0 else 0.0)
    else:
        lag = float(k)
    if lag <= 0:
        return 0.0
    return float(sr / lag)


def partial_track(x: np.ndarray, sr: float, n_partials: int = 6,
                  fmin: float = 30.0, fmax: float | None = None) -> list:
    """Dominant spectral partials (the inharmonic peak set for metallic voices).

    Returns up to ``n_partials`` ``{"freq": Hz, "mag": linear, "mag_db": dB}``
    dicts, sorted by descending magnitude. Peaks are picked from the windowed
    magnitude spectrum with a simple local-maximum + parabolic sub-bin refine,
    enforcing a minimum spacing so a single broad lobe isn't reported many
    times. This is the feature regime B uses for the six metallic oscillators
    (≈245/308/367/417/438/625 Hz) and the BD dual partials (calibration §6).
    """
    freqs, mags = _magnitude_spectrum(x, sr)
    if freqs.size == 0:
        return []
    if fmax is None:
        fmax = sr / 2.0
    band = (freqs >= fmin) & (freqs <= fmax)
    if not np.any(band):
        return []
    fb = freqs[band]
    mb = mags[band]
    if mb.max() <= 0.0:
        return []

    df = fb[1] - fb[0] if fb.size > 1 else sr
    min_spacing_hz = max(3.0 * df, fmin * 0.25)

    # local maxima
    peaks = []
    for i in range(1, mb.size - 1):
        if mb[i] > mb[i - 1] and mb[i] >= mb[i + 1]:
            sub = _parabolic_peak(mb, i)
            f = float(fb[0] + sub * df)
            peaks.append((mb[i], f))
    # if no interior maxima (e.g. a single bin), fall back to the global max bin
    if not peaks:
        i = int(np.argmax(mb))
        peaks.append((mb[i], float(fb[i])))

    peaks.sort(key=lambda p: p[0], reverse=True)
    ref = peaks[0][0]
    chosen = []
    for mag, f in peaks:
        if any(abs(f - cf) < min_spacing_hz for _, cf in chosen):
            continue
        chosen.append((mag, f))
        if len(chosen) >= n_partials:
            break

    out = []
    for mag, f in chosen:
        out.append({
            "freq": float(f),
            "mag": float(mag),
            "mag_db": float(20.0 * np.log10(mag / ref)) if ref > 0 else 0.0,
        })
    return out


# ---------------------------------------------------------------------------
# alias floor
# ---------------------------------------------------------------------------

def alias_floor(x: np.ndarray, sr: float, signal_hi_hz: float | None = None) -> dict:
    """Estimate out-of-band / alias energy relative to the in-band signal.

    The square-oscillator metallic bank is the biggest aliasing source
    (calibration §3.1, §4.5: "no cheap-clone inharmonic junk"). With no model
    of where the intended partials sit, this gives a generic, stable proxy:
    the ratio of spectral energy in a high band (default the top quarter of the
    spectrum, ``[0.75*Nyquist, Nyquist]``) to the total energy.

    Returns:

    - ``alias_band_hz``: ``[lo, hi]`` of the band measured as "alias".
    - ``alias_energy_ratio``: high-band energy / total energy (linear, 0..1).
    - ``alias_floor_db``: ``10*log10`` of that ratio (dB below total; -inf → a
      large negative sentinel ``-300``).
    """
    freqs, mags = _magnitude_spectrum(x, sr)
    nyq = sr / 2.0
    if signal_hi_hz is None:
        lo = 0.75 * nyq
    else:
        lo = signal_hi_hz
    out = {
        "alias_band_hz": [float(lo), float(nyq)],
        "alias_energy_ratio": 0.0,
        "alias_floor_db": -300.0,
    }
    if freqs.size == 0:
        return out
    power = mags * mags
    total = power.sum()
    if total <= 0.0:
        return out
    band = freqs >= lo
    ratio = float(power[band].sum() / total)
    out["alias_energy_ratio"] = ratio
    out["alias_floor_db"] = float(10.0 * np.log10(ratio)) if ratio > 0 else -300.0
    return out
