//! The fit (plan §4.3, §4.4): a response becomes a space and the absolute controls that reproduce
//! its descriptors, and the result is rendered and analysed by the same analyser before it is
//! returned. **The fitter never claims a match it did not measure.**

use core::fmt;
use core::ops::RangeInclusive;

use mxm_classic_verb_dsp::{
    Controls, DecayShape, MAX_DECAY_S, MAX_PRE_DELAY_S, MAX_SIZE_S, MIN_DECAY_S, MIN_SIZE_S, Space,
    predicted_decay_s, size_for_total_delay_s,
};

use crate::analysis::{Analysis, TailTexture, Width, WidthNotMeasured};
use crate::buffer::{self, OutOfMemory};
use crate::math::{clamped_f32, finite_f32, samples};
use crate::refusal::Refusal;
use crate::render::{Renderer, fold, slices};
use crate::search::{self, Problem, Settings, model, pre_delay_control, profile_distance};
use crate::solve;

// ---------------------------------------------------------------------------------------------------
// The fit's constants. Each reason is beside it and in the crate's NOTES.md.
// ---------------------------------------------------------------------------------------------------

/// A band read by T20 because its T30 was not measurable counts this much against one read by T30.
/// **Measured**: on the planted room T20's worst error was 1.1–1.7 times T30's band by band, so its
/// variance is about twice T30's, and inverse-variance weighting halves it.
pub const T20_WEIGHT: f64 = 0.5;
/// Frequencies the tone model samples across the two octaves about each band. **Chosen**: enough to
/// follow the analyser's band shape, whose skirts reach a neighbouring octave.
pub const TONE_POINTS: usize = 17;
/// High-cut corners the tone solve tries per octave, from `MIN_HIGH_CUT_HZ` up to just below open,
/// before it refines the best. **Chosen**: an eighth of an octave, finer than an octave band's level
/// resolves a corner.
pub const HIGH_CUT_STEPS_PER_OCTAVE: usize = 8;
/// Golden-section steps between the best grid corner's neighbours. **Chosen**: twelve narrow a
/// quarter of an octave to under a thousandth of one.
pub const HIGH_CUT_REFINE_STEPS: usize = 12;
/// The high cut stays open unless the curve it gives lies at least this much closer to the measured
/// tone than the shelves' alone, in root-mean-square dB. **Measured**: the ten round trips, rendered
/// from spaces with no cut, would each take one for at most 0.039 dB, and taking it moved the search —
/// the mono room's Size error went from +22.5 to +72.4 % and its 8 kHz tone error from +1.37 to
/// −3.05 dB. The planted dense onsets, whose high shelf clamps without a cut, improve by 0.23 and
/// 0.24 dB, and a steep rolloff by more than 3 dB. 0.1 dB lies between.
pub const HIGH_CUT_MIN_IMPROVEMENT_DB: f64 = 0.1;
/// The early/late balance compares energy from the direct-sound window's end to this long after the
/// pre-delay against the next window of the same length. **Chosen** to match the analyser's own
/// early window cap (`EARLY_WINDOW_MAX_S`), where the reflections the taps come from are read.
pub const BALANCE_WINDOW_S: f64 = crate::EARLY_WINDOW_MAX_S;
/// The first echo density point compared: past the half-window, so no compared window holds the
/// direct sound, which a render for the search does not carry.
pub const PROFILE_START_S: f64 = 0.011;
/// Profiles are compared up to this long after the pre-delay. **Chosen**: density grows in the
/// early part, and the analyser measured planted mixing times of 45–60 ms; past a quarter of a
/// second a dense field reads about one in any candidate and adds only noise to the distance.
pub const PROFILE_SPAN_S: f64 = 0.25;
/// The early energy envelope's window. **Chosen**: long enough to average a late field's fine
/// structure even at 22.05 kHz, short enough to place the network's onset within itself.
pub const ENVELOPE_WINDOW_S: f64 = 0.005;
/// No envelope point counts for more than this many dB of difference, so a silent gap in one and
/// energy in the other weighs as a clear miss without swamping every other point. **Chosen.**
pub const ENVELOPE_CLIP_DB: f64 = 20.0;
/// The early envelope's weight in a candidate's score, per dB, added to its echo density profile
/// distance. **Measured** over ten round trips: the mean |ln(fitted Size ÷ true Size)| was 0.549
/// with the profile alone, 0.224 at 0.005 and 0.01, and 0.170 at 0.02 and 0.04; the worst Diffusion
/// error fell from 0.81 to 0.20 and the worst pre-delay error from 16.7 to 8.0 ms. 0.02 is the
/// smallest weight on that plateau.
pub const ENVELOPE_WEIGHT_PER_DB: f64 = 0.02;
/// Diffusion candidates in the grid.
pub const DIFFUSION_GRID: [f32; 6] = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
/// The pattern search starts from this many of the best grid points. **Measured** while Size was
/// searched too: from the best alone it settled away from narrow basins the grid had already sampled
/// near. **Kept, not re-measured**, for Diffusion alone.
pub const REFINE_STARTS: usize = 4;
/// Evaluations each pattern search may spend.
pub const REFINE_MAX_EVALUATIONS: u32 = 24;
/// The pattern search stops once its step is under this much Diffusion.
pub const REFINE_MIN_DIFFUSION_STEP: f64 = 0.02;
/// **The fitted Size is the density floor**: the network's total delay is this share of the response's
/// longest measured band decay — the recipe's modal density floor, Σm ≥ 0.15·T₆₀·f_s
/// (`research:effects/feedback-delay-network-reverb.md` §3.4, §13 step 1) — and Size is not searched.
/// **Measured** at P3.5 on twenty of the owner's responses (this crate's `NOTES.md`, *Size is the
/// density floor*): searched from 2 ms, Size fell to lines of 2–14 ms that rang, which the echo density
/// profile cannot see; searched upward from the floor, it ran to its 300 ms top for most responses and
/// clicked. At the floor 17 of the twenty were within every tail-texture limit, against 15 at 1.25 and
/// 1.5 times it, 14 at twice it and 10 with no ceiling.
pub const DENSITY_FLOOR_DECAY_SHARE: f64 = 0.15;
/// …and at least this much total delay, seconds. **Measured** there: at 0.5 s a short room still rang.
pub const DENSITY_FLOOR_MIN_S: f64 = 1.0;
/// Bisection steps in the width match.
pub const WIDTH_BISECTIONS: usize = 20;
/// The verification render's leading silence before its direct impulse, so the onset rules have a
/// lead to read (`ONSET_GUARD_S` twice over, with room).
pub const VERIFY_LEAD_S: f64 = 0.05;
/// The verification render's wet is scaled to the response's direct-to-reverberant ratio, but never
/// so far that any wet sample reaches this fraction of the direct impulse, or the analyser would take
/// a reflection for the direct sound. **Chosen.**
pub const WET_PEAK_CEILING: f64 = 0.5;
/// A verification reflection within this of a target reflection counts as the same reflection.
/// **Chosen**: twice the analyser's reflection half-width.
pub const REFLECTION_MATCH_S: f64 = 0.001;

