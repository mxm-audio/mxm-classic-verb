//! The round trip over what a fit controls (`plans/plan-mxm-classic-verb.md` §4.6): the engine renders
//! a known space at known Decay, Size, Diffusion and Pre-delay with everything else at the fitted
//! centre, the fit recovers them, and its verification measures the descriptors. Nothing here reads a
//! file.
//!
//! **Size is the density floor** (`DENSITY_FLOOR_DECAY_SHARE`, `DENSITY_FLOOR_MIN_S`), calculated from
//! the response's longest band decay rather than searched, so every case renders at the floor its
//! decay gives: a total delay of 1 s for any decay under 6.7 s.
//!
//! **Where the tolerances come from.** Each is the worst over the ten cases of
//! `the_round_trip_population` (rooms, chambers, halls and plates at 44.1, 48 and 96 kHz, mono and
//! stereo; the crate's `AGENTS.md`) with headroom, not over the three cases asserted here.

mod known;

use known::{Known, canonical, render};
use mxm_classic_verb_dsp::{Space, size_for_total_delay_s};
use mxm_classic_verb_fit::{Fit, fit};

fn max_tap_gain(space: &Space) -> f32 {
    space
        .early
        .iter()
        .flat_map(|t| [t.gain_l, t.gain_r])
        .fold(0.0, f32::max)
}

fn worst(values: impl Iterator<Item = Option<f32>>) -> f32 {
    values.flatten().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// The ten round trips, every one at the density floor.
fn cases() -> Vec<(&'static str, Known)> {
    let floor = size_for_total_delay_s(1.0);
    let known = |space, decay_s, diffusion, pre_delay_s, rate, mono, seconds| Known {
        space,
        decay_s,
        size_s: floor,
        diffusion,
        pre_delay_s,
        rate,
        mono,
        seconds,
    };
    vec![
        (
            "chamber, 48 kHz",
            known(
                canonical(Space::CHAMBER),
                1.6,
                0.7,
                0.012,
                48_000.0,
                false,
                3.0,
            ),
        ),
        (
            "hall, 44.1 kHz",
            known(
                canonical(Space::HALL),
                2.4,
                0.5,
                0.025,
                44_100.0,
                false,
                4.0,
            ),
        ),
        (
            "room, 48 kHz",
            known(
                canonical(Space::ROOM),
                0.8,
                0.9,
                0.004,
                48_000.0,
                false,
                3.0,
            ),
        ),
        (
            "plate, 48 kHz",
            known(Space::PLATE, 2.0, 1.0, 0.0, 48_000.0, false, 5.0),
        ),
        (
            "room, 44.1 kHz mono",
            known(canonical(Space::ROOM), 1.0, 0.3, 0.008, 44_100.0, true, 3.0),
        ),
        (
            "hall, 96 kHz",
            known(
                canonical(Space::HALL),
                1.8,
                0.8,
                0.030,
                96_000.0,
                false,
                3.0,
            ),
        ),
        (
            "chamber, 44.1 kHz",
            known(
                canonical(Space::CHAMBER),
                1.2,
                0.4,
                0.020,
                44_100.0,
                false,
                3.0,
            ),
        ),
        (
            "plate, 44.1 kHz",
            known(Space::PLATE, 1.4, 0.6, 0.010, 44_100.0, false, 3.0),
        ),
        (
            "room, 44.1 kHz",
            known(
                canonical(Space::ROOM),
                0.6,
                0.6,
                0.002,
                44_100.0,
                false,
                3.0,
            ),
        ),
        (
            "hall, 48 kHz",
            known(
                canonical(Space::HALL),
                3.0,
                0.9,
                0.015,
                48_000.0,
                false,
                5.0,
            ),
        ),
    ]
}

fn case(name: &str) -> Known {
    cases()
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, k)| k)
        .unwrap_or_else(|| panic!("no case {name}"))
}

fn fitted(name: &str, known: &Known) -> Fit {
    let channels = render(known);
    let slices: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    fit(&slices, known.rate).unwrap_or_else(|r| panic!("{name} is fitted: {r}"))
}

