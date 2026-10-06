//! The search over Diffusion (plan §4.3): with Size the density floor (`fit.rs`), the one setting left
//! with no closed form. A
//! candidate is scored by how far its early part lies from the response's: its echo density profile
//! over the span where density grows — the whole profile, not the mixing time alone, whose "first
//! reaches one" reading has a long tail — and its early energy envelope, which carries when the
//! network starts after the pre-delay and how smoothly its energy builds (this crate's `AGENTS.md`).

use core::ops::RangeInclusive;

use mxm_classic_verb_dsp::space::{
    HIGH_CUT_OPEN_HZ, MAX_DECAY_RATIO, MAX_EARLY_LEVEL, MAX_SPACE_TONE_DB, MAX_SPACE_WIDTH,
    MIN_DECAY_RATIO, MIN_HIGH_CUT_HZ,
};
use mxm_classic_verb_dsp::{
    Controls, MAX_PRE_DELAY_S, MAX_SIZE_S, MIN_SIZE_S, Space, late_onset_s,
};

use crate::analysis::{Analysis, DensityPoint};
use crate::buffer::{self, OutOfMemory};
use crate::density;
use crate::fit::{
    ControlRanges, DIFFUSION_GRID, ENVELOPE_CLIP_DB, ENVELOPE_WEIGHT_PER_DB, ENVELOPE_WINDOW_S,
    FirstArrival, PROFILE_START_S, REFINE_MAX_EVALUATIONS, REFINE_MIN_DIFFUSION_STEP,
    REFINE_STARTS,
};
use crate::math::samples;
use crate::render::{Renderer, fold, slices};
use crate::solve::{
    Arrival, BalanceWindows, DecaySolution, DecayTarget, TapSet, ToneSolution, neutral_controls,
    solve_decay, solve_early_level, tap_set,
};

/// Everything the search holds fixed.
pub(crate) struct Problem<'a> {
    pub rate: f32,
    pub mono: bool,
    pub target: &'a Analysis,
    pub decay_targets: Vec<DecayTarget>,
    /// Every candidate's Size: the density floor (`fit.rs`), before a range clamps it.
    pub size_s: f64,
    pub tone: ToneSolution,
    pub arrivals: Vec<Arrival>,
    /// The measured pre-delay: direct sound to first reflection energy.
    pub pre_delay_s: f64,
    /// The response's direct-sound frame.
    pub direct: usize,
    pub windows: BalanceWindows,
    pub target_ratio: Option<f64>,
    /// The response's early energy envelope, one point per echo density point.
    pub target_envelope: Vec<Option<f64>>,
    pub ranges: &'a ControlRanges,
    pub profile_end_s: f64,
    pub excerpt_frames: usize,
}

/// A candidate's values as solved, before any range or bound is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    pub size_s: f64,
    pub diffusion: f64,
    pub arrival: FirstArrival,
    pub early_level: f64,
    pub decay: DecaySolution,
    pub taps: TapSet,
    pub width: f64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    pub settings: Settings,
    /// The score the search minimises.
    pub distance: Option<f64>,
    pub profile: Option<f64>,
    pub envelope: Option<f64>,
}

pub(crate) struct SearchOutcome {
    pub best: Candidate,
    pub evaluations: u32,
}

/// The pre-delay control for the measured pre-delay. A reflection first: the first tap sits at the
/// pre-delay. The late field first: its first energy reaches the output the network's shortest output
/// tap after the pre-delay (`mxm_classic_verb_dsp::late_onset_s`), so that much comes off.
pub(crate) fn pre_delay_control(pre_delay_s: f64, arrival: FirstArrival, size_s: f64) -> f64 {
    match arrival {
        FirstArrival::Reflection => pre_delay_s,
        FirstArrival::LateField => (pre_delay_s - f64::from(late_onset_s(size_s as f32))).max(0.0),
    }
}

fn clamp_to(value: f64, range: &RangeInclusive<f32>) -> f32 {
    (value as f32).clamp(*range.start(), *range.end())
}

