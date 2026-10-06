//! What a response becomes: the analysis and every reading in it.
//!
//! **Every numeric field is finite or absent.** A quantity that could not be measured is `None`,
//! never NaN, never an infinity and never a zero standing in for "could not tell". Times are in
//! seconds and levels in decibels, never samples, so a reading means the same at every rate.

/// Which Schroeder integration a band's decay is read with — Guski and Vorländer's labels, as
/// `research:effects/feedback-delay-network-reverb.md` §9.3 tabulates them.
///
/// [`crate::analyse`] always uses [`DecayMethod::NoiseCompensated`]. The other two exist so the
/// difference noise compensation makes can be measured on the same response
/// ([`crate::analyse_with_method`]); neither may feed a fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecayMethod {
    /// **Method E**: the noise floor's mean square subtracted, integrated to Lundeby's intersection
    /// time, plus a correction for the decay beyond it. The one Guski and Vorländer found with no
    /// systematic error.
    NoiseCompensated,
    /// **Method C**: integrated to the intersection time plus the same correction, with the noise
    /// left in. Biased short by the truncation, which the correction only partly repairs.
    Truncated,
    /// **Method A**, the ISO default: integrated to the end of the response with no compensation.
    /// Biased long, and more so the longer the noise after the decay.
    Uncompensated,
}

/// The analysis of one impulse response.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub sample_rate: f32,
    pub channels: u16,
    pub frames: usize,
    /// How every band's decay was integrated.
    pub method: DecayMethod,
    pub onset: Onset,
    /// From the direct-sound peak to the first reflection energy (see [`crate::PRE_DELAY_WINDOW_S`]).
    /// Absent when nothing after the direct-sound window rises above both the early level floor and
    /// the noise margin.
    pub pre_delay_s: Option<f32>,
    pub early: EarlyReflections,
    /// Lundeby's method on the broadband energy, which bounds the echo density, the reflections and
    /// the width window.
    pub broadband: BroadbandDecay,
    /// One entry per [`crate::OCTAVE_BANDS_HZ`] centre, in that order, including absent bands.
    pub bands: Vec<Band>,
    pub echo_density: EchoDensity,
    pub width: Width,
    /// The tail's texture over segment A: from [`crate::TEXTURE_A_START_T60`] of the mid-band T60 after
    /// the direct sound, for [`crate::TEXTURE_A_LENGTH_T60`] of it, each within its bounds and cut
    /// where the tail reaches its noise floor. Absent where no required band from 500 Hz to 2 kHz has
    /// a decay time, or the tail reaches its floor within [`crate::TEXTURE_MIN_SEGMENT_S`] of the
    /// segment's start. **Read on stated segments instead** where the analysis was given them
    /// ([`crate::analyse_with_segments`]), as a fit's verification is given the response's.
    pub texture_a: Option<TailTexture>,
    /// … over segment B, from A's end for up to [`crate::TEXTURE_B_MAX_S`] to the same cut. Absent
    /// where that leaves less than [`crate::TEXTURE_B_MIN_S`], or A has no T60 to be placed by.
    pub texture_b: Option<TailTexture>,
    pub validity: Validity,
}

impl Analysis {
    /// The band with this centre frequency, if it is one of [`crate::OCTAVE_BANDS_HZ`].
    pub fn band(&self, centre_hz: f32) -> Option<&Band> {
        self.bands.iter().find(|band| band.centre_hz == centre_hz)
    }

    /// The segments this analysis read its tail texture on, so another response can be read on the
    /// same ones ([`crate::analyse_with_segments`]).
    pub fn texture_segments(&self) -> TextureSegments {
        TextureSegments {
            a: self.texture_a.map(|t| t.segment),
            b: self.texture_b.map(|t| t.segment),
        }
    }
}

/// A stretch of the tail, in seconds after the direct sound, from `start_s` up to but not including
/// `end_s`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start_s: f32,
    pub end_s: f32,
}

/// Where a tail texture is read. [`crate::analyse`] places its own from the response's decay;
/// [`crate::analyse_with_segments`] is given them, and reads no texture where a segment is absent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureSegments {
    pub a: Option<Segment>,
    pub b: Option<Segment>,
}

