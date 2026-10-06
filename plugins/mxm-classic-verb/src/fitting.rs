//! **The one module that names `mxm-classic-verb-fit`.**
//!
//! Everything the plugin takes from the fitter passes through here and comes out in this plugin's own
//! types — [`Refusal`], [`Fitted`], [`Report`] — so a change to the fit crate's API or its field names
//! is a change to this file alone. What is used: `Header` and `preflight` (plan §4.5, stage 1), `fit`
//! with its `Fit`, `FitRefusal` and `Refusal`, the report's `DescriptorErrors` and `Clamp`s, and
//! `CONFIDENCE_FLOOR`.
//!
//! # Against P3's delivered fit crate
//!
//! Written against the fit crate as this branch has it (`fae3577`). **Every name used here exists
//! with the same shape in the P3 delivery**, and this file and `payload.rs`, unmodified, compile
//! against that delivery's `mxm-classic-verb-fit` and `mxm-classic-verb-dsp` (checked 2026-09-14 on
//! Windows, in a scratch crate outside the repository). So the merge needs nothing here to build.
//! What P3 adds, and what taking each would be — additive, a few lines apiece:
//!
//! - `DescriptorErrors::envelope_db`: one more `(Descriptor::Envelope, measured.envelope_db)` entry
//!   below, with a `Descriptor::Envelope` variant and a 1 dB scale in `payload.rs`. An older build
//!   reads the new descriptor as `Descriptor::Other`, shown and never ranked, so the payload version
//!   does not move.
//! - `FitReport::verification_refusal: Option<Refusal>`: a render the analyser would refuse as a
//!   response is no longer a `FitRefusal`; its `to_string()` could join `Report` as a hover note.
//! - `FitRefusal::Verification` now means a render that could not be analysed at all; it still maps
//!   to `Refusal::Input` through its `Display`, as below.
//! - `DescriptorErrors::texture_a` and `texture_b` (`TextureErrors`: spectral peakiness, kurtosis,
//!   echo density and periodicity over the response's tail segments, render minus response): up to
//!   eight more entries below, with a `Descriptor` variant and a scale each in `payload.rs`. Reported
//!   by the fit crate, never refused; an older build reads them as `Descriptor::Other`.

use mxm_classic_verb_fit::{CheckKind, Clamp, Fit, FitRefusal, FitValue};

use crate::decode::Header;
use crate::loading::{Fitted, FittedControls, Outcome, Refusal};
use crate::payload::{ClampNote, Clamped, Descriptor, DescriptorError, Report};

/// The fit crate's floor: a response under it is refused, with its weakest check named.
pub const CONFIDENCE_FLOOR: f32 = mxm_classic_verb_fit::CONFIDENCE_FLOOR as f32;

/// Stage 1 of plan §4.5, from a header, before anything proportional to the file exists.
pub fn preflight(header: &Header) -> Result<(), Refusal> {
    mxm_classic_verb_fit::preflight(&mxm_classic_verb_fit::Header {
        channels: header.channels,
        sample_rate: header.sample_rate,
        frames: header.frames,
    })
    .map_err(|refusal| Refusal::Input(refusal.to_string()))
}

/// Analyses, fits and verifies a decoded response. Not realtime: it allocates and runs for as long
/// as the response takes, which is why only the background task calls it.
pub fn fit(channels: &[&[f32]], sample_rate: u32) -> Outcome {
    let fit =
        mxm_classic_verb_fit::fit(channels, sample_rate as f32).map_err(|error| refusal(&error))?;
    judged(from_the_fit(&fit)?)
}

/// **The floor, checked here as well as in the analyser.** The fit crate refuses a response under
/// `CONFIDENCE_FLOOR`; this holds the plugin to the same line whatever that crate does next.
pub fn judged(fitted: Fitted) -> Outcome {
    let confidence = fitted.report.confidence;
    if confidence.is_finite() && confidence >= CONFIDENCE_FLOOR {
        Ok(fitted)
    } else {
        Err(Refusal::LowConfidence {
            confidence,
            floor: CONFIDENCE_FLOOR,
            check: "the response's overall validity".to_owned(),
        })
    }
}

fn refusal(error: &FitRefusal) -> Refusal {
    match error {
        FitRefusal::Analysis(mxm_classic_verb_fit::Refusal::LowConfidence {
            confidence,
            check,
        }) => Refusal::LowConfidence {
            confidence: *confidence,
            floor: CONFIDENCE_FLOOR,
            check: check_name(check),
        },
        FitRefusal::OutOfMemory { bytes }
        | FitRefusal::Analysis(mxm_classic_verb_fit::Refusal::OutOfMemory { bytes }) => {
            Refusal::OutOfMemory { bytes: *bytes }
        }
        FitRefusal::Analysis(refusal) => Refusal::Input(refusal.to_string()),
        other => Refusal::Input(other.to_string()),
    }
}