/// The space and controls a candidate's values give, with every control inside its range and every
/// space value inside the engine's bounds.
pub(crate) fn model(problem: &Problem, s: &Settings) -> (Space, Controls) {
    let bound = |v: f64, lo: f32, hi: f32| (v as f32).clamp(lo, hi);
    let space = Space {
        early: s.taps.taps,
        early_level: bound(s.early_level, 0.0, MAX_EARLY_LEVEL),
        decay_ratio_low: bound(s.decay.ratio_low, MIN_DECAY_RATIO, MAX_DECAY_RATIO),
        decay_ratio_high: bound(s.decay.ratio_high, MIN_DECAY_RATIO, MAX_DECAY_RATIO),
        decay_ratio_top: bound(s.decay.ratio_top, MIN_DECAY_RATIO, MAX_DECAY_RATIO),
        tone_low_db: bound(problem.tone.low_db, -MAX_SPACE_TONE_DB, MAX_SPACE_TONE_DB),
        tone_high_db: bound(problem.tone.high_db, -MAX_SPACE_TONE_DB, MAX_SPACE_TONE_DB),
        high_cut_hz: bound(problem.tone.high_cut_hz, MIN_HIGH_CUT_HZ, HIGH_CUT_OPEN_HZ),
        width: bound(s.width, 0.0, MAX_SPACE_WIDTH),
    };
    let ranges = problem.ranges;
    let controls = Controls {
        decay_s: clamp_to(s.decay.decay_s, &ranges.decay_s),
        size_s: clamp_to(s.size_s, &ranges.size_s),
        diffusion: clamp_to(s.diffusion, &ranges.diffusion),
        pre_delay_s: clamp_to(
            pre_delay_control(problem.pre_delay_s, s.arrival, s.size_s),
            &ranges.pre_delay_s,
        )
        .min(MAX_PRE_DELAY_S),
        ..neutral_controls()
    };
    (space, controls)
}

/// The root-mean-square difference of two echo density profiles over the points both measured,
/// from [`PROFILE_START_S`] to `end_s` after the direct sound. Both profiles step one millisecond
/// from the direct sound at the same rate, so their points align by index.
pub(crate) fn profile_distance(
    target: &[DensityPoint],
    other: &[Option<f64>],
    end_s: f64,
) -> Option<f64> {
    let (mut sum, mut count) = (0.0f64, 0usize);
    for (point, density) in target.iter().zip(other) {
        let t = f64::from(point.time_s);
        if !(PROFILE_START_S..=end_s).contains(&t) {
            continue;
        }
        if let (Some(a), Some(b)) = (point.density, density) {
            sum += (f64::from(a) - b).powi(2);
            count += 1;
        }
    }
    (count > 0).then(|| (sum / count as f64).sqrt())
}

/// The early energy envelope: the mean energy over [`ENVELOPE_WINDOW_S`] about each echo density
/// point, both channels summed, in dB against the mean energy of the balance's late window, so a
/// response and a render compare whatever their levels. A point whose energy is not `NOISE_MARGIN_DB`
/// over `noise_energy` is absent; a noiseless render's silence reads 60 dB under the late window.
pub(crate) fn envelope(
    channels: &[&[f32]],
    direct: usize,
    rate: f64,
    windows: &BalanceWindows,
    noise_energy: f64,
    points: usize,
) -> Vec<Option<f64>> {
    let len = channels[0].len();
    let hop = samples(crate::ECHO_DENSITY_HOP_S, rate);
    let half = samples(ENVELOPE_WINDOW_S, rate) / 2;
    let mean = |first: usize, end: usize| -> f64 {
        let (first, end) = (first.min(len), end.min(len));
        if end <= first {
            return 0.0;
        }
        channels
            .iter()
            .map(|c| {
                c[first..end]
                    .iter()
                    .map(|&s| f64::from(s) * f64::from(s))
                    .sum::<f64>()
            })
            .sum::<f64>()
            / (end - first) as f64
    };
    let late = mean(direct + windows.late.start, direct + windows.late.end);
    let gate = noise_energy * 10f64.powf(crate::NOISE_MARGIN_DB / 10.0);
    (0..points)
        .map(|i| {
            if late <= 0.0 {
                return None;
            }
            let centre = direct + i * hop;
            let m = mean(centre.saturating_sub(half), centre + half + 1);
            if noise_energy > 0.0 && m <= gate {
                return None;
            }
            Some(10.0 * (m.max(late * 1e-6) / late).log10())
        })
        .collect()
}

/// The root-mean-square difference of two envelopes over the points both measured, from
/// [`PROFILE_START_S`] to `end_s`, each point's difference held within ±[`ENVELOPE_CLIP_DB`].
pub(crate) fn envelope_distance(
    points: &[DensityPoint],
    target: &[Option<f64>],
    other: &[Option<f64>],
    end_s: f64,
) -> Option<f64> {
    let (mut sum, mut count) = (0.0f64, 0usize);
    for ((point, a), b) in points.iter().zip(target).zip(other) {
        let t = f64::from(point.time_s);
        if !(PROFILE_START_S..=end_s).contains(&t) {
            continue;
        }
        if let (Some(a), Some(b)) = (a, b) {
            sum += (a - b).clamp(-ENVELOPE_CLIP_DB, ENVELOPE_CLIP_DB).powi(2);
            count += 1;
        }
    }
    (count > 0).then(|| (sum / count as f64).sqrt())
}

