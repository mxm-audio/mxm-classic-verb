//! `classic_verb_generate`'s pipeline (plan §2.2, §5.1): every manifest row fitted, each fit's report
//! written as `spaces/<id>.json` in the plugin crate, and the selector's positions and the factory
//! table written as the plugin's generated module — **or nothing at all**.
//!
//! Every refusal is decided before anything is written. The outputs then go to temporary files beside
//! their destinations, and only once every one is written are they renamed into place and the reports
//! of spaces the manifest no longer lists removed.

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use mxm_classic_verb_dsp::{Controls, DecayShape, EarlyTap, Engine, Space};
use mxm_classic_verb_fit::{
    Fit, FittedControls, MAX_DURATION_S, MIN_DURATION_S, VERIFY_LEAD_S, analyse_with_segments, fit,
};

use super::manifest::{self, Row};
use super::report::report;
use super::{Json, load};

/// The package the generator writes into, checked against the target's `Cargo.toml`.
pub const PLUGIN_PACKAGE: &str = "mxm-classic-verb";
/// The committed reports, relative to the plugin crate.
pub const REPORTS: &str = "spaces";
/// The generated module, relative to the plugin crate.
pub const MODULE: [&str; 3] = ["src", "spaces", "generated.rs"];

pub struct Options {
    pub manifest: PathBuf,
    /// The plugin crate the spaces are written into.
    pub plugin: PathBuf,
    /// The repository's root. The manifest and every response file must lie outside it.
    pub repository: PathBuf,
}

/// What a finished run wrote.
#[derive(Debug)]
pub struct Written {
    pub ids: Vec<String>,
    /// Reports of spaces the manifest no longer lists, removed.
    pub removed: Vec<String>,
    pub seconds: f64,
}

/// How a fit's verification render is made again without its response: the fit's own recipe
/// (`src/fit.rs`), reproduced here and **checked bit for bit** against the fit's verification analysis
/// before a report is written — analysed as the fit analyses it, its tail texture on the response's
/// segments, which the report records. The space audit renders it through the plugin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recipe {
    pub sample_rate: u32,
    pub channels: usize,
    /// Leading silence before the unit direct impulse.
    pub lead_frames: usize,
    pub frames: usize,
    /// The wet signal's scale, which gives the render the response's direct-to-reverberant ratio.
    pub wet_gain: f32,
}

impl Recipe {
    pub fn of(fit: &Fit, sample_rate: u32, response: &[Vec<f32>]) -> Recipe {
        let rate = f64::from(sample_rate);
        let samples = |seconds: f64| ((seconds * rate).round() as usize).max(1);
        let lead_frames = samples(VERIFY_LEAD_S);
        let direct = (f64::from(fit.report.target.onset.direct_s) * rate).round() as usize;
        let frames = (lead_frames + response[0].len().saturating_sub(direct))
            .max(samples(MIN_DURATION_S) + 1)
            .min((MAX_DURATION_S * rate).floor() as usize);
        Recipe {
            sample_rate,
            channels: response.len(),
            lead_frames,
            frames,
            wet_gain: fit.report.wet_gain,
        }
    }

    pub fn json(&self) -> Json {
        Json::object(vec![
            ("sample_rate", Json::Number(f64::from(self.sample_rate))),
            ("channels", Json::Number(self.channels as f64)),
            ("lead_frames", Json::Number(self.lead_frames as f64)),
            ("frames", Json::Number(self.frames as f64)),
            ("wet_gain", Json::num(self.wet_gain)),
        ])
    }
}

/// The fitted space's verification render, made as the fit makes it: the engine at the fitted centre
/// (plan §2) and Mix one, a unit impulse after `lead_frames`, the dry leak at the impulse's own frame
/// taken off, a mono response's channels folded, the wet scaled, and the unit direct sound added.
pub fn render(space: &Space, controls: &FittedControls, recipe: &Recipe) -> Vec<Vec<f32>> {
    let mut engine = Engine::new(recipe.sample_rate as f32);
    engine.reset();
    engine.set_space(*space);
    engine.set_controls(Controls {
        mix: 1.0,
        decay_s: controls.decay_s,
        bass_mult: 1.0,
        treble_mult: 1.0,
        size_s: controls.size_s,
        diffusion: controls.diffusion,
        pre_delay_s: controls.pre_delay_s,
        early_late_db: 0.0,
        shape: DecayShape::Natural,
        mod_depth_s: 0.0,
        width: 1.0,
        tone_low_db: 0.0,
        tone_high_db: 0.0,
        duck: 0.0,
        ..Controls::default()
    });
    let (lead, frames) = (recipe.lead_frames, recipe.frames);
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let dry = core::f32::consts::FRAC_PI_2.cos();
    for n in lead..frames {
        let x = if n == lead { 1.0 } else { 0.0 };
        let (l, r) = engine.process(x, x);
        let (l, r) = if n == lead {
            (l - dry, r - dry)
        } else {
            (l, r)
        };
        left[n] = l;
        right[n] = r;
        if n > lead && engine.is_parked() {
            break;
        }
    }
    let wet = if recipe.channels == 1 {
        vec![
            left.iter()
                .zip(&right)
                .map(|(l, r)| 0.5 * (l + r))
                .collect(),
        ]
    } else {
        vec![left, right]
    };
    wet.into_iter()
        .map(|channel| {
            let mut v: Vec<f32> = channel.iter().map(|&x| x * recipe.wet_gain).collect();
            v[lead] += 1.0;
            v
        })
        .collect()
}

