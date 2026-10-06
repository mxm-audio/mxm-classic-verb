//! A decay read from an energy signal: Lundeby's noise floor, Schroeder integration, and the decay
//! times, straightness and initial level read off the curve.
//!
//! Every step cites `research:effects/feedback-delay-network-reverb.md`. Lundeby, Vigran, Bietz and
//! Vorländer 1995 was not obtained there; its steps (§9.3) are carried by Guski and Vorländer 2014,
//! and the constants below are those steps' numbers, not choices of ours.

use std::f64::consts::LN_10;

use crate::DecayMethod;
use crate::analysis::DecayFailure;
use crate::buffer::{self, OutOfMemory};
use crate::math::{Line, LineFit, energy_db, samples};

/// §9.3 step 1, in a band: blocks of (800 / f + 10) ms.
pub(crate) fn band_block_s(centre_hz: f64) -> f64 {
    (800.0 / centre_hz + 10.0) / 1000.0
}

/// §9.3 step 2 takes the noise from the last tenth of the blocks, so fewer than ten blocks leave no
/// block to take it from.
const MIN_BLOCKS: usize = 10;
/// §9.3 step 2.
const NOISE_TAIL_FRACTION: f64 = 0.1;
/// §9.3 step 3: the first line runs to the last block above noise + 10 dB …
const FIRST_FIT_MARGIN_DB: f64 = 10.0;
/// … and the method aborts if that range falls less than 5 dB.
const MIN_FIRST_RANGE_DB: f64 = 5.0;
/// §9.3 step 5: five blocks per 10 dB of decay.
const BLOCKS_PER_10_DB: f64 = 5.0;
/// §9.3 step 7: the noise is taken from 90 % of the length at the latest …
const NOISE_REGION_LATEST_FRACTION: f64 = 0.9;
/// … or from where the line sits 10 dB below the crossing level, whichever comes first.
const SAFETY_MARGIN_DB: f64 = 10.0;
/// §9.3 step 8: the late line is fitted from noise + 30 dB …
const FIT_TOP_DB: f64 = 30.0;
/// … down to noise + 10 dB.
const FIT_BOTTOM_DB: f64 = 10.0;
/// §9.3 step 9: iterate until the crossing moves less than 0.01 s …
const CONVERGENCE_S: f64 = 0.01;
/// … with a cap of 30 iterations.
const MAX_ITERATIONS: u32 = 30;

/// §9.4: T20 is fitted from −5 to −25 dB …
pub(crate) const T20_RANGE_DB: (f64, f64) = (-5.0, -25.0);
/// … T30 from −5 to −35 dB …
pub(crate) const T30_RANGE_DB: (f64, f64) = (-5.0, -35.0);
/// … and EDT from 0 to −10 dB.
pub(crate) const EDT_RANGE_DB: (f64, f64) = (0.0, -10.0);

/// A converged (or capped) run of Lundeby's method.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lundeby {
    /// The noise floor's mean square.
    pub noise_energy: f64,
    pub noise_db: f64,
    /// The late decay line in dB against seconds from the start of the signal.
    pub line: Line,
    /// Where the line meets the noise level, in seconds from the start of the signal.
    pub crossing_s: f64,
    pub iterations: u32,
    pub converged: bool,
}

/// What a run of Lundeby's method found, including what it knew before it aborted.
pub(crate) struct LundebyOutcome {
    /// The loudest block of step 1.
    pub max_block_db: Option<f64>,
    /// Step 2's first noise estimate.
    pub first_noise_energy: Option<f64>,
    pub result: Result<Lundeby, DecayFailure>,
}

/// Lundeby's iterative noise-floor and truncation method over an energy signal (squared samples,
/// summed over channels), starting at the signal's first sample.
pub(crate) fn lundeby(energy: &[f64], rate: f64, first_block_s: f64) -> LundebyOutcome {
    // Step 1.
    let block = samples(first_block_s, rate);
    let levels = block_levels(energy, block);
    if levels.len() < MIN_BLOCKS {
        return LundebyOutcome {
            max_block_db: None,
            first_noise_energy: None,
            result: Err(DecayFailure::TooShort),
        };
    }
    let (peak_block, max_db) = first_max(&levels);

    // Step 2.
    let tail_blocks = ((levels.len() as f64 * NOISE_TAIL_FRACTION).ceil() as usize).max(1);
    let noise = mean(&energy[(levels.len() - tail_blocks) * block..levels.len() * block]);

    LundebyOutcome {
        max_block_db: Some(max_db),
        first_noise_energy: Some(noise),
        result: iterate(energy, rate, &levels, block, peak_block, noise),
    }
}

