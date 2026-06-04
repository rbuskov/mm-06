//! MM-06 offline render binary (architecture.md §"Build & tooling",
//! calibration-and-testing-strategy.md §2.1).
//!
//! Reads a JSON `RenderRequest` (from a file argument or stdin), drives the
//! **same** `dsp_core` scheduler the worklet uses, and writes a deterministic
//! 32-bit-float WAV plus a small sidecar JSON of metrics. With `"voice"` set it
//! renders that voice in isolation (pre-mix, no bus coloration) via
//! `render_voice` — the calibration path, not a shipped output.
//!
//! Usage:
//!   render [input.json] [-o output.wav]
//!   cat req.json | render -o kick.wav

use std::io::{Read, Write};

use dsp_core::{render, render_voice, RenderRequest, TrigEvent, Voice};
use serde::Deserialize;

#[derive(Deserialize)]
struct JsonRequest {
    sample_rate: f64,
    #[serde(default)]
    seed: u64,
    #[serde(default)]
    variance_depth: f32,
    #[serde(default = "default_block")]
    block_size: usize,
    length_samples: usize,
    /// Optional: render this single voice in isolation (calibration).
    #[serde(default)]
    voice: Option<String>,
    #[serde(default)]
    events: Vec<JsonEvent>,
}

fn default_block() -> usize {
    128
}

#[derive(Deserialize)]
struct JsonEvent {
    sample_index: usize,
    voice: String,
    #[serde(default)]
    accent: bool,
}

fn parse_voice(s: &str) -> Result<Voice, String> {
    match s.to_ascii_lowercase().as_str() {
        "bd" => Ok(Voice::Bd),
        "sd" => Ok(Voice::Sd),
        "lt" => Ok(Voice::Lt),
        "ht" => Ok(Voice::Ht),
        "cy" => Ok(Voice::Cy),
        "oh" => Ok(Voice::Oh),
        "ch" => Ok(Voice::Ch),
        other => Err(format!("unknown voice '{other}'")),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("render: error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut input_path: Option<String> = None;
    let mut output_path = String::from("out.wav");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--out" => {
                output_path = args.next().ok_or("-o requires a path")?;
            }
            "-h" | "--help" => {
                println!("usage: render [input.json] [-o output.wav]   (stdin if no input)");
                return Ok(());
            }
            other => input_path = Some(other.to_string()),
        }
    }

    // read the request JSON
    let raw = match input_path {
        Some(p) => std::fs::read_to_string(&p).map_err(|e| format!("reading {p}: {e}"))?,
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| format!("reading stdin: {e}"))?;
            s
        }
    };
    let jr: JsonRequest = serde_json::from_str(&raw).map_err(|e| format!("parsing JSON: {e}"))?;

    let mut events = Vec::with_capacity(jr.events.len());
    for e in &jr.events {
        events.push(TrigEvent {
            sample_index: e.sample_index,
            voice: parse_voice(&e.voice)?,
            accent: e.accent,
        });
    }

    let req = RenderRequest {
        sample_rate: jr.sample_rate,
        seed: jr.seed,
        variance_depth: jr.variance_depth,
        block_size: jr.block_size,
        events,
        length_samples: jr.length_samples,
    };

    let samples = match &jr.voice {
        Some(v) => render_voice(&req, parse_voice(v)?),
        None => render(&req),
    };

    write_wav_f32(&output_path, &samples, jr.sample_rate as u32)?;

    // sidecar metrics JSON
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let rms = {
        let sumsq: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sumsq / samples.len().max(1) as f64).sqrt()
    };
    let sidecar = format!(
        "{{\n  \"output\": {output:?},\n  \"sample_rate\": {sr},\n  \"length_samples\": {len},\n  \"voice\": {voice},\n  \"seed\": {seed},\n  \"variance_depth\": {var},\n  \"peak\": {peak},\n  \"rms\": {rms}\n}}\n",
        output = output_path,
        sr = jr.sample_rate,
        len = samples.len(),
        voice = jr.voice.as_ref().map(|v| format!("{v:?}")).unwrap_or_else(|| "null".into()),
        seed = jr.seed,
        var = jr.variance_depth,
    );
    let sidecar_path = sidecar_path_for(&output_path);
    std::fs::write(&sidecar_path, sidecar).map_err(|e| format!("writing sidecar: {e}"))?;

    eprintln!(
        "render: wrote {output_path} ({} samples @ {} Hz, peak {peak:.4}) + {sidecar_path}",
        samples.len(),
        jr.sample_rate
    );
    Ok(())
}

fn sidecar_path_for(wav: &str) -> String {
    match wav.rfind('.') {
        Some(i) => format!("{}.json", &wav[..i]),
        None => format!("{wav}.json"),
    }
}

/// Minimal 32-bit IEEE-float mono WAV writer (format tag 3). No external deps,
/// so `dsp_core`'s determinism is never at the mercy of an encoder.
fn write_wav_f32(path: &str, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    let channels: u16 = 1;
    let bits: u16 = 32;
    let byte_rate = sample_rate * channels as u32 * (bits / 8) as u32;
    let block_align = channels * (bits / 8);
    let data_len = (samples.len() * 4) as u32;
    let riff_len = 36 + data_len;

    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_len.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    buf.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }

    let mut f = std::fs::File::create(path).map_err(|e| format!("creating {path}: {e}"))?;
    f.write_all(&buf).map_err(|e| format!("writing {path}: {e}"))?;
    Ok(())
}
