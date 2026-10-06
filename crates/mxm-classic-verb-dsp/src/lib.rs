//! Framework-free DSP for `mxm-classic-verb`: a feedback delay network reverberator.
//!
//! The plan is `plans/plan-mxm-classic-verb.md`; the technique, with every method's source, is
//! `research:effects/feedback-delay-network-reverb.md`.
//!
//! ```text
//!                     ┌─ early reflections: taps on the input history × Size ─────────────┐
//! in ─► history ─► pre-delay ─┤                                                            ├─► tone ─► width ─► duck ─► mix ─► out
//!                     └─ diffusion ─► network (16 lines, loop allpasses, decay filters) ─ late ┤
//!                                     or, for a shaped decay, feed-forward taps ─── late ─┘
//! in ──────────────────────────────────────────────── dry ─────────────────────────────────────────────────────► mix
//! ```
//!
//! **A space is shape; the controls are scale** (plan §2). [`Space`] holds what a fit produces;
//! [`Controls`] holds absolute quantities and offsets relative to the space, composed by the laws
//! plan §2 states. Every constant below that is not a definition is **chosen** and says so.

mod delay;
mod filter;
mod network;
mod shaped;
pub mod space;

pub use network::{
    Interpolation, LINES, LOOP_ALLPASS_GAIN, LOOP_ALLPASSES, OUTPUT_TAPS, RATIO_MAX, RATIO_MIN,
    RATIOS, late_onset_s, size_for_total_delay_s, total_delay_s,
};
pub use shaped::{SHAPE_MAX_S, SHAPE_MIN_S, SHAPE_TAPS};
pub use space::{EARLY_MAX_UNITS, EARLY_TAPS, EarlyTap, SPACE_VERSION, Space};

use core::f32::consts::{FRAC_PI_2, LN_10};
use delay::{DelayLine, Ramp, Smoother, flush, follower_coefficient};
use filter::{
    Allpass, BandGains, BilinearCoefficients, HighCut, HighCutCoefficients, Tilt, decay_magnitude,
    lowpass_response, pole, tilt_magnitude,
};
use network::Network;
use shaped::ShapedPath;

/// The sample rates the engine accepts; a rate outside is clamped. The validated set is the one the
/// tests sweep, 8–192 kHz.
pub const MIN_SAMPLE_RATE: f32 = 8_000.0;
pub const MAX_SAMPLE_RATE: f32 = 384_000.0;

/// Mid-band decay time. Very long, always finite: Freeze is `mxm-shimmer`'s. **Chosen.**
pub const MIN_DECAY_S: f32 = 0.1;
pub const MAX_DECAY_S: f32 = 60.0;
/// Bass and treble decay multipliers on the space's ratios. **Chosen.**
pub const MIN_BAND_MULT: f32 = 0.1;
pub const MAX_BAND_MULT: f32 = 10.0;
/// Size: the network's mean delay. From a boxy resonator to past any building. **Chosen.**
pub const MIN_SIZE_S: f32 = 0.002;
pub const MAX_SIZE_S: f32 = 0.3;
pub const MAX_PRE_DELAY_S: f32 = 0.5;
/// Early/late offset at or beyond which one part is silent outright. **Chosen**: at the end of the
/// travel the other part sits 12 dB up, loud enough to hear the move and no louder.
pub const EARLY_LATE_SILENCE_DB: f32 = 24.0;
pub const MAX_MOD_DEPTH_S: f32 = 0.004;
pub const MAX_MOD_RATE_HZ: f32 = 10.0;
pub const MIN_WIDTH: f32 = -1.0;
pub const MAX_WIDTH: f32 = 3.0;
pub const MAX_TONE_DB: f32 = 24.0;

/// The low band's edge, in the decay filter and the output tilt alike. **Chosen.**
pub const LOW_CROSSOVER_HZ: f32 = 250.0;
/// The output tilt's high shelf. **Chosen.** The decay filter's upper shelves have corners of their own.
pub const HIGH_CROSSOVER_HZ: f32 = 4_000.0;
/// The decay filter's upper shelves (plan D5's four bands): the high band's and the top band's.
/// **Chosen on a measured plateau**, each at the geometric mean of the calibration octaves either side
/// of it — 2 kHz between 1 and 4 kHz, 5.66 kHz between 4 and 8 kHz (this crate's `AGENTS.md`, *Four
/// decay bands*).
pub const DECAY_HIGH_CROSSOVER_HZ: f32 = 2_000.0;
pub const DECAY_TOP_CROSSOVER_HZ: f32 = 5_656.854;
/// Where the output tone is normalised to unity, so tone never poses as a mix control (plan §2).
pub const TONE_REFERENCE_HZ: f32 = 1_000.0;

/// The largest gain any decay filter may reach at any frequency, by construction (plan §3.2).
/// **Chosen**: a 60 s decay on the shortest line at the highest validated rate still sits below it.
pub const LOOP_GAIN_CEILING: f32 = 0.9999;

