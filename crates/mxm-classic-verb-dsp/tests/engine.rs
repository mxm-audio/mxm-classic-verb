//! The engine's contracts (plan §3), each measured.
//!
//! Thresholds are argued beside their assertions from measured values with headroom, as
//! `crates/mxm-measure/AGENTS.md` (in mxm-kit) asks: the crate supplies rulers, this file supplies
//! verdicts.

use mxm_classic_verb_dsp::space::{HIGH_CUT_OPEN_HZ, MIN_HIGH_CUT_HZ};
use mxm_classic_verb_dsp::{
    Controls, DecayShape, EARLY_LATE_SILENCE_DB, Engine, Interpolation, LOOP_GAIN_CEILING,
    MAX_DECAY_S, MAX_MOD_DEPTH_S, MAX_MOD_RATE_HZ, MAX_SIZE_S, MIN_SIZE_S, SPACE_FADE_S, Space,
    predicted_decay_s, predicted_decays_s, tone_magnitude,
};
use mxm_measure::observe::worst_step;

// ── Test-side measurement: band filtering and Schroeder decay ───────────────────────────────────

/// A constant-peak band-pass biquad (RBJ cookbook), one octave wide.
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn octave(hz: f64, fs: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * hz / fs;
        let q = std::f64::consts::SQRT_2;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b: [alpha / a0, 0.0, -alpha / a0],
            a: [-2.0 * w.cos() / a0, (1.0 - alpha) / a0],
        }
    }

    fn run(&self, x: &[f64]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&v| {
                let y = self.b[0] * v + self.b[1] * x1 + self.b[2] * x2
                    - self.a[0] * y1
                    - self.a[1] * y2;
                x2 = x1;
                x1 = v;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    }

    /// Filtered time-reversed, so the filter's own ringing lands in the discarded end
    /// (`research:effects/feedback-delay-network-reverb.md` §9.4).
    fn run_reversed(&self, x: &[f64]) -> Vec<f64> {
        let mut r: Vec<f64> = x.iter().rev().copied().collect();
        r = self.run(&r);
        r.reverse();
        r
    }
}

/// T₃₀ from Schroeder backward integration: the −5 to −35 dB span, fitted by least squares and
/// extrapolated to 60 dB. The response is rendered long and noise-free, so no truncation is needed.
///
/// T₃₀ rather than T₂₀: with allpasses in the loop, a band's energy decay ripples early, and on the
/// same renders T₂₀ scattered ±4.4 % at 1 kHz where T₃₀ scattered ±2.6 %.
fn t30(x: &[f64], fs: f64) -> f64 {
    let mut edc = vec![0.0; x.len()];
    let mut acc = 0.0;
    for i in (0..x.len()).rev() {
        acc += x[i] * x[i];
        edc[i] = acc;
    }
    let total = edc[0];
    let (mut n, mut st, mut sy, mut stt, mut sty) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, e) in edc.iter().enumerate() {
        let db = 10.0 * (e / total).log10();
        if (-35.0..=-5.0).contains(&db) {
            let t = i as f64 / fs;
            n += 1.0;
            st += t;
            sy += db;
            stt += t * t;
            sty += t * db;
        }
    }
    let slope = (n * sty - st * sy) / (n * stt - st * st);
    -60.0 / slope
}

fn impulse_response(engine: &mut Engine, seconds: f64) -> (Vec<f64>, Vec<f64>) {
    let n = (seconds * engine.sample_rate() as f64) as usize;
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (a, b) = engine.process(x, x);
        l.push(a as f64);
        r.push(b as f64);
    }
    (l, r)
}

/// A space whose bands all decay together and whose tone is flat, for measuring the decay rule alone.
fn flat_space() -> Space {
    let mut s = Space::PLATE;
    s.decay_ratio_low = 1.0;
    s.decay_ratio_high = 1.0;
    s.decay_ratio_top = 1.0;
    s.tone_low_db = 0.0;
    s.tone_high_db = 0.0;
    s
}

fn wet_only(controls: Controls) -> Controls {
    Controls {
        mix: 1.0,
        pre_delay_s: 0.0,
        ..controls
    }
}

fn band_t60(engine: &mut Engine, hz: f64, seconds: f64) -> f64 {
    let fs = engine.sample_rate() as f64;
    let (l, r) = impulse_response(engine, seconds);
    let mut band: Vec<f64> = l.iter().zip(&r).map(|(a, b)| a + b).collect();
    // Three passes: one second-order section leaves a slow tail in a neighbouring band loud enough to
    // own this band's late decay when the bands decay at very different rates.
    let filter = Biquad::octave(hz, fs);
    for _ in 0..3 {
        band = filter.run_reversed(&band);
    }
    t30(&band, fs)
}