/// How diffuse the tail is over one segment (this crate's `AGENTS.md`, *The tail texture*): four
/// readings the echo density profile cannot make, each beside what exponentially decaying Gaussian
/// noise reads through the same pipeline. Peakiness, kurtosis and periodicity read the segment
/// **decay-compensated** — each sample over the square root of its
/// [`crate::TEXTURE_COMPENSATION_WINDOW_S`] centred mean square. Each reading is per channel, then the
/// mean over the channels that have it, except periodicity, which is the larger channel's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TailTexture {
    /// Where it was read: shortened to the largest power of two of frames it holds when it holds at
    /// least [`crate::TEXTURE_POWER_OF_TWO_FROM`], and cut at the end of the samples.
    pub segment: Segment,
    /// **Spectral peakiness**: the median over sixth octaves, from [`crate::PEAKINESS_LOW_HZ`] to
    /// [`crate::PEAKINESS_HIGH_HZ`] or [`crate::PEAKINESS_HIGH_RATE_FRACTION`] of the rate, of the
    /// Hann-windowed periodogram's mean(P²) / mean(P)², each octave holding at least
    /// [`crate::PEAKINESS_MIN_BINS`] bins. Noise reads about 1.9; sparse, high-Q modes — metallic
    /// ringing — read far above. Absent where no sixth octave holds that many bins with power.
    pub peakiness: Option<f32>,
    /// **Kurtosis**, the mean removed. Noise reads 3; separate echoes read above. Absent where no
    /// channel varies.
    pub kurtosis: Option<f32>,
    /// **Echo density**: Abel and Huang's normalised count over [`crate::ECHO_DENSITY_WINDOW_S`]
    /// windows centred every [`crate::TEXTURE_DENSITY_HOP_S`] in the segment, read on the samples as
    /// they are, averaged over windows and channels. Noise reads 1; sparse echoes read below. Absent
    /// where no window varies.
    pub echo_density: Option<f32>,
    /// **Periodicity**: the largest normalised autocorrelation of the log energy envelope (a
    /// [`crate::PERIODICITY_ENVELOPE_S`] mean square, less its [`crate::PERIODICITY_TREND_S`] mean)
    /// past its main lobe, from [`crate::PERIODICITY_MIN_LAG_S`] to [`crate::PERIODICITY_MAX_LAG_S`] or
    /// half the segment. Repeating echoes and flutter read high at their period; noise reads low.
    /// **Zero** where the main lobe outlasts the search, which is a smooth envelope, not a repeat.
    /// Absent where no channel's envelope varies.
    pub periodicity: Option<f32>,
    /// Its lag. Absent with the periodicity, and where the search held no lag.
    pub periodicity_lag_s: Option<f32>,
}

/// Where the response starts, and how its direct sound stands against what surrounds it
/// (`research:effects/feedback-delay-network-reverb.md` §9.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Onset {
    /// The first sample whose energy is within [`crate::ONSET_BELOW_PEAK_DB`] of the direct-sound
    /// peak, in seconds from the first sample of the file.
    pub onset_s: f32,
    /// The direct-sound peak — the largest energy summed over channels — in seconds from the first
    /// sample of the file. Every other time in the analysis is measured from here.
    pub direct_s: f32,
    /// The direct-sound peak's energy summed over channels, in dB re a mean square of one.
    pub direct_peak_db: f32,
    /// The direct sound's mean square over the ±[`crate::DIRECT_HALF_WINDOW_S`] window over the mean
    /// square of everything before the onset, less [`crate::ONSET_GUARD_S`]. Capped at 300 dB, which
    /// is what digital silence before the onset reads. Absent when less than
    /// [`crate::ONSET_GUARD_S`] precedes that guard.
    pub prominence_db: Option<f32>,
    /// The ACE challenge's direct-to-reverberant ratio: energy within ±[`crate::DIRECT_HALF_WINDOW_S`]
    /// of the peak over everything else, with energy before the window counted as reverberant.
    /// Absent when there is no energy outside the window.
    pub drr_db: Option<f32>,
}

/// The strongest discrete peaks in the early window, in time order.
#[derive(Clone, Debug, PartialEq)]
pub struct EarlyReflections {
    /// Where the window opens, after the direct sound: just past the direct-sound window.
    pub window_start_s: f32,
    /// Where the window closes, after the direct sound.
    pub window_end_s: f32,
    /// Which rule closed the window.
    pub window_end: EarlyWindowEnd,
    pub reflections: Vec<Reflection>,
}

/// The rule that closed the early window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyWindowEnd {
    /// The mixing time: past it the field is Gaussian and no peak is discrete.
    MixingTime,
    /// [`crate::EARLY_WINDOW_MAX_S`] after the pre-delay, reached before any mixing time.
    Cap,
}

/// One early reflection.
#[derive(Clone, Debug, PartialEq)]
pub struct Reflection {
    /// After the direct-sound peak.
    pub delay_s: f32,
    /// Energy over ±[`crate::REFLECTION_HALF_WIDTH_S`] summed over channels, against the direct
    /// sound's over the same width.
    pub level_db: f32,
    /// The same, per channel, against that channel's direct sound. Absent for a channel whose direct
    /// sound holds no energy.
    pub channel_level_db: Vec<Option<f32>>,
}

