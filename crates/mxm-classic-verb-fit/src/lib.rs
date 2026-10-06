//! Impulse-response analysis and fitting for `mxm-classic-verb`: one or two channels of samples
//! become the descriptors a space is fitted from, then a space and the absolute controls that
//! reproduce them — each verified by rendering the result — or a refusal that names what failed.
//!
//! Plan: `plans/plan-mxm-classic-verb.md` §4.2 (analyse), §4.3 (fit), §4.4 (verify), §4.5 (the input
//! domain and how a mistaken file is judged) and §4.6 (what proves it). Method:
//! `research:effects/feedback-delay-network-reverb.md` §9 and §13 steps 7–8. [`analyse`] is P2's;
//! [`fit`] is P3's and renders candidates through `mxm-classic-verb-dsp`, this crate's one runtime
//! dependency.
//!
//! # Contracts
//!
//! - **No file I/O.** Samples in, analysis out; decoding is the caller's. [`preflight`] is the part of
//!   the input domain a caller can decide from a file's header before decoding it.
//! - **Never NaN.** Every numeric field of an [`Analysis`] is finite or `None`. A quantity that
//!   cannot be measured is absent, never NaN, an infinity, or a zero that means "could not tell".
//! - **Deterministic.** The same samples on the same build give a bit-identical analysis. Byte
//!   identity across platforms is not claimed (plan §4.3).
//! - **Not realtime.** It allocates in proportion to the response and runs for as long as that
//!   takes; [`MAX_DURATION_S`] and [`MAX_SAMPLE_RATE_HZ`] bound both.
//! - **Every buffer the response's length sizes is reserved fallibly** (`buffer`): memory that cannot
//!   be had is [`Refusal::OutOfMemory`] or [`FitRefusal::OutOfMemory`], never an abort of the process
//!   the analysis runs in.
//!
//! # What each reading is, and where it comes from
//!
//! | Reading | Method | Module |
//! |---|---|---|
//! | Onset, direct sound, DRR | ISO 3382-1's 20 dB onset rule; the ACE challenge's ±2.5 ms DRR | `early` |
//! | Pre-delay, early reflections | Direct sound to first reflection energy; strongest early peaks | `early` |
//! | Decay per octave band | Butterworth bands filtered time-reversed; Schroeder integration; Lundeby's noise floor with Guski and Vorländer's method E | `band`, `decay` |
//! | Echo density, mixing time | Abel and Huang's profile, read where it first reaches one (Lindau et al.) | `density` |
//! | Width | Late-field IACC | `width` |
//! | Tail texture | Spectral peakiness, kurtosis, echo density and envelope periodicity over two segments of the tail | `texture` |
//! | Confidence | The plan's §4.5 checks, weakest wins | `validity` |
//! | Decay, band ratios, tone | Closed form through the engine's own decay filter — each band read across its octave, as the analyser reads it — and its shelves and high cut | `solve` |
//! | Early pattern, balance, width | The reflections; an energy ratio and a coherence matched by render | `solve` |
//! | Size | The density floor: the smallest network the longest band decay allows | `fit` |
//! | Diffusion, first arrival | Searched against the echo density profile and the early envelope | `search` |
//! | Verification | The fitted reverb rendered and analysed by the same analyser | `fit` |
//!
//! Every constant below says whether it is the research page's, derived, measured or chosen, and
//! the crate's `AGENTS.md` carries the measurements behind each.

mod analysis;
mod band;
mod buffer;
mod decay;
mod density;
mod early;
mod fit;
mod math;
mod refusal;
mod render;
mod search;
mod solve;
mod texture;
mod validity;
mod width;

pub use analysis::{
    Analysis, Band, BandDecay, BandReading, BroadbandDecay, Check, CheckKind, Coherence,
    DecayFailure, DecayMethod, DensityPoint, EarlyReflections, EarlyWindowEnd, EchoDensity, Onset,
    Reflection, Segment, Straightness, TailTexture, TextureSegments, Validity, Width,
    WidthNotMeasured,
};
pub use fit::{
    BALANCE_WINDOW_S, BandError, Clamp, ControlRanges, DENSITY_FLOOR_DECAY_SHARE,
    DENSITY_FLOOR_MIN_S, DIFFUSION_GRID, DecayBand, DecayReport, DescriptorErrors, EarlyReport,
    FirstArrival, Fit, FitOptions, FitRefusal, FitReport, FitValue, FittedControls, PROFILE_SPAN_S,
    PROFILE_START_S, REFINE_MAX_EVALUATIONS, REFINE_MIN_DIFFUSION_STEP, REFINE_STARTS,
    REFLECTION_MATCH_S, SearchReport, T20_WEIGHT, TONE_POINTS, TextureErrors, ToneReport,
    VERIFY_LEAD_S, WET_PEAK_CEILING, WIDTH_BISECTIONS, WidthReport, fit, fit_with,
};
pub use fit::{ENVELOPE_CLIP_DB, ENVELOPE_WEIGHT_PER_DB, ENVELOPE_WINDOW_S};
pub use fit::{HIGH_CUT_MIN_IMPROVEMENT_DB, HIGH_CUT_REFINE_STEPS, HIGH_CUT_STEPS_PER_OCTAVE};
pub use refusal::Refusal;