/// Deterministic noise.
fn noise(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

// ── Silence, Off, reset ────────────────────────────────────────────────────────────────────────

#[test]
fn silence_after_a_tail_is_exact_zero_and_the_engine_parks() {
    let mut e = Engine::new(48_000.0);
    e.set_controls(Controls {
        decay_s: 1.0,
        ..Controls::default()
    });
    let _ = e.process(1.0, 1.0);
    let mut last_nonzero = 0;
    for i in 0..(48_000 * 12) {
        let (l, r) = e.process(0.0, 0.0);
        if l != 0.0 || r != 0.0 {
            last_nonzero = i;
        }
    }
    assert!(e.is_parked(), "a decayed tail parks");
    assert!(last_nonzero < 48_000 * 11, "and then stays at exact zero");
    assert_eq!(e.remaining_tail_seconds(), 0.0);
}

#[test]
fn mix_at_zero_is_dry_to_the_bit_in_both_channels_and_parks() {
    let mut e = Engine::new(44_100.0);
    e.set_controls(Controls {
        mix: 0.0,
        ..Controls::default()
    });
    let x = noise(20_000, 7);
    for (i, &v) in x.iter().enumerate() {
        let r = x[(i * 7) % x.len()];
        let (a, b) = e.process(v, r);
        assert_eq!((a, b), (v, r), "sample {i}");
    }
    assert!(e.is_parked());
}

#[test]
fn off_empties_so_re_engaging_does_not_replay_an_old_tail() {
    let mut e = Engine::new(48_000.0);
    e.set_controls(Controls {
        mix: 1.0,
        decay_s: 8.0,
        ..Controls::default()
    });
    for v in noise(24_000, 3) {
        e.process(v, v);
    }
    e.set_controls(Controls {
        mix: 0.0,
        decay_s: 8.0,
        ..Controls::default()
    });
    for _ in 0..4_800 {
        e.process(0.0, 0.0);
    }
    assert!(e.is_parked());
    e.set_controls(Controls {
        mix: 1.0,
        decay_s: 8.0,
        ..Controls::default()
    });
    for i in 0..48_000 {
        let (l, r) = e.process(0.0, 0.0);
        assert_eq!((l, r), (0.0, 0.0), "an old tail spilled at sample {i}");
    }
}

#[test]
fn reset_leaves_no_tail_and_is_reproducible() {
    let x = noise(30_000, 11);
    let render =
        |e: &mut Engine| -> Vec<(f32, f32)> { x.iter().map(|&v| e.process(v, -v)).collect() };
    let mut e = Engine::new(48_000.0);
    e.set_controls(Controls {
        mod_depth_s: 0.002,
        ..Controls::default()
    });
    let first = render(&mut e);
    e.reset();
    assert!(e.is_parked());
    assert_eq!(e.process(0.0, 0.0), (0.0, 0.0));
    e.reset();
    let second = render(&mut e);
    assert_eq!(
        first, second,
        "rendering after reset reproduces the samples"
    );
}

// ── Bounded by construction ────────────────────────────────────────────────────────────────────

#[test]
fn every_corner_of_the_domain_stays_finite_and_under_the_loop_ceiling() {
    let corners = [
        Controls {
            decay_s: MAX_DECAY_S,
            size_s: MIN_SIZE_S,
            mod_depth_s: MAX_MOD_DEPTH_S,
            mod_rate_hz: MAX_MOD_RATE_HZ,
            ..Controls::default()
        },
        Controls {
            decay_s: MAX_DECAY_S,
            size_s: MAX_SIZE_S,
            bass_mult: 10.0,
            treble_mult: 10.0,
            diffusion: 1.0,
            ..Controls::default()
        },
        Controls {
            decay_s: MAX_DECAY_S,
            bass_mult: 10.0,
            treble_mult: 0.1,
            width: 3.0,
            tone_low_db: 24.0,
            tone_high_db: 24.0,
            ..Controls::default()
        },
        Controls {
            decay_s: 0.1,
            size_s: MIN_SIZE_S,
            early_late_db: -60.0,
            width: -1.0,
            duck: 1.0,
            ..Controls::default()
        },
        Controls {
            shape: DecayShape::Reverse,
            decay_s: MAX_DECAY_S,
            ..Controls::default()
        },
    ];
    let mut hall = Space::HALL;
    hall.decay_ratio_low = 10.0;
    hall.decay_ratio_high = 10.0;
    hall.decay_ratio_top = 10.0;
    for fs in [8_000.0, 22_050.0, 44_100.0, 96_000.0, 192_000.0] {
        for (k, c) in corners.iter().enumerate() {
            let mut e = Engine::new(fs);
            e.set_space(hall);
            e.set_controls(Controls { mix: 1.0, ..*c });
            assert!(
                e.loop_gain_bound() <= LOOP_GAIN_CEILING,
                "corner {k} at {fs}: bound {}",
                e.loop_gain_bound()
            );
            let n = (fs * 1.5) as usize;
            let input = noise(n / 2, k as u64 + 1);
            let mut peak = 0.0f32;
            for i in 0..n {
                let v = input.get(i).copied().unwrap_or(0.0);
                let (l, r) = e.process(v, -v);
                assert!(
                    l.is_finite() && r.is_finite(),
                    "corner {k} at {fs} Hz went non-finite at {i}"
                );
                peak = peak.max(l.abs()).max(r.abs());
            }
            println!("corner {k} at {fs} Hz: peak {peak:.2}");
            // The claim is finiteness and a gain bound, not unity: a near-lossless loop fed
            // full-scale noise resonates above full scale. This ceiling catches a bound that has
            // actually broken, which grows without limit; the measured peaks are in the crate's NOTES.md.
            assert!(peak < 1_000.0, "corner {k} at {fs} Hz peaked at {peak}");
        }
    }
}

#[test]
fn a_non_finite_input_is_absorbed() {
    let mut e = Engine::new(48_000.0);
    e.set_controls(Controls {
        mix: 1.0,
        ..Controls::default()
    });
    for i in 0..20_000 {
        let v = match i % 997 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            _ => (i as f32 * 0.01).sin(),
        };
        let (l, r) = e.process(v, v);
        assert!(l.is_finite() && r.is_finite(), "sample {i}");
    }
}