/// Lundeby's method run on the broadband energy with [`crate::LUNDEBY_BROADBAND_BLOCK_S`] blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct BroadbandDecay {
    /// The noise floor's mean square summed over channels, dB re a mean square of one. Where the
    /// method aborted, the first estimate (the mean of the last tenth of the blocks).
    pub noise_floor_db: f32,
    /// Where the late decay line meets the noise floor, after the direct sound. Absent where the
    /// method aborted or the line meets the floor outside the response.
    pub intersection_s: Option<f32>,
    /// Why the method aborted, if it did.
    pub failure: Option<DecayFailure>,
}

/// Why Lundeby's method found no decay in a signal (`research:effects/feedback-delay-network-reverb.md`
/// §9.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecayFailure {
    /// Too few blocks after the direct sound to estimate a noise floor from the last tenth of them.
    TooShort,
    /// The decay does not fall far enough above the noise floor to fit a line (step 3), or leaves
    /// fewer than two blocks in a fitting range.
    TooLittleDecay,
    /// A fitted line does not fall (steps 3 and 8).
    SlopeNotNegative,
}

/// One octave band.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub centre_hz: f32,
    /// Whether this band must show a decay: its loudest block is within
    /// [`crate::BAND_REQUIRED_WITHIN_DB`] of the loudest band's. A required band without one refuses
    /// the response.
    pub required: bool,
    pub reading: BandReading,
}

impl Band {
    /// The band's decay, where it was measured.
    pub fn decay(&self) -> Option<&BandDecay> {
        match &self.reading {
            BandReading::Measured(decay) => Some(decay),
            _ => None,
        }
    }
}

/// What a band holds.
#[derive(Clone, Debug, PartialEq)]
pub enum BandReading {
    Measured(BandDecay),
    /// The band's upper edge is above [`crate::BAND_UPPER_EDGE_MAX_NYQUIST_FRACTION`] of Nyquist at
    /// this rate, so the band is not analysed at all.
    AboveNyquist,
    /// No decay was found in it. [`crate::analyse`] reads this only for a band that is not required,
    /// and refuses the response otherwise; a fit's verification render reads it for a required band
    /// too, and says so in `FitReport::verification_refusal`.
    Empty {
        /// How far the band's loudest block sits below the loudest band's.
        below_loudest_db: f32,
        failure: DecayFailure,
    },
}

/// A band's decay: Schroeder integration with Lundeby's noise floor, read by the method in
/// [`Analysis::method`].
#[derive(Clone, Debug, PartialEq)]
pub struct BandDecay {
    /// Fitted from −5 to −25 dB of the energy decay curve and extrapolated to 60 dB. Absent where the
    /// curve does not reach −25 dB before it ends.
    pub t20_s: Option<f32>,
    /// Fitted from −5 to −35 dB.
    pub t30_s: Option<f32>,
    /// Early decay time, fitted from 0 to −10 dB.
    pub edt_s: Option<f32>,
    /// The band's noise floor: the mean square of the band energy summed over channels, dB re a mean
    /// square of one.
    pub noise_floor_db: f32,
    /// Where Lundeby's late decay line meets the noise floor, after the direct sound, clamped to the
    /// response's end.
    pub intersection_s: f32,
    /// Whether that intersection lies inside the response. Where it does not, the decay was still
    /// falling when the response ended and was integrated to the end.
    pub noise_floor_reached: bool,
    pub lundeby_iterations: u32,
    /// Whether the intersection time settled within 0.01 s inside the thirty-iteration cap.
    pub lundeby_converged: bool,
    /// The slope of Lundeby's final line, in dB per second.
    pub late_slope_db_per_s: f32,
    /// The late decay's energy per sample at the direct sound, in dB re a mean square of one: the
    /// T30 fit (else T20) of the decay curve, extrapolated back and converted from integrated to
    /// instantaneous energy. Absent where neither fit exists.
    pub initial_level_db: Option<f32>,
    /// [`Self::initial_level_db`] against the 1 kHz band's, which is the tone curve. Absent where
    /// either level is.
    pub level_re_1k_db: Option<f32>,
    /// Lundeby's line at the direct sound over the noise floor: the decay's margin above the noise.
    pub margin_db: f32,
    /// How straight the decay runs. Absent where neither the T30 nor the T20 range exists.
    pub straightness: Option<Straightness>,
}

