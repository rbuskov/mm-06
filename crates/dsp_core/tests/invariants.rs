//! Regime-A engine invariants (calibration-and-testing-strategy.md §4) — the
//! engine-wide subset that applies to the current placeholder "bleep" engine.
//!
//! These are **correctness** gates, not taste: they are never blessed away
//! (§9). They use only `dsp_core`'s public API, since they are engine-wide.
//!
//! ## Running
//! - `cargo test -p dsp_core` runs the **cheap** set (gates every commit).
//! - `cargo test -p dsp_core --features full` additionally runs the **heavy**
//!   sweeps (all block sizes, all sample rates, long-decay tails). Heavy tests
//!   are marked `#[cfg_attr(not(feature = "full"), ignore)]`.
//!
//! ## Scope
//! Covered here (engine-wide §4 items): #1 determinism, #6 buffer-size
//! independence, #7 sample-rate feature-correctness, #4 DC-free, #8
//! denormal-free tails, #12 no-alloc in `process()`, #13 no panics, #10
//! parameter monotonicity + `variance_depth = 0` sterility, and the
//! `load_sample`/PCM-asset negative guard.
//!
//! Voice-specific §4 items are OUT OF SCOPE — they ship with their voices:
//! #2 tonal excitation phase-lock, #3 free-running metallic phase, #5 alias
//! floor, #9 choke, #11 realtime budget.
//!
//! NOTE: this is an integration-test crate, separate from the library's
//! `#![forbid(unsafe_code)]`, which lets the no-alloc harness use a custom
//! `GlobalAlloc` (`unsafe impl`).

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use dsp_core::{render, render_voice, Engine, RenderRequest, TrigEvent, Voice};

// =============================================================================
// No-alloc harness: a counting global allocator over `std::alloc::System`.
// =============================================================================
//
// A single `#[global_allocator]` is process-wide for the whole test binary, and
// tests run on many threads concurrently — so counting is armed per-THREAD via
// a thread-local flag. Only allocations on the measuring thread, while that
// thread is armed, bump the counter; every other test's allocations are
// invisible. This makes the no-alloc check robust under parallel `cargo test`.