/// A candidate's score: the profile distance plus the envelope distance at
/// [`ENVELOPE_WEIGHT_PER_DB`]. Without a profile there is no score.
fn score(profile: Option<f64>, envelope: Option<f64>) -> Option<f64> {
    profile.map(|p| p + ENVELOPE_WEIGHT_PER_DB * envelope.unwrap_or(0.0))
}

/// What depends on Size and the first arrival alone: the decay solve, the taps, and the early part's
/// render.
struct SizeCache {
    size_bits: u64,
    arrival: FirstArrival,
    decay: DecaySolution,
    taps: TapSet,
    early: Vec<Vec<f32>>,
}

/// One candidate rendered and scored. Its excerpts are as long as the response's pre-delay, so each is
/// reserved fallibly.
fn evaluate(
    problem: &Problem,
    renderer: &mut Renderer,
    caches: &mut Vec<SizeCache>,
    (size_s, diffusion, arrival): (f64, f64, FirstArrival),
) -> Result<Candidate, OutOfMemory> {
    let index = match caches
        .iter()
        .position(|c| c.size_bits == size_s.to_bits() && c.arrival == arrival)
    {
        Some(i) => i,
        None => {
            let tone = problem.tone.carried();
            let decay = solve_decay(&problem.decay_targets, size_s, problem.rate, tone)
                .expect("the fit refuses a response with no mid-band decay before searching");
            let shift =
                problem.pre_delay_s - pre_delay_control(problem.pre_delay_s, arrival, size_s);
            caches.push(SizeCache {
                size_bits: size_s.to_bits(),
                arrival,
                decay,
                taps: tap_set(&problem.arrivals, size_s, shift),
                early: Vec::new(),
            });
            caches.len() - 1
        }
    };
    let base = Settings {
        size_s,
        diffusion,
        arrival,
        early_level: 0.0,
        decay: caches[index].decay,
        taps: caches[index].taps,
        width: 1.0,
    };
    let frames = problem.excerpt_frames;
    let (late_space, controls) = model(problem, &base);
    let late = fold(
        renderer.impulse(&late_space, &controls, 0, frames)?,
        problem.mono,
    )?;

    // The engine is linear and deterministic, so the render at early level one less the render at
    // zero is the early part alone, exactly; it does not pass through the diffusers, so one render per
    // Size serves every Diffusion.
    if caches[index].early.is_empty() && base.taps.kept > 0 {
        let (early_space, _) = model(
            problem,
            &Settings {
                early_level: 1.0,
                ..base
            },
        );
        let both = fold(
            renderer.impulse(&early_space, &controls, 0, frames)?,
            problem.mono,
        )?;
        let mut early = Vec::with_capacity(both.len());
        for (b, l) in both.iter().zip(&late) {
            early.push(buffer::collected(
                b.len().min(l.len()),
                b.iter().zip(l).map(|(x, y)| x - y),
            )?);
        }
        caches[index].early = early;
    }
    let early = &caches[index].early;
    let early_level = match (early.is_empty(), problem.target_ratio) {
        (true, _) => 0.0,
        (false, Some(target)) => {
            solve_early_level(&slices(early), &slices(&late), &problem.windows, target)
        }
        (false, None) => 1.0,
    };
    let applied = early_level.clamp(0.0, f64::from(MAX_EARLY_LEVEL)) as f32;
    let combined: Vec<Vec<f32>> = if early.is_empty() {
        late
    } else {
        let mut combined = Vec::with_capacity(early.len());
        for (e, l) in early.iter().zip(&late) {
            combined.push(buffer::collected(
                e.len().min(l.len()),
                e.iter().zip(l).map(|(x, y)| applied * x + y),
            )?);
        }
        combined
    };
    let rate = f64::from(problem.rate);
    let points = &problem.target.echo_density.points;
    let densities: Vec<Option<f64>> = density::echo_density(&slices(&combined), rate, 0, 0.0)
        .points
        .iter()
        .map(|p| p.1)
        .collect();
    let profile = profile_distance(points, &densities, problem.profile_end_s);
    let candidate_envelope = envelope(
        &slices(&combined),
        0,
        rate,
        &problem.windows,
        0.0,
        points.len(),
    );
    let envelope = envelope_distance(
        points,
        &problem.target_envelope,
        &candidate_envelope,
        problem.profile_end_s,
    );
    Ok(Candidate {
        settings: Settings {
            early_level,
            ..base
        },
        distance: score(profile, envelope),
        profile,
        envelope,
    })
}

