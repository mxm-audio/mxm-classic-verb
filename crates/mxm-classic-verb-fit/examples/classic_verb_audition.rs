//! Renders one dry phrase through an impulse response, convolved offline, and through the reverb
//! fitted to it, as two WAV files for a person to compare at P3.5 (plan §5.1).
//!
//! ```bash
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_audition -- --out <folder> [--name <name>] <file>
//! ```
//!
//! Writes `<name>-response.wav` and `<name>-fitted.wav`, 32-bit float at the response's rate, matched
//! in loudness. **Its output is never committed**: the root's pre-release check forbids audio in the
//! tree. There is no default output folder. The convolution is an audition harness, not the
//! `mxm-convolution` product (plan §1).

#[path = "classic_verb_io/mod.rs"]
mod classic_verb_io;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use classic_verb_io::{load, slug};
use mxm_classic_verb_dsp::{Controls, Engine};
use mxm_classic_verb_fit::fit;
use mxm_measure::spectrum::{fft, ifft};

const USAGE: &str = "usage: classic_verb_audition --out <folder> [--name <name>] <file>";

/// A dry test phrase: percussive noise hits, a plucked line, a held chord, then silence for the
/// tails. Deterministic, so two auditions of one response are the same file.
fn phrase(rate: f64) -> Vec<f64> {
    let n = (rate * 7.0) as usize;
    let mut x = vec![0.0f64; n];
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut noise = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    };
    for hit in 0..4 {
        let start = (rate * (0.1 + 0.4 * hit as f64)) as usize;
        for i in 0..(rate * 0.08) as usize {
            x[start + i] += 0.7 * (-(i as f64) / (rate * 0.012)).exp() * noise();
        }
    }
    for (k, hz) in [392.0, 523.3, 659.3, 783.99].iter().enumerate() {
        let start = (rate * (1.9 + 0.3 * k as f64)) as usize;
        for i in 0..(rate * 0.25) as usize {
            let t = i as f64 / rate;
            x[start + i] += 0.35 * (-t / 0.06).exp() * (std::f64::consts::TAU * hz * t).sin();
        }
    }
    let chord = (rate * 3.4) as usize;
    for i in 0..(rate * 1.2) as usize {
        let t = i as f64 / rate;
        let env = (t / 0.02).min(1.0) * (1.0 - t / 1.2).max(0.0);
        let v: f64 = [220.0, 277.18, 329.63]
            .iter()
            .map(|f| (std::f64::consts::TAU * f * t).sin())
            .sum();
        x[chord + i] += 0.18 * env * v;
    }
    x
}

/// `dry` convolved with `response`, through the transform.
fn convolve(dry: &[f64], response: &[f64]) -> Vec<f64> {
    let len = dry.len() + response.len() - 1;
    let n = len.next_power_of_two();
    let transform = |x: &[f64]| {
        let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
        re[..x.len()].copy_from_slice(x);
        fft(&mut re, &mut im).expect("a power-of-two length transforms");
        (re, im)
    };
    let (xr, xi) = transform(dry);
    let (hr, hi) = transform(response);
    let (mut re, mut im): (Vec<f64>, Vec<f64>) = (0..n)
        .map(|k| (xr[k] * hr[k] - xi[k] * hi[k], xr[k] * hi[k] + xi[k] * hr[k]))
        .unzip();
    ifft(&mut re, &mut im).expect("a power-of-two length transforms");
    re.truncate(len);
    re
}

fn write(path: &Path, left: &[f64], right: &[f64], rate: u32, scale: f64) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for (l, r) in left.iter().zip(right) {
        writer
            .write_sample((l * scale) as f32)
            .map_err(|e| e.to_string())?;
        writer
            .write_sample((r * scale) as f32)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}

