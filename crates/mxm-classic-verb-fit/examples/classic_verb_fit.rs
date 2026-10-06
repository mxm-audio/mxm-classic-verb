//! Fits `mxm-classic-verb` spaces to impulse responses on disk (plan §5.1).
//!
//! ```bash
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_fit -- --out <folder> <file or folder>...
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_fit -- --out <folder> --name small-room <file>
//! ```
//!
//! For each WAV or AIFF response it writes `<name>.json` — the fitted space, the four controls, the
//! target descriptors, the verification's descriptors and the error for every one — and a
//! `summary.tsv` ranked by confidence, with refusals after. **A report names its source only by a
//! logical name**: the file's stem as a slug, or `--name`. Never a path. There is no default output
//! folder, so nothing is written anywhere, the repository included, unless asked.

#[path = "classic_verb_io/mod.rs"]
mod classic_verb_io;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use classic_verb_io::report::report;
use classic_verb_io::{collect, load, slug};
use mxm_classic_verb_fit::{Fit, fit};

const USAGE: &str = "usage: classic_verb_fit --out <folder> [--name <name>] <file or folder>...";

struct Fitted {
    name: String,
    fit: Fit,
    seconds: f64,
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut out, mut name, mut inputs) = (None, None, Vec::new());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--name" => name = args.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => inputs.push(PathBuf::from(arg)),
        }
    }
    let Some(out) = out else {
        eprintln!("{USAGE}\n--out is required: nothing is written anywhere by default");
        return ExitCode::from(2);
    };
    let files = collect(&inputs);
    if files.is_empty() {
        eprintln!("{USAGE}\nno WAV or AIFF file found");
        return ExitCode::from(2);
    }
    if name.is_some() && files.len() != 1 {
        eprintln!("--name names one response; {} were found", files.len());
        return ExitCode::from(2);
    }
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("the output folder could not be created: {e}");
        return ExitCode::from(1);
    }

    let mut used: Vec<String> = Vec::new();
    let mut fitted: Vec<Fitted> = Vec::new();
    let mut refused: Vec<(String, String)> = Vec::new();
    for path in &files {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        let base = slug(name.as_deref().or(stem.as_deref()).unwrap_or("response"));
        let mut unique = base.clone();
        let mut n = 2;
        while used.contains(&unique) {
            unique = format!("{base}-{n}");
            n += 1;
        }
        used.push(unique.clone());

        let decoded = match load(path) {
            Ok(decoded) => decoded,
            Err(e) => {
                println!("{unique}: {e}");
                refused.push((unique, e.to_string()));
                continue;
            }
        };
        let channels: Vec<&[f32]> = decoded.channels.iter().map(Vec::as_slice).collect();
        let started = Instant::now();
        let result = fit(&channels, decoded.sample_rate as f32);
        let seconds = started.elapsed().as_secs_f64();
        match result {
            Ok(result) => {
                let json = report(&unique, decoded.sample_rate, &result);
                if let Err(e) = std::fs::write(out.join(format!("{unique}.json")), json.render()) {
                    eprintln!("{unique}: the report could not be written: {e}");
                    return ExitCode::from(1);
                }
                println!(
                    "{unique}: confidence {:.2}, decay {:.2} s, size {:.1} ms, diffusion {:.2}, pre-delay {:.1} ms, fitted in {seconds:.2} s",
                    result.report.target.validity.confidence,
                    result.controls.decay_s,
                    1000.0 * result.controls.size_s,
                    result.controls.diffusion,
                    1000.0 * result.controls.pre_delay_s,
                );
                fitted.push(Fitted {
                    name: unique,
                    fit: result,
                    seconds,
                });
            }
            Err(e) => {
                println!("{unique}: refused: {e}");
                refused.push((unique, format!("refused: {e}")));
            }
        }
    }

    fitted.sort_by(|a, b| {
        b.fit
            .report
            .target
            .validity
            .confidence
            .total_cmp(&a.fit.report.target.validity.confidence)
            .then(a.name.cmp(&b.name))
    });
    let mut table = String::from(
        "rank\tname\tconfidence\tdecay_s\tsize_ms\tdiffusion\tpre_delay_ms\tfirst_arrival\tworst_t30_error_pct\tprofile_distance\tenvelope_error_db\tiacc_error\tearly_to_late_error_db\tclamps\tverification\tfit_s\n",
    );
    for (rank, f) in fitted.iter().enumerate() {
        let e = &f.fit.report.errors;
        let worst = e
            .bands
            .iter()
            .filter_map(|b| b.t30_percent)
            .fold(None, |w: Option<f32>, v| {
                Some(w.map_or(v.abs(), |w| w.max(v.abs())))
            });
        let cell = |v: Option<f32>| v.map_or("-".to_string(), |v| format!("{v:.3}"));
        let verification = f
            .fit
            .report
            .verification_refusal
            .as_ref()
            .map_or("measured".to_string(), |r| {
                format!("render would be refused: {r}")
            });
        table.push_str(&format!(
            "{}\t{}\t{:.3}\t{:.3}\t{:.2}\t{:.3}\t{:.2}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\n",
            rank + 1,
            f.name,
            f.fit.report.target.validity.confidence,
            f.fit.controls.decay_s,
            1000.0 * f.fit.controls.size_s,
            f.fit.controls.diffusion,
            1000.0 * f.fit.controls.pre_delay_s,
            f.fit.report.early.first_arrival,
            cell(worst),
            cell(e.profile_distance),
            cell(e.envelope_db),
            cell(e.iacc),
            cell(e.early_to_late_db),
            f.fit.report.clamps.len(),
            verification,
            f.seconds,
        ));
    }
    for (name, reason) in &refused {
        table.push_str(&format!("-\t{name}\t{reason}\n"));
    }
    if let Err(e) = std::fs::write(out.join("summary.tsv"), table) {
        eprintln!("the summary could not be written: {e}");
        return ExitCode::from(1);
    }
    let times: Vec<f64> = fitted.iter().map(|f| f.seconds).collect();
    let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
    let slowest = times.iter().copied().fold(0.0, f64::max);
    println!(
        "{} fitted, {} refused; fit time mean {mean:.2} s, slowest {slowest:.2} s",
        fitted.len(),
        refused.len()
    );
    ExitCode::SUCCESS
}