// ── The decay rule, measured ───────────────────────────────────────────────────────────────────

#[test]
fn the_realised_decay_follows_the_composed_decay_across_size_decay_and_rate() {
    let (mut worst_dense, mut worst_sparse) = (0.0f64, 0.0f64);
    for fs in [48_000.0f32, 96_000.0] {
        let decays: &[f32] = if fs == 48_000.0 {
            &[0.8, 2.5, 5.0]
        } else {
            &[0.8, 2.5]
        };
        for size in [0.02f32, 0.06, 0.15] {
            for &decay in decays {
                let mut e = Engine::new(fs);
                e.set_space(flat_space());
                e.set_controls(wet_only(Controls {
                    decay_s: decay,
                    size_s: size,
                    diffusion: 0.7,
                    ..Controls::default()
                }));
                let t = band_t60(&mut e, 1_000.0, decay as f64 * 0.9 + size as f64 * 4.0);
                let error = (t / decay as f64 - 1.0).abs();
                let trips = decay / size;
                println!(
                    "fs {fs} size {size} decay {decay} ({trips:.0} trips): T30-derived {t:.3} s ({:+.1} %)",
                    100.0 * (t / decay as f64 - 1.0)
                );
                if trips >= 25.0 {
                    worst_dense = worst_dense.max(error);
                } else {
                    worst_sparse = worst_sparse.max(error);
                }
            }
        }
    }
    // Where a decay spans at least 25 trips round the mean line, the formula is the calibration.
    // Against a just-noticeable difference of about 5 % (research page §4.3), with headroom over
    // the measured worst: 3.3 % (Size 20 ms, Decay 0.8 s at 96 kHz), read through the output taps.
    assert!(
        worst_dense < 0.04,
        "worst dense decay error {:.1} %",
        100.0 * worst_dense
    );
    // Fewer trips and the decay is a staircase of discrete returns, which a T20 fit reads with a
    // wander of its own (measured worst 16 %). Recorded as a property of short decays in long
    // networks, not a calibration error.
    assert!(
        worst_sparse < 0.25,
        "worst sparse decay error {:.1} %",
        100.0 * worst_sparse
    );
}

