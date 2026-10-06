//! The fit's calculated parts (plan §4.3): decay through the engine's own decay filter, tone through
//! its shelves and high cut, the early pattern from the measured reflections, the early/late balance,
//! and width.

use core::f64::consts::{PI, TAU};
use core::ops::Range;

use mxm_classic_verb_dsp::space::{HIGH_CUT_OPEN_HZ, MAX_SPACE_TONE_DB, MIN_HIGH_CUT_HZ};
use mxm_classic_verb_dsp::{
    Controls, DECAY_TOP_CROSSOVER_HZ, DecayShape, EARLY_MAX_UNITS, EARLY_TAPS, EarlyTap,
    HIGH_CROSSOVER_HZ, LOW_CROSSOVER_HZ, Space, predicted_decays_s,
};

use crate::analysis::{Analysis, Band, Width};
use crate::band::OctaveFilter;
use crate::fit::{
    BALANCE_WINDOW_S, HIGH_CUT_MIN_IMPROVEMENT_DB, HIGH_CUT_REFINE_STEPS,
    HIGH_CUT_STEPS_PER_OCTAVE, T20_WEIGHT, TONE_POINTS, WIDTH_BISECTIONS,
};
use crate::math::samples;

// ── A small damped least-squares solver ────────────────────────────────────────────────────────

/// Levenberg–Marquardt damped Gauss–Newton over a few parameters, with forward-difference
/// derivatives. Deterministic: a fixed difference step, a fixed iteration cap and no randomness.
pub(crate) fn least_squares(x0: &[f64], residuals: &dyn Fn(&[f64]) -> Vec<f64>) -> Vec<f64> {
    const MAX_ITERATIONS: usize = 60;
    const DIFFERENCE: f64 = 1e-3;
    let n = x0.len();
    let mut x = x0.to_vec();
    if n == 0 {
        return x;
    }
    let mut r = residuals(&x);
    let mut cost = sum_of_squares(&r);
    let mut damping = 1e-3;
    for _ in 0..MAX_ITERATIONS {
        let jacobian: Vec<Vec<f64>> = (0..n)
            .map(|j| {
                let mut probe = x.clone();
                probe[j] += DIFFERENCE;
                residuals(&probe)
                    .iter()
                    .zip(&r)
                    .map(|(a, b)| (a - b) / DIFFERENCE)
                    .collect()
            })
            .collect();
        let normal: Vec<Vec<f64>> = (0..n)
            .map(|a| (0..n).map(|b| dot(&jacobian[a], &jacobian[b])).collect())
            .collect();
        let gradient: Vec<f64> = (0..n).map(|a| -dot(&jacobian[a], &r)).collect();
        let mut improved = false;
        while damping < 1e12 {
            let mut system = normal.clone();
            for (k, row) in system.iter_mut().enumerate() {
                row[k] = normal[k][k] * (1.0 + damping) + 1e-12;
            }
            let Some(step) = solve_linear(system, gradient.clone()) else {
                damping *= 10.0;
                continue;
            };
            let trial: Vec<f64> = x.iter().zip(&step).map(|(a, b)| a + b).collect();
            let trial_r = residuals(&trial);
            let trial_cost = sum_of_squares(&trial_r);
            if trial_cost.is_finite() && trial_cost < cost {
                let moved = step.iter().fold(0.0f64, |m, s| m.max(s.abs()));
                x = trial;
                r = trial_r;
                cost = trial_cost;
                damping = (damping * 0.1).max(1e-9);
                improved = moved >= 1e-9;
                break;
            }
            damping *= 10.0;
        }
        if !improved {
            break;
        }
    }
    x
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn sum_of_squares(r: &[f64]) -> f64 {
    r.iter().map(|v| v * v).sum()
}

/// Gaussian elimination with partial pivoting; absent for a singular system.
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let (upper, lower) = a.split_at_mut(col + 1);
        let pivot_row = &upper[col];
        for (offset, target) in lower.iter_mut().enumerate() {
            let factor = target[col] / pivot_row[col];
            for (value, &p) in target[col..].iter_mut().zip(&pivot_row[col..]) {
                *value -= factor * p;
            }
            b[col + 1 + offset] -= factor * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let tail: f64 = (row + 1..n).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - tail) / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

// ── The model's neutral parts ──────────────────────────────────────────────────────────────────

const SILENT_TAP: EarlyTap = EarlyTap {
    time: 0.0,
    gain_l: 0.0,
    gain_r: 0.0,
};

/// A space with nothing but what a solve varies: no taps, flat tone with the cut open, unit width.
pub(crate) fn neutral_space() -> Space {
    Space {
        early: [SILENT_TAP; EARLY_TAPS],
        early_level: 0.0,
        decay_ratio_low: 1.0,
        decay_ratio_high: 1.0,
        decay_ratio_top: 1.0,
        tone_low_db: 0.0,
        tone_high_db: 0.0,
        high_cut_hz: HIGH_CUT_OPEN_HZ,
        width: 1.0,
    }
}

/// The controls a fit writes (plan §2), with the relative controls neutral, modulation depth zero
/// and the decay natural, before the four absolute controls are filled in.
pub(crate) fn neutral_controls() -> Controls {
    Controls {
        mix: 1.0,
        bass_mult: 1.0,
        treble_mult: 1.0,
        early_late_db: 0.0,
        shape: DecayShape::Natural,
        mod_depth_s: 0.0,
        width: 1.0,
        tone_low_db: 0.0,
        tone_high_db: 0.0,
        duck: 0.0,
        ..Controls::default()
    }
}

// ── Decay ──────────────────────────────────────────────────────────────────────────────────────

/// A band's measured decay time, and how much the fit trusts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecayTarget {
    pub centre_hz: f32,
    pub seconds: f32,
    pub weight: f64,
}

