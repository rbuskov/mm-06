# `analysis/` — MM-06 render feature-extraction layer

Turns a `render` WAV into a **metrics JSON**. This is the home of every feature
extractor regimes B and C consume
(`documents/calibration-and-testing-strategy.md` §2.2). It lives in Python
(numpy / scipy / soundfile / librosa) — a **different language and codebase from
the Rust `dsp_core`** — on purpose, so a bug in the synth isn't mirrored in the
thing that measures it.

## Install & test

```sh
cd analysis
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt   # pinned; or: pip install -e .
.venv/bin/python -m pytest -q                # all green
```

The venv and generated `*.wav` / `*.json` artifacts are git-ignored
(`analysis/.gitignore`) — only source, tests, pinned deps, and docs are
committed.

> Dep note: verified on macOS / CPython 3.14 (arm64). `librosa` (and its
> `numba`/`llvmlite` chain) installed cleanly, so it is a hard dependency; the
> core extractors are nonetheless implemented in scipy/numpy so they run even if
> librosa is unavailable on some target.

## API

```python
from analysis import analyze, analyze_array

metrics = analyze("kick.wav")                      # all features
metrics = analyze("kick.wav", ["decay", "fundamental"])  # a subset
metrics = analyze_array(x, sample_rate)            # in-memory numpy signal
```

`analyze(wav_path, feature_set=None)` loads a mono float WAV (the `render`
binary writes mono 32-bit-float, format tag 3 — `soundfile` reads it directly),
runs the requested extractors, and returns a JSON-serializable dict.

## CLI

```sh
# console script (after `pip install -e .` or installing requirements):
analyze kick.wav                                   # all features, pretty JSON
analyze kick.wav --features spectral_centroid,decay
analyze kick.wav --indent -1                        # compact single line

# or as a module (run from the analysis/ dir, or after install):
python -m analysis kick.wav -f fundamental,partials
```

The metrics JSON is printed to stdout (pipe into `jq` / the harness).

## Metrics JSON schema

`schema_version` is **1** (`analysis/schema.py :: SCHEMA_VERSION`). Bump it when
a field's name, units, or meaning changes; adding a new optional sub-field under
an existing feature is backward-compatible and needs no bump.

Top level:

| field            | type   | unit / meaning                                  |
|------------------|--------|-------------------------------------------------|
| `schema_version` | int    | this schema's version (currently `1`)           |
| `source`         | str    | analyzed file path (or `"<array>"`)             |
| `sample_rate`    | float  | Hz                                              |
| `n_samples`      | int    | number of mono samples analyzed                 |
| `duration_s`     | float  | seconds                                         |
| `features`       | object | one key per requested feature (below)           |

Feature keys (all frequencies in **Hz**; all levels are **linear full-scale**
unless the field name ends in `_db`):

| feature             | shape                                                          | meaning |
|---------------------|----------------------------------------------------------------|---------|
| `peak`              | float                                                          | peak absolute sample value |
| `rms`               | float                                                          | RMS level over the whole buffer |
| `dc_offset`         | float                                                          | signed mean sample value (regime A wants ≈ 0) |
| `spectral_centroid` | float                                                          | power-weighted spectral centroid (brightness) |
| `energy_envelope`   | `{envelope:[float], hop:int, peak:float, peak_time:float}`     | Hilbert analytic amplitude envelope, decimated to ≤512 points (`hop` = samples between points); `peak`/`peak_time` (s) of the envelope max |
| `decay`             | `{tau_e:float, slope_db_per_s:float, tau_fit:float}`           | `tau_e` = time (s) from peak to 1/e of peak; `slope_db_per_s` = slope of the log-envelope linear fit (negative); `tau_fit` = that slope expressed as a 1/e time constant (s) |
| `fundamental`       | float                                                          | estimated fundamental (autocorrelation); `0.0` if none found |
| `partials`          | `[{freq:float, mag:float, mag_db:float}, ...]`                 | up to 6 dominant spectral peaks, descending magnitude; `mag_db` is relative to the strongest peak |
| `alias`             | `{alias_band_hz:[lo,hi], alias_energy_ratio:float, alias_floor_db:float}` | out-of-band (default top quarter of spectrum) energy as a fraction of total, and `10·log10` of it (dB) |

### Which regime consumes what

- **Amplitude / DC** (`peak`, `rms`, `dc_offset`) → regime A DC-free gate (§4.4),
  level monotonicity (§4.10).
- **`spectral_centroid`** → regime B brightness targets, regime C shape (§6, §7.2).
- **`decay` / `energy_envelope`** → regime B per-voice RC decay times
  (CH short / OH long / cymbal long, §6) and regime C envelope shape (§7.2).
- **`fundamental` / `partials`** → regime B resonator pitches (BD ≈60/130 Hz)
  and the six metallic oscillator peaks (≈245/308/367/417/438/625 Hz), regime C
  inharmonic peak structure (§6, §7.2/§7.3).
- **`alias`** → regime A aliasing-floor gate (§4.5, "no cheap-clone inharmonic
  junk").

## Notes on accuracy

- Synthetic-truth unit tests (`tests/test_extractors.py`) pin every extractor to
  a numpy-generated signal with a known answer (880 Hz sine → centroid/pitch
  ≈ 880 Hz; exponential decay with known τ → measured τ; DC and Nyquist edge
  cases → sane values, no crashes).
- The smoke test (`tests/test_smoke_render.py`) builds + runs `crates/render` to
  produce a real 880 Hz decaying-sine bleep and analyzes it. The placeholder
  voice uses `dsp_core`'s **vendored `mathx::exp`** (a walking-skeleton
  polynomial, not libm), so the rendered bleep decays a little faster than a
  libm `exp(-t/0.10)` would (empirically ≈ 0.07 s vs the nominal `BASE_TAU`
  0.10 s). The smoke test asserts the measured decay lands in a sane band around
  the nominal τ; the synthetic tests pin the extractor's own accuracy. If
  `cargo`/`render` is unavailable the smoke test falls back to synthesizing the
  equivalent bleep in numpy and reports that it did so.
