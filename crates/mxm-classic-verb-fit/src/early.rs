//! The onset, the direct sound and what follows it first: prominence, the direct-to-reverberant
//! ratio, pre-delay and the early reflections (`research:effects/feedback-delay-network-reverb.md`
//! §9.1, §5).
//!
//! All of it reads the broadband energy summed over channels, so a stereo response has one onset
//! and one direct sound.

use crate::buffer::{self, OutOfMemory};
use crate::math::samples;

/// The energy of each frame, summed over channels. Reserved fallibly: the response's length sizes it.
pub(crate) fn combined_energy(channels: &[&[f32]]) -> Result<Vec<f64>, OutOfMemory> {
    let mut energy = buffer::filled(channels[0].len(), 0.0f64)?;
    for channel in channels {
        for (sum, &sample) in energy.iter_mut().zip(channel.iter()) {
            let sample = f64::from(sample);
            *sum += sample * sample;
        }
    }
    Ok(energy)
}

fn db_ratio(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}

/// Where the response starts and peaks.
pub(crate) struct Direct {
    pub onset: usize,
    pub peak: usize,
    pub peak_energy: f64,
}

/// The first largest energy, and the first sample within [`crate::ONSET_BELOW_PEAK_DB`] of it.
/// Absent for digital silence.
pub(crate) fn find_direct(energy: &[f64]) -> Option<Direct> {
    let (peak, peak_energy) =
        energy.iter().enumerate().fold(
            (0, 0.0f64),
            |best, (i, &e)| if e > best.1 { (i, e) } else { best },
        );
    if peak_energy <= 0.0 {
        return None;
    }
    let threshold = peak_energy * db_ratio(-crate::ONSET_BELOW_PEAK_DB);
    let onset = energy.iter().position(|&e| e >= threshold).unwrap_or(peak);
    Some(Direct {
        onset,
        peak,
        peak_energy,
    })
}

/// The direct window's first and last sample: ±[`crate::DIRECT_HALF_WINDOW_S`] about the peak.
fn direct_window(peak: usize, rate: f64, len: usize) -> (usize, usize) {
    let half = samples(crate::DIRECT_HALF_WINDOW_S, rate);
    (peak.saturating_sub(half), (peak + half).min(len - 1))
}

/// The direct sound's mean square over its window against the mean square of what precedes the
/// onset by more than [`crate::ONSET_GUARD_S`]; capped at 300 dB.
pub(crate) fn onset_prominence_db(energy: &[f64], direct: &Direct, rate: f64) -> Option<f64> {
    let guard = samples(crate::ONSET_GUARD_S, rate);
    let lead_end = direct.onset.checked_sub(guard)?;
    if lead_end < guard {
        return None;
    }
    let preceding = energy[..lead_end].iter().sum::<f64>() / lead_end as f64;
    let (first, last) = direct_window(direct.peak, rate, energy.len());
    let direct_level = energy[first..=last].iter().sum::<f64>() / (last + 1 - first) as f64;
    let floor = direct_level * db_ratio(-300.0);
    Some(10.0 * (direct_level / preceding.max(floor)).log10())
}

/// The ACE challenge's direct-to-reverberant ratio, energy before the window counted as reverberant.
pub(crate) fn direct_to_reverberant_db(energy: &[f64], peak: usize, rate: f64) -> Option<f64> {
    let (first, last) = direct_window(peak, rate, energy.len());
    let direct: f64 = energy[first..=last].iter().sum();
    let reverberant: f64 =
        energy[..first].iter().sum::<f64>() + energy[last + 1..].iter().sum::<f64>();
    if reverberant <= 0.0 {
        return None;
    }
    let ratio = 10.0 * (direct / reverberant).log10();
    ratio.is_finite().then_some(ratio)
}

