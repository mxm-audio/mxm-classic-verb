//! Every refusal: each preflight bound from the header and again from the samples, a non-finite
//! sample, digital silence, and responses that are not impulse responses at all
//! (`plans/plan-mxm-classic-verb.md` §4.5, §4.6). Nothing here reads a file.

mod support;

use mxm_classic_verb_fit::{
    CONFIDENCE_FLOOR, CheckKind, DecayFailure, Header, MAX_CHANNELS, MAX_DURATION_S,
    MAX_SAMPLE_RATE_HZ, MIN_CHANNELS, MIN_DURATION_S, MIN_SAMPLE_RATE_HZ, ONSET_BELOW_PEAK_DB,
    Refusal, analyse, preflight,
};
use support::{Layout, Plan, XorShift};

fn header(channels: u16, sample_rate: u32, seconds: f64) -> Header {
    Header {
        channels,
        sample_rate,
        frames: (seconds * f64::from(sample_rate)).ceil() as u64,
    }
}

#[test]
fn preflight_accepts_every_bound_itself() {
    for channels in [MIN_CHANNELS, MAX_CHANNELS] {
        for rate in [MIN_SAMPLE_RATE_HZ, 48_000, MAX_SAMPLE_RATE_HZ] {
            for seconds in [MIN_DURATION_S, 2.0, MAX_DURATION_S] {
                let h = Header {
                    channels,
                    sample_rate: rate,
                    frames: (seconds * f64::from(rate)).round() as u64,
                };
                assert_eq!(preflight(&h), Ok(()), "{h:?}");
            }
        }
    }
}

#[test]
fn preflight_refuses_each_bound_from_the_header_alone() {
    assert_eq!(
        preflight(&header(0, 48_000, 1.0)),
        Err(Refusal::ChannelCount { channels: 0 })
    );
    assert_eq!(
        preflight(&header(3, 48_000, 1.0)),
        Err(Refusal::ChannelCount { channels: 3 })
    );
    assert!(matches!(
        preflight(&Header {
            channels: 1,
            sample_rate: MIN_SAMPLE_RATE_HZ - 1,
            frames: 48_000
        }),
        Err(Refusal::SampleRate { .. })
    ));
    assert!(matches!(
        preflight(&Header {
            channels: 1,
            sample_rate: MAX_SAMPLE_RATE_HZ + 1,
            frames: 480_000
        }),
        Err(Refusal::SampleRate { .. })
    ));
    assert!(matches!(
        preflight(&Header {
            channels: 1,
            sample_rate: 0,
            frames: 48_000
        }),
        Err(Refusal::SampleRate { .. })
    ));
    // One frame under the floor, one frame over the ceiling.
    let under = Header {
        channels: 1,
        sample_rate: 48_000,
        frames: (MIN_DURATION_S * 48_000.0) as u64 - 1,
    };
    assert!(matches!(preflight(&under), Err(Refusal::TooShort { .. })));
    let over = Header {
        channels: 2,
        sample_rate: 48_000,
        frames: (MAX_DURATION_S * 48_000.0) as u64 + 1,
    };
    assert!(matches!(preflight(&over), Err(Refusal::TooLong { .. })));
    // A header at the ceiling's rate and length is refused without anything being allocated for it:
    // preflight takes no samples.
    let enormous = Header {
        channels: 2,
        sample_rate: 192_000,
        frames: u64::MAX,
    };
    assert!(matches!(preflight(&enormous), Err(Refusal::TooLong { .. })));
}

