//! The factory-space audit as a default test (plan §5.1; `AGENTS.md`, *The space audit*): every
//! factory space renders through the plugin as its committed report recorded, and a space moved by a
//! small fraction of a just-noticeable difference is caught and named.

#[path = "audit/mod.rs"]
mod audit;

use audit::{Figures, TOLERANCES, Through, Tolerances, audit_all, audit_with, orphaned_reports};
use mxm_classic_verb::spaces::{FACTORY, FactorySpace};

#[test]
fn every_factory_space_renders_as_its_report_recorded() {
    let audited = audit_all(&TOLERANCES);
    assert_eq!(audited.len(), FACTORY.len());
    let mut misses = orphaned_reports();
    for space in &audited {
        misses.extend(space.misses.iter().cloned());
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

type Change = fn(&mut FactorySpace);

/// Each change is well under a just-noticeable difference, and the audit still names the descriptor
/// it moved: a tolerance loose enough to miss these would let a DSP change move every factory space
/// unnoticed.
#[test]
fn a_moved_space_is_caught_and_the_descriptor_named() {
    let cases: [(&str, Change, &str); 5] = [
        ("Decay 1 % longer", |e| e.decay_s *= 1.01, "T30"),
        (
            "Size 1 % larger",
            |e| e.size_s *= 1.01,
            "the echo density profile",
        ),
        (
            "the high shelf 0.1 dB up",
            |e| e.space.tone_high_db += 0.1,
            "the level against 1 kHz",
        ),
        (
            "Pre-delay 0.1 ms later",
            |e| e.pre_delay_s += 0.0001,
            "the pre-delay",
        ),
        (
            "the width 1 % narrower",
            |e| e.space.width *= 0.99,
            "the IACC",
        ),
    ];
    let index = shortest();
    for (what, change, named) in cases {
        let mut entry = FACTORY[index];
        change(&mut entry);
        let audited = audit_with(index, &entry, Through::Position, &TOLERANCES);
        assert!(
            audited.misses.iter().any(|miss| miss.contains(named)),
            "{what} on {} was not named as `{named}`: {:?}",
            audited.id,
            audited.misses
        );
    }
}

/// The factory space with the shortest committed render, so the changed renders cost least.
fn shortest() -> usize {
    (0..FACTORY.len())
        .min_by_key(|&index| {
            audit::committed(&FACTORY[index]).map_or(usize::MAX, |c| c.recipe.frames)
        })
        .expect("at least one factory space")
}

/// **How the tolerances were chosen.** Every factory space rendered as generated, again through
/// `Loaded`, and again after each small change to its controls or its space, with every descriptor's
/// worst movement printed. Run it in release and read it against `AGENTS.md`, *The space audit*:
///
/// ```bash
/// cargo test -p mxm-classic-verb --release --test space_audit -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, printed for AGENTS.md"]
fn measure_what_small_changes_move() {
    let unbounded = Tolerances {
        decay_percent: f32::INFINITY,
        tone_db: f32::INFINITY,
        drr_db: f32::INFINITY,
        pre_delay_s: f32::INFINITY,
        mixing_time_s: f32::INFINITY,
        iacc: f32::INFINITY,
        profile: f32::INFINITY,
        profile_one_sided: usize::MAX,
        reflection_s: f32::INFINITY,
        reflection_db: f32::INFINITY,
        confidence: f32::INFINITY,
        peakiness: f32::INFINITY,
        kurtosis: f32::INFINITY,
        texture_density: f32::INFINITY,
        periodicity: f32::INFINITY,
    };
    let changes: [(&str, Change, Through); 13] = [
        ("as generated", |_| {}, Through::Position),
        ("as generated, via Loaded", |_| {}, Through::Loaded),
        ("Decay x1.01", |e| e.decay_s *= 1.01, Through::Position),
        ("Size x1.01", |e| e.size_s *= 1.01, Through::Position),
        (
            "Diffusion -0.01",
            |e| e.diffusion = (e.diffusion - 0.01).max(0.0),
            Through::Position,
        ),
        (
            "Pre-delay +0.1 ms",
            |e| e.pre_delay_s += 0.0001,
            Through::Position,
        ),
        (
            "high shelf +0.1 dB",
            |e| e.space.tone_high_db += 0.1,
            Through::Loaded,
        ),
        (
            "low shelf +0.1 dB",
            |e| e.space.tone_low_db += 0.1,
            Through::Loaded,
        ),
        (
            "high decay ratio x1.01",
            |e| e.space.decay_ratio_high *= 1.01,
            Through::Loaded,
        ),
        (
            "top decay ratio x1.01",
            |e| e.space.decay_ratio_top *= 1.01,
            Through::Loaded,
        ),
        ("width x0.99", |e| e.space.width *= 0.99, Through::Loaded),
        (
            "early level x1.02",
            |e| e.space.early_level *= 1.02,
            Through::Loaded,
        ),
        (
            "taps 0.01 Size later",
            |e| {
                for tap in &mut e.space.early {
                    tap.time += 0.01;
                }
            },
            Through::Loaded,
        ),
    ];
    println!("{:<26} {:<9} {}", "change", "space", Figures::HEADER);
    for (what, change, through) in changes {
        let mut worst = Figures::default();
        for (index, table) in FACTORY.iter().enumerate() {
            let mut entry = *table;
            change(&mut entry);
            let audited = audit_with(index, &entry, through, &unbounded);
            let presence: Vec<&String> = audited
                .misses
                .iter()
                .filter(|miss| miss.contains("when generated") || miss.contains("no longer"))
                .collect();
            println!(
                "{what:<26} {:<9} {}{}",
                table.id,
                audited.figures.row(),
                if presence.is_empty() {
                    String::new()
                } else {
                    format!("  presence: {presence:?}")
                }
            );
            worst = worst.worst(audited.figures);
        }
        println!("{what:<26} {:<9} {}", "worst", worst.row());
    }
}