fn iterate(
    energy: &[f64],
    rate: f64,
    levels: &[f64],
    block: usize,
    peak_block: usize,
    first_noise: f64,
) -> Result<Lundeby, DecayFailure> {
    // Step 3.
    let floor = energy_db(first_noise) + FIRST_FIT_MARGIN_DB;
    if levels[peak_block] - floor < MIN_FIRST_RANGE_DB {
        return Err(DecayFailure::TooLittleDecay);
    }
    let last = levels
        .iter()
        .rposition(|&level| level > floor)
        .filter(|&last| last > peak_block)
        .ok_or(DecayFailure::TooLittleDecay)?;
    let mut line =
        fit_blocks(levels, block, rate, peak_block, last).ok_or(DecayFailure::TooLittleDecay)?;
    if line.slope >= 0.0 {
        return Err(DecayFailure::SlopeNotNegative);
    }

    // Step 4.
    let mut noise_energy = first_noise;
    let mut noise_db = energy_db(noise_energy);
    let mut crossing = (noise_db - line.intercept) / line.slope;

    // Steps 5 and 6.
    let block = samples(10.0 / (BLOCKS_PER_10_DB * -line.slope), rate);
    let levels = block_levels(energy, block);
    if levels.len() < 2 {
        return Err(DecayFailure::TooShort);
    }
    let (peak_block, _) = first_max(&levels);

    for iteration in 1..=MAX_ITERATIONS {
        // Step 7.
        let after_line = (crossing + SAFETY_MARGIN_DB / -line.slope) * rate;
        let latest = NOISE_REGION_LATEST_FRACTION * energy.len() as f64;
        let start = after_line.min(latest).max(0.0) as usize;
        noise_energy = mean(&energy[start.min(energy.len() - 1)..]);
        noise_db = energy_db(noise_energy);

        // Step 8.
        let top = noise_db + FIT_TOP_DB;
        let bottom = noise_db + FIT_BOTTOM_DB;
        let first = (peak_block..levels.len())
            .find(|&i| levels[i] <= top)
            .ok_or(DecayFailure::TooLittleDecay)?;
        let last = levels
            .iter()
            .rposition(|&level| level > bottom)
            .filter(|&last| last > first)
            .ok_or(DecayFailure::TooLittleDecay)?;
        line = fit_blocks(&levels, block, rate, first, last).ok_or(DecayFailure::TooLittleDecay)?;
        if line.slope >= 0.0 {
            return Err(DecayFailure::SlopeNotNegative);
        }

        // Step 9.
        let next = (noise_db - line.intercept) / line.slope;
        let moved = (next - crossing).abs();
        crossing = next;
        if moved < CONVERGENCE_S {
            return Ok(Lundeby {
                noise_energy,
                noise_db,
                line,
                crossing_s: crossing,
                iterations: iteration,
                converged: true,
            });
        }
    }
    Ok(Lundeby {
        noise_energy,
        noise_db,
        line,
        crossing_s: crossing,
        iterations: MAX_ITERATIONS,
        converged: false,
    })
}

