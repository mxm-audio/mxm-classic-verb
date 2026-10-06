//! Synthetic responses with planted descriptors (`plans/plan-mxm-classic-verb.md` §4.6). The
//! analyser recovers what was put there, within tolerances measured first and asserted with headroom.
//!
//! **Where the tolerances come from.** Each is the worst error over many realisations of the same
//! plan (12 to 32, recorded in the crate's `NOTES.md`), not over the seeds asserted here, so an
//! assertion is not a lucky seed. Band-limited noise with few degrees of freedom is what limits the
//! low bands: an octave at 125 Hz is 88 Hz wide, and one realisation of its decay fluctuates by
//! several percent. That is the estimator's spread on one response, which is what a real measurement
//! has too. Nothing here reads a file.

mod support;

use mxm_classic_verb_fit::{
    Analysis, BandDecay, CheckKind, DecayMethod, ONE_SLOPE_MAX_DEVIATION_DB, Width,
    WidthNotMeasured, analyse, analyse_with_method,
};
use support::{Layout, Plan, ROOM_REFLECTIONS, SecondSlope, assert_finite, hann_pulse};

const RATE: f64 = 48_000.0;

/// T30 error allowed, in percent: the worst over 12 mono realisations was 9.7 % and 8.5 % at 125 and
/// 250 Hz and 6.3 % above.
fn t30_tolerance(band: usize) -> f64 {
    if band < 2 { 12.0 } else { 8.0 }
}

/// T20 error allowed, in percent: the worst over 12 mono realisations was 16.4 % at 125 Hz, then
/// 7.9 % and below.
fn t20_tolerance(band: usize) -> f64 {
    if band < 1 { 20.0 } else { 10.0 }
}

fn error_percent(measured: Option<f32>, truth: f64) -> f64 {
    let measured = measured.expect("a planted room has every decay time");
    100.0 * (f64::from(measured) / truth - 1.0)
}

fn decay(analysis: &Analysis, band: usize) -> &BandDecay {
    analysis.bands[band]
        .decay()
        .unwrap_or_else(|| panic!("band {band} holds a decay: {:?}", analysis.bands[band]))
}

/// The tone curve the plan puts there, read at the direct sound as the analyser reads it: each band's
/// late decay extrapolated back over the tail's start at its own rate.
fn planted_tone(plan: &Plan, band: usize) -> f64 {
    let back = |b: usize| plan.band_db[b] + 60.0 * plan.tail_start_s / plan.t60_s[b];
    back(band) - back(3)
}