fn band(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{} kHz", hz / 1_000.0)
    } else {
        format!("{hz:.0} Hz")
    }
}

fn check_name(check: &CheckKind) -> String {
    match check {
        CheckKind::OnsetProminence => "how far the onset stands over what precedes it".to_owned(),
        CheckKind::NoiseMargin { band_hz } => {
            format!(
                "the decay's margin over the noise floor at {}",
                band(*band_hz)
            )
        }
        CheckKind::Straightness { band_hz } => {
            format!("how straight the decay runs at {}", band(*band_hz))
        }
    }
}

fn from_the_fit(fit: &Fit) -> Outcome {
    let controls = FittedControls {
        decay_s: fit.controls.decay_s,
        size_s: fit.controls.size_s,
        diffusion: fit.controls.diffusion,
        pre_delay_s: fit.controls.pre_delay_s,
    };
    if ![
        controls.decay_s,
        controls.size_s,
        controls.diffusion,
        controls.pre_delay_s,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return Err(Refusal::Input(
            "the fit returned a control that is not a number".to_owned(),
        ));
    }

    let measured = &fit.report.errors;
    let mut errors = Vec::new();
    for band in &measured.bands {
        if let Some(value) = band.t30_percent.or(band.t20_percent) {
            errors.push(DescriptorError {
                what: Descriptor::Decay,
                hz: Some(band.centre_hz),
                value,
            });
        }
        if let Some(value) = band.tone_db {
            errors.push(DescriptorError {
                what: Descriptor::Tone,
                hz: Some(band.centre_hz),
                value,
            });
        }
    }
    for (what, value) in [
        (Descriptor::PreDelay, measured.pre_delay_s),
        (Descriptor::MixingTime, measured.mixing_time_s),
        (Descriptor::Iacc, measured.iacc),
        (Descriptor::EarlyToLate, measured.early_to_late_db),
        (Descriptor::ReflectionTime, measured.reflection_time_rms_s),
        (
            Descriptor::ReflectionLevel,
            measured.reflection_level_rms_db,
        ),
    ] {
        if let Some(value) = value {
            errors.push(DescriptorError {
                what,
                hz: None,
                value,
            });
        }
    }

    Ok(Fitted {
        space: fit.space.sanitised(),
        controls,
        report: Report {
            confidence: fit.report.target.validity.confidence,
            errors,
            clamps: fit.report.clamps.iter().map(clamp_note).collect(),
        },
    })
}

