//! Responses rendered by the engine itself from a known space and known controls, for the round trip
//! (`plans/plan-mxm-classic-verb.md` §4.6). Nothing here reads a file.
#![allow(dead_code)]

use mxm_classic_verb_dsp::{Controls, DecayShape, Engine, Space};

/// What a round trip renders from, and must recover.
#[derive(Clone, Copy, Debug)]
pub struct Known {
    pub space: Space,
    pub decay_s: f32,
    pub size_s: f32,
    pub diffusion: f32,
    pub pre_delay_s: f32,
    pub rate: f32,
    pub mono: bool,
    pub seconds: f32,
}

/// A space whose earliest sounding tap sits at time zero. A response cannot tell a pre-delay from
/// the same delay added to every tap, so a fit always returns a space of this form, and a round trip
/// renders from one.
pub fn canonical(mut space: Space) -> Space {
    let sounding = |t: &mxm_classic_verb_dsp::EarlyTap| t.gain_l != 0.0 || t.gain_r != 0.0;
    let first = space
        .early
        .iter()
        .filter(|t| sounding(t))
        .map(|t| t.time)
        .fold(f32::INFINITY, f32::min);
    if first.is_finite() {
        for t in space.early.iter_mut().filter(|t| sounding(t)) {
            t.time -= first;
        }
    }
    space
}

/// The engine's response at the fit's centre (plan §2: every relative control neutral, no
/// modulation, a natural decay): 50 ms of leading silence, a unit direct impulse, the wet signal at
/// half level, and a noise floor at −100 dB.
pub fn render(known: &Known) -> Vec<Vec<f32>> {
    let mut engine = Engine::new(known.rate);
    engine.set_space(known.space);
    engine.set_controls(Controls {
        mix: 1.0,
        decay_s: known.decay_s,
        bass_mult: 1.0,
        treble_mult: 1.0,
        size_s: known.size_s,
        diffusion: known.diffusion,
        pre_delay_s: known.pre_delay_s,
        early_late_db: 0.0,
        shape: DecayShape::Natural,
        mod_depth_s: 0.0,
        width: 1.0,
        tone_low_db: 0.0,
        tone_high_db: 0.0,
        duck: 0.0,
        ..Controls::default()
    });
    let frames = (known.seconds * known.rate) as usize;
    let lead = (0.05 * known.rate) as usize;
    // At Mix one the engine's dry gain is cos(π/2), not exactly zero; it is taken off the impulse.
    let dry = std::f32::consts::FRAC_PI_2.cos();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ((state >> 11) as f64 / (1u64 << 52) as f64 - 1.0) as f32 * 1e-5
    };
    let (mut left, mut right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    for n in 0..frames {
        let x = if n == lead { 1.0 } else { 0.0 };
        let (l, r) = if n >= lead {
            engine.process(x, x)
        } else {
            (0.0, 0.0)
        };
        left[n] = 0.5 * (l - x * dry) + noise() + x;
        right[n] = 0.5 * (r - x * dry) + noise() + x;
    }
    if known.mono {
        vec![
            left.iter()
                .zip(&right)
                .map(|(l, r)| 0.5 * (l + r))
                .collect(),
        ]
    } else {
        vec![left, right]
    }
}
