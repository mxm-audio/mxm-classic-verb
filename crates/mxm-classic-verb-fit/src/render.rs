//! The engine's response to an impulse, rendered for the search, the width match and the
//! verification.

use mxm_classic_verb_dsp::{Controls, Engine, Space};

use crate::buffer::{self, OutOfMemory};

/// One engine, reused for every render. `reset` parks it, and a parked engine applies a space and
/// controls at once (`crates/mxm-classic-verb-dsp/AGENTS.md`, *Off, parking and the tail*), so a
/// render depends on nothing rendered before it.
pub(crate) struct Renderer {
    engine: Engine,
}

impl Renderer {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            engine: Engine::new(sample_rate),
        }
    }

    /// The wet response of both channels to a unit impulse in both inputs at frame `lead`, `frames`
    /// long. Mix is forced to one and Ducking to zero, so what comes out is the wet signal alone.
    /// Both channels are reserved fallibly, before the engine is touched: a render is as long as the
    /// response in the verification, and as long as its pre-delay in the search.
    pub fn impulse(
        &mut self,
        space: &Space,
        controls: &Controls,
        lead: usize,
        frames: usize,
    ) -> Result<[Vec<f32>; 2], OutOfMemory> {
        let mut left = buffer::filled(frames, 0.0f32)?;
        let mut right = buffer::filled(frames, 0.0f32)?;
        let wet_only = Controls {
            mix: 1.0,
            duck: 0.0,
            ..*controls
        };
        self.engine.reset();
        self.engine.set_space(*space);
        self.engine.set_controls(wet_only);
        // At Mix one the engine's equal-power dry gain is cos(π/2), which is not exactly zero in f32:
        // the impulse's own frame carries that much of the input, and it is taken off here.
        let dry = core::f32::consts::FRAC_PI_2.cos();
        for n in lead..frames {
            let x = if n == lead { 1.0 } else { 0.0 };
            let (l, r) = self.engine.process(x, x);
            let (l, r) = if n == lead {
                (l - dry, r - dry)
            } else {
                (l, r)
            };
            left[n] = l;
            right[n] = r;
            // A parked engine passes its silent input through: every later frame is already zero.
            if n > lead && self.engine.is_parked() {
                break;
            }
        }
        Ok([left, right])
    }
}

/// The stereo render as the response's channels: both for a stereo response, their mean for a mono
/// one, which is what a mono measurement of the same reverb would record. The mean is reserved
/// fallibly.
pub(crate) fn fold(wet: [Vec<f32>; 2], mono: bool) -> Result<Vec<Vec<f32>>, OutOfMemory> {
    let [left, right] = wet;
    Ok(if mono {
        vec![buffer::collected(
            left.len(),
            left.iter().zip(&right).map(|(l, r)| 0.5 * (l + r)),
        )?]
    } else {
        vec![left, right]
    })
}

pub(crate) fn slices(channels: &[Vec<f32>]) -> Vec<&[f32]> {
    channels.iter().map(Vec::as_slice).collect()
}