#[test]
fn a_planted_room_is_recovered() {
    for seed in [1, 2, 3] {
        let mut plan = Plan::room(RATE, Layout::Mono);
        plan.seed = seed;
        let planted = plan.build();
        let a = analyse(&planted.slices(), RATE as f32).expect("a planted room is analysed");
        assert_finite(&a);

        // The direct sound is found to the sample (measured: exact), and the onset lies on its
        // pulse's rising half (measured: 5 samples, 0.10 ms, before the peak).
        let direct = (f64::from(a.onset.direct_s) * RATE).round() as usize;
        assert_eq!(direct, planted.direct_index, "seed {seed}");
        let rise = (hann_pulse(RATE).len() / 2) as f64 / RATE;
        let lead = f64::from(a.onset.direct_s - a.onset.onset_s);
        assert!(
            (0.0..=rise).contains(&lead),
            "seed {seed}: onset {lead} s before the peak"
        );

        // DRR against the planted components' energies. Measured: within 0.002 dB; the only
        // difference is the noise that falls inside the ±2.5 ms window.
        let drr = f64::from(a.onset.drr_db.expect("a room has reverberant energy"));
        assert!(
            (drr - planted.drr_db).abs() < 0.05,
            "seed {seed}: DRR {drr} vs {}",
            planted.drr_db
        );

        // Pre-delay and the five planted reflections, to the sample and within 0.1 dB. Measured:
        // every delay exact and every level within 0.005 dB. Anything else reported lies in the
        // tail, which starts at 45 ms and whose largest peaks are reflections by the rule.
        let sample = |seconds: f64| (seconds * RATE).round() as i64;
        let pre_delay = f64::from(a.pre_delay_s.expect("a room has a pre-delay"));
        assert!(
            (sample(pre_delay) - sample(ROOM_REFLECTIONS[0].delay_s)).abs() <= 1,
            "seed {seed}"
        );
        let reflections = &a.early.reflections;
        for (planted, found) in ROOM_REFLECTIONS.iter().zip(reflections) {
            assert!(
                (sample(f64::from(found.delay_s)) - sample(planted.delay_s)).abs() <= 1,
                "seed {seed}: reflection at {} s, planted {} s",
                found.delay_s,
                planted.delay_s
            );
            let want = 20.0 * planted.gain[0].log10();
            assert!(
                (f64::from(found.level_db) - want).abs() < 0.1,
                "seed {seed}: {found:?}"
            );
        }
        for extra in reflections.iter().skip(ROOM_REFLECTIONS.len()) {
            assert!(
                f64::from(extra.delay_s) >= plan.tail_start_s - 0.0005,
                "seed {seed}: {extra:?}"
            );
        }

        for band in 0..7 {
            let d = decay(&a, band);
            let t60 = plan.t60_s[band];
            let t30 = error_percent(d.t30_s, t60);
            assert!(
                t30.abs() < t30_tolerance(band),
                "seed {seed} band {band}: T30 {t30:+.2} %"
            );
            let t20 = error_percent(d.t20_s, t60);
            assert!(
                t20.abs() < t20_tolerance(band),
                "seed {seed} band {band}: T20 {t20:+.2} %"
            );

            // The tone curve. Worst over 24 realisations: 3.2 dB at 125 Hz, 2.0 dB at 250 Hz,
            // 1.7 dB above; the mean error was under 0.4 dB in every band.
            let tone = f64::from(d.level_re_1k_db.expect("every band has a level"));
            let tone_tolerance = if band == 0 { 4.0 } else { 2.5 };
            let tone_error = tone - planted_tone(&plan, band);
            assert!(
                tone_error.abs() < tone_tolerance,
                "seed {seed} band {band}: tone {tone_error:+.2} dB"
            );

            // The noise floor, per band. Worst over 12 realisations: 1.9 dB, the filter's noise
            // bandwidth being a little under an octave.
            let noise = f64::from(d.noise_floor_db) - plan.noise_db;
            assert!(
                noise.abs() < 2.5,
                "seed {seed} band {band}: noise floor {noise:+.2} dB"
            );

            // Where the decay meets the floor. Worst over 24 realisations: 10.7 % of the T60.
            let truth = plan.tail_start_s
                + t60 / 60.0 * (plan.tail_db + plan.band_db[band] - plan.noise_db);
            let crossing = 100.0 * (f64::from(d.intersection_s) - truth) / t60;
            assert!(
                crossing.abs() < 15.0,
                "seed {seed} band {band}: crossing {crossing:+.1} % of T60"
            );
            assert!(d.noise_floor_reached);

            let straightness = d.straightness.expect("a T30 range gives a straightness");
            assert!(
                straightness.single_slope,
                "seed {seed} band {band}: {straightness:?}"
            );
        }

        // The mixing time: the tail starts at 45 ms, and a 20 ms window lies wholly inside it from
        // 55 ms. Measured on these seeds: +1, −2 and +1 ms. Over 32 realisations the median error was
        // 4 ms and the 90th percentile 12 ms, with one 83 ms outlier (NOTES.md).
        let mixing = f64::from(a.echo_density.mixing_time_s.expect("a room mixes"));
        let planted_mixing = plan.tail_start_s + 0.010;
        assert!(
            (mixing - planted_mixing).abs() < 0.015,
            "seed {seed}: mixing {mixing} s"
        );

        assert_eq!(a.width, Width::NotMeasured(WidthNotMeasured::Mono));
        assert!(a.width.measured().is_none());
        // Measured: 1.0 on every seed asserted here.
        assert!(
            a.validity.confidence >= 0.9,
            "seed {seed}: {:?}",
            a.validity
        );
    }
}

#[test]
fn a_stereo_room_reads_each_channel_and_its_width() {
    // Decorrelated: each channel's reflection levels are its own, and the tails are independent.
    let plan = Plan::room(RATE, Layout::Decorrelated);
    let a = analyse(&plan.build().slices(), RATE as f32).expect("a stereo room is analysed");
    assert_finite(&a);
    for (planted, found) in ROOM_REFLECTIONS.iter().zip(&a.early.reflections) {
        for channel in 0..2 {
            let want = 20.0 * planted.gain[channel].log10();
            let got = f64::from(
                found.channel_level_db[channel].expect("both channels hold the direct sound"),
            );
            assert!(
                (got - want).abs() < 0.1,
                "channel {channel}: {got} dB, planted {want} dB"
            );
        }
    }
    for band in 0..7 {
        let d = decay(&a, band);
        let t30 = error_percent(d.t30_s, plan.t60_s[band]);
        assert!(
            t30.abs() < t30_tolerance(band),
            "band {band}: T30 {t30:+.2} %"
        );
        // Two channels' energies sum, so the floor reads 3 dB over each channel's.
        let noise = f64::from(d.noise_floor_db) - (plan.noise_db + 10.0 * 2f64.log10());
        assert!(noise.abs() < 2.5, "band {band}: noise floor {noise:+.2} dB");
    }
    // Independent tails: the largest |IACF| over ±1 ms was at most 0.153 over 24 realisations.
    let coherence = *a.width.measured().expect("a stereo late field is measured");
    assert!(coherence.iacc < 0.25, "{coherence:?}");

    // Identical channels are fully coherent, at zero lag and in phase.
    let identical = analyse(
        &Plan::room(RATE, Layout::Identical).build().slices(),
        RATE as f32,
    )
    .expect("an identical stereo room is analysed");
    let coherence = *identical.width.measured().expect("measured");
    assert!(
        coherence.iacc > 0.999 && coherence.peak_iacf > 0.999,
        "{coherence:?}"
    );
    assert_eq!(coherence.peak_lag_s, 0.0);
}