#[test]
fn analyse_checks_the_same_bounds_from_the_samples() {
    let second = vec![0.0f32; 48_000];
    assert_eq!(
        analyse(&[], 48_000.0).err(),
        Some(Refusal::ChannelCount { channels: 0 })
    );
    assert_eq!(
        analyse(&[&second, &second, &second], 48_000.0).err(),
        Some(Refusal::ChannelCount { channels: 3 })
    );
    for rate in [
        MIN_SAMPLE_RATE_HZ as f32 - 1.0,
        MAX_SAMPLE_RATE_HZ as f32 + 1.0,
        f32::NAN,
        f32::INFINITY,
        -48_000.0,
    ] {
        assert!(
            matches!(analyse(&[&second], rate), Err(Refusal::SampleRate { .. })),
            "rate {rate}"
        );
    }
    let short = vec![0.0f32; (MIN_DURATION_S * 48_000.0) as usize - 1];
    assert!(matches!(
        analyse(&[&short], 48_000.0),
        Err(Refusal::TooShort { .. })
    ));
    let long = vec![0.0f32; (MAX_DURATION_S * 22_050.0) as usize + 1];
    assert!(matches!(
        analyse(&[&long], 22_050.0),
        Err(Refusal::TooLong { .. })
    ));
    let shorter = vec![0.0f32; 47_999];
    assert_eq!(
        analyse(&[&second, &shorter], 48_000.0).err(),
        Some(Refusal::ChannelLengthsDiffer {
            first: 48_000,
            other: 47_999
        })
    );
}

#[test]
fn the_first_non_finite_sample_in_frame_order_is_named() {
    let planted = Plan::room(48_000.0, Layout::Decorrelated).build();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut left = planted.channels[0].clone();
        let mut right = planted.channels[1].clone();
        // The right channel's comes first in time, so it is the one named, whatever the channel order.
        left[20_000] = bad;
        right[10_000] = bad;
        assert_eq!(
            analyse(&[&left, &right], 48_000.0).err(),
            Some(Refusal::NonFiniteSample {
                channel: 1,
                frame: 10_000
            }),
            "{bad}"
        );
        // At the same frame, the lower channel is named.
        left[10_000] = bad;
        assert_eq!(
            analyse(&[&left, &right], 48_000.0).err(),
            Some(Refusal::NonFiniteSample {
                channel: 0,
                frame: 10_000
            })
        );
    }
}

#[test]
fn digital_silence_has_no_onset() {
    for channels in [1, 2] {
        let silent = vec![0.0f32; 48_000];
        let slices: Vec<&[f32]> = (0..channels).map(|_| silent.as_slice()).collect();
        assert_eq!(analyse(&slices, 48_000.0).err(), Some(Refusal::Silence));
    }
}

#[test]
fn every_refusal_says_what_failed() {
    let refusals = [
        Refusal::ChannelCount { channels: 3 },
        Refusal::SampleRate {
            sample_rate: 8_000.0,
        },
        Refusal::TooShort { seconds: 0.1 },
        Refusal::TooLong { seconds: 40.0 },
        Refusal::ChannelLengthsDiffer { first: 1, other: 2 },
        Refusal::NonFiniteSample {
            channel: 1,
            frame: 7,
        },
        Refusal::Silence,
        Refusal::NoIdentifiableOnset {
            prominence_db: 12.0,
        },
        Refusal::NoDecayAboveNoiseFloor {
            band_hz: 500.0,
            failure: DecayFailure::TooLittleDecay,
        },
    ];
    for refusal in refusals {
        assert!(!refusal.to_string().is_empty());
    }
}

const RATE: f64 = 48_000.0;

fn uniform(rng: &mut XorShift, n: usize, amplitude: f64) -> Vec<f64> {
    (0..n).map(|_| amplitude * rng.bipolar()).collect()
}

fn to_f32(x: &[f64]) -> Vec<f32> {
    x.iter().map(|&v| v as f32).collect()
}

#[test]
fn steady_signals_have_no_decay_and_are_refused() {
    // A non-response with no decay at all. Measured: the sine is refused at 500 Hz, where its leakage
    // is 26 dB down and so required; the noise at 125 Hz.
    let sine = mxm_measure::stimulus::sine(120_000, 1000.0, RATE, 0.5);
    assert!(matches!(
        analyse(&[&sine], RATE as f32),
        Err(Refusal::NoDecayAboveNoiseFloor { .. })
    ));
    let noise = mxm_measure::stimulus::noise(120_000, 7, 0.5);
    assert!(matches!(
        analyse(&[&noise], RATE as f32),
        Err(Refusal::NoDecayAboveNoiseFloor { .. })
    ));
}

