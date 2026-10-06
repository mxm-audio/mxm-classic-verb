//! The fit's contract (plan §2, §4.4, §4.5): what a fit writes and never writes, clamping reported,
//! refusals, determinism, and that every number it returns is finite. Nothing here reads a file.

mod known;
mod support;

use std::f64::consts::PI;

use known::{Known, canonical, render};
use mxm_classic_verb_dsp::space::HIGH_CUT_OPEN_HZ;
use mxm_classic_verb_dsp::{Controls, DecayShape, Space, tone_magnitude};
use mxm_classic_verb_fit::{
    ControlRanges, FitOptions, FitRefusal, FitValue, FittedControls, Refusal, TailTexture,
    TextureErrors, TextureSegments, WidthNotMeasured, WidthReport, analyse, analyse_with_segments,
    fit, fit_with,
};
use mxm_measure::spectrum::{fft, ifft};
use support::{Layout, Plan, assert_finite};

/// A short, ordinary room: cheap enough to fit several times in a debug build.
fn small_room(mono: bool) -> Known {
    Known {
        space: canonical(Space::ROOM),
        decay_s: 0.8,
        size_s: 0.015,
        diffusion: 0.7,
        pre_delay_s: 0.006,
        rate: 44_100.0,
        mono,
        seconds: 2.0,
    }
}

fn slices(channels: &[Vec<f32>]) -> Vec<&[f32]> {
    channels.iter().map(Vec::as_slice).collect()
}

#[test]
fn applying_a_fit_writes_its_contract_and_leaves_mix_ducking_and_rate_alone() {
    let fitted = FittedControls {
        decay_s: 2.3,
        size_s: 0.047,
        diffusion: 0.61,
        pre_delay_s: 0.019,
    };
    let current = Controls {
        mix: 0.42,
        decay_s: 9.0,
        bass_mult: 2.5,
        treble_mult: 0.3,
        size_s: 0.2,
        diffusion: 0.1,
        pre_delay_s: 0.3,
        early_late_db: -7.0,
        shape: DecayShape::Reverse,
        mod_depth_s: 0.002,
        mod_rate_hz: 3.3,
        width: -0.5,
        tone_low_db: 6.0,
        tone_high_db: -9.0,
        duck: 0.77,
    };
    let applied = fitted.apply(&current);
    // Written: the four absolute controls.
    assert_eq!(
        (
            applied.decay_s,
            applied.size_s,
            applied.diffusion,
            applied.pre_delay_s
        ),
        (2.3, 0.047, 0.61, 0.019)
    );
    // Returned to neutral: every relative control, modulation depth, and the decay shape.
    assert_eq!((applied.bass_mult, applied.treble_mult), (1.0, 1.0));
    assert_eq!(applied.early_late_db, 0.0);
    assert_eq!(applied.width, 1.0);
    assert_eq!((applied.tone_low_db, applied.tone_high_db), (0.0, 0.0));
    assert_eq!(applied.mod_depth_s, 0.0);
    assert_eq!(applied.shape, DecayShape::Natural);
    // Never written: Mix, Ducking and modulation rate, to the bit.
    assert_eq!(applied.mix.to_bits(), current.mix.to_bits());
    assert_eq!(applied.duck.to_bits(), current.duck.to_bits());
    assert_eq!(applied.mod_rate_hz.to_bits(), current.mod_rate_hz.to_bits());
}

#[test]
fn a_fit_is_deterministic_and_every_number_in_it_is_finite() {
    let channels = render(&small_room(false));
    let first = fit(&slices(&channels), 44_100.0).expect("fitted");
    let copy = channels.clone();
    let second = fit(&slices(&copy), 44_100.0).expect("fitted");
    // `Debug` prints every float as the shortest text that reads back to the same bits.
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    assert_finite(&first.report.target);
    assert_finite(&first.report.verification);
    let text = format!("{first:?}");
    assert!(!text.contains("NaN") && !text.contains("inf"), "{text}");
}