use math::{clamped_f32, energy_db, finite_f32};

// ---------------------------------------------------------------------------------------------------
// The input domain (plan §4.5).
// ---------------------------------------------------------------------------------------------------

/// A mono response.
pub const MIN_CHANNELS: u16 = 1;
/// A stereo response. True-stereo (four-response) and multichannel files are out of v1 (plan D10).
pub const MAX_CHANNELS: u16 = 2;
/// The lowest common production rate. Six of the seven octave bands exist there — the 8 kHz band's
/// upper edge, 11.3 kHz, lies above its Nyquist — and the shortest window, the ±0.5 ms reflection
/// width, is still 23 samples.
pub const MIN_SAMPLE_RATE_HZ: u32 = 22_050;
/// The top of the rates the collection measures at (`mxm-measure`'s `RATES`). A fit renders at the
/// file's own rate, so P3 narrows this to `mxm-classic-verb-dsp`'s validated range if that is
/// narrower.
pub const MAX_SAMPLE_RATE_HZ: u32 = 192_000;
/// Long enough to measure the shortest decay an ordinary room gives. *Derived*: a 0.2 s T60 takes
/// 0.2 s to fall 60 dB, and Lundeby's method estimates the noise from the last tenth of the response,
/// which 0.3 s leaves after the decay. **Measured**: a planted 0.2 s decay's T30 spread at 0.3 s is
/// no wider than the same plan's at 1.0 s (eight realisations each, at 22.05 and 48 kHz).
pub const MIN_DURATION_S: f64 = 0.3;
/// Three times the reverberation time of a large hall, and a bound on memory and time. **Measured**:
/// 30 s of stereo at 192 kHz — 11.5 million frames — analyses in 1.02 s in a release build, and the
/// whole process peaks at a 137 MB working set, 46 MB of it the input itself.
pub const MAX_DURATION_S: f64 = 30.0;

// ---------------------------------------------------------------------------------------------------
// Bands (§9.4).
// ---------------------------------------------------------------------------------------------------

/// The octave bands a decay is read in.
pub const OCTAVE_BANDS_HZ: [f32; 7] = [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];
/// The Butterworth low-pass prototype's order: second-order Butterworth, per IEC 61260 (§9.4).
/// **Measured consequence**: its skirts pass a neighbouring octave at roughly −15 dB, so a band next
/// to a slower-decaying one reads a few percent long — +3.7 % at 8 kHz beside a 4 kHz band decaying
/// 25 % slower, against +0.1 % with every band equal (eight realisations each).
pub const BAND_PROTOTYPE_ORDER: usize = 2;
/// A band is analysed only if its upper edge (centre × √2) is at most this fraction of Nyquist.
/// **Chosen.** At every standard rate in the accepted range the band set is the same for any fraction
/// from 0.52 to 1.0, so this decides only non-standard rates between 22.05 and 25.1 kHz, where it
/// drops the 8 kHz band rather than read an octave pressed against Nyquist.
pub const BAND_UPPER_EDGE_MAX_NYQUIST_FRACTION: f64 = 0.9;
/// A band must show a decay when its loudest block is within this many dB of the loudest band's.
/// **Chosen, to be revisited at P3.5 against real responses**: a band 30 dB down carries a
/// thousandth of the loudest band's energy. Measured consequence: a steady 1 kHz sine leaks into the
/// 500 Hz band 26 dB down, which makes that band required, and its missing decay refuses the sine.
pub const BAND_REQUIRED_WITHIN_DB: f64 = 30.0;

// ---------------------------------------------------------------------------------------------------
// Onset and direct sound (§9.1).
// ---------------------------------------------------------------------------------------------------

/// The onset is the first sample within this many dB of the direct-sound peak, and what precedes the
/// onset must lie at least this far below the direct sound or there is no identifiable onset
/// (ISO 3382-1's rule, §9.1, *secondary*).
pub const ONSET_BELOW_PEAK_DB: f64 = 20.0;
/// What is measured as preceding the onset stops this long before it, so a direct sound's own rising
/// edge is not counted against it; and at least this long must remain. **Chosen**: several times the
/// rise of the planted 0.3 ms direct pulse at every accepted rate.
pub const ONSET_GUARD_S: f64 = 0.001;
/// The direct sound is the energy within this far of its peak (the ACE challenge's n₀, 120 samples at
/// 48 kHz, §9.1).
pub const DIRECT_HALF_WINDOW_S: f64 = 0.0025;

// ---------------------------------------------------------------------------------------------------
// Decay (§9.3, §9.4). Lundeby's own step constants live with the method in `decay.rs`.
// ---------------------------------------------------------------------------------------------------