#[test]
fn modulation_costs_less_high_band_decay_through_the_allpass_than_through_linear_reads() {
    let measure = |interpolation: Interpolation| {
        let mut e = Engine::new(48_000.0);
        e.set_space(flat_space());
        e.set_interpolation(interpolation);
        e.set_controls(wet_only(Controls {
            decay_s: 2.0,
            size_s: 0.05,
            mod_depth_s: 0.002,
            mod_rate_hz: 1.0,
            ..Controls::default()
        }));
        band_t60(&mut e, 4_000.0, 1.6)
    };
    let allpass = measure(Interpolation::Allpass);
    let linear = measure(Interpolation::Linear);
    println!(
        "4 kHz T60 with 2 ms modulation: allpass {allpass:.3} s, linear {linear:.3} s, target 2.0 s"
    );
    assert!(
        (allpass - 2.0).abs() < (linear - 2.0).abs(),
        "the allpass read is the one that keeps the decay"
    );
}

#[test]
fn bass_and_treble_multipliers_tilt_the_decay_the_way_they_say() {
    let run = |bass: f32, treble: f32, hz: f64| {
        let mut e = Engine::new(48_000.0);
        e.set_space(flat_space());
        e.set_controls(wet_only(Controls {
            decay_s: 1.5,
            size_s: 0.05,
            bass_mult: bass,
            treble_mult: treble,
            ..Controls::default()
        }));
        band_t60(&mut e, hz, 3.0)
    };
    let low_long = run(2.0, 1.0, 125.0);
    let low_flat = run(1.0, 1.0, 125.0);
    let high_short = run(1.0, 0.5, 8_000.0);
    let high_flat = run(1.0, 1.0, 8_000.0);
    println!(
        "125 Hz: {low_flat:.2} s flat, {low_long:.2} s at bass ×2; 8 kHz: {high_flat:.2} s flat, {high_short:.2} s at treble ×0.5"
    );
    assert!(low_long > low_flat * 1.3);
    assert!(high_short < high_flat * 0.8);
}

#[test]
fn the_top_band_decays_apart_from_the_high_band() {
    // Plan D5: a room's 8 kHz octave can die well before its 4 kHz octave, which one high-band ratio
    // could not give. At the fit's density floor the top ratio shortens 8 kHz much further than it
    // drags 4 kHz, and the fit solves the two upper ratios together for the rest.
    let run = |top: f32, hz: f64| {
        let mut space = flat_space();
        space.decay_ratio_top = top;
        let mut e = Engine::new(48_000.0);
        e.set_space(space);
        e.set_controls(wet_only(Controls {
            decay_s: 1.5,
            size_s: 0.0563,
            ..Controls::default()
        }));
        band_t60(&mut e, hz, 3.0)
    };
    let (flat_4k, flat_8k) = (run(1.0, 4_000.0), run(1.0, 8_000.0));
    let (short_4k, short_8k) = (run(0.3, 4_000.0), run(0.3, 8_000.0));
    let (shortest_4k, shortest_8k) = (run(0.1, 4_000.0), run(0.1, 8_000.0));
    println!(
        "Size 56.3 ms, Decay 1.5 s, 4 kHz / 8 kHz: {flat_4k:.2} / {flat_8k:.2} s at top ratio 1, {short_4k:.2} / {short_8k:.2} s at 0.3, {shortest_4k:.2} / {shortest_8k:.2} s at 0.1"
    );
    // Measured at 0.3: 8 kHz falls to 0.47 of flat and 4 kHz to 0.70.
    assert!(short_8k < 0.55 * flat_8k, "{short_8k} against {flat_8k}");
    assert!(
        short_8k / flat_8k < 0.8 * (short_4k / flat_4k),
        "8 kHz {short_8k} against {flat_8k}, 4 kHz {short_4k} against {flat_4k}"
    );
}

#[test]
fn the_closed_form_over_many_frequencies_is_the_closed_form_at_each() {
    let hz = [60.0f32, 125.0, 1_000.0, 3_000.0, 8_000.0, 30_000.0];
    let mut out = [0.0f32; 6];
    let mut space = Space::HALL;
    space.decay_ratio_top = 0.3;
    let c = Controls {
        decay_s: 2.5,
        size_s: 0.0563,
        treble_mult: 0.7,
        ..Controls::default()
    };
    for fs in [44_100.0, 96_000.0] {
        predicted_decays_s(&space, &c, fs, &hz, &mut out);
        for (h, o) in hz.iter().zip(out) {
            let one = predicted_decay_s(&space, &c, fs, *h);
            assert_eq!(
                o.to_bits(),
                one.to_bits(),
                "{h} Hz at {fs}: {o} against {one}"
            );
        }
    }
}

