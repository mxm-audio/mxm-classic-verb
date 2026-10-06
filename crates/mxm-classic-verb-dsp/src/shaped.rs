//! Shaped decays: feed-forward taps whose gains carry the envelope (plan §3.3).
//!
//! A feedback network decays exponentially by nature, so a gated block or a reverse swell comes
//! from outside the loop. This is the structure Sean Costello describes for most reverse and
//! non-linear reverbs — delay lines with many output taps whose gains make the sound fade in or stop
//! — and the level-independent envelope Lexicon's Inverse Room documents
//! (`research:effects/feedback-delay-network-reverb.md` §8). It is linear and time-invariant.

use crate::DecayShape;
use crate::delay::DelayLine;
use crate::filter::Allpass;

/// Taps per channel. **Chosen**: with the two diffusers after them, dense enough to read as a
/// reverberant block across the whole length range, sparse enough to keep "a bit of grain".
pub const SHAPE_TAPS: usize = 96;
/// The shortest and longest shaped decay, in seconds.
pub const SHAPE_MIN_S: f32 = 0.05;
pub const SHAPE_MAX_S: f32 = 2.0;

const DIFFUSER_S_L: [f32; 2] = [0.0051, 0.0079];
const DIFFUSER_S_R: [f32; 2] = [0.0057, 0.0073];
/// **Chosen.**
const DIFFUSER_GAIN: f32 = 0.6;
/// The path's level against the network's. **Chosen.**
const SHAPED_LEVEL: f32 = 0.7;

#[derive(Debug, Clone)]
pub(crate) struct ShapedPath {
    position: [f32; SHAPE_TAPS],
    sign_l: [f32; SHAPE_TAPS],
    sign_r: [f32; SHAPE_TAPS],
    gain_l: [f32; SHAPE_TAPS],
    gain_r: [f32; SHAPE_TAPS],
    diff_l: [Allpass; 2],
    diff_r: [Allpass; 2],
}

/// A small deterministic generator, so every instance has the same taps.
struct Lcg(u32);

impl Lcg {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

impl ShapedPath {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let mut rng = Lcg(0x6d78_6d31);
        let mut position = [0.0; SHAPE_TAPS];
        let mut sign_l = [0.0; SHAPE_TAPS];
        let mut sign_r = [0.0; SHAPE_TAPS];
        for k in 0..SHAPE_TAPS {
            let jitter = 0.8 * (rng.unit() - 0.5);
            position[k] = ((k as f32 + 0.5 + jitter) / SHAPE_TAPS as f32).clamp(0.0, 1.0);
            sign_l[k] = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
            sign_r[k] = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        }
        let mut path = Self {
            position,
            sign_l,
            sign_r,
            gain_l: [0.0; SHAPE_TAPS],
            gain_r: [0.0; SHAPE_TAPS],
            diff_l: core::array::from_fn(|i| Allpass::new(DIFFUSER_S_L[i], sample_rate)),
            diff_r: core::array::from_fn(|i| Allpass::new(DIFFUSER_S_R[i], sample_rate)),
        };
        path.set_shape(DecayShape::Natural);
        path
    }

    /// The envelope at a fraction `u` of the length.
    fn envelope(shape: DecayShape, u: f32) -> f32 {
        match shape {
            DecayShape::Natural => 0.0,
            DecayShape::Gated => 1.0,
            DecayShape::Reverse => u * u,
        }
    }

    pub(crate) fn set_shape(&mut self, shape: DecayShape) {
        let energy: f32 = self
            .position
            .iter()
            .map(|&u| Self::envelope(shape, u).powi(2))
            .sum();
        let norm = if energy > 0.0 {
            SHAPED_LEVEL / energy.sqrt()
        } else {
            0.0
        };
        for k in 0..SHAPE_TAPS {
            let e = Self::envelope(shape, self.position[k]) * norm;
            self.gain_l[k] = e * self.sign_l[k];
            self.gain_r[k] = e * self.sign_r[k];
        }
    }

    pub(crate) fn invalidate(&mut self) {
        for ap in self.diff_l.iter_mut().chain(self.diff_r.iter_mut()) {
            ap.invalidate();
        }
    }

    /// One sample: taps across `length_samples` of input history, starting at `base_age`, scaled by
    /// `input_gain` before the diffusers.
    #[inline]
    pub(crate) fn process(
        &mut self,
        hist_l: &DelayLine,
        hist_r: &DelayLine,
        base_age: f32,
        length_samples: f32,
        input_gain: f32,
    ) -> (f32, f32) {
        let mut l = 0.0;
        let mut r = 0.0;
        for k in 0..SHAPE_TAPS {
            let age = base_age + self.position[k] * length_samples;
            l += self.gain_l[k] * hist_l.read_linear(age);
            r += self.gain_r[k] * hist_r.read_linear(age);
        }
        l *= input_gain;
        r *= input_gain;
        for ap in &mut self.diff_l {
            l = ap.process(l, DIFFUSER_GAIN);
        }
        for ap in &mut self.diff_r {
            r = ap.process(r, DIFFUSER_GAIN);
        }
        (l, r)
    }
}