/// How far a band's energy decay curve departs from one slope.
///
/// A decay that bends is reported here rather than averaged into one decay time: a single-slope
/// estimate of a decay that is not one slope is the failure the published analysis–synthesis work
/// names (`research:effects/feedback-delay-network-reverb.md` §11, §13 step 8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Straightness {
    /// The bottom of the range read: −35 dB where the T30 range exists, else −25 dB.
    pub range_bottom_db: f32,
    /// The largest distance, in dB, of the decay curve from its straight-line fit over −5 dB to
    /// [`Self::range_bottom_db`]. The largest rather than the root mean square, and read on the
    /// curve itself rather than on block means: both alternatives were measured and separate a
    /// two-slope decay from a single slope worse, because a bend shows most at the ends of the range.
    pub max_deviation_db: f32,
    /// The decay time fitted over the lower half of that range over the one fitted over the upper
    /// half. One for a single slope; above one where the decay slows as it falls.
    pub late_to_early: Option<f32>,
    /// `max_deviation_db` is at most [`crate::ONE_SLOPE_MAX_DEVIATION_DB`].
    pub single_slope: bool,
}

/// The echo density profile (Abel and Huang 2006, `research:effects/feedback-delay-network-reverb.md`
/// §9.6).
#[derive(Clone, Debug, PartialEq)]
pub struct EchoDensity {
    /// The window's actual length. The window is **rectangular**: every sample weighs the same, as
    /// the profile's formula is written.
    pub window_s: f32,
    /// A rectangular window's effective length is its length.
    pub effective_window_s: f32,
    pub hop_s: f32,
    /// From the direct sound for [`crate::ECHO_DENSITY_SPAN_S`] or to the end of the response.
    pub points: Vec<DensityPoint>,
    /// The first point where the profile reaches one — Abel and Huang's first reading, which Lindau,
    /// Kosanke and Weinzierl found predicts the perceived mixing time better than the paper's own
    /// criterion. After the direct sound; absent if the profile never reaches one.
    pub mixing_time_s: Option<f32>,
}

/// One point of the echo density profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityPoint {
    /// The window's centre, after the direct sound.
    pub time_s: f32,
    /// Normalised so a Gaussian field reads one. Absent where the window's energy is not
    /// [`crate::NOISE_MARGIN_DB`] above the broadband noise floor: noise alone is Gaussian and would
    /// read as a mixed field.
    pub density: Option<f32>,
}

/// The late field's interchannel coherence (`research:effects/feedback-delay-network-reverb.md` §9.7).
#[derive(Clone, Debug, PartialEq)]
pub enum Width {
    Measured(Coherence),
    NotMeasured(WidthNotMeasured),
}

impl Width {
    /// The coherence, where it was measured.
    pub fn measured(&self) -> Option<&Coherence> {
        match self {
            Width::Measured(coherence) => Some(coherence),
            Width::NotMeasured(_) => None,
        }
    }
}

/// Why the width was not measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthNotMeasured {
    /// A mono response has no second channel to compare.
    Mono,
    /// The late window is shorter than [`crate::WIDTH_LATE_MIN_S`].
    NoLateField,
    /// A channel holds no energy in the late window.
    SilentChannel,
}

/// The interchannel cross-correlation over the late window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coherence {
    /// The largest magnitude of the normalised cross-correlation over lags within
    /// ±[`crate::WIDTH_MAX_LAG_S`]. One for identical channels, near zero for independent ones.
    pub iacc: f32,
    /// That largest value with its sign kept, so an inverted channel reads −1 rather than 1.
    pub peak_iacf: f32,
    /// The lag of that value: positive where the right channel lags the left.
    pub peak_lag_s: f32,
    /// The late window, after the direct sound.
    pub window_start_s: f32,
    pub window_end_s: f32,
}

/// The validity checks of `plans/plan-mxm-classic-verb.md` §4.5 and the confidence they give.
#[derive(Clone, Debug, PartialEq)]
pub struct Validity {
    pub checks: Vec<Check>,
    /// The lowest score of any scored check, or one where no check was scored.
    pub confidence: f32,
    /// The check that set the confidence.
    pub weakest: Option<CheckKind>,
    /// [`crate::CONFIDENCE_FLOOR`]: a confidence below it refuses the response.
    pub floor: f32,
}

/// One validity check.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Check {
    pub kind: CheckKind,
    /// The measured value the score comes from, in dB.
    pub value_db: Option<f32>,
    /// From zero to one. Absent where the value could not be measured, and then the check does not
    /// count towards the confidence.
    pub score: Option<f32>,
}

/// What a check measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckKind {
    /// [`Onset::prominence_db`], scored from [`crate::ONSET_BELOW_PEAK_DB`] to
    /// [`crate::PROMINENCE_FULL_DB`].
    OnsetProminence,
    /// [`BandDecay::margin_db`] of a required band, scored from [`crate::MARGIN_ZERO_DB`] to
    /// [`crate::MARGIN_FULL_DB`].
    NoiseMargin { band_hz: f32 },
    /// [`Straightness::max_deviation_db`] of a required band, scored from
    /// [`crate::STRAIGHTNESS_FULL_DB`] down to [`crate::STRAIGHTNESS_ZERO_DB`].
    Straightness { band_hz: f32 },
}