#[test]
fn the_closed_form_predicts_the_realised_octave_decays() {
    let mut worst = 0.0f64;
    for (bass, treble, top) in [
        (1.0, 1.0, 1.0),
        (2.0, 1.0, 1.0),
        (1.0, 0.5, 1.0),
        (0.5, 3.0, 1.0),
        (1.0, 1.0, 0.4),
    ] {
        for hz in [125.0f64, 1_000.0, 4_000.0, 8_000.0] {
            let mut space = flat_space();
            space.decay_ratio_top = top;
            let mut e = Engine::new(48_000.0);
            e.set_space(space);
            e.set_controls(wet_only(Controls {
                decay_s: 1.5,
                size_s: 0.05,
                bass_mult: bass,
                treble_mult: treble,
                ..Controls::default()
            }));
            let predicted = f64::from(e.predicted_decay_s(hz as f32));
            let measured = band_t60(&mut e, hz, 3.5);
            let error = (predicted / measured - 1.0).abs();
            println!(
                "bass {bass} treble {treble} top {top} at {hz} Hz: predicted {predicted:.3} s, measured {measured:.3} s ({:+.1} %)",
                100.0 * (predicted / measured - 1.0)
            );
            worst = worst.max(error);
        }
    }
    // Measured worst 5.1 % (flat, at 125 Hz, read through the output taps), inside the ~5 % just-noticeable
    // difference's neighbourhood.
    assert!(
        worst < 0.06,
        "worst prediction error {:.1} %",
        100.0 * worst
    );
}

// ── Transitions ────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_space_change_fades_through_silence_without_a_step() {
    let fs = 48_000.0;
    let n = 96_000;
    let tone: Vec<f32> = (0..n)
        .map(|i| 0.5 * (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / fs).sin())
        .collect();
    let render = |from: Space, to: Option<Space>| {
        let mut e = Engine::new(fs);
        e.set_space(from);
        e.set_controls(Controls {
            mix: 1.0,
            ..Controls::default()
        });
        let mut out = Vec::with_capacity(n);
        for (i, &v) in tone.iter().enumerate() {
            if i == 48_000 {
                if let Some(space) = to {
                    e.set_space(space);
                }
            }
            out.push(e.process(v, v).0);
        }
        out
    };
    // Either space alone sets the calm step: the two play the tone at different levels, and the
    // louder one's own slope is not a step.
    let (_, hall) = worst_step(&render(Space::HALL, None)).unwrap();
    let (_, room) = worst_step(&render(Space::ROOM, None)).unwrap();
    let calm = hall.max(room);
    let (_, changed) = worst_step(&render(Space::HALL, Some(Space::ROOM))).unwrap();
    println!(
        "worst step: {hall:.4} in the hall alone, {room:.4} in the room alone, {changed:.4} across a change"
    );
    assert!(changed < calm * 1.5 + 0.01, "the change stepped the output");
}

#[test]
fn a_space_asked_for_during_a_fade_becomes_the_destination() {
    let mut e = Engine::new(48_000.0);
    e.set_controls(Controls {
        mix: 1.0,
        ..Controls::default()
    });
    for v in noise(4_800, 5) {
        e.process(v, v);
    }
    e.set_space(Space::ROOM);
    for _ in 0..400 {
        e.process(0.1, 0.1);
    }
    e.set_space(Space::CHAMBER);
    for _ in 0..9_600 {
        e.process(0.1, 0.1);
    }
    assert_eq!(e.space(), Space::CHAMBER);
}

// ── Activity ───────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_park_decision_does_not_depend_on_the_sample_rate() {
    let seconds_to_park = |fs: f32| {
        let mut e = Engine::new(fs);
        e.set_controls(Controls {
            decay_s: 0.8,
            ..Controls::default()
        });
        // Built from hertz and seconds, so it is the same signal at every rate — a unit impulse is
        // not, because its energy per hertz falls as the rate rises.
        let n = (0.1 * fs) as usize;
        for i in 0..n {
            let t = i as f32 / fs;
            let window = 0.5 - 0.5 * (std::f32::consts::TAU * t / 0.1).cos();
            let v = 0.5 * window * (std::f32::consts::TAU * 500.0 * t).sin();
            e.process(v, v);
        }
        let mut n = 0usize;
        while !e.is_parked() && n < (fs * 30.0) as usize {
            e.process(0.0, 0.0);
            n += 1;
        }
        n as f32 / fs
    };
    let at = [
        seconds_to_park(8_000.0),
        seconds_to_park(48_000.0),
        seconds_to_park(192_000.0),
    ];
    println!("seconds to park at 8 / 48 / 192 kHz: {at:?}");
    // Measured with a stimulus defined in hertz and seconds: 1.421 s at 192 kHz against 1.431 s at
    // 48 kHz. At 8 kHz the loop allpasses are only 10–39 samples, four pairs round onto one length, and
    // parking came 2.8 % later (1.471 s); with their coefficient at zero the three rates agreed within
    // 1.8 %. Later is the safe direction — no tail is cut — so 8 kHz may park up to 8 % later, never
    // earlier.
    assert!((at[2] - at[1]).abs() < 0.015 * at[1], "{at:?}");
    assert!(at[0] > 0.985 * at[1] && at[0] < 1.08 * at[1], "{at:?}");
}