/// Every required band's T30, else its T20 at [`T20_WEIGHT`].
pub(crate) fn decay_targets(bands: &[Band]) -> Vec<DecayTarget> {
    bands
        .iter()
        .filter(|band| band.required)
        .filter_map(|band| {
            let decay = band.decay()?;
            let (seconds, weight) = match (decay.t30_s, decay.t20_s) {
                (Some(t30), _) => (t30, 1.0),
                (None, Some(t20)) => (t20, T20_WEIGHT),
                (None, None) => return None,
            };
            Some(DecayTarget {
                centre_hz: band.centre_hz,
                seconds,
                weight,
            })
        })
        .collect()
}

fn is_low(hz: f32) -> bool {
    hz <= LOW_CROSSOVER_HZ
}

/// The high band's octave, 4 kHz: from the tone's high crossover up to the decay filter's top one.
fn is_high(hz: f32) -> bool {
    (HIGH_CROSSOVER_HZ..DECAY_TOP_CROSSOVER_HZ).contains(&hz)
}

/// The top band's octave, 8 kHz.
fn is_top(hz: f32) -> bool {
    hz >= DECAY_TOP_CROSSOVER_HZ
}

/// A band between the engine's crossovers: 500 Hz, 1 kHz and 2 kHz, where Decay is read.
pub(crate) fn is_mid(hz: f32) -> bool {
    !is_low(hz) && hz < HIGH_CROSSOVER_HZ
}

pub(crate) fn has_mid_band(targets: &[DecayTarget]) -> bool {
    targets.iter().any(|t| is_mid(t.centre_hz))
}

/// Decay and the three band ratios as solved, before any range is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecaySolution {
    pub decay_s: f64,
    pub ratio_low: f64,
    pub ratio_high: f64,
    pub ratio_top: f64,
    pub low_measured: bool,
    pub high_measured: bool,
    pub top_measured: bool,
}

/// The frequencies a band's realised decay is read at, evenly in octaves across this many octaves
/// either side of its centre. **Chosen**: a tenth of an octave apart, across the band and its skirts;
/// `the_decay_reading_is_as_fine_as_it_needs_to_be` measures it against twice as many points.
const DECAY_POINTS: usize = 31;
const DECAY_SPAN_OCTAVES: f64 = 1.5;
/// A point whose weight lies this far under its band's heaviest is left out of the band's energy
/// decay. **Chosen**: 80 dB, so no slower neighbour that could still own −35 dB is dropped.
const DECAY_WEIGHT_FLOOR: f64 = 1e-8;
/// The instants an energy decay is read at between −5 and −35 dB. **Chosen.**
const DECAY_CURVE_SAMPLES: usize = 32;

/// Where each measured band's realised decay is read, and the energy the analyser takes from each
/// point: the band filtered twice, the space's tone, and the frequency — what a late field flat per
/// hertz puts through a grid spaced evenly in octaves, as the tone model's `band_points` weights a band.
///
/// **Why a band, not its centre.** A band's T30 is read from every frequency the analyser's octave
/// passes, and the slower-decaying ones own its late energy. Where the decay changes steeply inside an
/// octave — the top octave of a room with air absorption — the realised T30 reads longer than the
/// closed form at the centre (`crates/mxm-classic-verb-fit/AGENTS.md`, *Decay is solved across each
/// band*).
pub(crate) struct DecayReading {
    hz: Vec<f32>,
    bands: Vec<(Range<usize>, Vec<f64>)>,
}

impl DecayReading {
    /// `tone` is the space's tone as it carries it (`ToneSolution::carried`).
    pub(crate) fn new(targets: &[DecayTarget], rate: f64, tone: (f64, f64, f64)) -> Self {
        Self::with_points(targets, rate, tone, DECAY_POINTS)
    }

    fn with_points(
        targets: &[DecayTarget],
        rate: f64,
        (low_db, high_db, high_cut_hz): (f64, f64, f64),
        points: usize,
    ) -> Self {
        let mut hz = Vec::new();
        let mut bands = Vec::with_capacity(targets.len());
        for target in targets {
            let centre = f64::from(target.centre_hz);
            let filter = OctaveFilter::new(centre, rate);
            let start = hz.len();
            let mut weights = Vec::with_capacity(points);
            for k in 0..points {
                let octaves = DECAY_SPAN_OCTAVES * (2.0 * k as f64 / (points - 1) as f64 - 1.0);
                let f = centre * 2f64.powf(octaves);
                if f >= 0.45 * rate {
                    break;
                }
                hz.push(f as f32);
                weights.push(
                    filter.gain_at(f, rate).powi(4)
                        * tone_power(low_db, high_db, high_cut_hz, f, rate)
                        * f,
                );
            }
            bands.push((start..hz.len(), weights));
        }
        Self { hz, bands }
    }

    /// Each band's T30, in the targets' order, for a space and controls: every point's closed-form
    /// decay (`predicted_decays_s`), its energy falling from its weight, summed over the band.
    pub(crate) fn t30s(&self, space: &Space, controls: &Controls, rate: f32) -> Vec<f64> {
        let mut decays = vec![0.0f32; self.hz.len()];
        predicted_decays_s(space, controls, rate, &self.hz, &mut decays);
        self.bands
            .iter()
            .map(|(range, weights)| energy_decay_t30(&decays[range.clone()], weights))
            .collect()
    }
}