// ---------------------------------------------------------------------------------------------------
// The result and its contract.
// ---------------------------------------------------------------------------------------------------

/// The range each control a fit writes may take. A fitted value outside is clamped and reported.
/// The default is the engine's own bounds, which are also the plugin's parameter ranges today.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlRanges {
    pub decay_s: RangeInclusive<f32>,
    pub size_s: RangeInclusive<f32>,
    pub diffusion: RangeInclusive<f32>,
    pub pre_delay_s: RangeInclusive<f32>,
}

impl Default for ControlRanges {
    fn default() -> Self {
        Self {
            decay_s: MIN_DECAY_S..=MAX_DECAY_S,
            size_s: MIN_SIZE_S..=MAX_SIZE_S,
            diffusion: 0.0..=1.0,
            pre_delay_s: 0.0..=MAX_PRE_DELAY_S,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FitOptions {
    pub ranges: ControlRanges,
}

/// The absolute controls a fit measured or searched (plan §2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FittedControls {
    pub decay_s: f32,
    pub size_s: f32,
    pub diffusion: f32,
    pub pre_delay_s: f32,
}

impl FittedControls {
    /// A fit's whole parameter contract (plan §2), applied to a player's current controls: the four
    /// absolute controls written; the bass and treble multipliers, early/late offset, width share and
    /// both tone offsets returned to neutral; modulation depth to zero; the decay shape Natural.
    /// **Mix, Ducking and modulation rate are left exactly as they were.**
    pub fn apply(&self, current: &Controls) -> Controls {
        Controls {
            mix: current.mix,
            decay_s: self.decay_s,
            bass_mult: 1.0,
            treble_mult: 1.0,
            size_s: self.size_s,
            diffusion: self.diffusion,
            pre_delay_s: self.pre_delay_s,
            early_late_db: 0.0,
            shape: DecayShape::Natural,
            mod_depth_s: 0.0,
            mod_rate_hz: current.mod_rate_hz,
            width: 1.0,
            tone_low_db: 0.0,
            tone_high_db: 0.0,
            duck: current.duck,
        }
    }
}

/// What a response's first energy after the direct sound was taken to be. A response cannot say:
/// the analyser reports the late field's first peaks as reflections too. So the search tries both
/// and keeps the one whose render matches better.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstArrival {
    /// A discrete reflection: the first tap sits at the pre-delay and the late field follows it.
    Reflection,
    /// The late field: its first return, a shortest line after the pre-delay, is the first energy,
    /// so the pre-delay control is that much earlier than the measured gap.
    LateField,
}

/// A value a fit produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitValue {
    Decay,
    Size,
    Diffusion,
    PreDelay,
    DecayRatioLow,
    DecayRatioHigh,
    DecayRatioTop,
    ToneLow,
    ToneHigh,
    HighCut,
    Width,
    EarlyLevel,
}