fn round_trip(name: &str) -> Fit {
    let known = case(name);
    let result = fitted(name, &known);
    let (c, s, e) = (result.controls, result.space, &result.report.errors);

    // The four controls a fit writes.
    // Decay: population worst −6.0 % (the mono room at 44.1 kHz). At the floor a short room's
    // decay spans only ten to twenty trips round the network, and its band decays read the wander of
    // that staircase (crates/mxm-classic-verb-dsp/AGENTS.md).
    let decay = (c.decay_s / known.decay_s - 1.0).abs();
    assert!(
        decay < 0.11,
        "{name}: Decay {} against {}",
        c.decay_s,
        known.decay_s
    );
    // Size is calculated, not searched: exactly the floor the render was made at.
    assert!(
        (c.size_s - known.size_s).abs() < 1e-6,
        "{name}: Size {} against {}",
        c.size_s,
        known.size_s
    );
    // Diffusion: population worst +0.40 (the room at 44.1 kHz).
    assert!(
        (c.diffusion - known.diffusion).abs() < 0.5,
        "{name}: Diffusion {}",
        c.diffusion
    );
    // Pre-delay: population worst +13.3 ms (the plate at 44.1 kHz: no taps, so its pre-delay is read
    // where the network's diffuse onset first stands above the early floor, later than it begins).
    assert!(
        (c.pre_delay_s - known.pre_delay_s).abs() < 0.018,
        "{name}: pre-delay {}",
        c.pre_delay_s
    );

    // The space's own values.
    // Band ratios: population worst −0.091 low and −0.172 high (the room at 44.1 kHz) and −0.143 top
    // (the mono room). Width: worst 0.039 (the plate at 44.1 kHz). Early level against its strongest
    // tap, since the fit normalises its taps to one: worst −42 % (the mono room).
    assert!(
        (s.decay_ratio_low - known.space.decay_ratio_low).abs() < 0.12,
        "{name}: {s:?}"
    );
    assert!(
        (s.decay_ratio_high - known.space.decay_ratio_high).abs() < 0.2,
        "{name}: {s:?}"
    );
    assert!(
        (s.decay_ratio_top - known.space.decay_ratio_top).abs() < 0.2,
        "{name}: {s:?}"
    );
    if !known.mono {
        assert!(
            (s.width - known.space.width).abs() < 0.1,
            "{name}: width {}",
            s.width
        );
    }
    if max_tap_gain(&known.space) > 0.0 {
        let early = s.early_level * max_tap_gain(&s)
            / (known.space.early_level * max_tap_gain(&known.space));
        assert!((early - 1.0).abs() < 0.6, "{name}: early level ×{early}");
    }

    // The descriptors, as the verification measured them. Population worsts: T30 13.2 % and tone
    // 4.20 dB (the mono room, both at 8 kHz: its short top octave renders 13 % short, and the
    // analyser reads a band's level off its decay's slope), profile distance 0.144 and envelope
    // 4.39 dB (the plate at 44.1 kHz), early-to-late 1.03 dB, IACC 0.001.
    assert!(
        worst(e.bands.iter().map(|b| b.t30_percent)) < 20.0,
        "{name}: {e:?}"
    );
    assert!(
        worst(e.bands.iter().map(|b| b.tone_db)) < 5.0,
        "{name}: {e:?}"
    );
    assert!(
        e.profile_distance.expect("profiles compared") < 0.2,
        "{name}: {e:?}"
    );
    assert!(
        e.envelope_db.expect("envelopes compared") < 6.0,
        "{name}: {e:?}"
    );
    assert!(
        e.early_to_late_db.expect("balance compared").abs() < 1.8,
        "{name}: {e:?}"
    );
    if !known.mono {
        assert!(
            e.iacc.expect("width compared").abs() < 0.02,
            "{name}: {e:?}"
        );
    }
    assert_eq!(result.report.verification_refusal, None, "{name}");
    assert!(
        result.report.clamps.is_empty(),
        "{name}: {:?}",
        result.report.clamps
    );
    result
}

/// Prints every case's recovered values and descriptor errors: the population the tolerances above,
/// and the crate's `AGENTS.md` table, are measured on.
#[test]
#[ignore]
fn the_round_trip_population() {
    println!(
        "case | Decay | Size ms | Diffusion | pre-delay ms | arrival | ratio low, high, top | width | early level | T30 | tone dB | profile | envelope dB | early-to-late dB | IACC | reflections | texture A peakiness, kurtosis"
    );
    for (name, known) in cases() {
        let result = fitted(name, &known);
        let (c, s, e) = (result.controls, result.space, &result.report.errors);
        let early = if max_tap_gain(&known.space) > 0.0 {
            format!(
                "{:+.0} %",
                100.0
                    * (s.early_level * max_tap_gain(&s)
                        / (known.space.early_level * max_tap_gain(&known.space))
                        - 1.0)
            )
        } else {
            "-".to_string()
        };
        let texture = &e.texture_a;
        println!(
            "{name} | {:+.1} % | {:.1} | {:+.2} | {:+.2} | {:?} | {:+.3}, {:+.3}, {:+.3} | {:+.3} | {early} | {:.1} % | {:.2} | {:.3} | {:.2} | {:+.2} | {:.3} | {} of {} | {:?}, {:?} | refusal {:?} | clamps {}",
            100.0 * (c.decay_s / known.decay_s - 1.0),
            1000.0 * c.size_s,
            c.diffusion - known.diffusion,
            1000.0 * (c.pre_delay_s - known.pre_delay_s),
            result.report.early.first_arrival,
            s.decay_ratio_low - known.space.decay_ratio_low,
            s.decay_ratio_high - known.space.decay_ratio_high,
            s.decay_ratio_top - known.space.decay_ratio_top,
            s.width - known.space.width,
            worst(e.bands.iter().map(|b| b.t30_percent)),
            worst(e.bands.iter().map(|b| b.tone_db)),
            e.profile_distance.unwrap_or(f32::NAN),
            e.envelope_db.unwrap_or(f32::NAN),
            e.early_to_late_db.unwrap_or(f32::NAN),
            e.iacc.map_or(f32::NAN, f32::abs),
            e.reflections_matched,
            e.reflections_compared,
            texture.peakiness,
            texture.kurtosis,
            result.report.verification_refusal,
            result.report.clamps.len(),
        );
    }
}

#[test]
fn a_chamber_round_trips() {
    round_trip("chamber, 44.1 kHz");
}

#[test]
fn a_plate_round_trips_with_its_late_field_first() {
    // No taps, so the first energy is the network's own return.
    let result = round_trip("plate, 48 kHz");
    assert_eq!(
        result.report.early.first_arrival,
        mxm_classic_verb_fit::FirstArrival::LateField
    );
}

#[test]
fn a_mono_room_round_trips() {
    let result = round_trip("room, 44.1 kHz mono");
    assert_eq!(
        result.space.width, 1.0,
        "a mono response leaves the width default"
    );
}