/// The T30 of a sum of exponential energy decays — each point's weight, falling 60 dB over its T60 —
/// read as the analyser reads a band: the least-squares line through the Schroeder curve from −5 to
/// −35 dB, extrapolated to 60 dB. The curve is exact here, so it is read at [`DECAY_CURVE_SAMPLES`]
/// instants spread evenly across that span.
fn energy_decay_t30(t60_s: &[f32], weights: &[f64]) -> f64 {
    let per_t60 = 6.0 * core::f64::consts::LN_10;
    let heaviest = weights.iter().copied().fold(0.0, f64::max);
    // Each point as its energy still to come at time zero and its time constant: the integral of
    // w·e^(−t/τ) from t on is w·τ·e^(−t/τ).
    let terms: Vec<(f64, f64)> = t60_s
        .iter()
        .zip(weights)
        .filter(|&(_, &w)| w > heaviest * DECAY_WEIGHT_FLOOR)
        .map(|(&t60, &w)| {
            let tau = f64::from(t60).clamp(1e-6, 1e6) / per_t60;
            (w * tau, tau)
        })
        .collect();
    let total: f64 = terms.iter().map(|&(energy, _)| energy).sum();
    let level = |t: f64| {
        let remaining: f64 = terms.iter().map(|&(e, tau)| e * (-t / tau).exp()).sum();
        10.0 * (remaining / total).max(1e-300).log10()
    };
    // By the slowest term's own crossing every term has fallen at least as far, so the curve's
    // crossing lies between zero and that.
    let slowest = terms.iter().map(|&(_, tau)| tau).fold(0.0, f64::max);
    let crossing = |db: f64| {
        let (mut early, mut late) = (0.0, slowest * per_t60 * (-db / 60.0));
        for _ in 0..60 {
            let t = 0.5 * (early + late);
            if level(t) > db {
                early = t;
            } else {
                late = t;
            }
        }
        0.5 * (early + late)
    };
    let (start, end) = (crossing(-5.0), crossing(-35.0));
    let n = DECAY_CURVE_SAMPLES as f64;
    let (mut st, mut sy, mut stt, mut sty) = (0.0, 0.0, 0.0, 0.0);
    for k in 0..DECAY_CURVE_SAMPLES {
        let t = start + (end - start) * k as f64 / (DECAY_CURVE_SAMPLES - 1) as f64;
        let y = level(t);
        st += t;
        sy += y;
        stt += t * t;
        sty += t * y;
    }
    -60.0 * (n * stt - st * st) / (n * sty - st * sy)
}

/// The space and controls a decay solve varies, everything else neutral.
pub(crate) fn decay_model(decay_s: f64, ratios: [f64; 3], size_s: f64) -> (Space, Controls) {
    (
        Space {
            decay_ratio_low: ratios[0] as f32,
            decay_ratio_high: ratios[1] as f32,
            decay_ratio_top: ratios[2] as f32,
            ..neutral_space()
        },
        Controls {
            decay_s: decay_s as f32,
            size_s: size_s as f32,
            ..neutral_controls()
        },
    )
}