/// A fitted value that lay outside its control's range or its space bound, and where it was put.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clamp {
    pub value: FitValue,
    pub fitted: f32,
    pub applied: f32,
}

/// Why a response was not fitted.
#[derive(Clone, Debug, PartialEq)]
pub enum FitRefusal {
    /// The analyser refused the response itself.
    Analysis(Refusal),
    /// No mid band (500 Hz–2 kHz) has a measured decay time, so Decay has nothing to read.
    NoMidBandDecay,
    /// The fitted reverb's render could not be analysed at all. Only the analyser's input bounds can
    /// do that, and the render is built inside them; a render the analyser would merely refuse as a
    /// response is reported in [`FitReport::verification_refusal`] instead.
    Verification(Refusal),
    /// A buffer the response's length sizes could not be reserved — for the response's analysis, a
    /// render, or the verification's analysis — and `bytes` is what was asked for. Named here
    /// whichever stage ran out, never as the analyser's refusal of the response: a machine with more
    /// memory would have fitted it.
    OutOfMemory { bytes: usize },
}

impl FitRefusal {
    /// The analyser's refusal of the response, with running out of memory lifted to its own variant.
    fn of_analysis(refusal: Refusal) -> Self {
        match refusal {
            Refusal::OutOfMemory { bytes } => FitRefusal::OutOfMemory { bytes },
            other => FitRefusal::Analysis(other),
        }
    }

    /// The same for the verification render's analysis.
    fn of_verification(refusal: Refusal) -> Self {
        match refusal {
            Refusal::OutOfMemory { bytes } => FitRefusal::OutOfMemory { bytes },
            other => FitRefusal::Verification(other),
        }
    }
}

impl From<OutOfMemory> for FitRefusal {
    fn from(error: OutOfMemory) -> Self {
        FitRefusal::OutOfMemory { bytes: error.bytes }
    }
}

impl fmt::Display for FitRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FitRefusal::Analysis(r) => write!(f, "the response was refused: {r}"),
            FitRefusal::NoMidBandDecay => {
                write!(f, "no mid band (500 Hz–2 kHz) has a measured decay time")
            }
            FitRefusal::Verification(r) => {
                write!(f, "the fitted reverb's render was refused: {r}")
            }
            FitRefusal::OutOfMemory { bytes } => {
                write!(f, "{}", Refusal::OutOfMemory { bytes: *bytes })
            }
        }
    }
}

impl std::error::Error for FitRefusal {}

