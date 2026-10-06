//! Late-field interchannel coherence (`research:effects/feedback-delay-network-reverb.md` §9.7):
//! IACF(τ) = ∫ p_L(t)·p_R(t+τ) dt / √(∫p_L² · ∫p_R²), and IACC its largest magnitude over
//! |τ| ≤ 1 ms, over the late window from 80 ms after the direct sound.

use crate::analysis::{Coherence, Width, WidthNotMeasured};
use crate::math::{clamped_f32, samples};

/// The coherence of the late window, which ends at `end` (a sample index): the broadband noise
/// floor's intersection, past which the window would hold the recording chain's noise rather than
/// the room.
pub(crate) fn late_coherence(channels: &[&[f32]], rate: f64, direct: usize, end: usize) -> Width {
    if channels.len() < 2 {
        return Width::NotMeasured(WidthNotMeasured::Mono);
    }
    let (left, right) = (channels[0], channels[1]);
    let frames = left.len();
    let start = direct + samples(crate::WIDTH_LATE_START_S, rate);
    let stop = end
        .min(frames)
        .min(start + samples(crate::WIDTH_LATE_SPAN_MAX_S, rate));
    if stop <= start || stop - start < samples(crate::WIDTH_LATE_MIN_S, rate) {
        return Width::NotMeasured(WidthNotMeasured::NoLateField);
    }
    let energy = |x: &[f32]| x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>();
    let (left_energy, right_energy) = (energy(&left[start..stop]), energy(&right[start..stop]));
    if left_energy <= 0.0 || right_energy <= 0.0 {
        return Width::NotMeasured(WidthNotMeasured::SilentChannel);
    }
    let norm = (left_energy * right_energy).sqrt();
    let max_lag = samples(crate::WIDTH_MAX_LAG_S, rate);

    let mut best = (0.0f64, 0isize);
    for lag in -(max_lag as isize)..=max_lag as isize {
        // t ranges over the window; t + lag is read from the whole response where it exists.
        let first = if lag < 0 {
            start.max(lag.unsigned_abs())
        } else {
            start
        };
        let last = if lag > 0 {
            stop.min(frames - lag as usize)
        } else {
            stop
        };
        let sum: f64 = if last > first {
            let shifted = (first as isize + lag) as usize;
            left[first..last]
                .iter()
                .zip(&right[shifted..shifted + (last - first)])
                .map(|(&l, &r)| f64::from(l) * f64::from(r))
                .sum()
        } else {
            0.0
        };
        let iacf = sum / norm;
        if iacf.abs() > best.0.abs() {
            best = (iacf, lag);
        }
    }
    Width::Measured(Coherence {
        iacc: clamped_f32(best.0.abs()),
        peak_iacf: clamped_f32(best.0),
        peak_lag_s: clamped_f32(best.1 as f64 / rate),
        window_start_s: clamped_f32((start - direct) as f64 / rate),
        window_end_s: clamped_f32((stop - direct) as f64 / rate),
    })
}