#[test]
fn a_sparse_network_does_not_park_between_its_own_returns() {
    let mut e = Engine::new(48_000.0);
    e.set_space(flat_space());
    e.set_controls(wet_only(Controls {
        decay_s: 4.0,
        size_s: MAX_SIZE_S,
        diffusion: 0.0,
        ..Controls::default()
    }));
    e.process(1.0, 1.0);
    let mut energy_after_a_second = 0.0f64;
    for i in 0..(48_000 * 3) {
        let (l, _) = e.process(0.0, 0.0);
        assert!(
            !(i < 48_000 && e.is_parked()),
            "parked at {i} while still ringing"
        );
        if i > 48_000 {
            energy_after_a_second += (l as f64).powi(2);
        }
    }
    assert!(energy_after_a_second > 0.0);
}

#[test]
fn the_tail_declaration_never_underestimates() {
    for (pre, decay, shape) in [
        (0.0, 1.2, DecayShape::Natural),
        (0.4, 0.6, DecayShape::Natural),
        (0.2, 1.0, DecayShape::Gated),
    ] {
        let fs = 48_000.0;
        let mut e = Engine::new(fs);
        e.set_controls(Controls {
            mix: 1.0,
            pre_delay_s: pre,
            decay_s: decay,
            shape,
            ..Controls::default()
        });
        for v in noise(4_800, 9) {
            e.process(v, v);
        }
        let mut claims = Vec::new();
        let mut i = 0usize;
        while !e.is_parked() {
            if i.is_multiple_of(480) {
                claims.push((i, e.remaining_tail_seconds()));
            }
            e.process(0.0, 0.0);
            i += 1;
            assert!(i < (fs * 60.0) as usize);
        }
        for (at, claim) in claims {
            let actual = (i - at) as f32 / fs;
            assert!(
                claim >= actual,
                "pre {pre} decay {decay} {shape:?}: at {at} claimed {claim:.3} s, quiet came {actual:.3} s later"
            );
        }
    }
}

// ── The composition laws and the shaped path ───────────────────────────────────────────────────

#[test]
fn width_never_moves_the_wet_mono_sum() {
    let x = noise(24_000, 21);
    let sums = |width: f32| -> Vec<f32> {
        let mut e = Engine::new(48_000.0);
        e.set_controls(Controls {
            mix: 1.0,
            width,
            ..Controls::default()
        });
        x.iter()
            .map(|&v| {
                let (l, r) = e.process(v, v * 0.3);
                l + r
            })
            .collect()
    };
    let (narrow, wide) = (sums(0.2), sums(2.5));
    let worst = narrow
        .iter()
        .zip(&wide)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-4, "the mono sum moved by {worst}");
}

#[test]
fn ducking_at_zero_changes_nothing_to_the_bit() {
    let x = noise(12_000, 4);
    let render = |duck: Option<f32>| -> Vec<(f32, f32)> {
        let mut e = Engine::new(48_000.0);
        let c = Controls {
            mix: 0.8,
            ..Controls::default()
        };
        e.set_controls(match duck {
            Some(d) => Controls { duck: d, ..c },
            None => c,
        });
        x.iter().map(|&v| e.process(v, v)).collect()
    };
    assert_eq!(render(None), render(Some(0.0)));
}

