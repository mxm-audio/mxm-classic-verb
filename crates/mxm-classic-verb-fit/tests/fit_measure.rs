//! The fit's measurements that no default test prints, for the crate's `NOTES.md`, *Measured accuracy —
//! the fit*: the dense onsets, the steep tilts, the high cut on planted rooms, the pack's aggregate tone
//! curves and the fit's cost. Every test here is ignored and asserts nothing. Run them in release, one
//! at a time so the cost is not measured under the others' load:
//!
//! ```bash
//! cargo test -p mxm-classic-verb-fit --release --test fit_measure -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Nothing here reads a file.

mod support;

use std::time::Instant;

use mxm_classic_verb_dsp::space::HIGH_CUT_OPEN_HZ;
use mxm_classic_verb_dsp::tone_magnitude;
use mxm_classic_verb_fit::{BandError, Fit, fit};
use support::{BANDS_HZ, Layout, Plan, PlantedReflection};

/// A white late field's band levels against 1 kHz, the planted bands being brick-wall octaves, with
/// `extra(hz)` dB on top.
fn white(extra: impl Fn(f64) -> f64) -> [f64; 7] {
    BANDS_HZ.map(|hz| 3.0 * (hz / 1000.0).log2() + extra(hz))
}

fn fitted(plan: &Plan) -> Fit {
    fit(&plan.build().slices(), plan.rate as f32)
        .unwrap_or_else(|r| panic!("{plan:?} is fitted: {r}"))
}

fn band_value(result: &Fit, hz: f32, read: fn(&BandError) -> Option<f32>) -> Option<f32> {
    result
        .report
        .errors
        .bands
        .iter()
        .find(|b| b.centre_hz == hz)
        .and_then(read)
}

/// The largest error over the bands, and the band it is in.
fn worst_band(result: &Fit, read: fn(&BandError) -> Option<f32>) -> String {
    result
        .report
        .errors
        .bands
        .iter()
        .filter_map(|b| read(b).map(|v| (b.centre_hz, v)))
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map_or("-".to_string(), |(hz, v)| format!("{v:+.2} at {hz} Hz"))
}

fn clamps(result: &Fit) -> String {
    if result.report.clamps.is_empty() {
        return "none".to_string();
    }
    result
        .report
        .clamps
        .iter()
        .map(|c| format!("{:?} {} -> {}", c.value, c.fitted, c.applied))
        .collect::<Vec<_>>()
        .join("; ")
}

fn optional(value: Option<f32>, scale: f32, decimals: usize) -> String {
    value.map_or("-".to_string(), |v| format!("{:+.*}", decimals, v * scale))
}

/// Everything the tables in `NOTES.md` read from one fit.
fn summary(result: &Fit) -> String {
    let (c, r) = (result.controls, &result.report);
    let e = &r.errors;
    format!(
        "Size {:.1} ms, Diffusion {:.2}, pre-delay {:.2} ms, {:?}; taps {}, beyond reach {}; profile {}, envelope {} dB, mixing time {} ms; worst T30 {} %, worst tone {} dB, early-to-late {} dB; reflections {} of {}; high cut {:.0} Hz (residual {} dB, open {} dB); clamps {}; verification refusal {:?}",
        1000.0 * c.size_s,
        c.diffusion,
        1000.0 * c.pre_delay_s,
        r.early.first_arrival,
        r.early.taps,
        r.early.beyond_reach,
        optional(e.profile_distance, 1.0, 3),
        optional(e.envelope_db, 1.0, 2),
        optional(e.mixing_time_s, 1000.0, 1),
        worst_band(result, |b| b.t30_percent),
        worst_band(result, |b| b.tone_db),
        optional(e.early_to_late_db, 1.0, 2),
        e.reflections_matched,
        e.reflections_compared,
        r.tone.high_cut_hz,
        optional(r.tone.residual_db, 1.0, 2),
        optional(r.tone.open_residual_db, 1.0, 2),
        clamps(result),
        r.verification_refusal,
    )
}