#[test]
fn the_verification_reads_the_tail_texture_on_the_responses_segments_and_reports_every_error() {
    let channels = render(&small_room(false));
    let result = fit(&slices(&channels), 44_100.0).expect("fitted");
    let report = &result.report;
    let (target, verification) = (&report.target, &report.verification);
    assert!(target.texture_a.is_some(), "{:?}", target.texture_a);

    // The render is read where the response was, so it cannot move its own ruler.
    assert_eq!(verification.texture_segments(), target.texture_segments());

    // Every error is render minus response, and present wherever both sides read the measure.
    let check = |errors: &TextureErrors, t: &Option<TailTexture>, v: &Option<TailTexture>| {
        let read =
            |x: &Option<TailTexture>, f: fn(&TailTexture) -> Option<f32>| x.as_ref().and_then(f);
        let difference = |f: fn(&TailTexture) -> Option<f32>| match (read(t, f), read(v, f)) {
            (Some(t), Some(v)) => Some((f64::from(v) - f64::from(t)) as f32),
            _ => None,
        };
        assert_eq!(errors.peakiness, difference(|x| x.peakiness));
        assert_eq!(errors.kurtosis, difference(|x| x.kurtosis));
        assert_eq!(errors.echo_density, difference(|x| x.echo_density));
        assert_eq!(errors.periodicity, difference(|x| x.periodicity));
    };
    check(
        &report.errors.texture_a,
        &target.texture_a,
        &verification.texture_a,
    );
    check(
        &report.errors.texture_b,
        &target.texture_b,
        &verification.texture_b,
    );
    let a = report.errors.texture_a;
    assert!(
        a.peakiness.is_some()
            && a.kurtosis.is_some()
            && a.echo_density.is_some()
            && a.periodicity.is_some(),
        "{a:?}"
    );

    // The texture enters no confidence: the response read with no texture at all has the same
    // validity.
    let untextured =
        analyse_with_segments(&slices(&channels), 44_100.0, &TextureSegments::default())
            .expect("analysed");
    assert_eq!(untextured.texture_a, None);
    assert_eq!(untextured.validity, target.validity);
    assert_eq!(report.errors.target_confidence, target.validity.confidence);
}

#[test]
fn a_value_outside_its_range_is_clamped_and_the_report_says_how_far() {
    // The room decays in 0.8 s; a Decay range that stops at 0.5 s must clamp it. Its 6 ms gap becomes
    // a pre-delay control of 6 ms or less (less when its first energy is read as the late field), so
    // a pre-delay range that starts at 30 ms must clamp it from below.
    let channels = render(&small_room(true));
    let options = FitOptions {
        ranges: ControlRanges {
            decay_s: 0.1..=0.5,
            pre_delay_s: 0.03..=0.05,
            ..ControlRanges::default()
        },
    };
    let result = fit_with(&slices(&channels), 44_100.0, &options).expect("fitted");
    assert_eq!(result.controls.decay_s, 0.5);
    assert_eq!(result.controls.pre_delay_s, 0.03);
    let clamp = |value| {
        result
            .report
            .clamps
            .iter()
            .find(|c| c.value == value)
            .unwrap_or_else(|| panic!("{value:?} is reported: {:?}", result.report.clamps))
    };
    let decay = clamp(FitValue::Decay);
    assert_eq!(decay.applied, 0.5);
    assert!(decay.fitted > 0.7, "{decay:?}");
    let pre = clamp(FitValue::PreDelay);
    assert_eq!(pre.applied, 0.03);
    assert!(pre.fitted < 0.01, "{pre:?}");
    // What was not clamped is not reported.
    assert!(
        result
            .report
            .clamps
            .iter()
            .all(|c| matches!(c.value, FitValue::Decay | FitValue::PreDelay)),
        "{:?}",
        result.report.clamps
    );
    // A mono response leaves the width at the space's default, and says it was not measured.
    assert_eq!(result.space.width, 1.0);
    assert_eq!(
        result.report.width,
        WidthReport::NotMeasured(WidthNotMeasured::Mono)
    );
}