/// Below this the signal counts as quiet: −100 dB. **Chosen.**
pub const QUIET_LEVEL: f32 = 1.0e-5;
pub const QUIET_DB: f32 = -100.0;
/// Quiet for this long beyond a full trip round the longest line, and the engine empties and parks.
pub const QUIET_HOLD_S: f32 = 0.05;
/// A space or shape change fades the wet through silence over this long each way (plan §3.4).
pub const SPACE_FADE_S: f32 = 0.03;
/// Size glides toward its target with this time constant (plan D8's default).
pub const SIZE_GLIDE_S: f32 = 0.12;
const CONTROL_SMOOTH_S: f32 = 0.01;
/// Mix ramps linearly, so Off — exactly zero — arrives in a bounded time.
const MIX_RAMP_S: f32 = 0.02;
/// A path started from empty by a shape change ramps its input in over this long, so the signal it
/// is fed does not begin with a step.
const PATH_ONSET_S: f32 = 0.05;
/// A shaped decay's diffusers ring on this long after its taps end. **Chosen**, conservatively.
const SHAPED_TAIL_S: f32 = 0.1;
const LEVEL_ATTACK_S: f32 = 0.0004;
const LEVEL_RELEASE_S: f32 = 0.05;
const DUCK_ATTACK_S: f32 = 0.005;
const DUCK_RELEASE_S: f32 = 0.25;
/// The input level at which Ducking reaches its full depth. **Chosen**: about −12 dBFS.
const DUCK_REFERENCE: f32 = 0.25;
/// The diffusion allpasses' coefficient at Diffusion 1. **Chosen**, inside Dattorro's 0.625–0.75.
pub const DIFFUSION_MAX_GAIN: f32 = 0.7;
const DIFFUSER_S_L: [f32; 4] = [0.00413, 0.00277, 0.00931, 0.00667];
const DIFFUSER_S_R: [f32; 4] = [0.00439, 0.00301, 0.00887, 0.00631];
/// Samples between recomputing the network's per-line gains. Counted internally, so the output
/// does not depend on how a host divides its blocks.
const CONTROL_TICK: u32 = 32;
/// The tail declaration's allowance for filters and modulation stretching the decay. **Chosen.**
const TAIL_MARGIN: f32 = 1.25;
/// The octaves each band's per-trip loss is calibrated at — low, mid, high, top: the fit's own band
/// centres, with the tone's reference as mid. **Chosen.**
const CALIBRATION_HZ: [f32; 4] = [125.0, 1_000.0, 4_000.0, 8_000.0];

/// The decay's envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecayShape {
    /// The network's exponential decay.
    Natural,
    /// A flat block that stops dead.
    Gated,
    /// A swell that stops.
    Reverse,
}

/// The performed controls, in absolute units or as offsets relative to the space (plan §2.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Controls {
    /// Dry/wet, equal-power. Zero is Off.
    pub mix: f32,
    /// Mid-band decay time, seconds. For a shaped decay, the shape's length.
    pub decay_s: f32,
    /// Multipliers on the space's decay ratios — bass on the low band's, treble on the high and top
    /// bands' — 1 is the space.
    pub bass_mult: f32,
    pub treble_mult: f32,
    /// The network's mean delay, seconds.
    pub size_s: f32,
    /// 0–1: the input diffusers' coefficient.
    pub diffusion: f32,
    pub pre_delay_s: f32,
    /// Offset on the space's early/late balance, dB; 0 is the space.
    pub early_late_db: f32,
    pub shape: DecayShape,
    pub mod_depth_s: f32,
    pub mod_rate_hz: f32,
    /// Signed share of the space's width; 1 is the space, 0 mono, negative flips the image.
    pub width: f32,
    /// Offsets on the space's low and high shelves, dB; 0 is the space. The space's high cut has no
    /// control of its own: it is the space's shape, and High tone offsets the shelf ahead of it.
    pub tone_low_db: f32,
    pub tone_high_db: f32,
    /// 0–1: how far the wet ducks under the input.
    pub duck: f32,
}

impl Default for Controls {
    /// A neutral, audible setting for tests. **Not** the plugin's Init, which P3.5 chooses by ear.
    fn default() -> Self {
        Self {
            mix: 0.3,
            decay_s: 1.8,
            bass_mult: 1.0,
            treble_mult: 1.0,
            size_s: 0.04,
            diffusion: 0.75,
            pre_delay_s: 0.01,
            early_late_db: 0.0,
            shape: DecayShape::Natural,
            mod_depth_s: 0.0,
            mod_rate_hz: 0.5,
            width: 1.0,
            tone_low_db: 0.0,
            tone_high_db: 0.0,
            duck: 0.0,
        }
    }
}

fn finite_clamp(x: f32, fallback: f32, lo: f32, hi: f32) -> f32 {
    if x.is_finite() {
        x.clamp(lo, hi)
    } else {
        fallback
    }
}

