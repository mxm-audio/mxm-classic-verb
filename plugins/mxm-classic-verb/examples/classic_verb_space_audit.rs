//! Renders every factory space through the plugin and measures it against its committed report (plan
//! §5.1; `plugins/mxm-classic-verb/AGENTS.md`, *The space audit*).
//!
//! ```bash
//! cargo run -p mxm-classic-verb --release --example classic_verb_space_audit
//! ```
//!
//! Prints each space's worst movement per descriptor, the worst across spaces and the tolerances, and
//! **exits non-zero on any miss**, naming the space and the descriptor. `tests/space_audit.rs` runs the
//! same audit by default.

#[path = "../tests/audit/mod.rs"]
mod audit;

use std::process::ExitCode;
use std::time::Instant;

use audit::{Figures, TOLERANCES, audit_all, orphaned_reports};

fn main() -> ExitCode {
    let started = Instant::now();
    let audited = audit_all(&TOLERANCES);
    println!("{:<24} {:<8} {}", "space", "seconds", Figures::HEADER);
    let mut worst = Figures::default();
    for space in &audited {
        println!(
            "{:<24} {:<8.2} {}",
            space.id,
            space.seconds,
            space.figures.row()
        );
        worst = worst.worst(space.figures);
    }
    let t = TOLERANCES;
    let tolerance = Figures {
        decay_percent: t.decay_percent,
        tone_db: t.tone_db,
        drr_db: t.drr_db,
        pre_delay_s: t.pre_delay_s,
        mixing_time_s: t.mixing_time_s,
        iacc: t.iacc,
        profile: t.profile,
        profile_one_sided: t.profile_one_sided,
        reflection_s: t.reflection_s,
        reflection_db: t.reflection_db,
        unmatched_reflections: 0,
        confidence: t.confidence,
        peakiness: t.peakiness,
        kurtosis: t.kurtosis,
        texture_density: t.texture_density,
        periodicity: t.periodicity,
    };
    println!("{:<24} {:<8} {}", "worst", "", worst.row());
    println!("{:<24} {:<8} {}", "tolerance", "", tolerance.row());

    let mut misses = orphaned_reports();
    for space in &audited {
        misses.extend(space.misses.iter().cloned());
    }
    println!(
        "{} factory spaces audited in {:.1} s",
        audited.len(),
        started.elapsed().as_secs_f64()
    );
    if misses.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!("{} miss(es):", misses.len());
        for miss in &misses {
            eprintln!("  {miss}");
        }
        ExitCode::FAILURE
    }
}
