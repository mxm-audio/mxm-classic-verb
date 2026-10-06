//! The echo density profile and the mixing time read from it.
//!
//! Abel and Huang 2006 (`research:effects/feedback-delay-network-reverb.md` §9.6):
//! η(t) = [1/erfc(1/√2)] / (2δ+1) · Σ_{τ=t−δ}^{t+δ} 1{|h(τ)| > σ}, with σ the standard deviation over
//! the same window. A Gaussian field reads one. The mixing time is where the profile *first reaches
//! one*, the criterion Lindau, Kosanke and Weinzierl 2012 found predicts the perceived mixing time
//! better than the paper's own.

use crate::math::samples;

/// erfc(1/√2): the fraction of a Gaussian's samples more than one standard deviation from its mean.
const GAUSSIAN_BEYOND_ONE_SIGMA: f64 = 0.317_310_507_862_914_1;

/// One window's statistics.
pub(crate) struct Window {
    pub mean_square: f64,
    /// The standard deviation, the window's mean removed.
    pub sigma: f64,
    /// The fraction of samples whose magnitude exceeds `sigma`, over a Gaussian's.
    pub density: f64,
}

/// One window's mean square and normalised echo density — the profile's, and the tail texture's
/// (`texture`).
pub(crate) fn window(window: &[f32]) -> Window {
    let count = window.len() as f64;
    let mean = window.iter().map(|&s| f64::from(s)).sum::<f64>() / count;
    let mean_square = window
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / count;
    let sigma = (mean_square - mean * mean).max(0.0).sqrt();
    let beyond = window
        .iter()
        .filter(|&&s| f64::from(s).abs() > sigma)
        .count();
    Window {
        mean_square,
        sigma,
        density: beyond as f64 / (count * GAUSSIAN_BEYOND_ONE_SIGMA),
    }
}

pub(crate) struct Profile {
    /// (seconds after the direct sound, density) per hop.
    pub points: Vec<(f64, Option<f64>)>,
    pub mixing_time_s: Option<f64>,
}

/// The profile from the direct sound for [`crate::ECHO_DENSITY_SPAN_S`], per channel and averaged.
///
/// `noise_energy` is the broadband noise floor's mean square summed over channels; a window whose
/// summed mean square is not [`crate::NOISE_MARGIN_DB`] above it has no density, because noise alone
/// is Gaussian and would read as a mixed field.
pub(crate) fn echo_density(
    channels: &[&[f32]],
    rate: f64,
    direct: usize,
    noise_energy: f64,
) -> Profile {
    let frames = channels[0].len();
    let half = samples(crate::ECHO_DENSITY_WINDOW_S, rate) / 2;
    let hop = samples(crate::ECHO_DENSITY_HOP_S, rate);
    let end = direct
        .saturating_add(samples(crate::ECHO_DENSITY_SPAN_S, rate))
        .min(frames);
    let floor = noise_energy * 10f64.powf(crate::NOISE_MARGIN_DB / 10.0);

    let mut points = Vec::new();
    let mut mixing_time_s = None;
    let mut centre = direct;
    while centre < end {
        let first = centre.saturating_sub(half);
        let last = (centre + half + 1).min(frames);
        let mut mean_square_sum = 0.0;
        let mut density_sum = 0.0;
        for channel in channels {
            let stats = window(&channel[first..last]);
            density_sum += stats.density;
            mean_square_sum += stats.mean_square;
        }
        let density = (mean_square_sum > floor && mean_square_sum > 0.0)
            .then(|| density_sum / channels.len() as f64);
        let time = (centre - direct) as f64 / rate;
        if mixing_time_s.is_none() && density.is_some_and(|d| d >= 1.0) {
            mixing_time_s = Some(time);
        }
        points.push((time, density));
        centre += hop;
    }
    Profile {
        points,
        mixing_time_s,
    }
}