/// Decay and the band ratios whose **realised** octave decays best match the measured ones, in the
/// weighted least-squares sense on log decay time, at this Size (plan §4.3: band targets are solved
/// through the decay filter, not assumed), each band read across its octave ([`DecayReading`]) under
/// `tone`, the space's tone as carried. The low ratio stays one with no measured band on its side;
/// an upper ratio with none follows the other upper ratio, or stays one if neither was measured.
/// Absent when no mid band was measured.
pub(crate) fn solve_decay(
    targets: &[DecayTarget],
    size_s: f64,
    rate: f32,
    tone: (f64, f64, f64),
) -> Option<DecaySolution> {
    let weighted_mean = |pick: &dyn Fn(f32) -> bool| {
        let (weight, sum) = targets
            .iter()
            .filter(|t| pick(t.centre_hz))
            .fold((0.0, 0.0), |(w, s), t| {
                (w + t.weight, s + t.weight * f64::from(t.seconds).ln())
            });
        (weight > 0.0).then(|| (sum / weight).exp())
    };
    let mid = weighted_mean(&is_mid)?;
    let low = weighted_mean(&is_low);
    let high = weighted_mean(&is_high);
    let top = weighted_mean(&is_top);
    let x0: Vec<f64> = [
        Some(mid.ln()),
        low.map(|v| (v / mid).ln()),
        high.map(|v| (v / mid).ln()),
        top.map(|v| (v / mid).ln()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let unpack = |x: &[f64]| {
        let mut next = 1;
        let mut take = |measured: bool| {
            measured.then(|| {
                next += 1;
                x[next - 1].exp()
            })
        };
        let (l, h, t) = (
            take(low.is_some()),
            take(high.is_some()),
            take(top.is_some()),
        );
        (
            x[0].exp(),
            [
                l.unwrap_or(1.0),
                h.or(t).unwrap_or(1.0),
                t.or(h).unwrap_or(1.0),
            ],
        )
    };
    let reading = DecayReading::new(targets, f64::from(rate), tone);
    let residuals = |x: &[f64]| {
        let (decay_s, ratios) = unpack(x);
        let (space, controls) = decay_model(decay_s, ratios, size_s);
        targets
            .iter()
            .zip(reading.t30s(&space, &controls, rate))
            .map(|(t, realised)| {
                t.weight.sqrt() * (realised.clamp(1e-6, 1e6).ln() - f64::from(t.seconds).ln())
            })
            .collect()
    };
    let x = least_squares(&x0, &residuals);
    let (decay_s, [ratio_low, ratio_high, ratio_top]) = unpack(&x);
    Some(DecaySolution {
        decay_s,
        ratio_low,
        ratio_high,
        ratio_top,
        low_measured: low.is_some(),
        high_measured: high.is_some(),
        top_measured: top.is_some(),
    })
}

// ── Tone ───────────────────────────────────────────────────────────────────────────────────────

/// The range the tone solve searches: twice the space's bound. Past the bound a first-order shelf
/// still moves the band levels a little, so an unbounded solve can wander to hundreds of decibels;
/// held here, a clamp reports how far past the bound the measured curve asked, up to that much again.
const TONE_SEARCH_DB: f64 = 2.0 * MAX_SPACE_TONE_DB as f64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ToneSolution {
    /// The shelves as solved, inside ±[`TONE_SEARCH_DB`].
    pub low_db: f64,
    pub high_db: f64,
    /// The high cut's corner as the space will store it, hertz; [`HIGH_CUT_OPEN_HZ`] when open.
    pub high_cut_hz: f64,
    pub bands_used: usize,
    /// The root-mean-square distance from the measured tone of the curve as the space carries it —
    /// the shelves inside the space's bound and the cut as fitted — in dB.
    pub residual_db: Option<f64>,
    /// The same with the cut open: what the shelves alone leave.
    pub open_residual_db: Option<f64>,
}

impl ToneSolution {
    /// The tone as a space carries it — the shelves inside the space's bound, the cut inside its own —
    /// as `search::model` stores it: shelves in dB and the corner in hertz.
    pub(crate) fn carried(&self) -> (f64, f64, f64) {
        let bound = |v: f64, lo: f32, hi: f32| f64::from((v as f32).clamp(lo, hi));
        (
            bound(self.low_db, -MAX_SPACE_TONE_DB, MAX_SPACE_TONE_DB),
            bound(self.high_db, -MAX_SPACE_TONE_DB, MAX_SPACE_TONE_DB),
            bound(self.high_cut_hz, MIN_HIGH_CUT_HZ, HIGH_CUT_OPEN_HZ),
        )
    }
}

impl Default for ToneSolution {
    /// Nothing measured: flat shelves, the cut open, no residual.
    fn default() -> Self {
        Self {
            low_db: 0.0,
            high_db: 0.0,
            high_cut_hz: f64::from(HIGH_CUT_OPEN_HZ),
            bands_used: 0,
            residual_db: None,
            open_residual_db: None,
        }
    }
}

fn one_pole_pole(hz: f64, rate: f64) -> f64 {
    (-TAU * hz.clamp(1.0, 0.45 * rate) / rate).exp()
}

fn lowpass_response(pole: f64, w: f64) -> (f64, f64) {
    let (re, im) = (1.0 - pole * w.cos(), pole * w.sin());
    let den = re * re + im * im;
    ((1.0 - pole) * re / den, -(1.0 - pole) * im / den)
}

/// The shelves' power at a frequency: the engine's low shelf at `LOW_CROSSOVER_HZ` into its high
/// shelf at `HIGH_CROSSOVER_HZ`, both first order.
fn tilt_power(low_db: f64, high_db: f64, hz: f64, rate: f64) -> f64 {
    let (gl, gh) = (10f64.powf(low_db / 20.0), 10f64.powf(high_db / 20.0));
    let w = TAU * hz / rate;
    let lo = lowpass_response(one_pole_pole(f64::from(LOW_CROSSOVER_HZ), rate), w);
    let hi = lowpass_response(one_pole_pole(f64::from(HIGH_CROSSOVER_HZ), rate), w);
    let (a, b) = (1.0 + (gl - 1.0) * lo.0, (gl - 1.0) * lo.1);
    let (c, d) = (gh + (1.0 - gh) * hi.0, (1.0 - gh) * hi.1);
    (a * c - b * d).powi(2) + (a * d + b * c).powi(2)
}

/// The space's high cut's power at a frequency: the engine's second-order Butterworth by the
/// prewarped bilinear transform, so exactly `1/(1 + (tan(πf/f_s)/tan(πf_c/f_s))⁴)`, its corner held
/// below Nyquist as the engine holds it; one when open.
fn cut_power(high_cut_hz: f64, hz: f64, rate: f64) -> f64 {
    if high_cut_hz.is_nan() || high_cut_hz >= f64::from(HIGH_CUT_OPEN_HZ) {
        return 1.0;
    }
    let corner = high_cut_hz.clamp(1.0, 0.45 * rate);
    let ratio = (PI * hz / rate).tan() / (PI * corner / rate).tan();
    1.0 / (1.0 + ratio.powi(4))
}

/// The output tone's power at a frequency: the shelves into the high cut
/// (`crates/mxm-classic-verb-dsp/AGENTS.md`, *A space is shape*). Its normalisation at 1 kHz cancels
/// in a level relative to 1 kHz.
fn tone_power(low_db: f64, high_db: f64, high_cut_hz: f64, hz: f64, rate: f64) -> f64 {
    tilt_power(low_db, high_db, hz, rate) * cut_power(high_cut_hz, hz, rate)
}

/// Frequencies across two octaves about a band's centre, each weighted by the analyser's
/// filtered-twice band power and by frequency, which is what a white late field puts through it.
fn band_points(centre_hz: f64, rate: f64) -> Vec<(f64, f64)> {
    let filter = OctaveFilter::new(centre_hz, rate);
    (0..TONE_POINTS)
        .filter_map(|k| {
            let hz = centre_hz * 2f64.powf(-1.0 + 2.0 * k as f64 / (TONE_POINTS - 1) as f64);
            (hz < 0.45 * rate).then(|| (hz, filter.gain_at(hz, rate).powi(4) * hz))
        })
        .collect()
}

/// A band's energy under the tone, in dB on a scale every band shares. The weights carry the band's
/// shape and its width in hertz, and are **not** normalised per band: the engine's late field is flat
/// per hertz, so the analyser reads it 3 dB louder for every octave up, and the model has to as well.
fn band_tone_db(
    points: &[(f64, f64)],
    low_db: f64,
    high_db: f64,
    high_cut_hz: f64,
    rate: f64,
) -> f64 {
    let power: f64 = points
        .iter()
        .map(|&(hz, g)| g * tone_power(low_db, high_db, high_cut_hz, hz, rate))
        .sum();
    10.0 * power.max(1e-300).log10()
}

/// The shelves solved for one high-cut corner, and how close the curve the space would carry lies to
/// the measured one.
#[derive(Clone, Copy, Debug)]
struct ToneCandidate {
    low_db: f64,
    high_db: f64,
    high_cut_hz: f64,
    /// Root-mean-square dB, with the shelves inside the space's bound.
    residual_db: f64,
}

/// The shelves' low and high gains and the high cut's corner whose band levels, relative to the
/// 1 kHz band's, best match the measured tone curve. A shelf with no usable band on its side of 1 kHz
/// stays at 0 dB, and with no usable band above 1 kHz the cut stays open.
///
/// **A band whose decay is not one slope is left out.** Its level is the extrapolation of a line
/// through a bend, which can sit many decibels from the late field's real level, and one such band
/// was seen driving a shelf past −500 dB. The gains are also held inside ±[`TONE_SEARCH_DB`] while
/// they are solved, so no band can run a shelf away.
///
/// **The cut is a one-dimensional search around the shelves' least squares.** For each corner the
/// shelves are solved as they always were, and the corner is scored on the curve the space will
/// carry — its shelves clamped to the space's bound — so a corner that only matches with a shelf past
/// the bound is not preferred for a match the engine cannot render. [`search_cut`] finds the corner,
/// and **the cut stays open unless it brings that curve at least [`HIGH_CUT_MIN_IMPROVEMENT_DB`]
/// closer** than the shelves alone. An open cut solves exactly the shelves it solved before the cut
/// existed.
pub(crate) fn solve_tone(analysis: &Analysis, rate: f64) -> ToneSolution {
    let measured: Vec<(f32, f64)> = analysis
        .bands
        .iter()
        .filter(|band| band.centre_hz != 1000.0)
        .filter_map(|band| {
            let decay = band.decay()?;
            if decay.straightness.is_some_and(|s| !s.single_slope) {
                return None;
            }
            Some((band.centre_hz, f64::from(decay.level_re_1k_db?)))
        })
        .collect();
    if measured.is_empty() {
        return ToneSolution::default();
    }
    let reference = band_points(1000.0, rate);
    let bands: Vec<(Vec<(f64, f64)>, f64)> = measured
        .iter()
        .map(|&(hz, tone)| (band_points(f64::from(hz), rate), tone))
        .collect();
    let mean = |pick: &dyn Fn(f32) -> bool| {
        let chosen: Vec<f64> = measured.iter().filter(|m| pick(m.0)).map(|m| m.1).collect();
        (!chosen.is_empty()).then(|| chosen.iter().sum::<f64>() / chosen.len() as f64)
    };
    let low = mean(&|hz| hz < 1000.0);
    let high = mean(&|hz| hz > 1000.0);
    let x0: Vec<f64> = [low, high].into_iter().flatten().collect();
    let unpack = |x: &[f64]| {
        let low_db = if low.is_some() { x[0] } else { 0.0 };
        let high_db = match (low.is_some(), high.is_some()) {
            (_, false) => 0.0,
            (true, true) => x[1],
            (false, true) => x[0],
        };
        (low_db, high_db)
    };
    let residuals_at = |low_db: f64, high_db: f64, high_cut_hz: f64| {
        let at_reference = band_tone_db(&reference, low_db, high_db, high_cut_hz, rate);
        bands
            .iter()
            .map(|(points, tone)| {
                band_tone_db(points, low_db, high_db, high_cut_hz, rate) - at_reference - tone
            })
            .collect::<Vec<f64>>()
    };
    let bound = f64::from(MAX_SPACE_TONE_DB);
    let shelves_at = |high_cut_hz: f64| {
        let residuals = |x: &[f64]| {
            let (low_db, high_db) = unpack(x);
            residuals_at(
                low_db.clamp(-TONE_SEARCH_DB, TONE_SEARCH_DB),
                high_db.clamp(-TONE_SEARCH_DB, TONE_SEARCH_DB),
                high_cut_hz,
            )
        };
        let x: Vec<f64> = least_squares(&x0, &residuals)
            .iter()
            .map(|v| v.clamp(-TONE_SEARCH_DB, TONE_SEARCH_DB))
            .collect();
        let (low_db, high_db) = unpack(&x);
        let r = residuals_at(
            low_db.clamp(-bound, bound),
            high_db.clamp(-bound, bound),
            high_cut_hz,
        );
        ToneCandidate {
            low_db,
            high_db,
            high_cut_hz,
            residual_db: (sum_of_squares(&r) / r.len() as f64).sqrt(),
        }
    };
    let open = shelves_at(f64::from(HIGH_CUT_OPEN_HZ));
    let chosen = match high {
        Some(_) => {
            let cut = search_cut(&shelves_at);
            if open.residual_db - cut.residual_db > HIGH_CUT_MIN_IMPROVEMENT_DB {
                cut
            } else {
                open
            }
        }
        None => open,
    };
    ToneSolution {
        low_db: chosen.low_db,
        high_db: chosen.high_db,
        high_cut_hz: chosen.high_cut_hz,
        bands_used: measured.len(),
        residual_db: Some(chosen.residual_db),
        open_residual_db: Some(open.residual_db),
    }
}

/// The corner from [`MIN_HIGH_CUT_HZ`] up to just below open whose tone lies closest to the measured
/// one: a grid of [`HIGH_CUT_STEPS_PER_OCTAVE`] corners an octave, then [`HIGH_CUT_REFINE_STEPS`]
/// golden-section steps between the best grid corner's neighbours, keeping the best corner seen. On a
/// tie the higher corner — the lighter cut — is kept. Corners are searched in octaves, and each one
/// tried is rounded to `f32` first, the value the space will store, so what is scored is what the
/// engine renders. Deterministic.
fn search_cut(shelves_at: &dyn Fn(f64) -> ToneCandidate) -> ToneCandidate {
    let floor = f64::from(MIN_HIGH_CUT_HZ);
    let step = 1.0 / HIGH_CUT_STEPS_PER_OCTAVE as f64;
    let octaves = (f64::from(HIGH_CUT_OPEN_HZ) / floor).log2();
    // Every grid corner lies strictly below open, and the refinement reaches half a step past the
    // last of them, still below it.
    let corners = (octaves / step).ceil() as usize;
    let top = (corners - 1) as f64 * step + 0.5 * step;
    let at = |octave: f64| shelves_at(f64::from((floor * octave.exp2()) as f32));
    let better = |a: &ToneCandidate, b: &ToneCandidate| a.residual_db < b.residual_db;

    let grid: Vec<ToneCandidate> = (0..corners).map(|k| at(k as f64 * step)).collect();
    let mut index = 0;
    for (k, candidate) in grid.iter().enumerate() {
        if !better(&grid[index], candidate) {
            index = k;
        }
    }
    let mut best = grid[index];

    let inverse_golden = (5f64.sqrt() - 1.0) / 2.0;
    let (mut a, mut b) = (
        (index as f64 - 1.0).max(0.0) * step,
        ((index + 1) as f64 * step).min(top),
    );
    let mut c = b - inverse_golden * (b - a);
    let mut d = a + inverse_golden * (b - a);
    let (mut at_c, mut at_d) = (at(c), at(d));
    for _ in 0..HIGH_CUT_REFINE_STEPS {
        if at_c.residual_db <= at_d.residual_db {
            (b, d, at_d) = (d, c, at_c);
            c = b - inverse_golden * (b - a);
            at_c = at(c);
        } else {
            (a, c, at_c) = (c, d, at_d);
            d = a + inverse_golden * (b - a);
            at_d = at(d);
        }
        for candidate in [at_c, at_d] {
            if better(&candidate, &best) {
                best = candidate;
            }
        }
    }
    best
}

// ── The early pattern ──────────────────────────────────────────────────────────────────────────

/// A measured reflection as a tap would carry it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Arrival {
    /// After the pre-delay.
    pub after_pre_s: f64,
    pub level_db: f64,
    pub gains: [f64; 2],
}

pub(crate) fn arrivals(analysis: &Analysis, pre_delay_s: f64, mono: bool) -> Vec<Arrival> {
    analysis
        .early
        .reflections
        .iter()
        .map(|r| {
            let amplitude = |db: Option<f32>| db.map_or(0.0, |db| 10f64.powf(f64::from(db) / 20.0));
            let gains = if mono {
                let a = amplitude(Some(r.level_db));
                [a, a]
            } else {
                [
                    amplitude(r.channel_level_db.first().copied().flatten()),
                    amplitude(r.channel_level_db.get(1).copied().flatten()),
                ]
            };
            Arrival {
                after_pre_s: (f64::from(r.delay_s) - pre_delay_s).max(0.0),
                level_db: f64::from(r.level_db),
                gains,
            }
        })
        .collect()
}

/// The taps a Size can carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TapSet {
    pub taps: [EarlyTap; EARLY_TAPS],
    /// Reflections carried as taps.
    pub kept: usize,
    /// Reflections later than `EARLY_MAX_UNITS` of this Size after the pre-delay, which no tap can
    /// reach.
    pub beyond_reach: usize,
}