impl Controls {
    /// Every value bounded; a non-finite value takes the default's.
    pub fn sanitised(self) -> Self {
        let d = Controls::default();
        Self {
            mix: finite_clamp(self.mix, 0.0, 0.0, 1.0),
            decay_s: finite_clamp(self.decay_s, d.decay_s, MIN_DECAY_S, MAX_DECAY_S),
            bass_mult: finite_clamp(self.bass_mult, 1.0, MIN_BAND_MULT, MAX_BAND_MULT),
            treble_mult: finite_clamp(self.treble_mult, 1.0, MIN_BAND_MULT, MAX_BAND_MULT),
            size_s: finite_clamp(self.size_s, d.size_s, MIN_SIZE_S, MAX_SIZE_S),
            diffusion: finite_clamp(self.diffusion, d.diffusion, 0.0, 1.0),
            pre_delay_s: finite_clamp(self.pre_delay_s, 0.0, 0.0, MAX_PRE_DELAY_S),
            early_late_db: finite_clamp(
                self.early_late_db,
                0.0,
                -EARLY_LATE_SILENCE_DB,
                EARLY_LATE_SILENCE_DB,
            ),
            shape: self.shape,
            mod_depth_s: finite_clamp(self.mod_depth_s, 0.0, 0.0, MAX_MOD_DEPTH_S),
            mod_rate_hz: finite_clamp(self.mod_rate_hz, d.mod_rate_hz, 0.0, MAX_MOD_RATE_HZ),
            width: finite_clamp(self.width, 1.0, MIN_WIDTH, MAX_WIDTH),
            tone_low_db: finite_clamp(self.tone_low_db, 0.0, -MAX_TONE_DB, MAX_TONE_DB),
            tone_high_db: finite_clamp(self.tone_high_db, 0.0, -MAX_TONE_DB, MAX_TONE_DB),
            duck: finite_clamp(self.duck, 0.0, 0.0, 1.0),
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    (db * LN_10 / 20.0).exp()
}

/// The gain per trip round a line of `samples` for a decay of `t60` seconds:
/// `10^(−3·m/(f_s·T₆₀))` (Jot and Chaigne, 1991, as restated in
/// `research:effects/feedback-delay-network-reverb.md` §4.1).
fn trip_gain(samples: f32, t60: f32, sample_rate: f32) -> f32 {
    (-3.0 * LN_10 * samples / (sample_rate * t60)).exp()
}

fn clamped_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() {
        sample_rate.clamp(MIN_SAMPLE_RATE, MAX_SAMPLE_RATE)
    } else {
        48_000.0
    }
}

/// The tone's reference in radians per sample: [`TONE_REFERENCE_HZ`], held below Nyquist.
fn reference_w(fs: f32) -> f32 {
    core::f32::consts::TAU * TONE_REFERENCE_HZ.min(0.45 * fs) / fs
}

/// Size in samples, never so small that a modulated read would reach inside 2 samples.
fn effective_size_samples(size_samples: f32, depth_samples: f32) -> f32 {
    size_samples.max((2.0 + depth_samples.min(0.25 * RATIO_MIN * size_samples)) / RATIO_MIN)
}

/// The composed decay time of each band, seconds: the octaves at 125 Hz, 1 kHz, 4 kHz and 8 kHz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandDecays {
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub top: f32,
}

impl BandDecays {
    /// The longest of the four.
    pub fn longest(&self) -> f32 {
        self.low.max(self.mid).max(self.high).max(self.top)
    }
}

/// The composed decay times for a space and controls, after plan §2's composition law and its
/// clamps. Treble multiplies both upper bands.
pub fn composed_decay_s(space: &Space, controls: &Controls) -> BandDecays {
    let (s, c) = (space.sanitised(), controls.sanitised());
    let mid = c.decay_s;
    let band = |ratio: f32, mult: f32| (mid * ratio * mult).clamp(MIN_DECAY_S, MAX_DECAY_S);
    BandDecays {
        low: band(s.decay_ratio_low, c.bass_mult),
        mid,
        high: band(s.decay_ratio_high, c.treble_mult),
        top: band(s.decay_ratio_top, c.treble_mult),
    }
}

/// Every line's decay-filter gains for a space and controls at `sample_rate`, limited so no filter
/// can exceed [`LOOP_GAIN_CEILING`] at any frequency.
fn line_gains(
    space: &Space,
    controls: &Controls,
    sample_rate: f32,
    loop_trips: &[[f32; 4]; LINES],
) -> [BandGains; LINES] {
    let d = composed_decay_s(space, controls);
    let c = controls.sanitised();
    let size_samples = effective_size_samples(c.size_s * sample_rate, c.mod_depth_s * sample_rate);
    core::array::from_fn(|i| {
        // A trip is the line plus its loop allpasses' mean group delay in the band calibrated.
        let m = RATIOS[i] * size_samples;
        let gain = |band: usize, t60: f32| trip_gain(m + loop_trips[i][band], t60, sample_rate);
        let (g_mid, g_high) = (gain(1, d.mid), gain(2, d.high));
        BandGains {
            low: gain(0, d.low),
            mid: g_mid,
            high_ratio: g_high / g_mid,
            top_ratio: gain(3, d.top) / g_high,
        }
        .limited(LOOP_GAIN_CEILING)
    })
}

/// Each line's loop allpasses' mean group delay in the octave of each [`CALIBRATION_HZ`], samples.
fn loop_trips(sample_rate: f32) -> [[f32; 4]; LINES] {
    core::array::from_fn(|i| {
        CALIBRATION_HZ.map(|hz| {
            network::loop_group_delay_samples(
                i,
                sample_rate,
                hz * core::f32::consts::FRAC_1_SQRT_2,
                hz * core::f32::consts::SQRT_2,
            )
        })
    })
}