/// Lundeby's first broadband block (§9.3 step 1: 30 ms, "the mean of the 10 to 50 ms recommended").
/// In a band the block is (800 / f + 10) ms.
pub const LUNDEBY_BROADBAND_BLOCK_S: f64 = 0.030;
/// A band decay is one slope when its curve departs from its straight fit by at most this.
/// **Measured**: over 24 realisations of the planted single-slope room the largest departure was
/// 3.05 dB at 125 Hz, 2.12 dB at 250 Hz and under 1.5 dB above; over 8 realisations of a planted
/// two-slope decay (0.3 s, plus a 2.5 s slope 20 dB down) the smallest was 3.61 dB at 125 Hz and over
/// 5 dB above. The two come closest at 125 Hz.
pub const ONE_SLOPE_MAX_DEVIATION_DB: f32 = 3.3;

// ---------------------------------------------------------------------------------------------------
// Echo density (§9.6).
// ---------------------------------------------------------------------------------------------------

/// The rectangular window's length, at the short end of Abel and Huang's "20–30 ms works well".
/// **Measured, the steadiest of the page's options**: over 32 planted rooms at 44.1 and 48 kHz the
/// mixing time's error had a median of 4.0 ms and a 90th percentile of 11.8 ms, against 5.1 and
/// 42 ms for a 30 ms rectangular window and 7.9 and 50 ms for a Hann window of 20 ms effective length.
/// Every variant has a long tail (the worst 79–150 ms): a decaying tail's profile hovers just under
/// one, so "first reaches one" becomes a first passage.
pub const ECHO_DENSITY_WINDOW_S: f64 = 0.020;
/// One profile point per millisecond.
pub const ECHO_DENSITY_HOP_S: f64 = 0.001;
/// The profile runs this long after the direct sound. **Chosen**: the 224X's manual puts a hall's
/// echoes indistinguishable after about 300 ms (§10), and Lindau et al.'s largest room (8,500 m³)
/// predicts a perceptual mixing time of about 150 ms (§9.6); half a second covers both.
pub const ECHO_DENSITY_SPAN_S: f64 = 0.5;

// ---------------------------------------------------------------------------------------------------
// What follows the direct sound (§5, §9.1).
// ---------------------------------------------------------------------------------------------------

/// A feature is read only this far above the broadband noise floor (pre-delay, reflections, echo
/// density). *Derived*: 20 dB leaves noise under 1 % of a window's energy, so a window of noise
/// alone — which is Gaussian and would read as a mixed field — never counts.
pub const NOISE_MARGIN_DB: f64 = 20.0;
/// Pre-delay and early reflections count only energy within this many dB of the direct sound's.
/// **Chosen**: well below the weakest tap in the tables the research page cites (Moorer's smallest
/// listed gain, 0.134, is −17.5 dB, §5).
pub const EARLY_MIN_LEVEL_DB: f64 = -40.0;
/// Pre-delay slides a window this long. **Chosen**; measured consequence: a planted reflection's
/// delay is read to the sample, and a tail with no reflection ahead of it up to 0.52 ms late (24
/// realisations), because the reading is the tail's largest sample within
/// [`REFLECTION_HALF_WIDTH_S`] of its arrival.
pub const PRE_DELAY_WINDOW_S: f64 = 0.001;
/// The early window closes at the mixing time, or this long after the pre-delay if that comes first
/// (§5: early reflections occupy the first 40–100 ms, and *PASP* "often" the first 100 ms).
pub const EARLY_WINDOW_MAX_S: f64 = 0.1;
/// At most this many early reflections are kept (§5: Costello's "up to a few dozen" taps).
pub const MAX_EARLY_REFLECTIONS: usize = 24;
/// A reflection is a local maximum within this far either side, and its level is the energy over the
/// same width. **Chosen**: holds the planted 0.3 ms pulse whole at every accepted rate, so a planted
/// level is read within 0.01 dB, and separates reflections 0.5 ms apart.
pub const REFLECTION_HALF_WIDTH_S: f64 = 0.0005;

// ---------------------------------------------------------------------------------------------------
// Width (§9.7).
// ---------------------------------------------------------------------------------------------------

/// The late window opens 80 ms after the direct sound (§9.7).
pub const WIDTH_LATE_START_S: f64 = 0.080;
/// IACC is the largest correlation magnitude within this lag (§9.7: |τ| < 1 ms).
pub const WIDTH_MAX_LAG_S: f64 = 0.001;
/// The late window closes at the broadband noise floor's intersection, or this long after it opens.
/// **A cost bound**; ISO's upper limit is *unverified* on the research page. *Derived*: a 3 s T60
/// leaves 1 % of its late energy past the first second, so the bound discards nothing a correlation
/// would weigh for ordinary rooms.
pub const WIDTH_LATE_SPAN_MAX_S: f64 = 1.0;
/// A late window shorter than one echo density window is not measured.
pub const WIDTH_LATE_MIN_S: f64 = ECHO_DENSITY_WINDOW_S;

// ---------------------------------------------------------------------------------------------------
// The tail texture (this crate's `AGENTS.md`, *The tail texture*). The definitions restate the
// diffuseness measurement made outside the repository on 2026-09-15, after the owner heard fitted
// spaces the echo density profile had passed as "not diffuse enough" and "like a lot of delays"; they
// are its choices, not tuned here. Where this crate reads differently, the constant says so.
// ---------------------------------------------------------------------------------------------------