/// The strongest [`EARLY_TAPS`] reflections within reach of this Size, in units of it, their gains
/// normalised so the loudest is one. `shift` is how far the pre-delay control sits before the
/// measured pre-delay, which every tap's time carries.
pub(crate) fn tap_set(arrivals: &[Arrival], size_s: f64, shift_s: f64) -> TapSet {
    let reach = f64::from(EARLY_MAX_UNITS) * size_s;
    let mut inside: Vec<&Arrival> = arrivals
        .iter()
        .filter(|a| a.after_pre_s + shift_s <= reach)
        .collect();
    let beyond_reach = arrivals.len() - inside.len();
    inside.sort_by(|a, b| {
        b.level_db
            .total_cmp(&a.level_db)
            .then(a.after_pre_s.total_cmp(&b.after_pre_s))
    });
    inside.truncate(EARLY_TAPS);
    inside.sort_by(|a, b| a.after_pre_s.total_cmp(&b.after_pre_s));
    let loudest = inside.iter().flat_map(|a| a.gains).fold(0.0f64, f64::max);
    let mut taps = [SILENT_TAP; EARLY_TAPS];
    if loudest > 0.0 {
        for (tap, a) in taps.iter_mut().zip(&inside) {
            *tap = EarlyTap {
                time: (((a.after_pre_s + shift_s) / size_s) as f32).min(EARLY_MAX_UNITS),
                gain_l: (a.gains[0] / loudest) as f32,
                gain_r: (a.gains[1] / loudest) as f32,
            };
        }
    }
    TapSet {
        taps,
        kept: if loudest > 0.0 { inside.len() } else { 0 },
        beyond_reach,
    }
}