/// A fitted space, the controls that go with it, and the report that says how well it matches.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub space: Space,
    pub controls: FittedControls,
    pub report: FitReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FitReport {
    /// The response's analysis: the target descriptors.
    pub target: Analysis,
    /// The fitted reverb's render, analysed with every measurement the response had — **its tail
    /// texture read on the response's segments** (`target.texture_segments()`), not on segments the
    /// render's own decay would place, so a render cannot move its own ruler.
    /// [`crate::analyse_with_segments`] makes the same analysis again from the render.
    pub verification: Analysis,
    /// What the analyser would have refused the render for, had it been a response: a required band
    /// with no decay above its floor, or a confidence under the floor. The render is analysed all the
    /// same, a band it could not measure is absent from [`DescriptorErrors`], and this names why — a
    /// fit never claims a match it did not measure (plan §4.4), and never leaves a failed
    /// verification unexplained.
    pub verification_refusal: Option<Refusal>,
    pub decay: DecayReport,
    pub tone: ToneReport,
    pub early: EarlyReport,
    pub width: WidthReport,
    pub search: SearchReport,
    /// The scale on the verification render's wet signal, set so it reads the response's DRR.
    pub wet_gain: f32,
    pub errors: DescriptorErrors,
    pub clamps: Vec<Clamp>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecayReport {
    pub bands: Vec<DecayBand>,
    /// Whether a band at or below the low crossover was measured; if not, the low ratio stays one.
    pub low_measured: bool,
    /// Whether the 4 kHz and the 8 kHz octaves were measured. Where neither was, both upper ratios
    /// stay one; where one was, the other follows it.
    pub high_measured: bool,
    pub top_measured: bool,
}

/// A band the decay was fitted against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecayBand {
    pub centre_hz: f32,
    pub target_s: f32,
    pub weight: f32,
    /// The engine's closed-form octave decay for the fitted space and controls, at the band's centre.
    pub predicted_s: f32,
    /// The same closed form read as the analyser reads a band — every frequency across it, weighted
    /// by the fitted tone — which is what the decay solve matched. It reads longer than `predicted_s`
    /// where the decay changes steeply inside the octave.
    pub modelled_s: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneReport {
    pub bands_used: usize,
    /// The closed form's root-mean-square distance from the measured tone curve, dB, for the tone as
    /// the space carries it: the shelves inside their bound and the high cut as fitted.
    pub residual_db: Option<f32>,
    /// The same distance with the high cut open — what the shelves alone leave. The cut is taken only
    /// where it improves on this by more than [`HIGH_CUT_MIN_IMPROVEMENT_DB`].
    pub open_residual_db: Option<f32>,
    /// The fitted high cut's corner, hertz, before the space's bound;
    /// `mxm_classic_verb_dsp::space::HIGH_CUT_OPEN_HZ` when the cut is open.
    pub high_cut_hz: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EarlyReport {
    pub first_arrival: FirstArrival,
    pub taps: usize,
    /// Reflections too late for any tap at the fitted Size (three units after the pre-delay).
    pub beyond_reach: usize,
    /// The response's early-to-late energy ratio, dB; absent where its late window held nothing.
    pub target_early_to_late_db: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WidthReport {
    NotMeasured(WidthNotMeasured),
    Matched {
        target_peak_iacf: f32,
        rendered_peak_iacf: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchReport {
    pub evaluations: u32,
    /// The chosen candidate's profile distance.
    pub profile_distance: Option<f32>,
    /// The chosen candidate's early envelope distance, dB.
    pub envelope_distance_db: Option<f32>,
    /// How long each candidate render was.
    pub excerpt_s: f32,
}

/// The verification: fitted render against response, descriptor by descriptor. A reading one side
/// could not measure is absent.
#[derive(Clone, Debug, PartialEq)]
pub struct DescriptorErrors {
    pub bands: Vec<BandError>,
    /// Render minus response, seconds.
    pub pre_delay_s: Option<f32>,
    pub mixing_time_s: Option<f32>,
    /// Root-mean-square echo density difference over the compared span.
    pub profile_distance: Option<f32>,
    /// Root-mean-square early energy envelope difference over the same span, dB.
    pub envelope_db: Option<f32>,
    /// Render minus response late IACC.
    pub iacc: Option<f32>,
    /// Render over response, dB.
    pub early_to_late_db: Option<f32>,
    /// Over the response's strongest reflections that the render also has within
    /// [`REFLECTION_MATCH_S`]: time, and level against each side's strongest reflection.
    pub reflection_time_rms_s: Option<f32>,
    pub reflection_level_rms_db: Option<f32>,
    pub reflections_matched: usize,
    pub reflections_compared: usize,
    /// The tail texture over the response's segment A, render minus response. **Reported only**: no
    /// texture error refuses a fit or enters either confidence.
    pub texture_a: TextureErrors,
    /// The same over the response's segment B.
    pub texture_b: TextureErrors,
    pub target_confidence: f32,
    pub verification_confidence: f32,
}

/// A tail texture's readings over one of the response's segments, render minus response. Each is absent
/// where either side lacks the reading, which includes a response with no such segment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureErrors {
    pub peakiness: Option<f32>,
    pub kurtosis: Option<f32>,
    pub echo_density: Option<f32>,
    pub periodicity: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandError {
    pub centre_hz: f32,
    /// Render over response, percent.
    pub t30_percent: Option<f32>,
    pub t20_percent: Option<f32>,
    /// Render minus response tone against 1 kHz, dB.
    pub tone_db: Option<f32>,
}

// ---------------------------------------------------------------------------------------------------
// The fit.
// ---------------------------------------------------------------------------------------------------

/// [`fit_with`] at the default ranges.
pub fn fit(channels: &[&[f32]], sample_rate: f32) -> Result<Fit, FitRefusal> {
    fit_with(channels, sample_rate, &FitOptions::default())
}

/// Analyses a response, fits a space and the four absolute controls to it, renders the result and
/// analyses that render, and returns the fit with the error for every descriptor — or a refusal.
pub fn fit_with(
    channels: &[&[f32]],
    sample_rate: f32,
    options: &FitOptions,
) -> Result<Fit, FitRefusal> {
    let target = crate::analyse(channels, sample_rate).map_err(FitRefusal::of_analysis)?;
    let rate = f64::from(sample_rate);
    let mono = channels.len() == 1;
    let decay_targets = solve::decay_targets(&target.bands);
    if !solve::has_mid_band(&decay_targets) {
        return Err(FitRefusal::NoMidBandDecay);
    }

    let problem = problem(&target, channels, sample_rate, options, decay_targets);
    let (direct, target_ratio) = (problem.direct, problem.target_ratio);
    let excerpt_s = problem.excerpt_frames as f64 / rate;

    let mut renderer = Renderer::new(sample_rate);
    let outcome = search::search(&problem, &mut renderer)?;
    let mut settings = outcome.best.settings;

    let width = match &target.width {
        Width::Measured(coherence) if !mono => {
            let end = samples(f64::from(coherence.window_end_s), rate);
            let (space, controls) = model(
                &problem,
                &Settings {
                    width: 1.0,
                    ..settings
                },
            );
            let wet = renderer.impulse(&space, &controls, 0, end + 1)?;
            match solve::solve_width(&wet, rate, end, coherence.peak_iacf) {
                Some(solution) => {
                    settings.width = solution.width;
                    WidthReport::Matched {
                        target_peak_iacf: coherence.peak_iacf,
                        rendered_peak_iacf: solution.peak_iacf,
                    }
                }
                None => WidthReport::NotMeasured(WidthNotMeasured::NoLateField),
            }
        }
        Width::Measured(_) => WidthReport::NotMeasured(WidthNotMeasured::Mono),
        Width::NotMeasured(why) => WidthReport::NotMeasured(*why),
    };

    let (space, controls) = model(&problem, &settings);
    let clamps = clamps(&problem, &settings, &space, &controls);

    // Verification: the fitted reverb's response at the file's rate, a direct impulse after leading
    // silence, the wet scaled to the response's DRR, analysed by the same analyser — its tail texture on
    // the response's segments, which the render's length after its direct sound always holds.
    let lead = samples(VERIFY_LEAD_S, rate);
    let frames = (lead + channels[0].len().saturating_sub(direct))
        .max(samples(crate::MIN_DURATION_S, rate) + 1)
        .min((crate::MAX_DURATION_S * rate).floor() as usize);
    let wet = fold(renderer.impulse(&space, &controls, lead, frames)?, mono)?;
    let wet_gain = wet_gain(&wet, lead, rate, target.onset.drr_db);
    let mut response: Vec<Vec<f32>> = Vec::with_capacity(wet.len());
    for channel in &wet {
        let mut v = buffer::collected(channel.len(), channel.iter().map(|&x| x * wet_gain))?;
        v[lead] += 1.0;
        response.push(v);
    }
    let (verification, verification_refusal) =
        crate::analyse_render(&slices(&response), sample_rate, &target.texture_segments())
            .map_err(FitRefusal::of_verification)?;
    let verification_direct = (f64::from(verification.onset.direct_s) * rate).round() as usize;
    let verification_ratio = solve::energy_ratio(
        &slices(&response),
        verification_direct,
        &problem.windows,
        0.0,
    );
    let verification_envelope = search::envelope(
        &slices(&response),
        verification_direct,
        rate,
        &problem.windows,
        0.0,
        target.echo_density.points.len(),
    );
    let errors = errors(
        &target,
        &verification,
        problem.profile_end_s,
        (target_ratio, verification_ratio),
        (&problem.target_envelope, &verification_envelope),
    );

    let modelled = solve::DecayReading::new(&problem.decay_targets, rate, problem.tone.carried())
        .t30s(&space, &controls, sample_rate);
    let decay = DecayReport {
        bands: problem
            .decay_targets
            .iter()
            .zip(modelled)
            .map(|(t, modelled_s)| DecayBand {
                centre_hz: t.centre_hz,
                target_s: t.seconds,
                weight: t.weight as f32,
                predicted_s: clamped_f32(f64::from(predicted_decay_s(
                    &space,
                    &controls,
                    sample_rate,
                    t.centre_hz,
                ))),
                modelled_s: clamped_f32(modelled_s),
            })
            .collect(),
        low_measured: settings.decay.low_measured,
        high_measured: settings.decay.high_measured,
        top_measured: settings.decay.top_measured,
    };
    let report = FitReport {
        decay,
        tone: ToneReport {
            bands_used: problem.tone.bands_used,
            residual_db: problem.tone.residual_db.and_then(finite_f32),
            open_residual_db: problem.tone.open_residual_db.and_then(finite_f32),
            high_cut_hz: clamped_f32(problem.tone.high_cut_hz),
        },
        early: EarlyReport {
            first_arrival: settings.arrival,
            taps: settings.taps.kept,
            beyond_reach: settings.taps.beyond_reach,
            target_early_to_late_db: target_ratio
                .filter(|r| *r > 0.0)
                .and_then(|r| finite_f32(10.0 * r.log10())),
        },
        width,
        search: SearchReport {
            evaluations: outcome.evaluations,
            profile_distance: outcome.best.profile.and_then(finite_f32),
            envelope_distance_db: outcome.best.envelope.and_then(finite_f32),
            excerpt_s: clamped_f32(excerpt_s),
        },
        wet_gain,
        errors,
        clamps,
        verification,
        verification_refusal,
        target,
    };
    Ok(Fit {
        space,
        controls: FittedControls {
            decay_s: controls.decay_s,
            size_s: controls.size_s,
            diffusion: controls.diffusion,
            pre_delay_s: controls.pre_delay_s,
        },
        report,
    })
}

/// What the search holds fixed for a response: its descriptors, the calculated parts, and the
/// windows and spans every candidate is read over.
pub(crate) fn problem<'a>(
    target: &'a Analysis,
    channels: &[&[f32]],
    sample_rate: f32,
    options: &'a FitOptions,
    decay_targets: Vec<solve::DecayTarget>,
) -> Problem<'a> {
    let rate = f64::from(sample_rate);
    let mono = channels.len() == 1;
    let pre_delay_s = target.pre_delay_s.map_or(0.0, f64::from);
    let windows = solve::balance_windows(pre_delay_s, rate);
    let direct = (f64::from(target.onset.direct_s) * rate).round() as usize;
    let noise = 10f64.powf(f64::from(target.broadband.noise_floor_db) / 10.0);
    let target_ratio = solve::energy_ratio(channels, direct, &windows, noise);
    let target_envelope = search::envelope(
        channels,
        direct,
        rate,
        &windows,
        noise,
        target.echo_density.points.len(),
    );
    let excerpt_s =
        pre_delay_s + PROFILE_SPAN_S.max(2.0 * BALANCE_WINDOW_S) + crate::ECHO_DENSITY_WINDOW_S;
    let longest_decay_s = decay_targets
        .iter()
        .map(|t| f64::from(t.seconds))
        .fold(0.0, f64::max);
    let size_s = f64::from(size_for_total_delay_s(
        (DENSITY_FLOOR_DECAY_SHARE * longest_decay_s).max(DENSITY_FLOOR_MIN_S) as f32,
    ));
    Problem {
        rate: sample_rate,
        mono,
        target,
        decay_targets,
        size_s,
        tone: solve::solve_tone(target, rate),
        arrivals: solve::arrivals(target, pre_delay_s, mono),
        pre_delay_s,
        direct,
        windows,
        target_ratio,
        target_envelope,
        ranges: &options.ranges,
        profile_end_s: pre_delay_s + PROFILE_SPAN_S,
        excerpt_frames: samples(excerpt_s, rate) + 1,
    }
}

/// Every value that lay outside its range or bound, against where it was put.
fn clamps(problem: &Problem, s: &Settings, space: &Space, controls: &Controls) -> Vec<Clamp> {
    let mut out = Vec::new();
    let mut check = |value: FitValue, fitted: f64, applied: f32| {
        let fitted = clamped_f32(fitted);
        if fitted != applied {
            out.push(Clamp {
                value,
                fitted,
                applied,
            });
        }
    };
    check(FitValue::Decay, s.decay.decay_s, controls.decay_s);
    // Size is the density floor; a range that moved it is reported against the floor.
    check(FitValue::Size, problem.size_s, controls.size_s);
    check(FitValue::Diffusion, s.diffusion, controls.diffusion);
    check(
        FitValue::PreDelay,
        pre_delay_control(problem.pre_delay_s, s.arrival, s.size_s),
        controls.pre_delay_s,
    );
    check(
        FitValue::DecayRatioLow,
        s.decay.ratio_low,
        space.decay_ratio_low,
    );
    check(
        FitValue::DecayRatioHigh,
        s.decay.ratio_high,
        space.decay_ratio_high,
    );
    check(
        FitValue::DecayRatioTop,
        s.decay.ratio_top,
        space.decay_ratio_top,
    );
    check(FitValue::ToneLow, problem.tone.low_db, space.tone_low_db);
    check(FitValue::ToneHigh, problem.tone.high_db, space.tone_high_db);
    check(
        FitValue::HighCut,
        problem.tone.high_cut_hz,
        space.high_cut_hz,
    );
    check(FitValue::Width, s.width, space.width);
    check(FitValue::EarlyLevel, s.early_level, space.early_level);
    out
}

/// The wet scale at which direct impulse plus wet reads the response's DRR, held under
/// [`WET_PEAK_CEILING`] of the direct impulse.
fn wet_gain(wet: &[Vec<f32>], lead: usize, rate: f64, drr_db: Option<f32>) -> f32 {
    let half = samples(crate::DIRECT_HALF_WINDOW_S, rate);
    let (first, last) = (lead.saturating_sub(half), lead + half);
    let (mut inside, mut outside, mut peak) = (0.0f64, 0.0f64, 0.0f64);
    for channel in wet {
        for (n, &v) in channel.iter().enumerate() {
            let v = f64::from(v);
            if (first..=last).contains(&n) {
                inside += v * v;
            } else {
                outside += v * v;
            }
            peak = peak.max(v.abs());
        }
    }
    let direct = wet.len() as f64;
    let gain = drr_db
        .map(|drr| 10f64.powf(f64::from(drr) / 10.0) * outside - inside)
        .filter(|denominator| *denominator > 0.0)
        .map_or(1.0, |denominator| (direct / denominator).sqrt());
    let ceiling = if peak > 0.0 {
        WET_PEAK_CEILING / peak
    } else {
        gain
    };
    clamped_f32(gain.min(ceiling))
}

fn percent(render: Option<f32>, response: Option<f32>) -> Option<f32> {
    match (render, response) {
        (Some(a), Some(b)) if b > 0.0 => finite_f32(100.0 * (f64::from(a) / f64::from(b) - 1.0)),
        _ => None,
    }
}

fn difference(render: Option<f32>, response: Option<f32>) -> Option<f32> {
    match (render, response) {
        (Some(a), Some(b)) => finite_f32(f64::from(a) - f64::from(b)),
        _ => None,
    }
}

fn errors(
    target: &Analysis,
    verification: &Analysis,
    profile_end_s: f64,
    (target_ratio, verification_ratio): (Option<f64>, Option<f64>),
    (target_envelope, verification_envelope): (&[Option<f64>], &[Option<f64>]),
) -> DescriptorErrors {
    let bands = target
        .bands
        .iter()
        .zip(&verification.bands)
        .map(|(t, v)| {
            let (td, vd) = (t.decay(), v.decay());
            BandError {
                centre_hz: t.centre_hz,
                t30_percent: percent(vd.and_then(|d| d.t30_s), td.and_then(|d| d.t30_s)),
                t20_percent: percent(vd.and_then(|d| d.t20_s), td.and_then(|d| d.t20_s)),
                tone_db: difference(
                    vd.and_then(|d| d.level_re_1k_db),
                    td.and_then(|d| d.level_re_1k_db),
                ),
            }
        })
        .collect();
    let densities: Vec<Option<f64>> = verification
        .echo_density
        .points
        .iter()
        .map(|p| p.density.map(f64::from))
        .collect();
    let iacc = match (target.width.measured(), verification.width.measured()) {
        (Some(t), Some(v)) => finite_f32(f64::from(v.iacc) - f64::from(t.iacc)),
        _ => None,
    };
    let early_to_late_db = match (target_ratio, verification_ratio) {
        (Some(t), Some(v)) if t > 0.0 && v > 0.0 => finite_f32(10.0 * (v / t).log10()),
        _ => None,
    };

    // The response's strongest reflections against the render's, each side's levels taken against its
    // own strongest reflection, since only the pattern — not its level against the direct — is the
    // space's.
    let strongest = |a: &Analysis| {
        let mut list: Vec<(f64, f64)> = a
            .early
            .reflections
            .iter()
            .map(|r| (f64::from(r.delay_s), f64::from(r.level_db)))
            .collect();
        list.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.total_cmp(&y.0)));
        list.truncate(mxm_classic_verb_dsp::EARLY_TAPS);
        list
    };
    let (t_list, v_list) = (strongest(target), strongest(verification));
    let (t_top, v_top) = (t_list.first().map(|r| r.1), v_list.first().map(|r| r.1));
    let (mut time_sq, mut level_sq, mut matched) = (0.0f64, 0.0f64, 0usize);
    if let (Some(t_top), Some(v_top)) = (t_top, v_top) {
        for &(time, level) in &t_list {
            let nearest = v_list
                .iter()
                .min_by(|a, b| (a.0 - time).abs().total_cmp(&(b.0 - time).abs()));
            if let Some(&(v_time, v_level)) = nearest {
                if (v_time - time).abs() <= REFLECTION_MATCH_S {
                    time_sq += (v_time - time).powi(2);
                    level_sq += ((v_level - v_top) - (level - t_top)).powi(2);
                    matched += 1;
                }
            }
        }
    }
    let rms = |sum: f64| {
        (matched > 0)
            .then(|| (sum / matched as f64).sqrt())
            .and_then(finite_f32)
    };

    // The render was read on the response's segments, so each reading compares like with like.
    let texture = |response: Option<&TailTexture>, render: Option<&TailTexture>| {
        let pair = |read: fn(&TailTexture) -> Option<f32>| {
            difference(render.and_then(read), response.and_then(read))
        };
        TextureErrors {
            peakiness: pair(|t| t.peakiness),
            kurtosis: pair(|t| t.kurtosis),
            echo_density: pair(|t| t.echo_density),
            periodicity: pair(|t| t.periodicity),
        }
    };

    DescriptorErrors {
        bands,
        pre_delay_s: difference(verification.pre_delay_s, target.pre_delay_s),
        mixing_time_s: difference(
            verification.echo_density.mixing_time_s,
            target.echo_density.mixing_time_s,
        ),
        profile_distance: profile_distance(&target.echo_density.points, &densities, profile_end_s)
            .and_then(finite_f32),
        envelope_db: search::envelope_distance(
            &target.echo_density.points,
            target_envelope,
            verification_envelope,
            profile_end_s,
        )
        .and_then(finite_f32),
        iacc,
        early_to_late_db,
        reflection_time_rms_s: rms(time_sq),
        reflection_level_rms_db: rms(level_sq),
        reflections_matched: matched,
        reflections_compared: t_list.len(),
        texture_a: texture(target.texture_a.as_ref(), verification.texture_a.as_ref()),
        texture_b: texture(target.texture_b.as_ref(), verification.texture_b.as_ref()),
        target_confidence: target.validity.confidence,
        verification_confidence: verification.validity.confidence,
    }
}