#[test]
fn the_noise_floor_matters() {
    // A floor 41–47 dB under the tail and two seconds of it after the slowest decay: the conditions
    // under which Guski and Vorländer found method A reads long and truncation alone (C) needs its
    // correction. The same response, three integrations.
    for seed in [1, 2] {
        let mut plan = Plan::room(RATE, Layout::Mono);
        plan.seed = seed;
        plan.noise_db = -75.0;
        plan.duration_s = 3.0;
        let planted = plan.build();
        let errors = |method: DecayMethod| -> Vec<f64> {
            let a = analyse_with_method(&planted.slices(), RATE as f32, method)
                .unwrap_or_else(|r| panic!("{method:?}: {r}"));
            (0..7)
                .map(|b| error_percent(decay(&a, b).t30_s, plan.t60_s[b]))
                .collect()
        };
        let mean = |e: &[f64]| e.iter().map(|x| x.abs()).sum::<f64>() / e.len() as f64;
        let e = errors(DecayMethod::NoiseCompensated);
        let c = errors(DecayMethod::Truncated);
        let a = errors(DecayMethod::Uncompensated);

        // Measured mean |T30 error| on seeds 1 and 2: method E 3.5 % and 2.6 %, C 5.7 % and 4.8 %,
        // A 630 % and 613 % — with this much noise after the decay, A's curve reaches −35 dB only
        // where the record runs out.
        assert!(mean(&e) < mean(&c) && mean(&c) < mean(&a), "seed {seed}");
        for band in 0..7 {
            // Noise left in the integral can only add energy late, so C reads longer than E in every
            // band. Measured: by 1.3 to 5.2 percentage points.
            assert!(
                c[band] > e[band],
                "seed {seed} band {band}: C {} vs E {}",
                c[band],
                e[band]
            );
            assert!(
                e[band].abs() < t30_tolerance(band),
                "seed {seed} band {band}: E {}",
                e[band]
            );
        }
    }
}

#[test]
fn a_two_slope_decay_is_flagged_and_a_single_slope_is_not() {
    // Two exponentials of very different rates summed in every band: 0.3 s, and 2.5 s starting 20 dB
    // down. Over 8 realisations the smallest departure from one slope was 3.61 dB (125 Hz) against
    // [`ONE_SLOPE_MAX_DEVIATION_DB`]; the single-slope rooms above stay under it (worst 3.05 dB over
    // 24). The lower half of the range always decays slower than the upper (ratio at least 1.11).
    for seed in [1, 2] {
        let mut plan = Plan::room(RATE, Layout::Mono);
        plan.seed = seed;
        plan.t60_s = [0.3; 7];
        plan.second_slope = Some(SecondSlope {
            t60_s: 2.5,
            below_db: 20.0,
        });
        plan.duration_s = 4.0;
        plan.noise_db = -100.0;
        let a = analyse(&plan.build().slices(), RATE as f32)
            .expect("a two-slope decay is reported, not refused");
        assert_finite(&a);
        for band in 0..7 {
            let s = decay(&a, band).straightness.expect("a straightness");
            assert!(!s.single_slope, "seed {seed} band {band}: {s:?}");
            assert!(s.max_deviation_db > ONE_SLOPE_MAX_DEVIATION_DB);
            assert!(
                s.late_to_early.expect("both halves fit") > 1.0,
                "seed {seed} band {band}: {s:?}"
            );
        }
        // It lowers the confidence, and names straightness as why. Measured: 0.673 and 0.667, both set
        // by the 2 kHz band.
        assert!(matches!(
            a.validity.weakest,
            Some(CheckKind::Straightness { .. })
        ));
        assert!(
            (0.4..0.9).contains(&a.validity.confidence),
            "seed {seed}: {:?}",
            a.validity
        );
    }
}