/// The output tone's magnitude at `hz`, in closed form: the total shelf settings in decibels — a
/// space's tone plus the controls' offsets — and the space's high-cut corner in hertz.
///
/// It is the engine's tone path: first-order shelves at [`LOW_CROSSOVER_HZ`] and
/// [`HIGH_CROSSOVER_HZ`], then the second-order Butterworth high cut at `high_cut_hz` — bypassed at
/// or above [`space::HIGH_CUT_OPEN_HZ`], held below Nyquist as every corner is — and the whole path
/// divided by its magnitude at [`TONE_REFERENCE_HZ`] exactly as the engine divides, so the
/// reference reads unity. The corner is taken as given, not bounded to [`space::MIN_HIGH_CUT_HZ`],
/// just as the shelves are not bounded to the space's range. Anything that has to predict the tone
/// rather than render it — a fit solving tone against a measured response — is held to this
/// (plan §4.3). `hz` is below Nyquist.
pub fn tone_magnitude(
    low_db: f32,
    high_db: f32,
    high_cut_hz: f32,
    sample_rate: f32,
    hz: f32,
) -> f32 {
    let fs = clamped_rate(sample_rate);
    let (pole_lo, pole_hi) = (pole(LOW_CROSSOVER_HZ, fs), pole(HIGH_CROSSOVER_HZ, fs));
    let (low, high) = (db_to_gain(low_db), db_to_gain(high_db));
    let cut = HighCutCoefficients::new(high_cut_hz, fs);
    let at = |w: f32| {
        tilt_magnitude(
            low,
            high,
            lowpass_response(pole_lo, w),
            lowpass_response(pole_hi, w),
        ) * cut.map_or(1.0, |c| c.magnitude(w))
    };
    at(core::f32::consts::TAU * hz / fs) / at(reference_w(fs)).max(1.0e-6)
}

/// The decay time a network with this space and these controls realises at `hz`, in closed form
/// and without allocating: each line's attenuation per sample at that frequency, from its decay
/// filter's exact response, averaged across the lines, which the orthogonal matrix mixes evenly.
///
/// First-order shelves only approach their band targets, so this — not [`composed_decay_s`] — is
/// what an editor draws, and what a fit reads across each band to match a measured response (plan
/// §4.3). Measured within 5.2 % of the realised octave decay (`tests/engine.rs`). A shaped decay has
/// no network decay, and this still answers for the network the settings would give.
pub fn predicted_decay_s(space: &Space, controls: &Controls, sample_rate: f32, hz: f32) -> f32 {
    let mut out = [0.0];
    predicted_decays_s(space, controls, sample_rate, &[hz], &mut out);
    out[0]
}