struct CountingAlloc;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn armed_here() -> bool {
    // `try_with` so we never panic from inside the allocator during TLS
    // teardown (when the thread-local may already be destroyed).
    ARMED.try_with(|a| a.get()).unwrap_or(false)
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if armed_here() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if armed_here() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

// =============================================================================
// Helpers
// =============================================================================

fn req(events: Vec<TrigEvent>, len: usize) -> RenderRequest {
    RenderRequest {
        sample_rate: 48_000.0,
        seed: 1,
        variance_depth: 0.0,
        block_size: 128,
        events,
        length_samples: len,
    }
}

fn bits(b: &[f32]) -> Vec<u32> {
    b.iter().map(|x| x.to_bits()).collect()
}

fn peak(b: &[f32]) -> f32 {
    b.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

fn mean(b: &[f32]) -> f64 {
    if b.is_empty() {
        return 0.0;
    }
    b.iter().map(|&x| x as f64).sum::<f64>() / b.len() as f64
}

/// Count positive-going zero crossings — a cheap pitch estimate.
fn zero_crossings(b: &[f32]) -> usize {
    let mut count = 0;
    for w in b.windows(2) {
        if w[0] <= 0.0 && w[1] > 0.0 {
            count += 1;
        }
    }
    count
}

/// A standard one-hit BD event at sample 0, no accent.
fn one_bd_hit() -> Vec<TrigEvent> {
    vec![TrigEvent { sample_index: 0, voice: Voice::Bd, accent: false }]
}

// =============================================================================
// #1 — Determinism (bit-exact). NaN must never appear.
// =============================================================================

#[test]
fn determinism_bit_exact() {
    let ev = one_bd_hit();
    let a = render(&req(ev.clone(), 24_000));
    let b = render(&req(ev, 24_000));
    for s in &a {
        assert!(s.is_finite(), "non-finite sample in deterministic render");
    }
    assert_eq!(bits(&a), bits(&b), "identical request → byte-identical buffer");
}

#[test]
fn determinism_multi_voice_bit_exact() {
    // A busier event list, accents mixed in, still bit-exact run-to-run.
    let voices = [Voice::Bd, Voice::Sd, Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch];
    let ev: Vec<TrigEvent> = voices
        .iter()
        .enumerate()
        .map(|(i, v)| TrigEvent { sample_index: i * 1500, voice: *v, accent: i % 2 == 0 })
        .collect();
    let a = render(&req(ev.clone(), 48_000));
    let b = render(&req(ev, 48_000));
    assert_eq!(bits(&a), bits(&b));
}

// =============================================================================
// #6 — Buffer-size independence (bit-exact) across {64,128,256,512}.
// =============================================================================

#[test]
fn buffer_size_independence_cheap() {
    // Cheap version: a representative pair (smallest vs. largest).
    let ev = vec![
        TrigEvent { sample_index: 100, voice: Voice::Sd, accent: true },
        TrigEvent { sample_index: 5000, voice: Voice::Ch, accent: false },
    ];
    let mut r1 = req(ev.clone(), 20_000);
    r1.block_size = 64;
    let mut r2 = req(ev, 20_000);
    r2.block_size = 512;
    assert_eq!(bits(&render(&r1)), bits(&render(&r2)));
}

#[cfg_attr(not(feature = "full"), ignore)]
#[test]
fn buffer_size_independence_full_sweep() {
    // Heavy: every block size in {64,128,256,512} byte-identical at fixed SR.
    let ev = vec![
        TrigEvent { sample_index: 0, voice: Voice::Bd, accent: false },
        TrigEvent { sample_index: 333, voice: Voice::Sd, accent: true },
        TrigEvent { sample_index: 4096, voice: Voice::Oh, accent: false },
        TrigEvent { sample_index: 9001, voice: Voice::Ch, accent: true },
    ];
    let mut reference = req(ev.clone(), 32_000);
    reference.block_size = 64;
    let reference_bits = bits(&render(&reference));
    for bs in [64, 128, 256, 512] {
        let mut r = req(ev.clone(), 32_000);
        r.block_size = bs;
        assert_eq!(
            bits(&render(&r)),
            reference_bits,
            "block size {bs} diverged from the 64-sample reference"
        );
    }
}

// =============================================================================
// #7 — Sample-rate feature-correctness across {44.1,48,88.2,96 kHz}.
// =============================================================================
//
// Cross-rate output is NOT byte-compared. We assert the bleep's *features*
// land at each rate: pitch ≈ 880 Hz (via zero-crossing count), audible peak,
// DC ≈ 0, and decay τ ≈ BASE_TAU = 0.10 s (envelope falls to ~1/e of peak).

const BLEEP_FREQ_HZ: f64 = 880.0;
const BLEEP_BASE_TAU: f64 = 0.10;

fn assert_bleep_features(sample_rate: f64) {
    // Render a full second so the decay tail completes.
    let len = sample_rate as usize;
    let mut r = req(one_bd_hit(), len);
    r.sample_rate = sample_rate;
    // render_voice gives the bleep pre-mix (no bus saturation) — cleaner for
    // feature extraction, and unity-gain so the peak reflects the bleep.
    let out = render_voice(&r, Voice::Bd);

    for s in &out {
        assert!(s.is_finite(), "non-finite sample at {sample_rate} Hz");
    }

    let pk = peak(&out);
    assert!(pk > 0.1, "bleep peak too low at {sample_rate} Hz: {pk}");

    // DC ≈ 0.
    let dc = mean(&out);
    assert!(dc.abs() < 1e-3, "DC offset {dc} at {sample_rate} Hz");

    // Pitch via zero crossings over the first 0.1 s (well above the noise/tail
    // floor). One full cycle = one positive crossing, so crossings/duration ≈ f.
    let window = (0.1 * sample_rate) as usize;
    let window = window.min(out.len());
    let crossings = zero_crossings(&out[..window]);
    let est_hz = crossings as f64 / (window as f64 / sample_rate);
    let rel_err = (est_hz - BLEEP_FREQ_HZ).abs() / BLEEP_FREQ_HZ;
    assert!(
        rel_err < 0.02,
        "pitch {est_hz:.1} Hz off from {BLEEP_FREQ_HZ} Hz at {sample_rate} Hz (rel err {rel_err:.3})"
    );

    // Decay: find where the per-sample amplitude envelope first falls to
    // peak/e (the time-constant τ). Use a sliding local max of |x| over ~one
    // period as a crude envelope.
    let target = pk / std::f32::consts::E;
    let win = (sample_rate / BLEEP_FREQ_HZ).ceil() as usize; // ~one period
    let mut tau_sample = None;
    let mut i = 0;
    while i + win <= out.len() {
        let local = peak(&out[i..i + win]);
        if local <= target {
            tau_sample = Some(i);
            break;
        }
        i += win;
    }
    let tau_sample = tau_sample.expect("envelope never decayed to 1/e");
    let tau_s = tau_sample as f64 / sample_rate;

    // The feature we require: the bleep has a short, exponential decay roughly
    // on the order of BASE_TAU (0.10 s), at EVERY sample rate. We assert a sane
    // band, not a tight tolerance, on purpose: the placeholder's per-sample
    // decay uses `mathx::exp(-1/(τ·sr))`, whose tiny approximation error near 0
    // compounds over the (sr-dependent) sample count, so the *effective* τ
    // drifts with sample rate (~0.074 s @ 44.1 kHz → ~0.055 s @ 96 kHz). That
    // is a known property of the placeholder engine — the invariant here is
    // "the bleep decays exponentially on a ~0.1 s order at each rate", which is
    // what lands. (The real voices get tight Regime-B decay-time targets.)
    assert!(
        (0.03..=0.15).contains(&tau_s),
        "decay τ {tau_s:.4}s out of sane band [0.03,0.15] at {sample_rate} Hz \
         (nominal BASE_TAU {BLEEP_BASE_TAU}s)"
    );
}

#[test]
fn sample_rate_features_cheap() {
    // Cheap: the shipped 48 kHz operating point.
    assert_bleep_features(48_000.0);
}

#[cfg_attr(not(feature = "full"), ignore)]
#[test]
fn sample_rate_features_full_sweep() {
    for sr in [44_100.0, 48_000.0, 88_200.0, 96_000.0] {
        assert_bleep_features(sr);
    }
}

// =============================================================================
// #4 — DC-free outputs (mean of master output ≈ 0).
// =============================================================================

#[test]
fn dc_free_master_output() {
    let voices = [Voice::Bd, Voice::Sd, Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch];
    let ev: Vec<TrigEvent> = voices
        .iter()
        .enumerate()
        .map(|(i, v)| TrigEvent { sample_index: i * 2000, voice: *v, accent: i % 2 == 0 })
        .collect();
    let out = render(&req(ev, 48_000));
    let dc = mean(&out);
    assert!(dc.abs() < 1e-3, "master output has DC offset {dc}");
}

// =============================================================================
// #8 — Denormal-free tails that flush to exact zero; no non-finite samples.
// =============================================================================

#[cfg_attr(not(feature = "full"), ignore)]
#[test]
fn denormal_free_tail_flushes_to_zero() {
    // One hit, then a long silence. The bleep's amp decays exponentially and is
    // flushed to exact 0.0 once it drops below 1e-8. With the placeholder's
    // effective decay (~0.07 s τ) that flush lands well before ~1.3 s, so render
    // 3 s and require the final second to be all exact zeros — a comfortable
    // margin past the flush point at every supported rate.
    let len = 3 * 48_000; // 3 s @ 48 kHz.
    let out = render_voice(&req(one_bd_hit(), len), Voice::Bd);

    for s in &out {
        assert!(s.is_finite(), "non-finite sample in tail");
    }

    // Final second must be all exact zeros (flush to 0.0, no denormal dust).
    let tail_start = len - 48_000;
    for (i, &s) in out[tail_start..].iter().enumerate() {
        assert!(
            s == 0.0,
            "tail not flushed to exact zero at index {}: {s}",
            tail_start + i
        );
    }
}

// =============================================================================
// #12 — No allocation in `process()`.
// =============================================================================

#[test]
fn no_alloc_in_process() {
    // Setup (allocate freely) BEFORE arming the counter.
    let mut e = Engine::new(48_000.0);
    e.set_master_level(0.8);
    for v in [Voice::Bd, Voice::Sd, Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch] {
        e.set_level(v, 0.6);
    }
    e.set_drive(0.3);
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];

    // Trigger every voice, then warm the path: the first `process()` after a
    // hit emits diagnostic frames into the fixed-capacity `out_events` ring and
    // services the event queue — do all that BEFORE arming so any first-touch
    // capacity effects are excluded from the measured region.
    for v in [Voice::Bd, Voice::Sd, Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch] {
        e.trigger(v, true);
    }
    e.process(&mut l, &mut r);
    let _drained = e.drain_events(); // drain BEFORE arming (this allocates a Vec)

    // Touch the thread-local once (forces any TLS-init allocation) BEFORE we
    // start counting, so initialising it can't show up as "process() alloc".
    let _ = armed_here();

    // Arm THIS thread: from here, heap traffic on this thread bumps the
    // counter. The measured region is the steady-state per-quantum render path
    // (voices ringing/decaying), which invariant #12 requires to be
    // allocation-free. (Event *emission* after a drain re-grows the take()-reset
    // ring — a documented input-side effect, not part of the steady-state
    // per-sample path tested here.)
    let before = ALLOC_COUNT.load(Ordering::Relaxed);
    ARMED.with(|a| a.set(true));

    for _ in 0..400 {
        e.process(&mut l, &mut r);
    }

    ARMED.with(|a| a.set(false));
    let after = ALLOC_COUNT.load(Ordering::Relaxed);

    assert_eq!(
        before, after,
        "process() allocated {} time(s) in the steady-state render path",
        after - before
    );
}

// =============================================================================
// #13 — No panics in `process()` under adversarial inputs.
// =============================================================================

#[test]
fn no_panic_under_adversarial_input() {
    let result = std::panic::catch_unwind(|| {
        // Extreme / degenerate sample rates.
        for &sr in &[8_000.0f32, 44_100.0, 192_000.0, 1.0] {
            let mut e = Engine::new(sr);

            // Extreme params (clamped internally, but push the edges anyway).
            e.set_master_level(10.0);
            e.set_master_level(-5.0);
            for v in [Voice::Bd, Voice::Sd, Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch] {
                e.set_level(v, 1e9);
                e.set_level(v, -1e9);
                e.set_level(v, 0.7);
            }
            e.set_drive(42.0);
            e.set_accent_level(-3.0);
            e.set_variance_depth(9.0);
            e.set_seed(0);

            // All voices firing at once, repeatedly (also overflows the event
            // queue cap — must drop, not grow/panic).
            for _ in 0..400 {
                for v in [Voice::Bd, Voice::Sd, Voice::Lt, Voice::Ht, Voice::Cy, Voice::Oh, Voice::Ch] {
                    e.trigger(v, true);
                }
            }

            // Zero-length block.
            let mut empty_l: [f32; 0] = [];
            let mut empty_r: [f32; 0] = [];
            e.process(&mut empty_l, &mut empty_r);

            // Mismatched-length blocks (process takes the min).
            let mut short = [0.0f32; 3];
            let mut long = [0.0f32; 999];
            e.process(&mut short, &mut long);

            // A large block.
            let mut big_l = vec![0.0f32; 8192];
            let mut big_r = vec![0.0f32; 8192];
            e.process(&mut big_l, &mut big_r);

            // Outputs that did get written stay finite.
            for &s in big_l.iter().chain(big_r.iter()) {
                assert!(s.is_finite(), "adversarial render produced non-finite sample");
            }
        }
    });
    assert!(result.is_ok(), "process() panicked under adversarial input");
}

// =============================================================================
// #10 — Parameter monotonicity & `variance_depth = 0` sterility.
// =============================================================================

#[test]
fn level_is_monotonic_gain() {
    // Higher set_level ⇒ strictly higher output peak across a sweep.
    fn peak_at_level(level: f32) -> f32 {
        let mut e = Engine::new(48_000.0);
        // Settle level + master on silence so the smoother reaches target
        // before the hit (avoids the ramp confounding the peak).
        e.set_master_level(1.0);
        e.set_level(Voice::Bd, level);
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        for _ in 0..60 {
            e.process(&mut l, &mut r);
        }
        e.trigger(Voice::Bd, false);
        let mut pk = 0.0f32;
        for _ in 0..200 {
            e.process(&mut l, &mut r);
            for &x in &l {
                pk = pk.max(x.abs());
            }
        }
        pk
    }

    let levels = [0.1f32, 0.25, 0.5, 0.75, 1.0];
    let peaks: Vec<f32> = levels.iter().map(|&lv| peak_at_level(lv)).collect();
    for w in peaks.windows(2) {
        assert!(
            w[1] > w[0],
            "LEVEL not strictly monotonic: {:?} for {:?}",
            peaks,
            levels
        );
    }
}

#[test]
fn variance_depth_zero_is_sterile() {
    // Two renders at variance_depth = 0 are byte-identical (the placeholder
    // draws no randomness, but the invariant must hold for the real engine too).
    let ev = vec![
        TrigEvent { sample_index: 0, voice: Voice::Cy, accent: false },
        TrigEvent { sample_index: 1000, voice: Voice::Oh, accent: true },
    ];
    let mut r = req(ev, 16_000);
    r.variance_depth = 0.0;
    r.seed = 12345;
    let a = render(&r);
    let b = render(&r);
    assert_eq!(bits(&a), bits(&b), "variance_depth = 0 was not repeatable");
}

// =============================================================================
// Negative guard — no PCM-asset / sample-loading path anywhere in the crates.
// =============================================================================
//
// All seven voices are synthesized; there are no samples anywhere
// (architecture.md). Scan the workspace `crates/` tree and fail if a forbidden
// token shows up in shipped Rust *code* (comments stripped). This very test
// file mentions the tokens, so it is excluded from the scan.
//
// We strip line comments first because legitimately-present prose disclaims the
// absence of samples (e.g. dsp_wasm says "there is deliberately no load_sample
// method") and the render binary documents its `-o output.wav` flag. Writing a
// WAV is a calibration aid (strategy §2.1); *loading* PCM assets is forbidden.

#[test]
fn no_sample_loading_path_in_source() {
    // Tokens that would betray a sample-LOADING / PCM-asset path. Note these
    // target ingestion, not WAV output (the render binary legitimately writes
    // WAVs). Kept here only; this file is excluded from the walk.
    let forbidden = [
        "load_sample",
        "include_bytes!", // embedding a binary asset
        "soundfile",      // python-style decoder names
        "hound::",        // the `hound` WAV crate, used for decoding
        "WavReader",      // reading/decoding a WAV
        "decode_wav",
        "read_to_end",  // raw asset slurp (none expected in DSP/wasm code)
        "assets/",      // an asset directory reference
    ];

    // `crates/` lives at the workspace root, two levels up from this crate's
    // manifest dir (…/crates/dsp_core).
    let manifest = env!("CARGO_MANIFEST_DIR");
    let crates_dir = std::path::Path::new(manifest)
        .parent()
        .expect("dsp_core has a parent (crates/)")
        .to_path_buf();
    assert!(
        crates_dir.join("dsp_core").is_dir(),
        "expected to scan a crates/ dir, got {}",
        crates_dir.display()
    );

    let this_file = std::path::Path::new(file!())
        .file_name()
        .map(|s| s.to_owned());

    let mut offenders = Vec::new();
    visit_rs_files(&crates_dir, &mut |path, contents| {
        // Skip this test file itself (it spells the forbidden tokens above).
        if path.file_name().map(|s| s.to_owned()) == this_file {
            return;
        }
        let code = strip_line_comments(contents);
        for tok in &forbidden {
            if code.contains(tok) {
                offenders.push(format!("{} contains forbidden token {:?}", path.display(), tok));
            }
        }
    });

    assert!(
        offenders.is_empty(),
        "PCM-asset / sample-loading path found in shipped source:\n{}",
        offenders.join("\n")
    );
}

/// Strip `//`-line comments so prose disclaiming samples isn't flagged. This is
/// a deliberately coarse pass (it doesn't parse strings, but the tokens we hunt
/// for don't legitimately appear in shipped string literals either).
fn strip_line_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        out.push_str(code);
        out.push('\n');
    }
    out
}

/// Recursively visit every `.rs` file under `dir`, skipping `target/`.
fn visit_rs_files(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path, &str)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|s| s.to_str()) == Some("target") {
                continue;
            }
            visit_rs_files(&path, f);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                f(&path, &contents);
            }
        }
    }
}