#[test]
fn the_shaped_decays_have_their_envelopes() {
    let energy_halves = |shape: DecayShape| {
        let fs = 48_000.0;
        let mut e = Engine::new(fs);
        // The early reflections silenced, so what is measured is the shaped path alone.
        e.set_controls(wet_only(Controls {
            shape,
            decay_s: 0.5,
            early_late_db: -EARLY_LATE_SILENCE_DB,
            ..Controls::default()
        }));
        let (l, r) = impulse_response(&mut e, 1.0);
        let energy = |a: usize, b: usize| (a..b).map(|i| l[i] * l[i] + r[i] * r[i]).sum::<f64>();
        let q = (fs * 0.25) as usize;
        (energy(0, q), energy(q, 2 * q), energy(2 * q + 2_400, 4 * q))
    };
    let (g1, g2, g_after) = energy_halves(DecayShape::Gated);
    let (r1, r2, r_after) = energy_halves(DecayShape::Reverse);
    println!(
        "gated halves {g1:.4} / {g2:.4}, after {g_after:.2e}; reverse halves {r1:.4} / {r2:.4}, after {r_after:.2e}"
    );
    assert!((g1 / g2 - 1.0).abs() < 0.5, "a gated block is flat");
    assert!(r2 > 4.0 * r1, "a reverse swell grows");
    assert!(
        g_after < 1e-3 * (g1 + g2) && r_after < 1e-3 * (r1 + r2),
        "and both stop"
    );
}

// ── The space's high cut ───────────────────────────────────────────────────────────────────────

fn with_cut(mut space: Space, hz: f32) -> Space {
    space.high_cut_hz = hz;
    space
}

#[test]
fn an_open_high_cut_is_bypassed_to_the_bit() {
    let x = noise(24_000, 17);
    let render = |space: Space| -> Vec<(f32, f32)> {
        let mut e = Engine::new(48_000.0);
        e.set_space(space);
        e.set_controls(Controls {
            mix: 1.0,
            ..Controls::default()
        });
        x.iter().map(|&v| e.process(v, -0.5 * v)).collect()
    };
    let open = render(Space::HALL);
    // Every corner the space bounds to open renders the hand-authored hall's samples exactly.
    for hz in [HIGH_CUT_OPEN_HZ, 1.0e9, f32::INFINITY, f32::NAN] {
        assert!(render(with_cut(Space::HALL, hz)) == open, "{hz}");
    }
    // Just below open the cut is a filter, and it is heard.
    assert!(render(with_cut(Space::HALL, 19_000.0)) != open);
}

#[test]
fn the_high_cut_follows_its_closed_form_through_the_engine_at_every_rate() {
    // The cut is outside the loop and the engine is linear with no modulation, so two engines fed one
    // sine differ only in their tone paths: in steady state the ratio of their levels is the ratio of
    // the closed forms, which is exactly one at the reference, where the normalisation holds the wet
    // level whatever the cut.
    let corner = 2_500.0;
    let hall = Space::HALL;
    let mut worst = 0.0f64;
    for fs in [44_100.0f32, 48_000.0, 96_000.0] {
        for hz in [250.0f64, 1_000.0, 2_500.0, 8_000.0] {
            let level = |space: Space| {
                let mut e = Engine::new(fs);
                e.set_space(space);
                e.set_controls(wet_only(Controls {
                    decay_s: 0.2,
                    size_s: 0.02,
                    ..Controls::default()
                }));
                // One second: the transient is over 100 dB down by the half that is measured, which
                // holds a whole number of cycles of every frequency here.
                let n = fs as usize;
                let mut sum = 0.0f64;
                for i in 0..n {
                    let phase = std::f64::consts::TAU * hz * i as f64 / f64::from(fs);
                    let v = (0.25 * phase.sin()) as f32;
                    let (l, r) = e.process(v, v);
                    if i >= n / 2 {
                        sum += f64::from(l).powi(2) + f64::from(r).powi(2);
                    }
                }
                sum.sqrt()
            };
            let rendered = level(with_cut(hall, corner)) / level(hall);
            let closed = |cut: f32| {
                f64::from(tone_magnitude(
                    hall.tone_low_db,
                    hall.tone_high_db,
                    cut,
                    fs,
                    hz as f32,
                ))
            };
            let expected = closed(corner) / closed(HIGH_CUT_OPEN_HZ);
            println!(
                "{fs} Hz, {hz} Hz: rendered {rendered:.5}, closed form {expected:.5} ({:+.3} %)",
                100.0 * (rendered / expected - 1.0)
            );
            worst = worst.max((rendered / expected - 1.0).abs());
        }
    }
    assert!(worst < 0.01, "worst {:.3} %", 100.0 * worst);
}