/// Segments are placed from the direct sound plus this: just past the direct-sound window's
/// ±[`DIRECT_HALF_WINDOW_S`].
pub const TEXTURE_ORIGIN_S: f64 = 0.003;
/// Segment A starts this fraction of the mid-band T60 after the origin …
pub const TEXTURE_A_START_T60: f64 = 0.15;
/// … but no sooner than this …
pub const TEXTURE_A_START_MIN_S: f64 = 0.02;
/// … and no later than this.
pub const TEXTURE_A_START_MAX_S: f64 = 0.1;
/// Segment A lasts this fraction of the mid-band T60 …
pub const TEXTURE_A_LENGTH_T60: f64 = 0.5;
/// … at least this …
pub const TEXTURE_A_LENGTH_MIN_S: f64 = 0.1;
/// … and at most this, where it is cut to the usable end.
pub const TEXTURE_A_LENGTH_MAX_S: f64 = 0.6;
/// Segment B follows A's end, as cut and before A is shortened, for up to this …
pub const TEXTURE_B_MAX_S: f64 = 0.8;
/// … and is not read when the usable end leaves it less than this.
pub const TEXTURE_B_MIN_S: f64 = 0.15;
/// **Ours**: segment A is not read when the usable end leaves it less than one echo density window,
/// where the reference has no floor for A and would divide by an empty segment.
pub const TEXTURE_MIN_SEGMENT_S: f64 = ECHO_DENSITY_WINDOW_S;
/// The usable end is the last frame whose centred mean square over this window, summed over channels
/// and counted from the origin, stands [`NOISE_MARGIN_DB`] over the broadband noise floor. **Ours**:
/// that floor is Lundeby's ([`Analysis::broadband`]), where the reference took the window's mean over
/// the response's last tenth.
pub const TEXTURE_USABLE_WINDOW_S: f64 = 0.02;
/// A segment holding at least this many frames is cut to the largest power of two it holds, so the
/// transforms can be radix 2; a shorter one is read at its own length.
pub const TEXTURE_POWER_OF_TWO_FROM: usize = 2048;
/// Decay compensation divides each sample by the square root of its centred mean square over this.
pub const TEXTURE_COMPENSATION_WINDOW_S: f64 = 0.03;
/// Spectral peakiness reads sixth octaves from here …
pub const PEAKINESS_LOW_HZ: f64 = 150.0;
/// … while an octave's upper edge is at most this …
pub const PEAKINESS_HIGH_HZ: f64 = 10_000.0;
/// … and at most this fraction of the sample rate.
pub const PEAKINESS_HIGH_RATE_FRACTION: f64 = 0.4;
/// Bands per octave: each band 2^(1/6) wide.
pub const PEAKINESS_BANDS_PER_OCTAVE: f64 = 6.0;
/// A band is read only when it holds at least this many periodogram bins.
pub const PEAKINESS_MIN_BINS: usize = 10;
/// The texture's echo density is read in [`ECHO_DENSITY_WINDOW_S`] windows centred this far apart.
/// **Rounded** to whole frames as every window here is, where the reference truncates: 221 frames
/// against 220 at 44.1 kHz, the same at 48 kHz.
pub const TEXTURE_DENSITY_HOP_S: f64 = 0.005;
/// Periodicity reads a log energy envelope: the centred mean square over this, in log10 …
pub const PERIODICITY_ENVELOPE_S: f64 = 0.001;
/// … less its own centred mean over this, which removes what is left of the decay.
pub const PERIODICITY_TREND_S: f64 = 0.05;
/// Periodicity is searched from this lag, or from the end of the autocorrelation's main lobe where
/// that comes later …
pub const PERIODICITY_MIN_LAG_S: f64 = 0.003;
/// … up to this lag, or half the segment where that is shorter.
pub const PERIODICITY_MAX_LAG_S: f64 = 0.25;

// ---------------------------------------------------------------------------------------------------
// Validity (plan §4.5).
// ---------------------------------------------------------------------------------------------------

/// Onset prominence scores fully here: ISO's recommended 35 dB peak-to-noise ratio (§9.3, via Guski
/// and Vorländer). It scores zero at [`ONSET_BELOW_PEAK_DB`], below which the onset is refused.
pub const PROMINENCE_FULL_DB: f64 = 35.0;
/// A required band's margin over its noise floor scores zero here. **Measured**: across a sweep of
/// planted noise floors (seven floors, four realisations each), no band with a margin under 25 dB had
/// a T20, and none under 35 dB had a T30. Lundeby's method itself stops at 15 dB (§9.3 step 3).
pub const MARGIN_ZERO_DB: f64 = 25.0;
/// … and fully here: the peak-to-noise ratio Guski and Vorländer found truncation with correction
/// needs (§9.3, method C: about 45 dB). Measured in the same sweep: above 45 dB every T30 was within
/// 7.4 %.
pub const MARGIN_FULL_DB: f64 = 45.0;
/// Straightness scores fully up to the one-slope threshold …
pub const STRAIGHTNESS_FULL_DB: f64 = ONE_SLOPE_MAX_DEVIATION_DB as f64;
/// … and zero from here. **Measured, and set so straightness reports rather than refuses**, as the
/// plan requires of a decay that is not one slope: the strongest planted two-slope decay (a 3.0 s
/// slope 25 dB under a 0.3 s one) departs by at most 11.6 dB and scores 0.5. On its own this check
/// refuses only past 15.8 dB, which no planted response reached.
pub const STRAIGHTNESS_ZERO_DB: f64 = 20.0;
/// A confidence below this refuses the response. **Measured**: with the margin ramp above, the floor
/// refuses a required band whose margin is under 30 dB. In the noise-floor sweep, bands with 25 to
/// 30 dB of margin had a T20 only 25 times in 28 and never a T30; from 30 dB every band had a T20.
pub const CONFIDENCE_FLOOR: f64 = 0.25;