fn better(candidate: &Candidate, best: &Option<Candidate>) -> bool {
    match (candidate.distance, best.as_ref().and_then(|b| b.distance)) {
        (Some(d), Some(b)) => d < b,
        (Some(_), None) => true,
        (None, _) => best.is_none(),
    }
}

/// A grid over Diffusion and the first arrival at the Size the density floor sets, then a pattern
/// search over Diffusion from each of the [`REFINE_STARTS`] best grid points, and the closest candidate
/// of all — the first of equal distances, so the choice is deterministic within one build. Which
/// first-arrival hypothesis a response fits is settled by the match alone. A render that cannot be
/// reserved ends the search.
pub(crate) fn search(
    problem: &Problem,
    renderer: &mut Renderer,
) -> Result<SearchOutcome, OutOfMemory> {
    let ranges = problem.ranges;
    // Candidates render at the Size the fit applies, so a range that clamps the floor is heard here
    // too; the fit reports that clamp against the floor.
    let size = f64::from(clamp_to(problem.size_s, &ranges.size_s).clamp(MIN_SIZE_S, MAX_SIZE_S));
    let (d_lo, d_hi) = (
        f64::from(ranges.diffusion.start().clamp(0.0, 1.0)),
        f64::from(ranges.diffusion.end().clamp(0.0, 1.0)),
    );
    let arrivals: &[FirstArrival] = if problem.arrivals.is_empty() {
        &[FirstArrival::LateField]
    } else {
        &[FirstArrival::Reflection, FirstArrival::LateField]
    };
    let mut caches = Vec::new();
    let mut grid: Vec<Candidate> = Vec::new();
    for &arrival in arrivals {
        for &diffusion in &DIFFUSION_GRID {
            let diffusion = f64::from(diffusion).clamp(d_lo, d_hi);
            grid.push(evaluate(
                problem,
                renderer,
                &mut caches,
                (size, diffusion, arrival),
            )?);
        }
    }

    let mut all: Vec<Candidate> = grid.clone();
    // Stable: equal distances keep grid order, so the starts are deterministic.
    grid.sort_by(|a, b| match (a.distance, b.distance) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => core::cmp::Ordering::Less,
        (None, Some(_)) => core::cmp::Ordering::Greater,
        (None, None) => core::cmp::Ordering::Equal,
    });
    for start in grid.iter().take(REFINE_STARTS) {
        refine(
            problem,
            renderer,
            &mut caches,
            *start,
            (d_lo, d_hi),
            &mut all,
        )?;
    }
    let best = all
        .iter()
        .filter(|c| c.distance.is_some())
        .min_by(|a, b| {
            a.distance
                .unwrap_or(0.0)
                .total_cmp(&b.distance.unwrap_or(0.0))
        })
        .copied()
        .unwrap_or(grid[0]);
    Ok(SearchOutcome {
        best,
        evaluations: all.len() as u32,
    })
}

/// A pattern search over Diffusion from `start`, its Size and first arrival held: both neighbours
/// tried in a fixed order, a move to either that improves, and the step halved when neither does,
/// until the step is under [`REFINE_MIN_DIFFUSION_STEP`] or [`REFINE_MAX_EVALUATIONS`] are spent.
/// Every candidate it evaluates is added to `all`.
fn refine(
    problem: &Problem,
    renderer: &mut Renderer,
    caches: &mut Vec<SizeCache>,
    start: Candidate,
    (d_lo, d_hi): (f64, f64),
    all: &mut Vec<Candidate>,
) -> Result<Candidate, OutOfMemory> {
    let mut best = start;
    let (size, arrival) = (best.settings.size_s, best.settings.arrival);
    let mut step = 0.5 / (DIFFUSION_GRID.len() - 1) as f64;
    let mut spent = 0u32;
    while spent < REFINE_MAX_EVALUATIONS && step > REFINE_MIN_DIFFUSION_STEP {
        let diffusion = best.settings.diffusion;
        let mut moved = false;
        for d in [(diffusion + step).min(d_hi), (diffusion - step).max(d_lo)] {
            if d == diffusion || spent >= REFINE_MAX_EVALUATIONS {
                continue;
            }
            let candidate = evaluate(problem, renderer, caches, (size, d, arrival))?;
            spent += 1;
            all.push(candidate);
            if better(&candidate, &Some(best)) {
                best = candidate;
                moved = true;
            }
        }
        if !moved {
            step *= 0.5;
        }
    }
    Ok(best)
}