fn clamp_note(clamp: &Clamp) -> ClampNote {
    ClampNote {
        what: match clamp.value {
            FitValue::Decay => Clamped::Decay,
            FitValue::Size => Clamped::Size,
            FitValue::Diffusion => Clamped::Diffusion,
            FitValue::PreDelay => Clamped::PreDelay,
            FitValue::DecayRatioLow => Clamped::DecayRatioLow,
            FitValue::DecayRatioHigh => Clamped::DecayRatioHigh,
            FitValue::DecayRatioTop => Clamped::DecayRatioTop,
            FitValue::ToneLow => Clamped::ToneLow,
            FitValue::ToneHigh => Clamped::ToneHigh,
            FitValue::Width => Clamped::Width,
            FitValue::EarlyLevel => Clamped::EarlyLevel,
            FitValue::HighCut => Clamped::HighCut,
        },
        fitted: clamp.fitted,
        applied: clamp.applied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loaded::Status;
    use crate::loading::{self, GestureWatch, Settled};
    use crate::params::{MxmClassicVerbParams, SpaceChoice};
    use crate::testing::{self, ApplyingHost, WavFormat};
    use mxm_classic_verb_dsp::Space;
    use std::sync::Arc;

    #[test]
    fn the_header_stage_is_the_fit_crates_own_preflight() {
        let header = |channels, sample_rate, frames| Header {
            channels,
            sample_rate,
            frames,
        };
        assert_eq!(preflight(&header(2, 48_000, 96_000)), Ok(()));
        for (refused, says) in [
            (header(3, 48_000, 96_000), "3 channels"),
            (header(1, 8_000, 16_000), "sample rate"),
            (header(1, 48_000, 4_800), "shorter"),
            (header(2, 48_000, 48_000 * 60), "longer"),
        ] {
            match preflight(&refused) {
                Err(Refusal::Input(why)) => assert!(why.contains(says), "{why}"),
                other => panic!("{refused:?} gave {other:?}"),
            }
        }
    }

    #[test]
    fn a_low_confidence_analysis_names_its_weakest_check() {
        let refused = refusal(&FitRefusal::Analysis(
            mxm_classic_verb_fit::Refusal::LowConfidence {
                confidence: 0.12,
                check: CheckKind::NoiseMargin { band_hz: 500.0 },
            },
        ));
        assert_eq!(
            refused,
            Refusal::LowConfidence {
                confidence: 0.12,
                floor: CONFIDENCE_FLOOR,
                check: "the decay's margin over the noise floor at 500 Hz".to_owned(),
            }
        );
        assert_eq!(
            refused.to_string(),
            "confidence 0.12 is under 0.25; the weakest check is the decay's margin over the noise floor at 500 Hz"
        );
        assert_eq!(
            refusal(&FitRefusal::Analysis(
                mxm_classic_verb_fit::Refusal::Silence
            )),
            Refusal::Input("every sample is zero".to_owned())
        );
    }

    /// The fit crate names running out of memory as its own refusal, whichever stage ran out; here it
    /// stays that refusal, never an input the response was refused for.
    #[test]
    fn a_fit_that_runs_out_of_memory_is_refused_as_that() {
        let bytes = 46_080_000;
        for refused in [
            FitRefusal::OutOfMemory { bytes },
            FitRefusal::Analysis(mxm_classic_verb_fit::Refusal::OutOfMemory { bytes }),
        ] {
            assert_eq!(
                refusal(&refused),
                Refusal::OutOfMemory { bytes },
                "{refused:?}"
            );
        }
        assert_eq!(
            Refusal::OutOfMemory { bytes }.to_string(),
            "not enough memory for this response (43.9 MB could not be reserved)"
        );
    }

    #[test]
    fn a_fit_under_the_floor_is_refused_here_too() {
        assert!(matches!(
            judged(testing::fitted(Space::ROOM, CONFIDENCE_FLOOR - 0.01)),
            Err(Refusal::LowConfidence { .. })
        ));
        assert!(judged(testing::fitted(Space::ROOM, f32::NAN)).is_err());
        assert!(judged(testing::fitted(Space::ROOM, CONFIDENCE_FLOOR)).is_ok());
    }

    #[test]
    fn digital_silence_is_refused_by_the_analysis_and_named() {
        let silence = [vec![0.0f32; 24_000]];
        let channels: Vec<&[f32]> = silence.iter().map(Vec::as_slice).collect();
        assert_eq!(
            fit(&channels, 24_000),
            Err(Refusal::Input("every sample is zero".to_owned()))
        );
    }

    /// **The whole path on a synthetic response**: a mono WAV written to a temporary file is
    /// preflighted, decoded and fitted by the real fitter, then lands as a space and gestures. No
    /// response file from anywhere else is read.
    #[test]
    fn a_dropped_wav_is_fitted_by_the_real_fitter_and_lands() {
        let planted = testing::response(24_000, 1.2, 0.5, 1, 11);
        let file = testing::TempFile::new(
            "pipeline.wav",
            &testing::wav_bytes(&planted, 24_000, WavFormat::Float32),
        );
        let params = MxmClassicVerbParams::default();
        let host = Arc::new(ApplyingHost::default());
        let watch = GestureWatch::new(nice_plug::context::gui::GuiContext::new(host.clone()));

        let started = std::time::Instant::now();
        let loading::LoadTask::Fit { generation, path } =
            loading::begin(&params.loaded, file.path());
        loading::run(&params.loaded, generation, &path, &fit);
        eprintln!("fitted in {:.2} s", started.elapsed().as_secs_f64());

        assert_eq!(
            loading::settle(&params, &watch.setter()),
            Settled::Loaded,
            "{:?}",
            params.loaded.status()
        );
        assert_eq!(params.loaded.status(), Status::Idle);
        let space = params.loaded.space().expect("a space is held");
        assert_eq!(space, space.sanitised());
        let report = params.loaded.report().expect("its report is kept");
        assert!(report.confidence >= CONFIDENCE_FLOOR);
        assert!(
            !report.errors.is_empty(),
            "the fit measured nothing about itself"
        );
        assert_eq!(params.space.value(), SpaceChoice::Loaded);
        assert!(
            params.decay.value() > 0.2 && params.decay.value() < 1.5,
            "{}",
            params.decay.value()
        );
        assert_eq!(host.begins(), host.ends());
    }
}