struct Generated {
    row: Row,
    fit: Fit,
    report: String,
}

/// The whole run. `log` hears each space as it is fitted.
pub fn run(options: &Options, log: &mut dyn FnMut(String)) -> Result<Written, Vec<String>> {
    let started = Instant::now();
    check_plugin(&options.plugin).map_err(|problem| vec![problem])?;
    if inside(&options.repository, &options.manifest) {
        return Err(vec![
            "the manifest lies inside the repository; it names response files and lives outside it"
                .to_owned(),
        ]);
    }
    let text = std::fs::read_to_string(&options.manifest)
        .map_err(|e| vec![format!("the manifest could not be read: {e}")])?;
    let rows = manifest::parse(&text)?;
    let outside: Vec<String> = rows
        .iter()
        .filter(|row| inside(&options.repository, &row.response))
        .map(|row| {
            format!(
                "line {}: {}: the response file lies inside the repository",
                row.line, row.id
            )
        })
        .collect();
    if !outside.is_empty() {
        return Err(outside);
    }

    let mut problems = Vec::new();
    let mut generated = Vec::new();
    for row in rows {
        match generate(&row, log) {
            Ok(done) => generated.push(done),
            Err(problem) => {
                log(format!("{}: refused: {problem}", row.id));
                problems.push(format!("line {}: {}: {problem}", row.line, row.id));
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let module = module(
        &generated
            .iter()
            .map(|g| (&g.row, &g.fit.space, &g.fit.controls))
            .collect::<Vec<_>>(),
    );
    for g in &generated {
        if let Some(found) = names_the_response(&g.report, &g.row.response, true)
            .or_else(|| names_the_response(&module, &g.row.response, false))
        {
            problems.push(format!(
                "line {}: {}: the output would carry `{found}`, which names the response or its folder",
                g.row.line, g.row.id
            ));
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let removed = commit(&options.plugin, &generated, &module)?;
    Ok(Written {
        ids: generated.iter().map(|g| g.row.id.clone()).collect(),
        removed,
        seconds: started.elapsed().as_secs_f64(),
    })
}

fn generate(row: &Row, log: &mut dyn FnMut(String)) -> Result<Generated, String> {
    let decoded = load(&row.response).map_err(|e| format!("the response {e}"))?;
    let channels: Vec<&[f32]> = decoded.channels.iter().map(Vec::as_slice).collect();
    let started = Instant::now();
    let fitted = fit(&channels, decoded.sample_rate as f32)
        .map_err(|refusal| format!("the fit refused: {refusal}"))?;
    if let Some(refusal) = &fitted.report.verification_refusal {
        return Err(format!(
            "the fitted space's verification render would be refused: {refusal}"
        ));
    }
    let recipe = Recipe::of(&fitted, decoded.sample_rate, &decoded.channels);
    let rendered = render(&fitted.space, &fitted.controls, &recipe);
    let slices: Vec<&[f32]> = rendered.iter().map(Vec::as_slice).collect();
    match analyse_with_segments(
        &slices,
        decoded.sample_rate as f32,
        &fitted.report.target.texture_segments(),
    ) {
        Ok(analysis) if analysis == fitted.report.verification => {}
        Ok(_) => {
            return Err(
                "the recorded render recipe does not reproduce the fit's verification; the fit's own recipe has changed and this generator must follow it"
                    .to_owned(),
            );
        }
        Err(refusal) => return Err(format!("the recorded render was refused: {refusal}")),
    }
    log(format!(
        "{}: confidence {:.2}, decay {:.2} s, size {:.1} ms, diffusion {:.2}, pre-delay {:.1} ms, {} clamp(s), fitted in {:.2} s",
        row.id,
        fitted.report.target.validity.confidence,
        fitted.controls.decay_s,
        1000.0 * fitted.controls.size_s,
        fitted.controls.diffusion,
        1000.0 * fitted.controls.pre_delay_s,
        fitted.report.clamps.len(),
        started.elapsed().as_secs_f64(),
    ));
    let mut json = report(&row.id, decoded.sample_rate, &fitted);
    if let Json::Object(fields) = &mut json {
        fields.push(("render".to_owned(), recipe.json()));
    }
    Ok(Generated {
        row: row.clone(),
        report: json.render(),
        fit: fitted,
    })
}

/// Refuses a folder that is not the plugin crate, so a mistyped `--plugin` writes nothing.
fn check_plugin(plugin: &Path) -> Result<(), String> {
    let manifest = plugin.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("the plugin crate's Cargo.toml could not be read: {e}"))?;
    let expected = format!("name = \"{PLUGIN_PACKAGE}\"");
    if text.lines().any(|line| line.trim() == expected) {
        Ok(())
    } else {
        Err(format!(
            "the folder given as the plugin crate is not `{PLUGIN_PACKAGE}`"
        ))
    }
}

/// A path made absolute through its nearest existing ancestor, so a folder not yet created still
/// resolves.
fn resolved(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let mut rest = Vec::new();
    let mut ancestor = absolute.as_path();
    loop {
        if let Ok(real) = ancestor.canonicalize() {
            let mut out = real;
            for component in rest.iter().rev() {
                out.push(component);
            }
            return Some(out);
        }
        match (ancestor.file_name(), ancestor.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(PathBuf::from(name));
                ancestor = parent;
            }
            _ => return None,
        }
    }
}

/// Whether `path` lies inside `root`, both resolved.
pub fn inside(root: &Path, path: &Path) -> bool {
    match (resolved(root), resolved(path)) {
        (Some(root), Some(path)) => {
            let normal = |p: &Path| -> Vec<String> {
                p.components()
                    .filter(|c| !matches!(c, Component::CurDir))
                    .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                    .collect()
            };
            let (root, path) = (normal(&root), normal(&path));
            path.len() >= root.len() && path[..root.len()] == root[..]
        }
        _ => false,
    }
}

/// Something in `text` that names the response — its path as given or resolved, its file name, or
/// (in a report) its folder's name — or, in a report, any path separator at all.
fn names_the_response(text: &str, response: &Path, report: bool) -> Option<String> {
    let mut needles = vec![response.to_string_lossy().into_owned()];
    if let Some(real) = resolved(response) {
        needles.push(real.to_string_lossy().into_owned());
    }
    if let Some(name) = response.file_name() {
        needles.push(name.to_string_lossy().into_owned());
    }
    if report {
        if let Some(folder) = response.parent().and_then(Path::file_name) {
            needles.push(folder.to_string_lossy().into_owned());
        }
        needles.push("/".to_owned());
        needles.push("\\".to_owned());
    }
    needles
        .into_iter()
        .find(|needle| !needle.is_empty() && text.contains(needle.as_str()))
}

fn literal(value: f32) -> String {
    assert!(value.is_finite(), "a fitted value is not finite: {value}");
    format!("{value:?}")
}

fn tap(t: &EarlyTap) -> String {
    format!(
        "tap({}, {}, {})",
        literal(t.time),
        literal(t.gain_l),
        literal(t.gain_r)
    )
}

const HEADER: &str = r#"//! **Generated by `classic_verb_generate`. Do not edit by hand.**
//!
//! The factory spaces and the Space selector's positions, in the order of the manifest they were
//! generated from, with `Loaded` last (plan §2.2). Each space's fit report is `spaces/<id>.json` in
//! this crate, and `classic_verb_space_audit` renders every space here against it. Regenerating is a
//! deliberate run by hand, from a manifest that lives outside the repository
//! (`crates/mxm-classic-verb-fit/AGENTS.md`, *The factory-space generator*):
//!
//! ```bash
//! cargo run -p mxm-classic-verb-fit --release --example classic_verb_generate -- --manifest <manifest>
//! cargo test -p mxm-classic-verb
//! cargo run -p mxm-classic-verb --release --example classic_verb_space_audit
//! ```
// A fitted number can land on a named constant's leading digits by chance.
#![allow(clippy::approx_constant)]

use mxm_classic_verb_dsp::Space;
use nice_plug::prelude::Enum;

use super::{FactorySpace, SpaceFamily, tap};

/// The Space selector's positions: every factory space in manifest order, then `Loaded` (plan §2.2).
/// **The order is permanent once released**, because a preset stores the normalised position.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceChoice {
"#;

/// The generated module: the selector's enum and the factory table, formatted as `rustfmt` leaves it.
pub fn module(entries: &[(&Row, &Space, &FittedControls)]) -> String {
    let mut out = String::from(HEADER);
    for (row, _, _) in entries {
        let _ = writeln!(out, "    #[id = {:?}]", row.id);
        let _ = writeln!(out, "    #[name = {:?}]", row.name);
        let _ = writeln!(out, "    {},", manifest::variant(&row.id));
    }
    let _ = writeln!(out, "    #[id = {:?}]", manifest::LOADED_ID);
    let _ = writeln!(out, "    #[name = {:?}]", manifest::LOADED_NAME);
    out.push_str("    Loaded,\n}\n\n");
    out.push_str(
        "/// Every factory space in the selector's order: its permanent id, its display name, its family,\n\
         /// the space as fitted, and the fitted Decay, Size, Diffusion and Pre-delay.\n\
         pub const FACTORY: &[FactorySpace] = &[\n",
    );
    for (row, space, controls) in entries {
        out.push_str("    FactorySpace {\n");
        let _ = writeln!(out, "        id: {:?},", row.id);
        let _ = writeln!(out, "        name: {:?},", row.name);
        let _ = writeln!(out, "        family: SpaceFamily::{},", row.family_variant);
        out.push_str("        space: Space {\n            early: [\n");
        for t in &space.early {
            let _ = writeln!(out, "                {},", tap(t));
        }
        out.push_str("            ],\n");
        for (field, value) in [
            ("early_level", space.early_level),
            ("decay_ratio_low", space.decay_ratio_low),
            ("decay_ratio_high", space.decay_ratio_high),
            ("decay_ratio_top", space.decay_ratio_top),
            ("tone_low_db", space.tone_low_db),
            ("tone_high_db", space.tone_high_db),
            ("high_cut_hz", space.high_cut_hz),
            ("width", space.width),
        ] {
            let _ = writeln!(out, "            {field}: {},", literal(value));
        }
        out.push_str("        },\n");
        for (field, value) in [
            ("decay_s", controls.decay_s),
            ("size_s", controls.size_s),
            ("diffusion", controls.diffusion),
            ("pre_delay_s", controls.pre_delay_s),
        ] {
            let _ = writeln!(out, "        {field}: {},", literal(value));
        }
        out.push_str("    },\n");
    }
    out.push_str("];\n");
    out
}

/// Writes every output to a temporary file beside its destination, then renames them into place and
/// removes the reports of spaces no longer listed. **A failed write removes every temporary file and
/// changes nothing.** A failed rename cannot be undone across files, and says how far it got.
fn commit(
    plugin: &Path,
    generated: &[Generated],
    module: &str,
) -> Result<Vec<String>, Vec<String>> {
    let reports = plugin.join(REPORTS);
    let module_path = MODULE.iter().fold(plugin.to_path_buf(), |p, c| p.join(c));
    for folder in [
        reports.as_path(),
        module_path.parent().expect("the module sits in a folder"),
    ] {
        std::fs::create_dir_all(folder)
            .map_err(|e| vec![format!("a folder could not be created: {e}")])?;
    }

    let mut outputs: Vec<(PathBuf, &str)> = generated
        .iter()
        .map(|g| {
            (
                reports.join(format!("{}.json", g.row.id)),
                g.report.as_str(),
            )
        })
        .collect();
    outputs.push((module_path, module));
    let temporary = |destination: &Path| {
        let mut name = destination.as_os_str().to_owned();
        name.push(".tmp");
        PathBuf::from(name)
    };

    let mut written: Vec<PathBuf> = Vec::new();
    for (destination, text) in &outputs {
        let staged = temporary(destination);
        if let Err(e) = std::fs::write(&staged, text) {
            for file in written.iter().chain(std::iter::once(&staged)) {
                let _ = std::fs::remove_file(file);
            }
            return Err(vec![format!(
                "an output could not be written, and nothing was changed: {e}"
            )]);
        }
        written.push(staged);
    }
    for (done, (destination, _)) in outputs.iter().enumerate() {
        if let Err(e) = std::fs::rename(temporary(destination), destination) {
            return Err(vec![format!(
                "renaming into place failed after {done} of {} files, so the plugin is partly regenerated; run again: {e}",
                outputs.len()
            )]);
        }
    }

    let ids: Vec<String> = generated
        .iter()
        .map(|g| format!("{}.json", g.row.id))
        .collect();
    let mut removed = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&reports) {
        let mut stale: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_report = name.ends_with(".json") || name.ends_with(".json.tmp");
                (is_report && !ids.contains(&name)).then(|| (name, entry.path()))
            })
            .collect();
        stale.sort();
        for (name, path) in stale {
            if std::fs::remove_file(&path).is_ok() {
                removed.push(name);
            }
        }
    }
    Ok(removed)
}
