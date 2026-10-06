//! Why a response was refused. One variant per failing check, each naming what it measured.

use std::fmt;

use crate::analysis::{CheckKind, DecayFailure};

/// A response the analyser will not describe, and why.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// Not one or two channels.
    ChannelCount { channels: usize },
    /// Outside [`crate::MIN_SAMPLE_RATE_HZ`]..=[`crate::MAX_SAMPLE_RATE_HZ`], or not finite.
    SampleRate { sample_rate: f64 },
    /// Shorter than [`crate::MIN_DURATION_S`].
    TooShort { seconds: f64 },
    /// Longer than [`crate::MAX_DURATION_S`].
    TooLong { seconds: f64 },
    /// The channels hold different numbers of frames.
    ChannelLengthsDiffer { first: usize, other: usize },
    /// The first non-finite sample in frame order.
    NonFiniteSample { channel: usize, frame: usize },
    /// Every sample is exactly zero, so there is no onset to find.
    Silence,
    /// What precedes the onset is not [`crate::ONSET_BELOW_PEAK_DB`] below the direct sound.
    NoIdentifiableOnset { prominence_db: f32 },
    /// A required band shows no decay above its noise floor.
    NoDecayAboveNoiseFloor { band_hz: f32, failure: DecayFailure },
    /// The confidence fell below [`crate::CONFIDENCE_FLOOR`]; `check` is the weakest check.
    LowConfidence { confidence: f32, check: CheckKind },
    /// A buffer the response's length sizes could not be reserved: `bytes` is what was asked for.
    /// **Not a judgement of the response**, which a machine with more memory would read; it is a
    /// refusal so that running out is named instead of aborting the process the analysis runs in.
    OutOfMemory { bytes: usize },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::ChannelCount { channels } => write!(
                f,
                "{channels} channels; an impulse response has {} or {}",
                crate::MIN_CHANNELS,
                crate::MAX_CHANNELS
            ),
            Refusal::SampleRate { sample_rate } => write!(
                f,
                "sample rate {sample_rate} Hz is outside {}..={} Hz",
                crate::MIN_SAMPLE_RATE_HZ,
                crate::MAX_SAMPLE_RATE_HZ
            ),
            Refusal::TooShort { seconds } => write!(
                f,
                "{seconds:.3} s is shorter than the {} s needed to measure a decay",
                crate::MIN_DURATION_S
            ),
            Refusal::TooLong { seconds } => write!(
                f,
                "{seconds:.3} s is longer than the {} s ceiling",
                crate::MAX_DURATION_S
            ),
            Refusal::ChannelLengthsDiffer { first, other } => write!(
                f,
                "channels hold different lengths ({first} and {other} frames)"
            ),
            Refusal::NonFiniteSample { channel, frame } => {
                write!(f, "sample {frame} of channel {channel} is not finite")
            }
            Refusal::Silence => write!(f, "every sample is zero"),
            Refusal::NoIdentifiableOnset { prominence_db } => write!(
                f,
                "no identifiable onset: the direct sound stands only {prominence_db:.1} dB over what precedes it, under {} dB",
                crate::ONSET_BELOW_PEAK_DB
            ),
            Refusal::NoDecayAboveNoiseFloor { band_hz, failure } => write!(
                f,
                "no decay above the noise floor in the {band_hz} Hz band ({failure:?})"
            ),
            Refusal::LowConfidence { confidence, check } => write!(
                f,
                "confidence {confidence:.2} is under the floor of {}; weakest check: {check:?}",
                crate::CONFIDENCE_FLOOR
            ),
            Refusal::OutOfMemory { bytes } => write!(
                f,
                "not enough memory: {:.1} MB could not be reserved",
                *bytes as f64 / 1_048_576.0
            ),
        }
    }
}

impl std::error::Error for Refusal {}