// ── The early/late balance ─────────────────────────────────────────────────────────────────────

/// The windows the early-to-late energy ratio is read over, in frames after the direct sound.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BalanceWindows {
    pub early: Range<usize>,
    pub late: Range<usize>,
}

/// Early: from the end of the direct-sound window to [`BALANCE_WINDOW_S`] after the pre-delay.
/// Late: the next [`BALANCE_WINDOW_S`].
pub(crate) fn balance_windows(pre_delay_s: f64, rate: f64) -> BalanceWindows {
    let first = samples(crate::DIRECT_HALF_WINDOW_S, rate) + 1;
    let split = (((pre_delay_s + BALANCE_WINDOW_S) * rate).round() as usize).max(first);
    BalanceWindows {
        early: first..split,
        late: split..split + samples(BALANCE_WINDOW_S, rate),
    }
}

fn clipped(len: usize, direct: usize, window: &Range<usize>) -> Range<usize> {
    let start = (direct + window.start).min(len);
    start..(direct + window.end).min(len)
}

fn window_sum(a: &[&[f32]], b: &[&[f32]], direct: usize, window: &Range<usize>) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let span = clipped(x.len().min(y.len()), direct, window);
            x[span.clone()]
                .iter()
                .zip(&y[span])
                .map(|(p, q)| f64::from(*p) * f64::from(*q))
                .sum::<f64>()
        })
        .sum()
}