// ---------------------------------------------------------------------------------------------------
// The public seams.
// ---------------------------------------------------------------------------------------------------

/// What a file header says, which is all [`preflight`] needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u64,
}

/// Stage 1 of §4.5: decided from a file header, before anything proportional to the file is
/// allocated.
pub fn preflight(header: &Header) -> Result<(), Refusal> {
    check_channels(usize::from(header.channels))?;
    let rate = f64::from(header.sample_rate);
    check_sample_rate(rate)?;
    check_duration(header.frames as f64 / rate)
}

/// Stage 2 of §4.5 (every sample finite) plus the whole analysis, by noise-compensated decay
/// ([`DecayMethod::NoiseCompensated`]). Stage 1 is checked again from the samples themselves.
pub fn analyse(channels: &[&[f32]], sample_rate: f32) -> Result<Analysis, Refusal> {
    analyse_with_method(channels, sample_rate, DecayMethod::NoiseCompensated)
}

/// [`analyse`], with the band decay times reported by a stated method. Only
/// [`DecayMethod::NoiseCompensated`] is a fit's input; the others exist to measure what noise
/// compensation changes on the same response. **Validity is judged on the noise-compensated decay
/// whichever method is asked for**, so a comparison sees the same refusals and confidence.
pub fn analyse_with_method(
    channels: &[&[f32]],
    sample_rate: f32,
    method: DecayMethod,
) -> Result<Analysis, Refusal> {
    analyse_inner(channels, sample_rate, method, true, None).map(|(analysis, _)| analysis)
}

/// [`analyse`], with the tail texture read **on stated segments** rather than on those the response's
/// own decay places; a segment that is absent reads no texture. A fit's verification render is read
/// on its response's segments (`FitReport::verification`), so a render cannot move its own ruler, and
/// this is how that analysis is made again: `classic_verb_generate` checks a recorded render with it,
/// and `mxm-classic-verb`'s space audit measures its renders with it.
pub fn analyse_with_segments(
    channels: &[&[f32]],
    sample_rate: f32,
    segments: &TextureSegments,
) -> Result<Analysis, Refusal> {
    analyse_inner(
        channels,
        sample_rate,
        DecayMethod::NoiseCompensated,
        true,
        Some(segments),
    )
    .map(|(analysis, _)| analysis)
}

/// The analysis of a fit's own verification render (plan §4.4): every measurement [`analyse`] makes,
/// its tail texture read on the response's `segments`, but a required band without a decay and a
/// confidence under the floor are **reported rather than refused**. The render is an impulse response
/// by construction; what the strict analyser would have refused it for comes back beside the analysis,
/// and the fit report carries it.
pub(crate) fn analyse_render(
    channels: &[&[f32]],
    sample_rate: f32,
    segments: &TextureSegments,
) -> Result<(Analysis, Option<Refusal>), Refusal> {
    analyse_inner(
        channels,
        sample_rate,
        DecayMethod::NoiseCompensated,
        false,
        Some(segments),
    )
}

