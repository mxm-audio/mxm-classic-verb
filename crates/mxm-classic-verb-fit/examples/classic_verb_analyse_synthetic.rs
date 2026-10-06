//! Prints the analysis of a planted synthetic impulse response beside what was planted, for a human
//! to read.
//!
//! ```bash
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_analyse_synthetic
//! ```
//!
//! The response is the tests' planted stereo room: five reflections, a tail from 45 ms decaying at a
//! known T60 in each octave band, and a noise floor. Nothing is read from or written to disk.

#[path = "../tests/support/mod.rs"]
mod support;

use mxm_classic_verb_fit::{BandReading, Width, analyse};
use support::{Layout, Plan, ROOM_REFLECTIONS};

fn opt(value: Option<f32>, scale: f32, unit: &str) -> String {
    value.map_or_else(|| "—".to_string(), |v| format!("{:.2}{unit}", v * scale))
}

fn main() {
    let plan = Plan::room(48_000.0, Layout::Decorrelated);
    let planted = plan.build();
    let analysis = match analyse(&planted.slices(), plan.rate as f32) {
        Ok(analysis) => analysis,
        Err(refusal) => {
            println!("refused: {refusal}");
            return;
        }
    };

    println!(
        "Planted stereo room at {} Hz, {} s, seed {}",
        plan.rate, plan.duration_s, plan.seed
    );
    println!();
    let onset = &analysis.onset;
    println!(
        "Onset           {:.4} s   direct sound {:.4} s (planted {:.4} s)",
        onset.onset_s, onset.direct_s, plan.direct_s
    );
    println!("Prominence      {}", opt(onset.prominence_db, 1.0, " dB"));
    println!(
        "DRR             {} (planted {:.2} dB)",
        opt(onset.drr_db, 1.0, " dB"),
        planted.drr_db
    );
    println!(
        "Pre-delay       {} (planted {:.2} ms)",
        opt(analysis.pre_delay_s, 1000.0, " ms"),
        ROOM_REFLECTIONS[0].delay_s * 1000.0
    );
    println!(
        "Mixing time     {} (the tail starts at {:.0} ms)",
        opt(analysis.echo_density.mixing_time_s, 1000.0, " ms"),
        plan.tail_start_s * 1000.0
    );
    println!(
        "Broadband floor {:.1} dB, meets the decay at {}",
        analysis.broadband.noise_floor_db,
        opt(analysis.broadband.intersection_s, 1.0, " s")
    );
    println!();

    println!(
        "Early reflections, {:.1}–{:.1} ms after the direct sound (closed by {:?}):",
        analysis.early.window_start_s * 1000.0,
        analysis.early.window_end_s * 1000.0,
        analysis.early.window_end
    );
    println!(
        "  {:>9}  {:>9}  {:>9}  {:>9}",
        "delay ms", "both dB", "left dB", "right dB"
    );
    for reflection in &analysis.early.reflections {
        let channel = |c: usize| {
            opt(
                reflection.channel_level_db.get(c).copied().flatten(),
                1.0,
                "",
            )
        };
        println!(
            "  {:>9.2}  {:>9.2}  {:>9}  {:>9}",
            reflection.delay_s * 1000.0,
            reflection.level_db,
            channel(0),
            channel(1)
        );
    }
    println!("  planted:");
    for reflection in ROOM_REFLECTIONS {
        println!(
            "  {:>9.2}  {:>9}  {:>9.2}  {:>9.2}",
            reflection.delay_s * 1000.0,
            "",
            20.0 * reflection.gain[0].log10(),
            20.0 * reflection.gain[1].log10()
        );
    }
    println!();

    println!(
        "  {:>5}  {:>7}  {:>7}  {:>7}  {:>7}  {:>8}  {:>7}  {:>7}  {:>7}  {:>6}",
        "Hz", "planted", "T20", "T30", "EDT", "floor dB", "margin", "tone", "bend dB", "1 slope"
    );
    for (band, entry) in analysis.bands.iter().enumerate() {
        match &entry.reading {
            BandReading::Measured(d) => {
                let s = d.straightness;
                println!(
                    "  {:>5}  {:>7.2}  {:>7}  {:>7}  {:>7}  {:>8.1}  {:>7.1}  {:>7}  {:>7}  {:>6}",
                    entry.centre_hz,
                    plan.t60_s[band],
                    opt(d.t20_s, 1.0, ""),
                    opt(d.t30_s, 1.0, ""),
                    opt(d.edt_s, 1.0, ""),
                    d.noise_floor_db,
                    d.margin_db,
                    opt(d.level_re_1k_db, 1.0, ""),
                    opt(s.map(|s| s.max_deviation_db), 1.0, ""),
                    s.map_or("—", |s| if s.single_slope { "yes" } else { "no" })
                );
            }
            other => println!("  {:>5}  {other:?}", entry.centre_hz),
        }
    }
    println!();

    println!(
        "Echo density (rectangular {:.0} ms window), every 5 ms:",
        analysis.echo_density.window_s * 1000.0
    );
    for point in analysis.echo_density.points.iter().step_by(5).take(21) {
        let density = point.density.unwrap_or(0.0);
        let bar = "#".repeat((density * 40.0).round().clamp(0.0, 60.0) as usize);
        println!(
            "  {:>5.0} ms  {:>5}  {bar}",
            point.time_s * 1000.0,
            opt(point.density, 1.0, "")
        );
    }
    println!();

    match &analysis.width {
        Width::Measured(c) => println!(
            "Width: IACC {:.3} (peak {:+.3} at {:+.2} ms) over {:.0}–{:.0} ms",
            c.iacc,
            c.peak_iacf,
            c.peak_lag_s * 1000.0,
            c.window_start_s * 1000.0,
            c.window_end_s * 1000.0
        ),
        Width::NotMeasured(why) => println!("Width: not measured ({why:?})"),
    }
    println!();

    println!("Validity checks (floor {:.2}):", analysis.validity.floor);
    for check in &analysis.validity.checks {
        println!(
            "  {:<40} {:>9}  score {}",
            format!("{:?}", check.kind),
            opt(check.value_db, 1.0, " dB"),
            opt(check.score, 1.0, "")
        );
    }
    println!(
        "Confidence {:.2}, weakest {:?}",
        analysis.validity.confidence, analysis.validity.weakest
    );
}