/// Early energy over late energy, the broadband noise floor's mean square subtracted from each.
/// Absent when the late window holds nothing above the floor.
pub(crate) fn energy_ratio(
    channels: &[&[f32]],
    direct: usize,
    windows: &BalanceWindows,
    noise_energy: f64,
) -> Option<f64> {
    let len = channels[0].len();
    let net = |window: &Range<usize>| {
        let frames = clipped(len, direct, window).len() as f64;
        (window_sum(channels, channels, direct, window) - noise_energy * frames).max(0.0)
    };
    let (early, late) = (net(&windows.early), net(&windows.late));
    (late > 0.0).then(|| early / late)
}

/// The early level at which `level·early + late` reads `target` as its early-to-late ratio. Both
/// windows' energies are quadratic in the level, so it is a root, not a search. Zero where the late
/// field alone already carries at least that much early energy, and very large where no level can
/// reach it; the caller's bound applies either way.
pub(crate) fn solve_early_level(
    early: &[&[f32]],
    late: &[&[f32]],
    windows: &BalanceWindows,
    target: f64,
) -> f64 {
    let e = |w: &Range<usize>| window_sum(early, early, 0, w);
    let l = |w: &Range<usize>| window_sum(late, late, 0, w);
    let x = |w: &Range<usize>| window_sum(early, late, 0, w);
    if e(&windows.early) + e(&windows.late) <= 0.0 {
        return 0.0;
    }
    let a2 = e(&windows.early) - target * e(&windows.late);
    let a1 = 2.0 * (x(&windows.early) - target * x(&windows.late));
    let a0 = l(&windows.early) - target * l(&windows.late);
    if a0 >= 0.0 {
        return 0.0;
    }
    if a2 > 0.0 {
        return (-a1 + (a1 * a1 - 4.0 * a2 * a0).max(0.0).sqrt()) / (2.0 * a2);
    }
    if a1 > 0.0 {
        return -a0 / a1;
    }
    1e6
}

// ── Width ──────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WidthSolution {
    pub width: f64,
    pub peak_iacf: f32,
}