#[test]
fn a_space_change_reaches_the_high_cut_at_the_fades_silent_point() {
    let fs = 48_000.0;
    let (n, change) = (96_000, 48_000);
    let tone: Vec<f32> = (0..n)
        .map(|i| 0.5 * (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / fs).sin())
        .collect();
    let render = |to: Space, at: usize| {
        let mut e = Engine::new(fs);
        e.set_controls(Controls {
            mix: 1.0,
            ..Controls::default()
        });
        let mut out = Vec::with_capacity(n);
        for (i, &v) in tone.iter().enumerate() {
            if i == at {
                e.set_space(to);
            }
            out.push(e.process(v, v).0);
        }
        (out, e.space())
    };
    let cut = with_cut(Space::HALL, MIN_HIGH_CUT_HZ);
    // The comparison changes only the high shelf, which the swap applies. It keeps the hall's decay
    // ratios, because a fade's network gains follow the space it is heading for from the request on.
    let mut darker = Space::HALL;
    darker.tone_high_db = -12.0;
    let (calm, _) = render(Space::HALL, usize::MAX);
    let (to_shelf, _) = render(darker, change);
    let (to_cut, in_force) = render(cut, change);
    assert_eq!(in_force, cut);
    // Until the fade reaches silence both changes still play the hall's tone, so the cut is not heard
    // before the swap; after it, it is — at the same sample as the shelf.
    let silent = change + (SPACE_FADE_S * fs) as usize - 2;
    assert!(
        to_cut[..silent] == to_shelf[..silent],
        "the cut arrived before the fade's silent point"
    );
    assert!(to_cut[silent + 4..] != to_shelf[silent + 4..]);
    let (_, calm_step) = worst_step(&calm).unwrap();
    let (_, cut_step) = worst_step(&to_cut).unwrap();
    println!("worst step: {calm_step:.4} without a change, {cut_step:.4} across one to a cut");
    assert!(
        cut_step < calm_step * 1.5 + 0.01,
        "the change stepped the output"
    );
}

#[test]
fn the_high_cut_stays_finite_at_its_floor_and_past_nyquist_at_every_rate() {
    for fs in [8_000.0f32, 22_050.0, 44_100.0, 96_000.0, 192_000.0] {
        // 5 kHz lies past Nyquist at 8 kHz and 19.999 kHz below 44.1 kHz: both are held under it.
        for corner in [MIN_HIGH_CUT_HZ, 5_000.0, 19_999.0] {
            let mut e = Engine::new(fs);
            e.set_space(with_cut(Space::HALL, corner));
            e.set_controls(Controls {
                mix: 1.0,
                tone_low_db: 24.0,
                tone_high_db: 24.0,
                ..Controls::default()
            });
            let n = (fs * 0.6) as usize;
            let input = noise(n / 2, 29);
            let mut peak = 0.0f32;
            for i in 0..n {
                let v = input.get(i).copied().unwrap_or(0.0);
                let (l, r) = e.process(v, -v);
                assert!(
                    l.is_finite() && r.is_finite(),
                    "{corner} Hz at {fs} Hz went non-finite at {i}"
                );
                peak = peak.max(l.abs()).max(r.abs());
            }
            // As in the corner sweep: a broken bound grows without limit.
            assert!(peak < 1_000.0, "{corner} Hz at {fs} Hz peaked at {peak}");
            for hz in [100.0, 1_000.0, 0.45 * fs] {
                let m = tone_magnitude(24.0, 24.0, corner, fs, hz);
                assert!(
                    m.is_finite() && m >= 0.0,
                    "{corner} Hz at {fs} Hz, {hz} Hz: {m}"
                );
            }
        }
    }
}

#[test]
fn settings_sent_again_at_block_boundaries_leave_the_high_cut_untouched() {
    // A host hands settings over between blocks. Sending the same ones again must not restart the
    // cut's state or recompute it differently, whatever the block length.
    let x = noise(20_000, 23);
    let space = with_cut(Space::HALL, 3_000.0);
    let controls = Controls {
        mix: 0.7,
        ..Controls::default()
    };
    let render = |block: usize| -> Vec<(f32, f32)> {
        let mut e = Engine::new(44_100.0);
        e.set_space(space);
        e.set_controls(controls);
        x.iter()
            .enumerate()
            .map(|(i, &v)| {
                if i % block == 0 {
                    e.set_space(space);
                    e.set_controls(controls);
                }
                e.process(v, 0.5 * v)
            })
            .collect()
    };
    let once = render(x.len());
    for block in [1, 64, 511] {
        assert!(render(block) == once, "block {block}");
    }
}