/// The analysis. Strict, it refuses a required band without a decay and a confidence under the floor;
/// otherwise it records the first such refusal and goes on. The tail texture is read on `segments`
/// where they are given, and on the segments the response's decay places where not.
fn analyse_inner(
    channels: &[&[f32]],
    sample_rate: f32,
    method: DecayMethod,
    strict: bool,
    segments: Option<&TextureSegments>,
) -> Result<(Analysis, Option<Refusal>), Refusal> {
    check_channels(channels.len())?;
    let rate = f64::from(sample_rate);
    check_sample_rate(rate)?;
    let frames = channels[0].len();
    if let Some(other) = channels.iter().find(|c| c.len() != frames) {
        return Err(Refusal::ChannelLengthsDiffer {
            first: frames,
            other: other.len(),
        });
    }
    check_duration(frames as f64 / rate)?;
    if let Some((channel, frame)) = first_non_finite(channels) {
        return Err(Refusal::NonFiniteSample { channel, frame });
    }

    // The tail texture reads the samples as given, trailing silence included, so a window reaching past
    // the last sound counts it as the silence it is.
    let input = channels;

    // Trailing digital silence is not part of the response. Past the last non-zero sample a band
    // holds only the analysis filter's own ringing sliding into numeric dust, and with no noise floor
    // to stop it Lundeby's late fit would land there. Ending at the last non-zero sample is exactly
    // what a truncation at that point would cut, and changes nothing for a response that ends in
    // noise.
    let content = channels
        .iter()
        .filter_map(|c| c.iter().rposition(|&s| s != 0.0))
        .max()
        .map_or(0, |last| last + 1);
    let trimmed: Vec<&[f32]> = channels.iter().map(|c| &c[..content]).collect();
    let channels = trimmed.as_slice();

    // Onset and direct sound.
    let energy = early::combined_energy(channels)?;
    let direct = early::find_direct(&energy).ok_or(Refusal::Silence)?;
    let prominence = early::onset_prominence_db(&energy, &direct, rate);
    if let Some(prominence) = prominence.filter(|&p| p < ONSET_BELOW_PEAK_DB) {
        return Err(Refusal::NoIdentifiableOnset {
            prominence_db: clamped_f32(prominence),
        });
    }
    let drr = early::direct_to_reverberant_db(&energy, direct.peak, rate);

    // The broadband noise floor bounds everything read after the direct sound.
    let segment = &energy[direct.peak..];
    let broadband = decay::lundeby(segment, rate, LUNDEBY_BROADBAND_BLOCK_S);
    let broadband_noise = match &broadband.result {
        Ok(lundeby) => lundeby.noise_energy,
        Err(_) => broadband.first_noise_energy.unwrap_or(0.0),
    };
    let broadband_crossing = broadband
        .result
        .as_ref()
        .ok()
        .map(|l| l.crossing_s)
        .filter(|&t| t > 0.0 && t * rate < segment.len() as f64);
    let broadband_end = broadband_crossing
        .map(|t| direct.peak + (t * rate).ceil() as usize)
        .unwrap_or(content);

    // Density, pre-delay and the early window.
    let profile = density::echo_density(channels, rate, direct.peak, broadband_noise);
    let pre_delay_s = early::pre_delay(&energy, direct.peak, rate, broadband_noise)
        .map(|i| (i - direct.peak) as f64 / rate);
    let cap_s = pre_delay_s.unwrap_or(0.0) + EARLY_WINDOW_MAX_S;
    let (window_end_s, window_end) = match profile.mixing_time_s {
        Some(mixing) if mixing < cap_s => (mixing, EarlyWindowEnd::MixingTime),
        _ => (cap_s, EarlyWindowEnd::Cap),
    };
    let reflections = early::early_reflections(
        channels,
        &energy,
        direct.peak,
        rate,
        broadband_noise,
        direct.peak + (window_end_s * rate).round() as usize,
    );
    let direct_half = math::samples(DIRECT_HALF_WINDOW_S, rate);
    // Where the tail texture's segments must end, read on the combined energy before it goes.
    let usable = texture::usable_end(
        &energy,
        direct.peak + math::samples(TEXTURE_ORIGIN_S, rate),
        rate,
        broadband_noise,
    );
    drop(energy);

    // Bands.
    let (bands, deviations, mut withheld) =
        analyse_bands(channels, rate, direct.peak, method, strict)?;

    let width = width::late_coherence(channels, rate, direct.peak, broadband_end);
    let texture_frames = match segments {
        Some(given) => [given.a, given.b].map(|s| s.and_then(|s| texture::frames_of(&s, rate))),
        None => texture::choose(texture::mid_band_t60(&bands), usable, rate),
    };
    let [texture_a, texture_b] = texture_frames
        .map(|frames| frames.and_then(|r| texture::measure(input, rate, direct.peak, r)));
    let validity = validity::assess(prominence, &bands, &deviations);
    if f64::from(validity.confidence) < CONFIDENCE_FLOOR {
        if let Some(check) = validity.weakest {
            let refusal = Refusal::LowConfidence {
                confidence: validity.confidence,
                check,
            };
            if strict {
                return Err(refusal);
            }
            withheld.get_or_insert(refusal);
        }
    }

    let analysis = Analysis {
        sample_rate,
        channels: channels.len() as u16,
        frames,
        method,
        onset: Onset {
            onset_s: clamped_f32(direct.onset as f64 / rate),
            direct_s: clamped_f32(direct.peak as f64 / rate),
            direct_peak_db: clamped_f32(energy_db(direct.peak_energy)),
            prominence_db: prominence.and_then(finite_f32),
            drr_db: drr.and_then(finite_f32),
        },
        pre_delay_s: pre_delay_s.and_then(finite_f32),
        early: EarlyReflections {
            window_start_s: clamped_f32((direct_half + 1) as f64 / rate),
            window_end_s: clamped_f32(window_end_s),
            window_end,
            reflections: reflections
                .into_iter()
                .map(|peak| Reflection {
                    delay_s: clamped_f32((peak.index - direct.peak) as f64 / rate),
                    level_db: clamped_f32(peak.level_db),
                    channel_level_db: peak
                        .channel_level_db
                        .into_iter()
                        .map(|level| level.and_then(finite_f32))
                        .collect(),
                })
                .collect(),
        },
        broadband: BroadbandDecay {
            noise_floor_db: clamped_f32(energy_db(broadband_noise)),
            intersection_s: broadband_crossing.and_then(finite_f32),
            failure: broadband.result.err(),
        },
        bands,
        echo_density: EchoDensity {
            window_s: ECHO_DENSITY_WINDOW_S as f32,
            effective_window_s: ECHO_DENSITY_WINDOW_S as f32,
            hop_s: ECHO_DENSITY_HOP_S as f32,
            points: profile
                .points
                .into_iter()
                .map(|(time, density)| DensityPoint {
                    time_s: clamped_f32(time),
                    density: density.and_then(finite_f32),
                })
                .collect(),
            mixing_time_s: profile.mixing_time_s.and_then(finite_f32),
        },
        width,
        texture_a,
        texture_b,
        validity,
    };
    Ok((analysis, withheld))
}