/// The space width at which the render's late field reads the target's signed peak IACF.
///
/// Width scales only the wet side signal, after everything else (plan §2), so one render at width
/// one gives the mid and side signals and every other width is `mid ± width·side`, exactly. The
/// peak falls from one at width zero through zero towards −1 as width grows, so a bracket and a
/// bisection find it.
pub(crate) fn solve_width(
    wet: &[Vec<f32>; 2],
    rate: f64,
    end: usize,
    target: f32,
) -> Option<WidthSolution> {
    let mid: Vec<f32> = wet[0]
        .iter()
        .zip(&wet[1])
        .map(|(l, r)| 0.5 * (l + r))
        .collect();
    let side: Vec<f32> = wet[0]
        .iter()
        .zip(&wet[1])
        .map(|(l, r)| 0.5 * (l - r))
        .collect();
    let peak_at = |w: f64| -> Option<f32> {
        let w = w as f32;
        let left: Vec<f32> = mid.iter().zip(&side).map(|(m, s)| m + w * s).collect();
        let right: Vec<f32> = mid.iter().zip(&side).map(|(m, s)| m - w * s).collect();
        match crate::width::late_coherence(&[&left, &right], rate, 0, end) {
            Width::Measured(c) => Some(c.peak_iacf),
            Width::NotMeasured(_) => None,
        }
    };
    if target >= peak_at(0.0)? {
        return Some(WidthSolution {
            width: 0.0,
            peak_iacf: peak_at(0.0)?,
        });
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let mut at_hi = peak_at(hi)?;
    while at_hi > target && hi < 16.0 {
        lo = hi;
        hi *= 2.0;
        at_hi = peak_at(hi)?;
    }
    if at_hi > target {
        return Some(WidthSolution {
            width: hi,
            peak_iacf: at_hi,
        });
    }
    for _ in 0..WIDTH_BISECTIONS {
        let middle = 0.5 * (lo + hi);
        if peak_at(middle)? > target {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    let width = 0.5 * (lo + hi);
    Some(WidthSolution {
        width,
        peak_iacf: peak_at(width)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{DECAY_POINTS, DecayReading, DecayTarget, decay_model, tone_power};
    use mxm_classic_verb_dsp::space::{HIGH_CUT_OPEN_HZ, MIN_HIGH_CUT_HZ};
    use mxm_classic_verb_dsp::tone_magnitude;

    /// The tone solve's shelves and high cut are a copy in f64, so its least squares stays in one
    /// precision. This holds the copy to the DSP crate's public closed form: a change to the engine's
    /// tone fails here rather than silently skewing every fitted space's tone.
    fn every_band(seconds: f32) -> Vec<DecayTarget> {
        crate::OCTAVE_BANDS_HZ
            .iter()
            .map(|&centre_hz| DecayTarget {
                centre_hz,
                seconds,
                weight: 1.0,
            })
            .collect()
    }

    #[test]
    fn the_decay_reading_is_as_fine_as_it_needs_to_be() {
        // A dark space — the upper bands falling to a third of mid, under a darkening tone — read at
        // the chosen points and at twice as many; and a flat one read against the closed form at each
        // centre, which a band with nothing changing inside it should barely move.
        let size_s = f64::from(mxm_classic_verb_dsp::size_for_total_delay_s(1.0));
        let (dark_tone, flat_tone) = ((3.0, -12.0, 6_000.0), (0.0, 0.0, 20_000.0));
        let finer = 2 * DECAY_POINTS - 1;
        let mut worst = (0.0f64, 0.0f64);
        for rate in [44_100.0f32, 96_000.0] {
            let targets = every_band(2.0);
            let (dark, controls) = decay_model(2.0, [1.2, 0.6, 0.3], size_s);
            let chosen = DecayReading::new(&targets, f64::from(rate), dark_tone)
                .t30s(&dark, &controls, rate);
            let fine = DecayReading::with_points(&targets, f64::from(rate), dark_tone, finer)
                .t30s(&dark, &controls, rate);
            let (flat, controls) = decay_model(2.0, [1.0, 1.0, 1.0], size_s);
            let read = DecayReading::new(&targets, f64::from(rate), flat_tone)
                .t30s(&flat, &controls, rate);
            for (k, hz) in crate::OCTAVE_BANDS_HZ.into_iter().enumerate() {
                let centre = f64::from(mxm_classic_verb_dsp::predicted_decay_s(
                    &flat, &controls, rate, hz,
                ));
                println!(
                    "{rate} Hz, {hz} Hz band: dark {:.4} s at {DECAY_POINTS} points, {:.4} s at {finer}; flat read {:.4} s, centre {centre:.4} s",
                    chosen[k], fine[k], read[k]
                );
                worst.0 = worst.0.max((chosen[k] / fine[k] - 1.0).abs());
                worst.1 = worst.1.max((read[k] / centre - 1.0).abs());
            }
        }
        assert!(worst.0 < 0.002, "points: {worst:?}");
        assert!(worst.1 < 0.03, "flat against centre: {worst:?}");
    }

    #[test]
    fn the_tone_model_is_the_engines_public_tone_curve() {
        // Open; the floor; a corner inside the analysed bands; and one held below Nyquist at 44.1 kHz.
        let cuts = [
            f64::from(HIGH_CUT_OPEN_HZ),
            f64::from(MIN_HIGH_CUT_HZ),
            2_500.0,
            7_000.0,
            19_999.0,
        ];
        for rate in [44_100.0_f64, 48_000.0, 96_000.0] {
            for (low_db, high_db) in [(0.0, 0.0), (12.0, -12.0), (-24.0, 18.0), (48.0, -48.0)] {
                for cut_hz in cuts {
                    let reference = tone_power(low_db, high_db, cut_hz, 1_000.0, rate);
                    for hz in [63.0, 125.0, 500.0, 2_000.0, 8_000.0, 16_000.0] {
                        let model_db = 10.0
                            * (tone_power(low_db, high_db, cut_hz, hz, rate) / reference).log10();
                        let engine = tone_magnitude(
                            low_db as f32,
                            high_db as f32,
                            cut_hz as f32,
                            rate as f32,
                            hz as f32,
                        );
                        let engine_db = 20.0 * f64::from(engine).log10();
                        assert!(
                            (model_db - engine_db).abs() < 0.01,
                            "{rate} Hz, {low_db}/{high_db} dB, cut {cut_hz} Hz, at {hz} Hz: model {model_db} dB, engine {engine_db} dB"
                        );
                    }
                }
            }
        }
    }
}