#[test]
fn the_analysers_refusals_are_the_fits() {
    let silence = vec![0.0f32; 44_100];
    assert_eq!(
        fit(&[&silence], 44_100.0).err(),
        Some(FitRefusal::Analysis(Refusal::Silence))
    );
    let short = vec![0.0f32; 1_000];
    assert!(matches!(
        fit(&[&short], 44_100.0),
        Err(FitRefusal::Analysis(Refusal::TooShort { .. }))
    ));
    let noise = mxm_measure::stimulus::noise(88_200, 3, 0.5);
    assert!(matches!(
        fit(&[&noise], 44_100.0),
        Err(FitRefusal::Analysis(Refusal::NoDecayAboveNoiseFloor { .. }))
    ));
}

/// `x` with everything between `edges[1]` and `edges[2]` removed, raised-cosine transitions from
/// `edges[0]` and up to `edges[3]` (so its ringing stays short), scaled to a peak of one.
fn band_stop(x: &[f32], rate: f64, edges: [f64; 4]) -> Vec<f32> {
    let n = x.len().next_power_of_two();
    let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
    for (r, &v) in re.iter_mut().zip(x) {
        *r = f64::from(v);
    }
    fft(&mut re, &mut im).expect("a power-of-two length transforms");
    let [f1, f2, f3, f4] = edges;
    let half_cosine =
        |from: f64, to: f64, f: f64| 0.5 + 0.5 * (PI * (f - from) / (to - from)).cos();
    for k in 0..n {
        let f = k.min(n - k) as f64 * rate / n as f64;
        let gain = if f <= f1 || f >= f4 {
            1.0
        } else if f < f2 {
            half_cosine(f1, f2, f)
        } else if f <= f3 {
            0.0
        } else {
            half_cosine(f4, f3, f)
        };
        re[k] *= gain;
        im[k] *= gain;
    }
    ifft(&mut re, &mut im).expect("a power-of-two length transforms");
    let peak = re.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    re[..x.len()].iter().map(|v| (v / peak) as f32).collect()
}

#[test]
fn a_response_with_no_mid_band_decay_is_refused() {
    // Planting quiet mid-band tails is not enough: the direct sound is broadband, so every band's
    // loudest block is within 30 dB of the loudest band's and every band is required, and a required
    // band either reads a decay (from its neighbours' leakage and the reflections' ringing, measured)
    // or is refused by the analyser first. So the planted room loses everything from 300 Hz to 3.3 kHz,
    // direct sound included. Its 500 Hz, 1 kHz and 2 kHz bands then fall more than 30 dB under the
    // loudest and are not required (measured: all three still read a decay from leakage, which a fit
    // must not take as Decay), and Decay has nothing to be read from.
    let planted = Plan::room(44_100.0, Layout::Mono).build();
    let stopped = band_stop(
        &planted.channels[0],
        44_100.0,
        [180.0, 300.0, 3300.0, 5000.0],
    );
    let analysis = analyse(&[&stopped], 44_100.0).expect("the analyser measures it");
    assert!(
        analysis
            .bands
            .iter()
            .filter(|b| (500.0..=2000.0).contains(&b.centre_hz))
            .all(|b| !b.required),
        "{analysis:?}"
    );
    assert_eq!(
        fit(&[&stopped], 44_100.0).err(),
        Some(FitRefusal::NoMidBandDecay)
    );
}

/// The render is analysed like the response: every band the response had a T30 in is measured again,
/// and nothing would be refused but a bent decay — a straightness note, which a band that is not one
/// slope earns and a band that is gone cannot.
fn assert_every_band_is_measured_again(result: &mxm_classic_verb_fit::Fit, name: &str) {
    assert!(
        matches!(
            result.report.verification_refusal,
            None | Some(Refusal::LowConfidence {
                check: mxm_classic_verb_fit::CheckKind::Straightness { .. },
                ..
            })
        ),
        "{name}: {:?}; Size {}; response bands {:?}; render bands {:?}",
        result.report.verification_refusal,
        result.controls.size_s,
        result.report.target.bands,
        result.report.verification.bands
    );
    for (target, error) in result
        .report
        .target
        .bands
        .iter()
        .zip(&result.report.errors.bands)
    {
        if target.decay().and_then(|d| d.t30_s).is_some() {
            assert!(
                error.t30_percent.is_some(),
                "{name}: {} Hz lost its T30",
                target.centre_hz
            );
        }
    }
}