fn rms(channels: &[&[f64]]) -> f64 {
    let (sum, count) = channels.iter().fold((0.0, 0usize), |(s, c), x| {
        (s + x.iter().map(|v| v * v).sum::<f64>(), c + x.len())
    });
    (sum / count.max(1) as f64).sqrt()
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut out, mut name, mut inputs) = (None, None, Vec::new());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--name" => name = args.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => inputs.push(PathBuf::from(arg)),
        }
    }
    let (Some(out), [input]) = (out, inputs.as_slice()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned());
    let name = slug(name.as_deref().or(stem.as_deref()).unwrap_or("response"));
    let decoded = match load(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{name}: {e}");
            return ExitCode::from(1);
        }
    };
    let rate = decoded.sample_rate;
    let channels: Vec<&[f32]> = decoded.channels.iter().map(Vec::as_slice).collect();
    let fitted = match fit(&channels, rate as f32) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{name}: refused: {e}");
            return ExitCode::from(1);
        }
    };

    let dry = phrase(f64::from(rate));
    // The response from a millisecond before its direct sound, scaled so the direct sound peaks at
    // one — the level the fitted render's own direct sound has.
    let direct =
        (f64::from(fitted.report.target.onset.direct_s) * f64::from(rate)).round() as usize;
    let start = direct.saturating_sub(rate as usize / 1000);
    let peak = decoded
        .channels
        .iter()
        .map(|c| c[direct].abs())
        .fold(0.0f32, f32::max)
        .max(1e-9);
    let response: Vec<Vec<f64>> = decoded
        .channels
        .iter()
        .map(|c| c[start..].iter().map(|&v| f64::from(v / peak)).collect())
        .collect();
    let tail = response[0].len();
    let convolved: Vec<Vec<f64>> = response.iter().map(|h| convolve(&dry, h)).collect();
    let (conv_l, conv_r) = (&convolved[0], convolved.get(1).unwrap_or(&convolved[0]));

    // The fitted reverb: the dry phrase, plus the wet signal at the gain the verification read the
    // response's DRR at.
    let mut engine = Engine::new(rate as f32);
    engine.set_space(fitted.space);
    engine.set_controls(fitted.controls.apply(&Controls {
        mix: 1.0,
        duck: 0.0,
        ..Controls::default()
    }));
    let gain = f64::from(fitted.report.wet_gain);
    let mono = decoded.channels.len() == 1;
    let dry_gain = f64::from(std::f32::consts::FRAC_PI_2.cos());
    let (mut fit_l, mut fit_r) = (Vec::new(), Vec::new());
    for i in 0..dry.len() + tail {
        let x = dry.get(i).copied().unwrap_or(0.0);
        let (l, r) = engine.process(x as f32, x as f32);
        let (l, r) = (f64::from(l) - x * dry_gain, f64::from(r) - x * dry_gain);
        let (l, r) = if mono {
            (0.5 * (l + r), 0.5 * (l + r))
        } else {
            (l, r)
        };
        fit_l.push(x + gain * l);
        fit_r.push(x + gain * r);
    }

    // Matched loudness, and a common scale so neither clips.
    let (response_rms, fitted_rms) = (rms(&[conv_l, conv_r]), rms(&[&fit_l, &fit_r]));
    let match_gain = if fitted_rms > 0.0 {
        response_rms / fitted_rms
    } else {
        1.0
    };
    let peak_of = |xs: &[&[f64]], g: f64| {
        xs.iter()
            .flat_map(|x| x.iter())
            .fold(0.0f64, |m, v| m.max((v * g).abs()))
    };
    let loudest = peak_of(&[conv_l, conv_r], 1.0).max(peak_of(&[&fit_l, &fit_r], match_gain));
    let scale = if loudest > 0.9 { 0.9 / loudest } else { 1.0 };

    if let Err(e) = std::fs::create_dir_all(&out)
        .map_err(|e| e.to_string())
        .and_then(|_| {
            write(
                &out.join(format!("{name}-response.wav")),
                conv_l,
                conv_r,
                rate,
                scale,
            )
        })
        .and_then(|_| {
            write(
                &out.join(format!("{name}-fitted.wav")),
                &fit_l,
                &fit_r,
                rate,
                scale * match_gain,
            )
        })
    {
        eprintln!("{name}: could not be written: {e}");
        return ExitCode::from(1);
    }
    println!(
        "{name}: written; confidence {:.2}, decay {:.2} s, size {:.1} ms",
        fitted.report.target.validity.confidence,
        fitted.controls.decay_s,
        1000.0 * fitted.controls.size_s
    );
    ExitCode::SUCCESS
}