/// Block means of an energy signal in dB, dropping a final partial block.
fn block_levels(energy: &[f64], block: usize) -> Vec<f64> {
    energy
        .chunks_exact(block)
        .map(|chunk| energy_db(chunk.iter().sum::<f64>() / block as f64))
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// The first largest value and its index.
fn first_max(values: &[f64]) -> (usize, f64) {
    values
        .iter()
        .enumerate()
        .fold((0, f64::NEG_INFINITY), |best, (i, &v)| {
            if v > best.1 { (i, v) } else { best }
        })
}

/// A line through blocks `first..=last`, each placed at its centre.
fn fit_blocks(levels: &[f64], block: usize, rate: f64, first: usize, last: usize) -> Option<Line> {
    let mut fit = LineFit::default();
    for (i, &level) in levels.iter().enumerate().take(last + 1).skip(first) {
        fit.add((i as f64 + 0.5) * block as f64 / rate, level);
    }
    fit.line()
}

/// A Schroeder energy decay curve in dB re its value at the first sample, which ends where the
/// integration limit or a non-positive remainder ends it.
pub(crate) struct DecayCurve {
    pub db: Vec<f64>,
    /// The integral at the first sample, in summed squared samples.
    pub total: f64,
}

/// Schroeder backward integration (§9.2) by one of Guski and Vorländer's methods (§9.3).
pub(crate) fn decay_curve(
    energy: &[f64],
    rate: f64,
    lundeby: &Lundeby,
    method: DecayMethod,
) -> Result<DecayCurve, OutOfMemory> {
    let len = energy.len();
    let (end, subtract, correction) = match method {
        DecayMethod::Uncompensated => (len, 0.0, 0.0),
        DecayMethod::Truncated | DecayMethod::NoiseCompensated => {
            let limit = lundeby.crossing_s * rate;
            let end = if limit.is_finite() && limit > 0.0 {
                (limit.ceil() as usize).clamp(1, len)
            } else if limit > 0.0 {
                len
            } else {
                1
            };
            let subtract = if method == DecayMethod::NoiseCompensated {
                lundeby.noise_energy
            } else {
                0.0
            };
            (
                end,
                subtract,
                correction(&lundeby.line, end as f64 / rate, rate),
            )
        }
    };

    // Reserved fallibly: the response's length sizes it.
    let mut curve = buffer::filled(end, 0.0f64)?;
    let mut remaining = correction;
    for n in (0..end).rev() {
        remaining += energy[n] - subtract;
        curve[n] = remaining;
    }
    let total = curve.first().copied().unwrap_or(0.0);
    if total <= 0.0 || !total.is_finite() {
        return Ok(DecayCurve {
            db: Vec::new(),
            total: 0.0,
        });
    }
    // With the noise subtracted the remainder can reach zero before the limit; the curve ends there.
    let usable = curve.iter().position(|&s| s <= 0.0).unwrap_or(end);
    curve.truncate(usable);
    for value in &mut curve {
        *value = 10.0 * (*value / total).log10();
    }
    Ok(DecayCurve { db: curve, total })
}

/// The energy the truncation discards, assuming the late decay line continues past the limit:
/// the line's energy per sample at the limit times the decay's energy time constant in samples.
fn correction(line: &Line, limit_s: f64, rate: f64) -> f64 {
    if line.slope >= 0.0 {
        return 0.0;
    }
    let at_limit = 10f64.powf(line.at(limit_s) / 10.0);
    let time_constant_samples = 10.0 / (-line.slope * LN_10) * rate;
    let correction = at_limit * time_constant_samples;
    if correction.is_finite() {
        correction
    } else {
        0.0
    }
}

/// A straight line fitted to part of a decay curve.
pub(crate) struct CurveFit {
    pub line: Line,
    pub start: usize,
    pub end: usize,
}

/// The fit over the curve from its first sample at or below `top_db` to its first sample below
/// `bottom_db`; absent where the curve does not reach `bottom_db`.
pub(crate) fn fit_range(
    curve: &DecayCurve,
    rate: f64,
    (top_db, bottom_db): (f64, f64),
) -> Option<CurveFit> {
    let start = curve.db.iter().position(|&d| d <= top_db)?;
    let end = curve.db[start..].iter().position(|&d| d < bottom_db)? + start;
    if end < start + 2 {
        return None;
    }
    let mut fit = LineFit::default();
    for (n, &d) in curve.db.iter().enumerate().take(end).skip(start) {
        fit.add(n as f64 / rate, d);
    }
    Some(CurveFit {
        line: fit.line()?,
        start,
        end,
    })
}

/// The time to fall 60 dB at the fit's slope.
pub(crate) fn decay_time(fit: &CurveFit) -> Option<f64> {
    (fit.line.slope < 0.0).then(|| -60.0 / fit.line.slope)
}

/// How far a curve departs from one straight fit.
pub(crate) struct Departure {
    pub max_db: f64,
    /// The decay time over the range's lower half over the one over its upper half.
    pub late_to_early: Option<f64>,
}

/// How far the curve departs from the fit over the fit's range, and the ratio of the decay times of
/// the range's lower and upper halves.
pub(crate) fn straightness(
    curve: &DecayCurve,
    rate: f64,
    fit: &CurveFit,
    (top_db, bottom_db): (f64, f64),
) -> Departure {
    let max_db = curve.db[fit.start..fit.end]
        .iter()
        .enumerate()
        .map(|(i, &d)| (d - fit.line.at((fit.start + i) as f64 / rate)).abs())
        .fold(0.0, f64::max);
    let middle = 0.5 * (top_db + bottom_db);
    let upper = fit_range(curve, rate, (top_db, middle)).and_then(|f| decay_time(&f));
    let lower = fit_range(curve, rate, (middle, bottom_db)).and_then(|f| decay_time(&f));
    let late_to_early = match (upper, lower) {
        (Some(upper), Some(lower)) => Some(lower / upper),
        _ => None,
    };
    Departure {
        max_db,
        late_to_early,
    }
}

/// The late decay's energy per sample at the first sample, in dB.
///
/// For an energy `A·10^(s·t/10)` per sample, the remaining integral is `A·rate·τ·10^(s·t/10)` with
/// `τ = 10 / (|s|·ln 10)`, so the curve's intercept `b` (dB re the total) gives
/// `A = b + 10·log₁₀(total) − 10·log₁₀(rate·τ)`. *Derived*; it is exact for a single exponential.
pub(crate) fn initial_level_db(curve: &DecayCurve, fit: &CurveFit, rate: f64) -> Option<f64> {
    if fit.line.slope >= 0.0 || curve.total <= 0.0 {
        return None;
    }
    let time_constant_samples = 10.0 / (-fit.line.slope * LN_10) * rate;
    let level =
        fit.line.intercept + 10.0 * curve.total.log10() - 10.0 * time_constant_samples.log10();
    level.is_finite().then_some(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An exact exponential decay at `t60` from 0 dB, followed by a constant noise floor.
    fn decay_then_floor(rate: f64, t60: f64, seconds: f64, noise_db: f64) -> Vec<f64> {
        let noise = 10f64.powf(noise_db / 10.0);
        (0..(seconds * rate) as usize)
            .map(|n| 10f64.powf(-6.0 * (n as f64 / rate) / t60) + noise)
            .collect()
    }

    #[test]
    fn lundeby_finds_a_constant_floor_and_the_decay_that_meets_it() {
        // Closed form: a 1.0 s T60 from 0 dB meets a −60 dB floor at 1.0 s. With no fluctuation in
        // either, measured: the floor at −59.984 dB and the crossing at 1.0075 s, in one iteration.
        // Tolerances are about ten times that; a wrong step (the noise taken from the whole signal,
        // or no safety margin) moves the floor by decibels and the crossing by tenths of a second.
        let rate = 8_000.0;
        let energy = decay_then_floor(rate, 1.0, 3.0, -60.0);
        let outcome = lundeby(&energy, rate, 0.030);
        let result = outcome.result.expect("a clean decay is found");
        assert!(
            (result.noise_db + 60.0).abs() < 0.2,
            "noise {}",
            result.noise_db
        );
        assert!(
            (result.crossing_s - 1.0).abs() < 0.05,
            "crossing {}",
            result.crossing_s
        );
        assert!(result.converged);
    }

    #[test]
    fn noise_compensation_recovers_an_exact_decay_time() {
        let rate = 8_000.0;
        let energy = decay_then_floor(rate, 1.0, 3.0, -50.0);
        let lundeby = lundeby(&energy, rate, 0.030).result.expect("decay");
        let curve = decay_curve(&energy, rate, &lundeby, DecayMethod::NoiseCompensated).unwrap();
        let t30 = decay_time(&fit_range(&curve, rate, T30_RANGE_DB).expect("T30 range")).unwrap();
        let uncompensated =
            decay_curve(&energy, rate, &lundeby, DecayMethod::Uncompensated).unwrap();
        let t30_a =
            decay_time(&fit_range(&uncompensated, rate, T30_RANGE_DB).expect("T30 range")).unwrap();
        let truncated = decay_curve(&energy, rate, &lundeby, DecayMethod::Truncated).unwrap();
        let t30_c =
            decay_time(&fit_range(&truncated, rate, T30_RANGE_DB).expect("T30 range")).unwrap();
        // A 1.0 s decay over a constant floor 50 dB down, with two seconds of floor after it.
        // Measured: method E 1.0003 s, method C 1.0126 s, method A 1.358 s — Guski and Vorländer's
        // ordering, with A's overestimate the one their table warns of. E is asserted to 1 %, thirty
        // times what it reads; C and A only in the direction the noise pushes them.
        assert!((t30 - 1.0).abs() < 0.01, "method E T30 {t30}");
        assert!(
            t30_c > t30,
            "method C T30 {t30_c} should read longer than E's {t30}"
        );
        assert!(t30_a > 1.2, "method A T30 {t30_a} should read long");
    }

    #[test]
    fn a_rising_or_flat_signal_is_not_a_decay() {
        let rate = 8_000.0;
        let flat = vec![1.0f64; 8_000];
        assert_eq!(
            lundeby(&flat, rate, 0.030).result.err(),
            Some(DecayFailure::TooLittleDecay)
        );
        let short = vec![1.0f64; 100];
        assert_eq!(
            lundeby(&short, rate, 0.030).result.err(),
            Some(DecayFailure::TooShort)
        );
    }
}