#[test]
fn a_shelf_at_its_bound_does_not_kill_a_band_in_the_render() {
    // Planted rooms tilted far past what a first-order shelf can follow — the shape of target behind
    // the pool's render-stage refusals, where a shelf past its reach left the render with no decay
    // above its floor in a band.
    let mut low_cut = Plan::room(48_000.0, Layout::Decorrelated);
    low_cut.noise_db = -95.0;
    let mut dark = low_cut.clone();

    // 125 Hz 30 dB under 1 kHz: the low shelf is still driven to the space's bound (measured: it
    // asked for −48 dB).
    low_cut.band_db = [-30.0, -18.0, -6.0, 0.0, 1.0, 2.0, 3.0];
    let result = fit(&low_cut.build().slices(), 48_000.0).expect("fitted");
    let low = result
        .report
        .clamps
        .iter()
        .find(|c| c.value == FitValue::ToneLow)
        .expect("the low shelf is clamped");
    assert_eq!(low.applied, -24.0);
    assert_every_band_is_measured_again(&result, "low cut");

    // 8 kHz 30 dB under 1 kHz. Before the space had a high cut this drove the high shelf to its bound
    // and left an 8 kHz tone error of +20.4 dB. The cut now carries the fall, so no tone value is
    // clamped (measured: corner 1.67 kHz, 8 kHz tone error −1.5 dB), and
    // a render that dark must keep its bands just the same. With the output taps its 8 kHz decay reads
    // straight; before them, at the density floor, it bent (straightness at 8 kHz, confidence 0.23), which
    // is why a straightness note stays allowed.
    dark.band_db = [3.0, 2.0, 1.0, 0.0, -6.0, -18.0, -30.0];
    let result = fit(&dark.build().slices(), 48_000.0).expect("fitted");
    assert!(
        result.report.clamps.is_empty(),
        "{:?}",
        result.report.clamps
    );
    assert!(
        result.space.high_cut_hz < 4_000.0,
        "the cut carries the fall: {}",
        result.space.high_cut_hz
    );
    assert_every_band_is_measured_again(&result, "dark");
}

/// A white late field's band levels against 1 kHz, the planted bands being brick-wall octaves.
fn white(extra: impl Fn(f64) -> f64) -> [f64; 7] {
    support::BANDS_HZ.map(|hz| 3.0 * (hz / 1000.0).log2() + extra(hz))
}

fn tone_error(result: &mxm_classic_verb_fit::Fit, hz: f32) -> f32 {
    result
        .report
        .errors
        .bands
        .iter()
        .find(|b| b.centre_hz == hz)
        .and_then(|b| b.tone_db)
        .unwrap_or_else(|| panic!("{hz} Hz has a tone error"))
}