/// Each band's T30 and tone error.
fn bands(result: &Fit) -> String {
    result
        .report
        .errors
        .bands
        .iter()
        .map(|b| {
            format!(
                "{} Hz: T30 {} %, tone {} dB",
                b.centre_hz,
                optional(b.t30_percent, 1.0, 1),
                optional(b.tone_db, 1.0, 2)
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// `Plan::room`, stereo at 48 kHz for 3.5 s, with no planted reflections and its tail starting 0.5 ms
/// after the direct sound, T60 2.2, 2.0, 1.8, 1.6, 1.4, 1.25 and 1.1 s from 125 Hz to 8 kHz; then the
/// same with ten reflections of gain 0.3, evenly from 15 to 45 ms.
#[test]
#[ignore = "a measurement, printed for AGENTS.md"]
fn measure_the_dense_onsets() {
    let mut plain = Plan::room(48_000.0, Layout::Decorrelated);
    plain.duration_s = 3.5;
    plain.reflections.clear();
    plain.tail_start_s = 0.0005;
    plain.t60_s = [2.2, 2.0, 1.8, 1.6, 1.4, 1.25, 1.1];
    let mut reflected = plain.clone();
    reflected.reflections = (0..10)
        .map(|i| PlantedReflection {
            delay_s: 0.015 + 0.030 * f64::from(i) / 9.0,
            gain: [0.3, 0.3],
        })
        .collect();
    for (name, plan) in [
        ("no reflections", plain),
        ("ten reflections at 15-45 ms", reflected),
    ] {
        let result = fitted(&plan);
        println!("{name}: {}", summary(&result));
        println!("  {}", bands(&result));
    }
}

/// `Plan::room`, stereo at 48 kHz, noise −95 dB, band levels against 1 kHz from 125 Hz to 8 kHz as
/// named.
#[test]
#[ignore = "a measurement, printed for AGENTS.md"]
fn measure_the_steep_tilts() {
    let plants: [(&str, [f64; 7]); 3] = [
        ("dark", [3.0, 2.0, 1.0, 0.0, -6.0, -18.0, -30.0]),
        ("bright", [-12.0, -8.0, -4.0, 0.0, 6.0, 12.0, 18.0]),
        ("low cut", [-30.0, -18.0, -6.0, 0.0, 1.0, 2.0, 3.0]),
    ];
    for (name, band_db) in plants {
        let mut plan = Plan::room(48_000.0, Layout::Decorrelated);
        plan.noise_db = -95.0;
        plan.band_db = band_db;
        let result = fitted(&plan);
        println!("{name}: {}", summary(&result));
        println!("  {}", bands(&result));
    }
}

#[derive(Default)]
struct CutPopulation {
    fits: usize,
    taken: usize,
    corner: Option<(f32, f32)>,
    clamped: usize,
    refused: usize,
    /// Worst |tone error| at 2, 4 and 8 kHz.
    tone: [f32; 3],
    /// The largest amount a taken cut moves the tone below the 8 kHz band, dB.
    effect: f32,
}

/// A plant's name, and the dB it adds to a white late field at a frequency.
type Tilt = (&'static str, fn(f64) -> f64);

/// `Plan::room` with a white late field and the plants named, 24 realisations of each: stereo seeds 1–8
/// and mono seeds 1–4, at 44.1 and 48 kHz.
#[test]
#[ignore = "a measurement, printed for AGENTS.md"]
fn measure_the_high_cut_on_planted_rooms() {
    let plants: [Tilt; 3] = [
        ("-12 dB/oct above 3 kHz", |hz| {
            if hz > 3000.0 {
                -12.0 * (hz / 3000.0).log2()
            } else {
                0.0
            }
        }),
        ("white", |_| 0.0),
        ("+2 dB/oct above 2 kHz", |hz| {
            if hz > 2000.0 {
                2.0 * (hz / 2000.0).log2()
            } else {
                0.0
            }
        }),
    ];
    for (name, extra) in plants {
        let mut population = CutPopulation::default();
        for rate in [44_100.0, 48_000.0] {
            for (layout, seeds) in [(Layout::Decorrelated, 1..=8), (Layout::Mono, 1..=4)] {
                for seed in seeds {
                    let mut plan = Plan::room(rate, layout);
                    plan.band_db = white(extra);
                    plan.seed = seed;
                    let result = fitted(&plan);
                    population.fits += 1;
                    let cut = result.space.high_cut_hz;
                    if cut < HIGH_CUT_OPEN_HZ {
                        population.taken += 1;
                        population.corner = Some(
                            population
                                .corner
                                .map_or((cut, cut), |(lo, hi)| (lo.min(cut), hi.max(cut))),
                        );
                        let effect = [125.0, 250.0, 500.0, 2_000.0, 4_000.0, 5_657.0]
                            .map(|hz| 20.0 * tone_magnitude(0.0, 0.0, cut, rate as f32, hz).log10())
                            .iter()
                            .fold(0.0f32, |m, v| m.max(v.abs()));
                        population.effect = population.effect.max(effect);
                    }
                    if !result.report.clamps.is_empty() {
                        population.clamped += 1;
                        println!(
                            "  {name}, {rate} Hz, {layout:?}, seed {seed}: clamps {}",
                            clamps(&result)
                        );
                    }
                    if result.report.verification_refusal.is_some() {
                        population.refused += 1;
                        println!(
                            "  {name}, {rate} Hz, {layout:?}, seed {seed}: {:?}",
                            result.report.verification_refusal
                        );
                    }
                    for (worst, hz) in population.tone.iter_mut().zip([2_000.0, 4_000.0, 8_000.0]) {
                        if let Some(v) = band_value(&result, hz, |b| b.tone_db) {
                            *worst = worst.max(v.abs());
                        }
                    }
                }
            }
        }
        println!(
            "{name}: cut taken {} of {}, corners {:?} Hz, largest effect below 8 kHz {:.2} dB; clamped {}; verification refused {}; worst tone error at 2, 4 and 8 kHz {:.2}, {:.2}, {:.2} dB",
            population.taken,
            population.fits,
            population.corner,
            population.effect,
            population.clamped,
            population.refused,
            population.tone[0],
            population.tone[1],
            population.tone[2],
        );
    }
}

/// `Plan::room`, seed 1, stereo, at 44.1 and 48 kHz: a white late field below 2 kHz, and at 2, 4 and
/// 8 kHz the band levels against 1 kHz that the owner's pack read at P3 (aggregates only).
#[test]
#[ignore = "a measurement, printed for AGENTS.md"]
fn measure_the_packs_tone_curves() {
    let curves: [(&str, [f64; 3]); 3] = [
        ("clamped median", [2.0, 1.1, -4.5]),
        ("clamped tenth percentile", [-3.0, -10.2, -17.2]),
        ("unclamped median", [2.8, 5.0, 6.4]),
    ];
    for (name, high) in curves {
        for rate in [44_100.0, 48_000.0] {
            let mut plan = Plan::room(rate, Layout::Decorrelated);
            plan.band_db = white(|_| 0.0);
            plan.band_db[4..].copy_from_slice(&high);
            let result = fitted(&plan);
            println!("{name}, {rate} Hz: {}", summary(&result));
        }
    }
}

/// One fit of a planted room at each length, rate and layout named, three runs each. Run alone
/// (`--test-threads=1`).
#[test]
#[ignore = "a measurement, printed for AGENTS.md"]
fn measure_the_cost() {
    let cases: [(&str, f64, Layout, f64); 6] = [
        (
            "2 s stereo at 44.1 kHz",
            44_100.0,
            Layout::Decorrelated,
            2.0,
        ),
        (
            "3 s stereo at 44.1 kHz",
            44_100.0,
            Layout::Decorrelated,
            3.0,
        ),
        (
            "3.5 s stereo at 44.1 kHz",
            44_100.0,
            Layout::Decorrelated,
            3.5,
        ),
        (
            "6 s stereo at 44.1 kHz",
            44_100.0,
            Layout::Decorrelated,
            6.0,
        ),
        ("3 s mono at 44.1 kHz", 44_100.0, Layout::Mono, 3.0),
        ("4 s stereo at 96 kHz", 96_000.0, Layout::Decorrelated, 4.0),
    ];
    for (name, rate, layout, seconds) in cases {
        let mut plan = Plan::room(rate, layout);
        plan.duration_s = seconds;
        let planted = plan.build();
        let times: Vec<f64> = (0..3)
            .map(|_| {
                let start = Instant::now();
                fit(&planted.slices(), rate as f32).expect("fitted");
                start.elapsed().as_secs_f64()
            })
            .collect();
        println!("{name}: {times:.2?} s");
    }
}