/// The bands, each band's noise-compensated straightness statistic, and the refusal a non-strict
/// analysis withheld.
type BandReadings = (Vec<Band>, Vec<Option<f32>>, Option<Refusal>);

/// Every octave band's decay, or the refusal of the first required band that has none — returned
/// beside the bands rather than instead of them when not `strict`.
///
/// Alongside the bands, each band's straightness statistic **as read by the noise-compensated
/// method**, whatever `method` reports: validity is a property of the response, not of the
/// integration a comparison asked for, and an uncompensated curve bends by construction.
fn analyse_bands(
    channels: &[&[f32]],
    rate: f64,
    start: usize,
    method: DecayMethod,
    strict: bool,
) -> Result<BandReadings, Refusal> {
    struct Work {
        centre_hz: f32,
        max_block_db: Option<f64>,
        result: Option<Result<(BandDecay, Option<f32>), DecayFailure>>,
    }

    let frames = channels[0].len();
    let mut work = Vec::with_capacity(OCTAVE_BANDS_HZ.len());
    for &centre_hz in &OCTAVE_BANDS_HZ {
        let centre = f64::from(centre_hz);
        if !band::band_is_present(centre, rate) {
            work.push(Work {
                centre_hz,
                max_block_db: None,
                result: None,
            });
            continue;
        }
        let filter = band::OctaveFilter::new(centre, rate);
        let mut energy = buffer::filled(frames - start, 0.0f64)?;
        for channel in channels {
            let filtered = filter.filter(channel)?;
            for (sum, value) in energy.iter_mut().zip(&filtered[start..]) {
                *sum += value * value;
            }
        }
        let outcome = decay::lundeby(&energy, rate, decay::band_block_s(centre));
        let result = match outcome.result {
            Ok(lundeby) => {
                let decay = measure_band(&energy, rate, &lundeby, method)?;
                let compensated = if method == DecayMethod::NoiseCompensated {
                    decay.straightness
                } else {
                    measure_band(&energy, rate, &lundeby, DecayMethod::NoiseCompensated)?
                        .straightness
                };
                Ok((decay, compensated.map(|s| s.max_deviation_db)))
            }
            Err(failure) => Err(failure),
        };
        work.push(Work {
            centre_hz,
            max_block_db: outcome.max_block_db,
            result: Some(result),
        });
    }

    let loudest = work
        .iter()
        .filter_map(|w| w.max_block_db)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut bands = Vec::with_capacity(work.len());
    let mut deviations = Vec::with_capacity(work.len());
    let mut withheld = None;
    for w in work {
        let Some(result) = w.result else {
            bands.push(Band {
                centre_hz: w.centre_hz,
                required: false,
                reading: BandReading::AboveNyquist,
            });
            deviations.push(None);
            continue;
        };
        // A band too short even to block-average cannot show a decay, so it is required.
        let required = w
            .max_block_db
            .is_none_or(|max| max >= loudest - BAND_REQUIRED_WITHIN_DB);
        let reading = match result {
            Ok((decay, deviation)) => {
                deviations.push(deviation);
                BandReading::Measured(decay)
            }
            Err(failure) if required && strict => {
                return Err(Refusal::NoDecayAboveNoiseFloor {
                    band_hz: w.centre_hz,
                    failure,
                });
            }
            Err(failure) => {
                if required {
                    withheld.get_or_insert(Refusal::NoDecayAboveNoiseFloor {
                        band_hz: w.centre_hz,
                        failure,
                    });
                }
                deviations.push(None);
                BandReading::Empty {
                    below_loudest_db: clamped_f32(loudest - w.max_block_db.unwrap_or(loudest)),
                    failure,
                }
            }
        };
        bands.push(Band {
            centre_hz: w.centre_hz,
            required,
            reading,
        });
    }

    let reference = bands
        .iter()
        .find(|band| band.centre_hz == 1000.0)
        .and_then(Band::decay)
        .and_then(|decay| decay.initial_level_db);
    if let Some(reference) = reference {
        for band in &mut bands {
            if let BandReading::Measured(decay) = &mut band.reading {
                decay.level_re_1k_db = decay.initial_level_db.map(|level| level - reference);
            }
        }
    }
    Ok((bands, deviations, withheld))
}

