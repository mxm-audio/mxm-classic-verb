//! Generates `mxm-classic-verb`'s factory spaces from a manifest of impulse responses (plan §2.2,
//! §5.1; `crates/mxm-classic-verb-fit/AGENTS.md`, *The factory-space generator*).
//!
//! ```bash
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_generate -- --manifest <manifest outside the repository>
//! ```
//!
//! Every row is fitted. Its report is written as `plugins/mxm-classic-verb/spaces/<id>.json`, and the
//! selector's positions with the factory table as `plugins/mxm-classic-verb/src/spaces/generated.rs`.
//! **Any refusal refuses the whole run, and nothing is written.** `--plugin <folder>` writes into
//! another copy of the plugin crate instead.

#[path = "classic_verb_io/mod.rs"]
mod classic_verb_io;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use classic_verb_io::generate::{Options, run};

const USAGE: &str = "usage: classic_verb_generate --manifest <file outside the repository> [--plugin <plugin crate folder>]";

fn main() -> ExitCode {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let mut args = std::env::args().skip(1);
    let (mut manifest, mut plugin) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => manifest = args.next().map(PathBuf::from),
            "--plugin" => plugin = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("{USAGE}\nunexpected argument `{other}`");
                return ExitCode::from(2);
            }
        }
    }
    let Some(manifest) = manifest else {
        eprintln!("{USAGE}\n--manifest is required");
        return ExitCode::from(2);
    };
    let options = Options {
        manifest,
        plugin: plugin.unwrap_or_else(|| repository.join("plugins").join("mxm-classic-verb")),
        repository,
    };
    match run(&options, &mut |line| println!("{line}")) {
        Ok(written) => {
            println!(
                "{} factory spaces generated in {:.1} s, in this order: {}",
                written.ids.len(),
                written.seconds,
                written.ids.join(", ")
            );
            if !written.removed.is_empty() {
                println!(
                    "removed the reports of spaces no longer listed: {}",
                    written.removed.join(", ")
                );
            }
            ExitCode::SUCCESS
        }
        Err(problems) => {
            eprintln!("the run was refused:");
            for problem in problems {
                eprintln!("  {problem}");
            }
            ExitCode::from(1)
        }
    }
}