#[test]
fn a_steep_rolloff_is_fitted_with_a_high_cut_instead_of_a_clamped_shelf() {
    // A white late field falling 12 dB an octave above 3 kHz: 8 kHz 17 dB under white, the fall the
    // pack's clamped fits showed and no first-order shelf follows; before the space had a high cut, the
    // shelf was clamped.
    //
    // Measured over 24 realisations (seeds 1–8 stereo and 1–4 mono, at 44.1 and 48 kHz, sixteen lines
    // at the density floor, with output taps): a cut every time, at 2.16–3.76 kHz, no clamp, no
    // verification refusal, and a worst tone error of 0.87, 1.38 and 2.67 dB at 2, 4 and 8 kHz. This
    // realisation stays under the 2 dB asserted; the population's 8 kHz worst does not.
    let mut plan = Plan::room(48_000.0, Layout::Decorrelated);
    plan.band_db = white(|hz| {
        if hz > 3000.0 {
            -12.0 * (hz / 3000.0).log2()
        } else {
            0.0
        }
    });
    let result = fit(&plan.build().slices(), 48_000.0).expect("fitted");
    assert!(
        result.report.clamps.is_empty(),
        "no shelf is clamped: {:?}",
        result.report.clamps
    );
    let cut = result.space.high_cut_hz;
    assert!((1_500.0..5_000.0).contains(&cut), "the cut: {cut} Hz");
    assert_eq!(result.report.tone.high_cut_hz, cut);
    let (carried, open) = (
        result.report.tone.residual_db.expect("a tone residual"),
        result
            .report
            .tone
            .open_residual_db
            .expect("an open residual"),
    );
    assert!(
        f64::from(open - carried) > mxm_classic_verb_fit::HIGH_CUT_MIN_IMPROVEMENT_DB,
        "the cut earns its place: {carried} dB against {open} dB open"
    );
    assert_eq!(result.report.verification_refusal, None);
    for hz in [2_000.0, 4_000.0, 8_000.0] {
        let error = tone_error(&result, hz);
        assert!(error.abs() < 2.0, "{hz} Hz: tone error {error} dB");
    }
}

#[test]
fn a_flat_or_bright_response_keeps_its_high_cut_open() {
    // A white late field, and one brightening 2 dB an octave above 2 kHz. Measured over 48
    // realisations of the two (the same seeds and rates as the steep rolloff): the cut stayed open in
    // 47, and the one cut taken (10.6 kHz, a mono white room) moved the tone below 8 kHz by 0.18 dB at
    // most. With the cut open the fit is the one made before the cut existed; its worst tone errors
    // over those realisations were 2.59 dB (white) and 2.14 dB (bright), at 2, 4 or 8 kHz.
    let flat = ("white", 48_000.0, white(|_| 0.0));
    let bright = (
        "bright",
        44_100.0,
        white(|hz| {
            if hz > 2000.0 {
                2.0 * (hz / 2000.0).log2()
            } else {
                0.0
            }
        }),
    );
    for (name, rate, band_db) in [flat, bright] {
        let mut plan = Plan::room(rate, Layout::Decorrelated);
        plan.band_db = band_db;
        let result = fit(&plan.build().slices(), rate as f32).expect("fitted");
        let cut = result.space.high_cut_hz;
        // The cut's own effect on the tone below the 8 kHz band, normalised as the engine normalises.
        let effect = [125.0, 250.0, 500.0, 2_000.0, 4_000.0, 5_657.0]
            .map(|hz| 20.0 * tone_magnitude(0.0, 0.0, cut, rate as f32, hz).log10())
            .iter()
            .fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            effect < 0.5,
            "{name}: a {cut} Hz cut moves the tone by {effect} dB"
        );
        if cut == HIGH_CUT_OPEN_HZ {
            assert_eq!(
                result.report.tone.residual_db, result.report.tone.open_residual_db,
                "{name}: an open cut leaves the shelves' own solve"
            );
        }
        assert!(
            result.report.clamps.is_empty(),
            "{name}: {:?}",
            result.report.clamps
        );
        assert_eq!(result.report.verification_refusal, None, "{name}");
        for hz in [2_000.0, 4_000.0, 8_000.0] {
            let error = tone_error(&result, hz);
            assert!(
                error.abs() < 3.0,
                "{name} at {hz} Hz: tone error {error} dB"
            );
        }
    }
}

#[test]
fn the_planted_rooms_fit_without_refusal() {
    // The analyser's own planted responses are not renders of this engine, so nothing is asserted of
    // their errors here: only that each is fitted, verified and finite.
    for layout in [Layout::Mono, Layout::Decorrelated] {
        let planted = Plan::room(44_100.0, layout).build();
        let result = fit(&planted.slices(), 44_100.0)
            .unwrap_or_else(|r| panic!("{layout:?} is fitted: {r}"));
        assert_finite(&result.report.verification);
        assert!(result.report.errors.verification_confidence.is_finite());
    }
}
