//! Writes synthetic impulse responses of the DSP crate's four hand-authored spaces, and a manifest
//! naming them, so `classic_verb_generate` runs end to end without anybody's response files.
//!
//! ```bash
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_synthetic_responses -- --out <folder outside the repository>
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_generate -- --manifest <that folder>/manifest.tsv
//! ```
//!
//! Each response is the engine's own render (`tests/known`, the round trip's): 50 ms of silence, a
//! unit direct sound, the wet signal at half level and a noise floor at −100 dB, stereo at 48 kHz.
//! Decay, Size, Diffusion and Pre-delay are the provisional factory presets of the same names
//! (`plugins/mxm-classic-verb/src/preset.rs`, 2026-09-14), everything else the fitted centre. **They
//! are chosen inputs, not claims**: nothing holds them equal to the presets, and nothing needs to.

#[path = "classic_verb_io/mod.rs"]
mod classic_verb_io;
#[path = "../tests/known/mod.rs"]
mod known;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use classic_verb_io::generate::inside;
use known::{Known, render};
use mxm_classic_verb_dsp::Space;

const USAGE: &str = "usage: classic_verb_synthetic_responses --out <folder outside the repository>";

const RATE: u32 = 48_000;

struct Synthetic {
    id: &'static str,
    name: &'static str,
    family: &'static str,
    space: Space,
    decay_s: f32,
    size_s: f32,
    diffusion: f32,
    pre_delay_s: f32,
    /// Long enough for the slowest band to fall to the noise floor.
    seconds: f32,
}

const RESPONSES: [Synthetic; 4] = [
    Synthetic {
        id: "room",
        name: "Room",
        family: "room",
        space: Space::ROOM,
        decay_s: 0.9,
        size_s: 0.018,
        diffusion: 0.7,
        pre_delay_s: 0.004,
        seconds: 2.5,
    },
    Synthetic {
        id: "chamber",
        name: "Chamber",
        family: "chamber",
        space: Space::CHAMBER,
        decay_s: 1.6,
        size_s: 0.030,
        diffusion: 0.75,
        pre_delay_s: 0.012,
        seconds: 3.0,
    },
    Synthetic {
        id: "hall",
        name: "Hall",
        family: "hall",
        space: Space::HALL,
        decay_s: 2.8,
        size_s: 0.070,
        diffusion: 0.75,
        pre_delay_s: 0.025,
        seconds: 6.0,
    },
    Synthetic {
        id: "plate",
        name: "Plate",
        family: "plate",
        space: Space::PLATE,
        decay_s: 2.2,
        size_s: 0.035,
        diffusion: 1.0,
        pre_delay_s: 0.0,
        seconds: 4.0,
    },
];

fn write_wav(path: &Path, channels: &[Vec<f32>]) -> Result<(), hound::Error> {
    let spec = hound::WavSpec {
        channels: channels.len() as u16,
        sample_rate: RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for frame in 0..channels[0].len() {
        for channel in channels {
            writer.write_sample(channel[frame])?;
        }
    }
    writer.finalize()
}

fn main() -> ExitCode {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let mut args = std::env::args().skip(1);
    let mut out = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("{USAGE}\nunexpected argument `{other}`");
                return ExitCode::from(2);
            }
        }
    }
    let Some(out) = out else {
        eprintln!("{USAGE}\n--out is required");
        return ExitCode::from(2);
    };
    if inside(&repository, &out) {
        eprintln!("the folder lies inside the repository, which holds no audio");
        return ExitCode::from(2);
    }
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("the folder could not be created: {e}");
        return ExitCode::from(1);
    }
    let out = std::path::absolute(&out).unwrap_or(out);

    let mut manifest = String::from(
        "# Synthetic responses of the four hand-authored spaces, from classic_verb_synthetic_responses.\n# id\tname\tfamily\tresponse\n",
    );
    for response in &RESPONSES {
        let channels = render(&Known {
            space: response.space,
            decay_s: response.decay_s,
            size_s: response.size_s,
            diffusion: response.diffusion,
            pre_delay_s: response.pre_delay_s,
            rate: RATE as f32,
            mono: false,
            seconds: response.seconds,
        });
        let path = out.join(format!("synthetic-{}.wav", response.id));
        if let Err(e) = write_wav(&path, &channels) {
            eprintln!("{}: the WAV could not be written: {e}", response.id);
            return ExitCode::from(1);
        }
        manifest.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            response.id,
            response.name,
            response.family,
            path.display()
        ));
        println!("{}: {} s written", response.id, response.seconds);
    }
    let manifest_path = out.join("manifest.tsv");
    if let Err(e) = std::fs::write(&manifest_path, manifest) {
        eprintln!("the manifest could not be written: {e}");
        return ExitCode::from(1);
    }
    println!(
        "manifest: {}\nnext: cargo run -p mxm-classic-verb-fit --release --example classic_verb_generate -- --manifest \"{}\"",
        manifest_path.display(),
        manifest_path.display()
    );
    ExitCode::SUCCESS
}
