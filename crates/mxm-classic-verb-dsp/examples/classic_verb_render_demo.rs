//! Renders the hand-authored spaces and the shaped decays to WAV, so a person can hear P1.
//!
//! `cargo run -p mxm-classic-verb-dsp --release --example classic_verb_render_demo`
//! writes into `target/classic_verb_demo/` (ignored by git) and prints each render's figures.

use mxm_classic_verb_dsp::{Controls, DecayShape, Engine, Space};
use std::path::{Path, PathBuf};

const RATE: f32 = 48_000.0;

/// A dry phrase: four percussive hits of decaying noise, then a held chord, then silence.
fn phrase() -> Vec<f32> {
    let n = (RATE * 6.0) as usize;
    let mut x = vec![0.0f32; n];
    let mut s = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        ((s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    };
    for hit in 0..4 {
        let start = (RATE * (0.1 + 0.35 * hit as f32)) as usize;
        for i in 0..(RATE * 0.08) as usize {
            let env = (-(i as f32) / (RATE * 0.012)).exp();
            x[start + i] += 0.8 * env * rnd();
        }
    }
    let chord_start = (RATE * 1.8) as usize;
    for i in 0..(RATE * 1.2) as usize {
        let t = i as f32 / RATE;
        let env = (t / 0.02).min(1.0) * (1.0 - t / 1.2).max(0.0);
        let v: f32 = [220.0f32, 277.2, 329.6]
            .iter()
            .map(|f| (std::f32::consts::TAU * f * t).sin())
            .sum();
        x[chord_start + i] += 0.2 * env * v;
    }
    x
}

fn render(name: &str, space: Space, controls: Controls, dry: &[f32], out_dir: &Path) {
    let mut e = Engine::new(RATE);
    e.set_space(space);
    e.set_controls(controls);
    let mut interleaved = Vec::with_capacity(dry.len() * 2);
    let mut peak = 0.0f32;
    for &v in dry {
        let (l, r) = e.process(v, v);
        peak = peak.max(l.abs()).max(r.abs());
        interleaved.push(l);
        interleaved.push(r);
    }
    let scale = if peak > 0.95 { 0.95 / peak } else { 1.0 };
    for s in &mut interleaved {
        *s *= scale;
    }
    let path = out_dir.join(format!("{name}.wav"));
    // The collection's encoder, whose 16-bit rule is the one `mxm-measure`'s retired writer used.
    mxm_audio_file::write(
        &path,
        &interleaved,
        2,
        RATE as u32,
        mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
    )
    .expect("the demo is written");
    let d = e.composed_decay_s();
    println!(
        "{name:>18}: decay low/mid/high/top {:.2}/{:.2}/{:.2}/{:.2} s, peak {peak:.3}, scaled ×{scale:.2}, parked at end: {}",
        d.low,
        d.mid,
        d.high,
        d.top,
        e.is_parked()
    );
}

fn main() {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/classic_verb_demo");
    std::fs::create_dir_all(&out_dir).expect("the output folder is created");
    let dry = phrase();
    let base = Controls {
        mix: 0.35,
        ..Controls::default()
    };
    render(
        "room",
        Space::ROOM,
        Controls {
            decay_s: 0.9,
            size_s: 0.018,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "chamber",
        Space::CHAMBER,
        Controls {
            decay_s: 1.6,
            size_s: 0.03,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "hall",
        Space::HALL,
        Controls {
            decay_s: 2.8,
            size_s: 0.07,
            pre_delay_s: 0.025,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "plate",
        Space::PLATE,
        Controls {
            decay_s: 2.2,
            size_s: 0.035,
            diffusion: 1.0,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "hall_modulated",
        Space::HALL,
        Controls {
            decay_s: 3.5,
            size_s: 0.08,
            mod_depth_s: 0.0015,
            mod_rate_hz: 0.7,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "glass_treble",
        Space::HALL,
        Controls {
            decay_s: 2.5,
            size_s: 0.06,
            treble_mult: 3.0,
            bass_mult: 0.5,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "resonator",
        Space::ROOM,
        Controls {
            decay_s: 3.0,
            size_s: 0.003,
            diffusion: 0.2,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "gated",
        Space::ROOM,
        Controls {
            shape: DecayShape::Gated,
            decay_s: 0.35,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "reverse",
        Space::HALL,
        Controls {
            shape: DecayShape::Reverse,
            decay_s: 0.8,
            ..base
        },
        &dry,
        &out_dir,
    );
    render(
        "slapback",
        Space::HALL,
        Controls {
            decay_s: 1.2,
            size_s: 0.09,
            early_late_db: 18.0,
            diffusion: 0.1,
            ..base
        },
        &dry,
        &out_dir,
    );
    println!("written to {}", out_dir.display());
}