/// One band's decay times, straightness and levels from its energy and its Lundeby run, or the
/// refusal that its decay curve could not be reserved.
fn measure_band(
    energy: &[f64],
    rate: f64,
    lundeby: &decay::Lundeby,
    method: DecayMethod,
) -> Result<BandDecay, buffer::OutOfMemory> {
    let curve = decay::decay_curve(energy, rate, lundeby, method)?;
    let t20 = decay::fit_range(&curve, rate, decay::T20_RANGE_DB);
    let t30 = decay::fit_range(&curve, rate, decay::T30_RANGE_DB);
    let edt = decay::fit_range(&curve, rate, decay::EDT_RANGE_DB);
    let time = |fit: &Option<decay::CurveFit>| {
        fit.as_ref()
            .and_then(decay::decay_time)
            .and_then(finite_f32)
    };

    let widest = match (&t30, &t20) {
        (Some(fit), _) => Some((fit, decay::T30_RANGE_DB)),
        (None, Some(fit)) => Some((fit, decay::T20_RANGE_DB)),
        (None, None) => None,
    };
    let straightness = widest.map(|(fit, range)| {
        let departure = decay::straightness(&curve, rate, fit, range);
        let max_deviation_db = clamped_f32(departure.max_db);
        Straightness {
            range_bottom_db: range.1 as f32,
            max_deviation_db,
            late_to_early: departure.late_to_early.and_then(finite_f32),
            single_slope: max_deviation_db <= ONE_SLOPE_MAX_DEVIATION_DB,
        }
    });
    let initial_level_db = widest
        .and_then(|(fit, _)| decay::initial_level_db(&curve, fit, rate))
        .and_then(finite_f32);

    let length_s = energy.len() as f64 / rate;
    Ok(BandDecay {
        t20_s: time(&t20),
        t30_s: time(&t30),
        edt_s: time(&edt),
        noise_floor_db: clamped_f32(lundeby.noise_db),
        intersection_s: clamped_f32(lundeby.crossing_s.clamp(0.0, length_s)),
        noise_floor_reached: lundeby.crossing_s > 0.0 && lundeby.crossing_s < length_s,
        lundeby_iterations: lundeby.iterations,
        lundeby_converged: lundeby.converged,
        late_slope_db_per_s: clamped_f32(lundeby.line.slope),
        initial_level_db,
        level_re_1k_db: None,
        margin_db: clamped_f32(lundeby.line.intercept - lundeby.noise_db),
        straightness,
    })
}

fn check_channels(channels: usize) -> Result<(), Refusal> {
    if (usize::from(MIN_CHANNELS)..=usize::from(MAX_CHANNELS)).contains(&channels) {
        Ok(())
    } else {
        Err(Refusal::ChannelCount { channels })
    }
}

fn check_sample_rate(rate: f64) -> Result<(), Refusal> {
    if rate.is_finite()
        && rate >= f64::from(MIN_SAMPLE_RATE_HZ)
        && rate <= f64::from(MAX_SAMPLE_RATE_HZ)
    {
        Ok(())
    } else {
        Err(Refusal::SampleRate { sample_rate: rate })
    }
}

fn check_duration(seconds: f64) -> Result<(), Refusal> {
    if seconds < MIN_DURATION_S {
        Err(Refusal::TooShort { seconds })
    } else if seconds > MAX_DURATION_S {
        Err(Refusal::TooLong { seconds })
    } else {
        Ok(())
    }
}

/// The first non-finite sample in frame order, and its channel.
fn first_non_finite(channels: &[&[f32]]) -> Option<(usize, usize)> {
    channels
        .iter()
        .enumerate()
        .filter_map(|(channel, samples)| {
            samples
                .iter()
                .position(|s| !s.is_finite())
                .map(|frame| (channel, frame))
        })
        .min_by_key(|&(channel, frame)| (frame, channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loud click, a 0.4 s decay, and a floor about 25 dB under the decay's start: a response the
    /// analyser refuses for its margin, but whose onset stays identifiable.
    fn low_margin_response() -> Vec<f32> {
        let rate = 44_100usize;
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 52) as f64 - 1.0
        };
        (0..2 * rate)
            .map(|i| {
                let t = i as f64 / rate as f64;
                let click = if i == 2_205 { 10.0 } else { 0.0 };
                let tail = if i > 2_205 {
                    0.3 * 10f64.powf(-3.0 * (t - 0.05) / 0.4) * noise()
                } else {
                    0.0
                };
                (click + tail + 0.01 * noise()) as f32
            })
            .collect()
    }

    #[test]
    fn a_render_is_analysed_where_a_response_would_be_refused_and_says_why() {
        let x = low_margin_response();
        let strict = analyse(&[&x], 44_100.0).expect_err("refused as a response");
        assert!(
            !matches!(strict, Refusal::NoIdentifiableOnset { .. }),
            "the onset must stay identifiable for this to test anything: {strict:?}"
        );
        let (analysis, withheld) = analyse_render(&[&x], 44_100.0, &TextureSegments::default())
            .expect("a render is analysed");
        assert_eq!(withheld, Some(strict));
        assert_eq!(analysis.bands.len(), OCTAVE_BANDS_HZ.len());
    }
}