/// The first reflection energy after the direct window: the first [`crate::PRE_DELAY_WINDOW_S`]
/// sliding window whose energy reaches both [`crate::EARLY_MIN_LEVEL_DB`] of the direct sound's in
/// the same width and [`crate::NOISE_MARGIN_DB`] over the noise floor. Its arrival is the first
/// sample in that window at or above the window's mean, and the pre-delay is the largest energy
/// within [`crate::REFLECTION_HALF_WIDTH_S`] of that arrival — a reflection's peak, measured the way
/// the direct sound's is. A sample index.
pub(crate) fn pre_delay(
    energy: &[f64],
    peak: usize,
    rate: f64,
    noise_energy: f64,
) -> Option<usize> {
    let width = samples(crate::PRE_DELAY_WINDOW_S, rate);
    let reference_first = peak.saturating_sub(width / 2);
    let reference: f64 = energy[reference_first..(reference_first + width).min(energy.len())]
        .iter()
        .sum();
    let threshold = (reference * db_ratio(crate::EARLY_MIN_LEVEL_DB))
        .max(noise_energy * width as f64 * db_ratio(crate::NOISE_MARGIN_DB));

    let (_, direct_last) = direct_window(peak, rate, energy.len());
    let mut start = direct_last + 1;
    if start + width > energy.len() {
        return None;
    }
    let mut running: f64 = energy[start..start + width].iter().sum();
    loop {
        if running >= threshold {
            // The running sum drifts by rounding; the decision is taken on the exact sum.
            let window = &energy[start..start + width];
            let exact: f64 = window.iter().sum();
            if exact >= threshold && exact > 0.0 {
                let mean = exact / width as f64;
                let arrival = start + window.iter().position(|&e| e >= mean)?;
                let reach =
                    (arrival + samples(crate::REFLECTION_HALF_WIDTH_S, rate) + 1).min(energy.len());
                // The first largest energy within reach of the arrival.
                let mut largest = arrival;
                for i in arrival + 1..reach {
                    if energy[i] > energy[largest] {
                        largest = i;
                    }
                }
                return Some(largest);
            }
        }
        if start + width >= energy.len() {
            return None;
        }
        running += energy[start + width] - energy[start];
        start += 1;
    }
}

/// An early reflection found in the samples.
pub(crate) struct Peak {
    pub index: usize,
    pub level_db: f64,
    pub channel_level_db: Vec<Option<f64>>,
}

/// The strongest local maxima between the direct window and `window_end` (a sample index).
///
/// A candidate is the first largest energy within ±[`crate::REFLECTION_HALF_WIDTH_S`]; it counts if
/// its energy over that width reaches [`crate::EARLY_MIN_LEVEL_DB`] of the direct sound's and
/// [`crate::NOISE_MARGIN_DB`] over the noise floor. The [`crate::MAX_EARLY_REFLECTIONS`] strongest are
/// kept, in time order.
pub(crate) fn early_reflections(
    channels: &[&[f32]],
    energy: &[f64],
    peak: usize,
    rate: f64,
    noise_energy: f64,
    window_end: usize,
) -> Vec<Peak> {
    let half = samples(crate::REFLECTION_HALF_WIDTH_S, rate);
    let len = energy.len();
    let span = |centre: usize| (centre.saturating_sub(half), (centre + half + 1).min(len));
    let window_energy = |signal: &dyn Fn(usize) -> f64, centre: usize| {
        let (first, last) = span(centre);
        (first..last).map(signal).sum::<f64>()
    };
    let combined = |i: usize| energy[i];
    let direct = window_energy(&combined, peak);
    let direct_channels: Vec<f64> = channels
        .iter()
        .map(|c| window_energy(&|i: usize| f64::from(c[i]) * f64::from(c[i]), peak))
        .collect();
    let level_floor = direct * db_ratio(crate::EARLY_MIN_LEVEL_DB);
    let noise_floor = noise_energy * (2 * half + 1) as f64 * db_ratio(crate::NOISE_MARGIN_DB);

    let (_, direct_last) = direct_window(peak, rate, len);
    let first = direct_last + 1;
    let end = window_end.min(len);
    let mut candidates: Vec<(usize, f64)> = Vec::new();
    for i in first..end {
        let e = energy[i];
        if e <= 0.0 {
            continue;
        }
        let (lo, hi) = span(i);
        let is_peak = (lo..i).all(|j| energy[j] < e) && (i + 1..hi).all(|j| energy[j] <= e);
        if !is_peak {
            continue;
        }
        let around = window_energy(&combined, i);
        if around >= level_floor && around >= noise_floor {
            candidates.push((i, around));
        }
    }
    // Strongest first; an exact tie keeps the earlier.
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    candidates.truncate(crate::MAX_EARLY_REFLECTIONS);
    candidates.sort_by_key(|&(i, _)| i);

    candidates
        .into_iter()
        .map(|(index, around)| Peak {
            index,
            level_db: 10.0 * (around / direct).log10(),
            channel_level_db: channels
                .iter()
                .zip(&direct_channels)
                .map(|(c, &reference)| {
                    (reference > 0.0).then(|| {
                        let e = window_energy(&|i: usize| f64::from(c[i]) * f64::from(c[i]), index);
                        10.0 * (e.max(reference * db_ratio(-300.0)) / reference).log10()
                    })
                })
                .collect(),
        })
        .collect()
}