#[test]
fn shaped_non_responses_are_refused() {
    // Each over a −80 dB floor. Measured over three realisations each: every one refused for want of
    // a decay in a required band.
    let n = 120_000;
    let mut rng = XorShift::new(1);

    // Ten 80 ms noise bursts at random times and levels.
    let mut bursts = uniform(&mut rng, n, 1e-4);
    for _ in 0..10 {
        let start = ((rng.bipolar() * 0.5 + 0.5) * (n - 4_000) as f64) as usize;
        let level = 10f64.powf(-(rng.bipolar() * 0.5 + 0.5) * 15.0 / 20.0);
        for k in 0..3_840 {
            bursts[start + k] += level * rng.bipolar();
        }
    }
    // A click, then noise whose amplitude falls linearly to nothing over 1.2 s.
    let mut fade = uniform(&mut rng, n, 1e-4);
    fade[2_400] += 1.0;
    for k in 0..57_600 {
        fade[2_401 + k] += 0.3 * (1.0 - k as f64 / 57_600.0) * rng.bipolar();
    }
    // A click, then 0.4 s of steady noise that stops dead.
    let mut gated = uniform(&mut rng, n, 1e-4);
    gated[2_400] += 1.0;
    for k in 0..19_200 {
        gated[2_401 + k] += 0.3 * rng.bipolar();
    }
    // A reversed decay: noise swelling over 0.5 s to a stop.
    let mut reverse = uniform(&mut rng, n, 1e-4);
    for k in 0..24_000 {
        reverse[2_400 + k] +=
            0.3 * 10f64.powf(-3.0 * (24_000 - k) as f64 / RATE / 0.5) * rng.bipolar();
    }

    for (name, signal) in [
        ("bursts", bursts),
        ("linear fade", fade),
        ("gated", gated),
        ("reverse", reverse),
    ] {
        let samples = to_f32(&signal);
        let result = analyse(&[&samples], RATE as f32);
        assert!(
            matches!(result, Err(Refusal::NoDecayAboveNoiseFloor { .. })),
            "{name}: {result:?}"
        );
    }
}

#[test]
fn a_response_too_close_to_its_noise_floor_scores_below_the_floor() {
    // The planted room with its floor raised to −55 dB: the bands keep a decay, but the 4 kHz band's
    // margin falls under the 25 dB below which the sweep found no T20. Measured: refused with a
    // confidence of 0, naming that band's margin.
    let mut plan = Plan::room(RATE, Layout::Mono);
    plan.noise_db = -55.0;
    let refusal = analyse(&plan.build().slices(), RATE as f32).expect_err("refused");
    match refusal {
        Refusal::LowConfidence { confidence, check } => {
            assert!(f64::from(confidence) < CONFIDENCE_FLOOR);
            assert!(matches!(check, CheckKind::NoiseMargin { .. }), "{check:?}");
        }
        other => panic!("refused for the wrong reason: {other:?}"),
    }

    // At −70 dB every band keeps at least 40 dB, so it is analysed with a confidence under one that
    // says why. Measured: margins of 40.5 to 44.0 dB, and a confidence of 0.77 set by the 8 kHz band.
    plan.noise_db = -70.0;
    let a = analyse(&plan.build().slices(), RATE as f32).expect("analysed");
    assert!(f64::from(a.validity.confidence) >= CONFIDENCE_FLOOR && a.validity.confidence < 1.0);
    assert!(matches!(
        a.validity.weakest,
        Some(CheckKind::NoiseMargin { .. })
    ));
}

#[test]
fn an_onset_buried_in_what_precedes_it_is_not_identifiable() {
    // The planted room with noise at ±0.05 over its 50 ms lead: each sample stays under the 20 dB
    // onset threshold, but their mean square sits close under the direct sound's. Measured: the
    // direct sound stands 14.3 dB over what precedes it.
    let planted = Plan::room(RATE, Layout::Mono).build();
    let mut samples = planted.channels[0].clone();
    let mut rng = XorShift::new(5);
    for s in &mut samples[..planted.direct_index - 100] {
        *s += (0.05 * rng.bipolar()) as f32;
    }
    match analyse(&[&samples], RATE as f32) {
        Err(Refusal::NoIdentifiableOnset { prominence_db }) => {
            assert!(
                f64::from(prominence_db) < ONSET_BELOW_PEAK_DB,
                "{prominence_db}"
            );
        }
        other => panic!("{other:?}"),
    }
}