/// [`predicted_decay_s`] at many frequencies — each `hz` answered in `out` at the same index, up to
/// the shorter of the two — with the lines' gains computed once. For anything that reads the closed
/// form across a band, where a call per frequency would recompute every line's gains.
pub fn predicted_decays_s(
    space: &Space,
    controls: &Controls,
    sample_rate: f32,
    hz: &[f32],
    out: &mut [f32],
) {
    let fs = clamped_rate(sample_rate);
    let c = controls.sanitised();
    let size_samples = effective_size_samples(c.size_s * fs, c.mod_depth_s * fs);
    let gains = line_gains(space, controls, fs, &loop_trips(fs));
    let pole_lo = pole(LOW_CROSSOVER_HZ, fs);
    let high = BilinearCoefficients::new(DECAY_HIGH_CROSSOVER_HZ, fs);
    let top = BilinearCoefficients::new(DECAY_TOP_CROSSOVER_HZ, fs);
    for (&hz, out) in hz.iter().zip(out.iter_mut()) {
        let w = core::f32::consts::TAU * hz.clamp(1.0, 0.45 * fs) / fs;
        let (lo, hi, tp) = (
            lowpass_response(pole_lo, w),
            high.response(w),
            top.response(w),
        );
        let (band_lo, band_hi) = (
            hz * core::f32::consts::FRAC_1_SQRT_2,
            hz * core::f32::consts::SQRT_2,
        );
        let mut db_per_sample = 0.0f64;
        for (i, g) in gains.iter().enumerate() {
            let m = f64::from(
                RATIOS[i] * size_samples
                    + network::loop_group_delay_samples(i, fs, band_lo, band_hi),
            );
            let magnitude = f64::from(decay_magnitude(*g, lo, hi, tp).max(1.0e-9));
            db_per_sample += 20.0 * magnitude.log10() / m;
        }
        let rate = db_per_sample / LINES as f64;
        *out = if rate >= 0.0 {
            f32::INFINITY
        } else {
            (-60.0 / (rate * f64::from(fs))) as f32
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fade {
    Idle,
    Out,
    In,
}

/// The reverberator.
#[derive(Debug, Clone)]
pub struct Engine {
    sample_rate: f32,
    controls: Controls,
    space: Space,
    pending_space: Option<Space>,
    shape: DecayShape,
    pending_shape: Option<DecayShape>,
    fade: Fade,
    fade_gain: f32,
    fade_step: f32,
    hist_l: DelayLine,
    hist_r: DelayLine,
    diff_l: [Allpass; 4],
    diff_r: [Allpass; 4],
    network: Network,
    /// [`loop_trips`] at this engine's rate, computed once.
    loop_trips: [[f32; 4]; LINES],
    shaped: ShapedPath,
    tilt_l: Tilt,
    tilt_r: Tilt,
    pole_lo: f32,
    pole_hi: f32,
    tone_ref_lo: (f32, f32),
    tone_ref_hi: (f32, f32),
    /// The space's high cut in force — `None` when open — and its magnitude at the tone's reference.
    high_cut: Option<HighCutCoefficients>,
    high_cut_ref: f32,
    cut_l: HighCut,
    cut_r: HighCut,
    mix: Ramp,
    pre_delay: Smoother,
    size: Smoother,
    diffusion: Smoother,
    early_gain: Smoother,
    late_gain: Smoother,
    width: Smoother,
    tone_low: Smoother,
    tone_high: Smoother,
    duck: Smoother,
    mod_depth: Smoother,
    shape_length: Smoother,
    tick: u32,
    onset: f32,
    onset_step: f32,
    duck_env: f32,
    duck_attack: f32,
    duck_release: f32,
    wet_level: f32,
    input_level: f32,
    level_attack: f32,
    level_release: f32,
    quiet_samples: u32,
    samples_since_input: u32,
    parked: bool,
}

impl Engine {
    /// An engine at `sample_rate` with the default controls and [`Space::HALL`]. Everything is
    /// allocated here; nothing afterwards allocates.
    pub fn new(sample_rate: f32) -> Self {
        let fs = clamped_rate(sample_rate);
        let history_s = MAX_PRE_DELAY_S + (EARLY_MAX_UNITS * MAX_SIZE_S).max(SHAPE_MAX_S);
        let history = (history_s * fs).ceil() as usize + 8;
        let network_max = (RATIO_MAX * MAX_SIZE_S * fs + MAX_MOD_DEPTH_S * fs).ceil() as usize + 8;
        let (pole_lo, pole_hi) = (pole(LOW_CROSSOVER_HZ, fs), pole(HIGH_CROSSOVER_HZ, fs));
        let w_ref = reference_w(fs);
        let controls = Controls::default();
        let mut engine = Self {
            sample_rate: fs,
            controls,
            space: Space::HALL,
            pending_space: None,
            shape: controls.shape,
            pending_shape: None,
            fade: Fade::Idle,
            fade_gain: 1.0,
            fade_step: 1.0 / (SPACE_FADE_S * fs),
            hist_l: DelayLine::new(history),
            hist_r: DelayLine::new(history),
            diff_l: core::array::from_fn(|i| Allpass::new(DIFFUSER_S_L[i], fs)),
            diff_r: core::array::from_fn(|i| Allpass::new(DIFFUSER_S_R[i], fs)),
            network: Network::new(
                fs,
                network_max,
                pole_lo,
                BilinearCoefficients::new(DECAY_HIGH_CROSSOVER_HZ, fs),
                BilinearCoefficients::new(DECAY_TOP_CROSSOVER_HZ, fs),
            ),
            loop_trips: loop_trips(fs),
            shaped: ShapedPath::new(fs),
            tilt_l: Tilt::default(),
            tilt_r: Tilt::default(),
            pole_lo,
            pole_hi,
            tone_ref_lo: lowpass_response(pole_lo, w_ref),
            tone_ref_hi: lowpass_response(pole_hi, w_ref),
            high_cut: None,
            high_cut_ref: 1.0,
            cut_l: HighCut::default(),
            cut_r: HighCut::default(),
            mix: Ramp::new(0.0, MIX_RAMP_S, fs),
            pre_delay: Smoother::new(0.0, CONTROL_SMOOTH_S, fs),
            size: Smoother::new(controls.size_s, SIZE_GLIDE_S, fs),
            diffusion: Smoother::new(0.0, CONTROL_SMOOTH_S, fs),
            early_gain: Smoother::new(0.0, CONTROL_SMOOTH_S, fs),
            late_gain: Smoother::new(1.0, CONTROL_SMOOTH_S, fs),
            width: Smoother::new(1.0, CONTROL_SMOOTH_S, fs),
            tone_low: Smoother::new(1.0, CONTROL_SMOOTH_S, fs),
            tone_high: Smoother::new(1.0, CONTROL_SMOOTH_S, fs),
            duck: Smoother::new(0.0, CONTROL_SMOOTH_S, fs),
            mod_depth: Smoother::new(0.0, CONTROL_SMOOTH_S, fs),
            shape_length: Smoother::new(controls.decay_s * fs, CONTROL_SMOOTH_S, fs),
            tick: 0,
            onset: 1.0,
            onset_step: 1.0 / (PATH_ONSET_S * fs),
            duck_env: 0.0,
            duck_attack: follower_coefficient(DUCK_ATTACK_S, fs),
            duck_release: follower_coefficient(DUCK_RELEASE_S, fs),
            wet_level: 0.0,
            input_level: 0.0,
            level_attack: follower_coefficient(LEVEL_ATTACK_S, fs),
            level_release: follower_coefficient(LEVEL_RELEASE_S, fs),
            quiet_samples: 0,
            samples_since_input: u32::MAX,
            parked: true,
        };
        engine.shaped.set_shape(engine.shape);
        engine.apply_targets();
        engine.mix.snap();
        engine.snap_smoothers();
        engine.update_network_gains();
        engine
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn controls(&self) -> Controls {
        self.controls
    }

    /// The space in force, or the one a fade is heading for.
    pub fn space(&self) -> Space {
        self.pending_space.unwrap_or(self.space)
    }

    /// Whether the engine is parked: empty, and running none of its core.
    pub fn is_parked(&self) -> bool {
        self.parked
    }

    /// The composed decay times of the four bands, after the composition law and its clamps.
    pub fn composed_decay_s(&self) -> BandDecays {
        composed_decay_s(&self.space(), &self.controls)
    }

    /// Selects how the network reads a fractional delay. The measurement seam for plan §3.1's
    /// modulation decision; the default is [`Interpolation::Allpass`].
    pub fn set_interpolation(&mut self, interpolation: Interpolation) {
        self.network.set_interpolation(interpolation);
    }

    pub fn set_controls(&mut self, controls: Controls) {
        let controls = controls.sanitised();
        let heading_for = self.pending_shape.unwrap_or(self.shape);
        self.controls = controls;
        if controls.shape != heading_for {
            if self.parked {
                self.shape = controls.shape;
                self.pending_shape = None;
                self.shaped.set_shape(self.shape);
            } else {
                self.pending_shape = Some(controls.shape);
                self.fade = Fade::Out;
            }
        }
        self.apply_targets();
        if self.parked {
            self.snap_smoothers();
            if self.samples_since_input == u32::MAX {
                // Nothing has been heard since the engine emptied, so no dry signal is passing that
                // a jump in the mix could click: it settles at once, as in a fresh engine.
                self.mix.snap();
            }
            self.update_network_gains();
        }
    }

    /// Changes the space. While sounding, the wet fades through silence, the space is swapped and
    /// the wet fades back in; a space asked for during a fade becomes the destination (plan §3.4).
    pub fn set_space(&mut self, space: Space) {
        let space = space.sanitised();
        if self.parked {
            self.space = space;
            self.pending_space = None;
            self.apply_targets();
            self.snap_smoothers();
            self.update_network_gains();
            return;
        }
        if self.pending_space.is_none() && space == self.space {
            return;
        }
        self.pending_space = Some(space);
        self.fade = Fade::Out;
    }

    /// Clears every state that can sound; the next excitation starts from empty.
    pub fn reset(&mut self) {
        self.empty();
        self.parked = true;
        self.mix.snap();
    }

    /// How long until the output is quiet with no new input. It may overestimate; it must not
    /// underestimate.
    pub fn remaining_tail_seconds(&self) -> f32 {
        if self.parked {
            return 0.0;
        }
        let fs = self.sample_rate;
        let since = self.samples_since_input as f32;
        let span = self.history_span_samples();
        let history = (span - since).max(0.0) / fs;
        // While the history still holds input, the wet may yet reach full scale.
        let level_db = if since < span {
            0.0
        } else {
            (20.0 * self.wet_level.max(1.0e-12).log10()).max(-40.0)
        };
        let above = level_db - QUIET_DB;
        // The level follower takes its own time to fall to quiet after the signal has.
        let follower = LEVEL_RELEASE_S * above / 20.0 * LN_10;
        let body = match self.shape {
            DecayShape::Natural => self.composed_decay_s().longest() * TAIL_MARGIN * above / 60.0,
            _ => SHAPED_TAIL_S,
        };
        history + body + follower + self.hold_samples() as f32 / fs + 2.0 * SPACE_FADE_S
    }

    /// One stereo sample. A mono host passes the same sample twice.
    #[inline]
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let in_l = if in_l.is_finite() { in_l } else { 0.0 };
        let in_r = if in_r.is_finite() { in_r } else { 0.0 };
        let peak = in_l.abs().max(in_r.abs());
        let mix = self.mix.next();

        if mix == 0.0 && self.mix.target() == 0.0 {
            if !self.parked {
                self.empty();
                self.parked = true;
            }
            if peak > QUIET_LEVEL {
                self.samples_since_input = 0;
            }
            return (in_l, in_r);
        }
        let dry = (mix * FRAC_PI_2).cos();
        if self.parked {
            if peak <= QUIET_LEVEL {
                return (in_l * dry, in_r * dry);
            }
            self.parked = false;
            self.tick = 0;
        }

        let fs = self.sample_rate;
        self.hist_l.push(in_l);
        self.hist_r.push(in_r);
        if self.tick == 0 {
            self.update_network_gains();
        }
        self.tick += 1;
        if self.tick >= CONTROL_TICK {
            self.tick = 0;
        }

        let depth_target = self.mod_depth.next();
        let size_now = self.size.next() * fs;
        let size_samples = effective_size_samples(size_now, depth_target);
        let depth = depth_target.min(0.25 * RATIO_MIN * size_samples);
        let base_age = 1.0 + self.pre_delay.next();
        let coefficient = self.diffusion.next();
        let length = self.shape_length.next();
        let onset = self.onset;
        if self.onset < 1.0 {
            self.onset = (self.onset + self.onset_step).min(1.0);
        }

        let (mut early_l, mut early_r) = (0.0, 0.0);
        for tap in &self.space.early {
            if tap.gain_l == 0.0 && tap.gain_r == 0.0 {
                continue;
            }
            let age = base_age + tap.time * size_samples;
            let m = 0.5 * (self.hist_l.read_linear(age) + self.hist_r.read_linear(age));
            early_l += tap.gain_l * m;
            early_r += tap.gain_r * m;
        }

        let (late_l, late_r) = match self.shape {
            DecayShape::Natural => {
                let mut dl = self.hist_l.read_linear(base_age) * onset;
                let mut dr = self.hist_r.read_linear(base_age) * onset;
                for ap in &mut self.diff_l {
                    dl = ap.process(dl, coefficient);
                }
                for ap in &mut self.diff_r {
                    dr = ap.process(dr, coefficient);
                }
                self.network
                    .process(dl, dr, size_samples, depth, self.controls.mod_rate_hz)
            }
            _ => self
                .shaped
                .process(&self.hist_l, &self.hist_r, base_age, length, onset),
        };

        let (eg, lg) = (self.early_gain.next(), self.late_gain.next());
        let mut wl = eg * early_l + lg * late_l;
        let mut wr = eg * early_r + lg * late_r;

        // The tone: both shelves, then the space's high cut, the whole path divided by its magnitude
        // at the reference (plan §2). An open cut is bypassed and its reference magnitude is exactly
        // one, so a space without a cut renders to the bit as it did before the cut existed.
        let (tl, th) = (self.tone_low.next(), self.tone_high.next());
        let tone_norm = 1.0
            / (tilt_magnitude(tl, th, self.tone_ref_lo, self.tone_ref_hi) * self.high_cut_ref)
                .max(1.0e-6);
        wl = self.tilt_l.process(wl, tl, th, self.pole_lo, self.pole_hi);
        wr = self.tilt_r.process(wr, tl, th, self.pole_lo, self.pole_hi);
        if let Some(cut) = &self.high_cut {
            wl = self.cut_l.process(wl, cut);
            wr = self.cut_r.process(wr, cut);
        }
        wl *= tone_norm;
        wr *= tone_norm;

        let side_scale = self.width.next();
        let mid = 0.5 * (wl + wr);
        let side = 0.5 * (wl - wr) * side_scale;
        wl = mid + side;
        wr = mid - side;

        let coef = if peak > self.duck_env {
            self.duck_attack
        } else {
            self.duck_release
        };
        self.duck_env = flush(self.duck_env + coef * (peak - self.duck_env));
        let duck = self.duck.next();
        let gain = (1.0 - duck * (self.duck_env / DUCK_REFERENCE).min(1.0)) * self.advance_fade();
        wl *= gain;
        wr *= gain;

        self.follow_levels(peak, wl.abs().max(wr.abs()));
        let wet = (mix * FRAC_PI_2).sin();
        let out = (in_l * dry + wl * wet, in_r * dry + wr * wet);

        if self.input_level < QUIET_LEVEL
            && self.wet_level < QUIET_LEVEL
            && self.fade == Fade::Idle
            && self.samples_since_input as f32 > self.history_span_samples()
        {
            self.quiet_samples = self.quiet_samples.saturating_add(1);
            if self.quiet_samples > self.hold_samples() {
                self.empty();
                self.parked = true;
            }
        } else {
            self.quiet_samples = 0;
        }
        out
    }

    fn follow_levels(&mut self, input: f32, wet: f32) {
        let coef = if input > self.input_level {
            self.level_attack
        } else {
            self.level_release
        };
        self.input_level = flush(self.input_level + coef * (input - self.input_level));
        let coef = if wet > self.wet_level {
            self.level_attack
        } else {
            self.level_release
        };
        self.wet_level = flush(self.wet_level + coef * (wet - self.wet_level));
        if input > QUIET_LEVEL {
            self.samples_since_input = 0;
        } else {
            self.samples_since_input = self.samples_since_input.saturating_add(1);
        }
    }

    /// How far back in the input history the engine is still reading.
    fn history_span_samples(&self) -> f32 {
        let fs = self.sample_rate;
        let pre = self.pre_delay.target();
        let early = EARLY_MAX_UNITS * self.size.target() * fs;
        let shaped = if self.shape == DecayShape::Natural {
            0.0
        } else {
            self.shape_length.target()
        };
        1.0 + pre + early.max(shaped) + 0.05 * fs
    }

    /// Quiet has to outlast a full trip round the longest line, or a sparse network parks between
    /// its own returns.
    fn hold_samples(&self) -> u32 {
        let longest = self
            .network
            .longest_trip_samples(self.size.target() * self.sample_rate)
            + self.mod_depth.target();
        (QUIET_HOLD_S * self.sample_rate + longest) as u32
    }

    fn advance_fade(&mut self) -> f32 {
        match self.fade {
            Fade::Idle => 1.0,
            Fade::Out => {
                self.fade_gain -= self.fade_step;
                if self.fade_gain <= 0.0 {
                    self.fade_gain = 0.0;
                    self.swap_at_silence();
                    self.fade = Fade::In;
                }
                self.fade_gain
            }
            Fade::In => {
                self.fade_gain += self.fade_step;
                if self.fade_gain >= 1.0 {
                    self.fade_gain = 1.0;
                    self.fade = Fade::Idle;
                }
                self.fade_gain
            }
        }
    }

    /// Applies what a fade was heading for.
    ///
    /// **A space change leaves the paths running.** Line lengths belong to Size, not to the space, so
    /// nothing in the network was computed for anything the swap replaces; emptying it would cut off
    /// the signal feeding it and start its returns with a step. What the fade hides is the early
    /// pattern, tone and width jumping. **A shape change starts a path from empty** and ramps that
    /// path's input in, so the signal it is fed does not begin with a step.
    fn swap_at_silence(&mut self) {
        if let Some(space) = self.pending_space.take() {
            self.space = space;
        }
        if let Some(shape) = self.pending_shape.take() {
            if shape != self.shape {
                self.shape = shape;
                self.shaped.set_shape(shape);
                self.invalidate_paths();
                self.onset = 0.0;
            }
        }
        self.apply_targets();
        self.early_gain.snap();
        self.late_gain.snap();
        self.width.snap();
        self.tone_low.snap();
        self.tone_high.snap();
        self.update_network_gains();
    }

    /// Forgets the wet paths' state: the network, both diffuser chains and the shaped path.
    fn invalidate_paths(&mut self) {
        self.network.invalidate();
        for ap in self.diff_l.iter_mut().chain(self.diff_r.iter_mut()) {
            ap.invalidate();
        }
        self.shaped.invalidate();
    }

    /// Forgets everything that can sound, in constant time, and settles on the current targets.
    fn empty(&mut self) {
        self.hist_l.invalidate();
        self.hist_r.invalidate();
        self.fade = Fade::Idle;
        self.fade_gain = 1.0;
        self.swap_at_silence();
        self.invalidate_paths();
        self.onset = 1.0;
        self.tilt_l.reset();
        self.tilt_r.reset();
        self.cut_l.reset();
        self.cut_r.reset();
        self.duck_env = 0.0;
        self.wet_level = 0.0;
        self.input_level = 0.0;
        self.quiet_samples = 0;
        self.samples_since_input = u32::MAX;
        self.tick = 0;
        self.snap_smoothers();
        self.update_network_gains();
    }

    /// Sets every smoother's target from the controls and the space in force, by plan §2's laws.
    fn apply_targets(&mut self) {
        let c = self.controls;
        let s = self.space;
        let fs = self.sample_rate;
        self.mix.set(c.mix);
        self.pre_delay.set(c.pre_delay_s * fs);
        self.size.set(c.size_s);
        self.diffusion.set(c.diffusion * DIFFUSION_MAX_GAIN);
        let early = if c.early_late_db <= -EARLY_LATE_SILENCE_DB {
            0.0
        } else {
            s.early_level * db_to_gain(0.5 * c.early_late_db)
        };
        let late = if c.early_late_db >= EARLY_LATE_SILENCE_DB {
            0.0
        } else {
            db_to_gain(-0.5 * c.early_late_db)
        };
        self.early_gain.set(early);
        self.late_gain.set(late);
        self.width.set(s.width * c.width);
        self.tone_low.set(db_to_gain(s.tone_low_db + c.tone_low_db));
        self.tone_high
            .set(db_to_gain(s.tone_high_db + c.tone_high_db));
        // The high cut is the space's alone, so it changes only where the space does: at a fade's
        // silent point or while parked, never under a sounding wet signal (plan §3.4). A cut closing
        // from open starts from rest, since its state was not running.
        let cut = HighCutCoefficients::new(s.high_cut_hz, fs);
        if cut != self.high_cut {
            if self.high_cut.is_none() {
                self.cut_l.reset();
                self.cut_r.reset();
            }
            self.high_cut = cut;
            self.high_cut_ref = cut.map_or(1.0, |c| c.magnitude(reference_w(fs)));
        }
        self.duck.set(c.duck);
        self.mod_depth.set(c.mod_depth_s * fs);
        self.shape_length
            .set(c.decay_s.clamp(SHAPE_MIN_S, SHAPE_MAX_S) * fs);
    }

    fn snap_smoothers(&mut self) {
        for s in [
            &mut self.pre_delay,
            &mut self.size,
            &mut self.diffusion,
            &mut self.early_gain,
            &mut self.late_gain,
            &mut self.width,
            &mut self.tone_low,
            &mut self.tone_high,
            &mut self.duck,
            &mut self.mod_depth,
            &mut self.shape_length,
        ] {
            s.snap();
        }
    }

    /// The per-line decay filters' gains for the composed decay and the current line lengths,
    /// limited so no filter can exceed [`LOOP_GAIN_CEILING`] at any frequency.
    fn update_network_gains(&mut self) {
        let gains = line_gains(
            &self.space(),
            &self.controls,
            self.sample_rate,
            &self.loop_trips,
        );
        self.network.set_gains(gains);
    }

    /// The largest gain any line's decay filter can reach — the quantity plan §3.2 bounds.
    pub fn loop_gain_bound(&self) -> f32 {
        self.network
            .gains()
            .iter()
            .map(BandGains::bound)
            .fold(0.0, f32::max)
    }

    /// The decay time the network realises at `hz` with this engine's space and controls — the free
    /// [`predicted_decay_s`] applied to them.
    pub fn predicted_decay_s(&self, hz: f32) -> f32 {
        predicted_decay_s(&self.space(), &self.controls, self.sample_rate, hz)
    }
}