#[test]
fn t30_does_not_depend_on_the_sample_rate() {
    // The generator's frequency bins coincide at 48 and 96 kHz, so those two are one continuous
    // response sampled twice; 44.1 kHz is a second realisation of the same plan.
    let at = |rate: f64| {
        let plan = Plan::room(rate, Layout::Mono);
        let a = analyse(&plan.build().slices(), rate as f32).expect("analysed");
        assert_finite(&a);
        let t30: Vec<f64> = (0..7)
            .map(|b| error_percent(decay(&a, b).t30_s, plan.t60_s[b]))
            .collect();
        (t30, f64::from(a.echo_density.mixing_time_s.expect("mixes")))
    };
    let (t30_44, _) = at(44_100.0);
    let (t30_48, mixing_48) = at(48_000.0);
    let (t30_96, mixing_96) = at(96_000.0);
    for band in 0..7 {
        for (rate, t30) in [(44_100, &t30_44), (48_000, &t30_48), (96_000, &t30_96)] {
            assert!(
                t30[band].abs() < t30_tolerance(band),
                "{rate} Hz band {band}: {:+.2} %",
                t30[band]
            );
        }
        // One response at two rates: measured within 0.26 percentage points over six realisations.
        assert!(
            (t30_48[band] - t30_96[band]).abs() < 0.5,
            "band {band}: {} vs {}",
            t30_48[band],
            t30_96[band]
        );
    }
    // Measured: the same millisecond at both rates on every realisation.
    assert!(
        (mixing_48 - mixing_96).abs() <= 0.002,
        "{mixing_48} vs {mixing_96}"
    );
}

#[test]
fn the_duration_floor_measures_a_short_room() {
    // A 0.2 s decay in every band, 20 ms of lead, in a file exactly as long as the floor. Worst over
    // 8 realisations: 27.6 % at 125 Hz, 13.7 % at 250 Hz, 8.6 % above — no worse than the same plan
    // given a full second.
    for rate in [22_050.0, 48_000.0] {
        let mut plan = Plan::room(rate, Layout::Mono);
        plan.reflections.clear();
        plan.direct_s = 0.02;
        plan.tail_start_s = 0.002;
        plan.t60_s = [0.2; 7];
        plan.duration_s = mxm_classic_verb_fit::MIN_DURATION_S;
        plan.noise_db = -90.0;
        let a = analyse(&plan.build().slices(), rate as f32).expect("a short room is analysed");
        assert_finite(&a);
        for (band, entry) in a.bands.iter().enumerate() {
            let Some(d) = entry.decay() else {
                assert!(
                    rate < 44_100.0 && band == 6,
                    "{rate} Hz band {band}: {entry:?}"
                );
                continue;
            };
            let tolerance = [35.0, 20.0, 12.0, 12.0, 12.0, 12.0, 12.0][band];
            let t30 = error_percent(d.t30_s, 0.2);
            assert!(
                t30.abs() < tolerance,
                "{rate} Hz band {band}: T30 {t30:+.2} %"
            );
        }
    }
}

#[test]
fn early_decay_time_is_recovered_where_the_tail_starts_at_the_direct_sound() {
    // EDT reads 0 to −10 dB, so a planted gap before the tail would be part of it; here the tail
    // starts with the direct sound. Worst over 24 realisations: 34 % and 31 % at 125 and 250 Hz,
    // 18 % at 500 Hz, 12 % above — the shortest range, and so the widest spread.
    for seed in [1, 2] {
        let mut plan = Plan::room(RATE, Layout::Mono);
        plan.seed = seed;
        plan.reflections.clear();
        plan.tail_start_s = 0.0005;
        let a = analyse(&plan.build().slices(), RATE as f32).expect("analysed");
        for band in 0..7 {
            let tolerance = if band < 3 { 40.0 } else { 15.0 };
            let edt = error_percent(decay(&a, band).edt_s, plan.t60_s[band]);
            assert!(
                edt.abs() < tolerance,
                "seed {seed} band {band}: EDT {edt:+.2} %"
            );
        }
    }
}

#[test]
fn pre_delay_finds_a_tail_with_no_reflection_ahead_of_it() {
    // Worst over 24 realisations, mono and stereo: 0.52 ms late, the tail's largest sample within
    // half a millisecond of its arrival.
    let mut plan = Plan::room(RATE, Layout::Mono);
    plan.reflections.clear();
    plan.tail_start_s = 0.020;
    let a = analyse(&plan.build().slices(), RATE as f32).expect("analysed");
    let pre_delay = f64::from(a.pre_delay_s.expect("the tail is found"));
    assert!(
        (pre_delay - 0.020).abs() < 0.00075,
        "pre-delay {pre_delay} s"
    );
}
