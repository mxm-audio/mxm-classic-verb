//! `classic_verb_generate`: the manifest and every refusal it names, a refused run that writes
//! nothing, and one whole run on a synthetic response into a temporary copy of the plugin's folders
//! (`AGENTS.md`, *The factory-space generator*). Nothing here reads a file it did not write.

#[path = "../examples/classic_verb_io/mod.rs"]
mod classic_verb_io;
mod known;

use std::path::{Path, PathBuf};

use classic_verb_io::generate::{Options, inside, run};
use classic_verb_io::manifest::{parse, variant};
use known::{Known, render};
use mxm_classic_verb_dsp::Space;

/// A folder in the temporary directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "mxm-classic-verb-generate-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch folder is created");
        Scratch(dir)
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("its folder");
        std::fs::write(&path, text).expect("the file is written");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every file under `dir`, relative, with its contents, sorted.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).expect("readable").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("under the root")
                    .to_path_buf();
                out.push((relative, std::fs::read(&path).expect("readable")));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

const PLUGIN_TOML: &str = "[package]\nname = \"mxm-classic-verb\"\n";

#[test]
fn a_manifest_row_is_an_id_a_name_a_family_and_a_file() {
    let rows = parse(
        "\u{feff}# id\tname\tfamily\tresponse\r\n\r\nsmall-room\t Small room \tdrum room\tD:\\responses\\a response.aif\r\nplate-2\tPlate two\tplate\t/responses/b.wav\n",
    )
    .expect("the manifest parses");
    assert_eq!(rows.len(), 2);
    let first = &rows[0];
    assert_eq!(
        (
            first.line,
            first.id.as_str(),
            first.name.as_str(),
            first.family,
            first.family_variant
        ),
        (3, "small-room", "Small room", "drum room", "DrumRoom")
    );
    assert_eq!(
        first.response,
        PathBuf::from("D:\\responses\\a response.aif")
    );
    assert_eq!((rows[1].line, rows[1].family_variant), (4, "Plate"));
    assert_eq!(variant("small-room"), "SmallRoom");
    assert_eq!(variant("plate-2"), "Plate2");
}

#[test]
fn every_manifest_mistake_is_named_with_its_line() {
    for (text, says) in [
        (
            "room\tRoom\troom",
            "line 1: expected four tab-separated fields",
        ),
        ("\tRoom\troom\tx.wav", "line 1: the id is empty"),
        ("rooms/big\tRoom\troom\tx.wav", "contains a path separator"),
        ("rooms\\big\tRoom\troom\tx.wav", "contains a path separator"),
        (
            "Room\tRoom\troom\tx.wav",
            "is not lowercase letters and digits",
        ),
        (
            "2-rooms\tRoom\troom\tx.wav",
            "is not lowercase letters and digits",
        ),
        (
            "big--room\tRoom\troom\tx.wav",
            "is not lowercase letters and digits",
        ),
        (
            "big room\tRoom\troom\tx.wav",
            "is not lowercase letters and digits",
        ),
        ("loaded\tAnything\troom\tx.wav", "`Loaded` position"),
        ("self\tMyself\troom\tx.wav", "which is a keyword"),
        ("room\t\troom\tx.wav", "the name is empty"),
        ("room\tBig/Room\troom\tx.wav", "contains a path separator"),
        (
            "room\tBig\u{7}Room\troom\tx.wav",
            "contains a control character",
        ),
        ("room\tLOADED\troom\tx.wav", "`Loaded` position"),
        (
            "room\tRoom\tcathedral\tx.wav",
            "is not one of: room, ambience",
        ),
        ("room\tRoom\tRoom\tx.wav", "is not one of"),
        ("room\tRoom\troom\t", "no response file is named"),
        ("", "names no space"),
        ("# only a comment\n\n", "names no space"),
        ("ok\tFine\troom\tx.wav\nbad id\tBad\troom\tx.wav", "line 2:"),
    ] {
        let problems = parse(text).expect_err(text);
        assert!(
            problems.iter().any(|problem| problem.contains(says)),
            "{text:?} gave {problems:?}"
        );
    }
}

#[test]
fn a_repeated_id_name_or_variant_refuses_the_manifest() {
    let problems = parse(
        "room\tRoom\troom\ta.wav\nroom\tAnother\troom\tb.wav\nbig\troom\thall\tc.wav\na1\tA\troom\td.wav\na-1\tB\troom\te.wav\n",
    )
    .expect_err("repeats refuse");
    for says in [
        "line 2: the id `room` repeats line 1",
        "line 3: the name `room` repeats line 1's `Room`",
        "line 5: the id `a-1` names the variant `A1`, as line 4's `a1` does",
    ] {
        assert!(
            problems.iter().any(|problem| problem.contains(says)),
            "{says}: {problems:?}"
        );
    }
}

#[test]
fn inside_compares_whole_folders_and_reads_through_ones_not_yet_made() {
    let scratch = Scratch::new("inside");
    let root = &scratch.0;
    assert!(inside(root, root));
    assert!(inside(root, &root.join("not").join("made").join("yet.wav")));
    std::fs::create_dir_all(root.join("a")).expect("a folder");
    assert!(!inside(&root.join("a"), &root.join("ab")));
    assert!(!inside(&root.join("a"), &root.join("b")));
    assert!(inside(&repository(), Path::new(env!("CARGO_MANIFEST_DIR"))));
    assert!(!inside(&repository(), root));
}

#[test]
fn a_refused_run_writes_nothing() {
    let scratch = Scratch::new("refused");
    let plugin = scratch.0.join("plugin");
    scratch.write("plugin/Cargo.toml", PLUGIN_TOML);
    scratch.write("plugin/spaces/old.json", "{ \"sentinel\": true }\n");
    scratch.write("plugin/src/spaces/generated.rs", "// sentinel\n");
    let before = snapshot(&plugin);
    let options = |manifest: PathBuf, plugin: PathBuf| Options {
        manifest,
        plugin,
        repository: repository(),
    };

    // Absolute, in the scratch folder: a relative path resolves against the working directory, which
    // for a test is inside the repository and refused before it is read.
    let missing = scratch.write(
        "missing.tsv",
        &format!(
            "room\tRoom\troom\t{}\nhall\tHall\thall\t{}\n",
            scratch.0.join("no-such-response.wav").display(),
            scratch.0.join("nor-this.aif").display()
        ),
    );
    let problems = run(&options(missing.clone(), plugin.clone()), &mut |_| {})
        .expect_err("files that cannot be read refuse");
    assert_eq!(
        problems.len(),
        2,
        "every row's refusal is named: {problems:?}"
    );
    assert!(problems[0].starts_with("line 1: room: the response could not be read"));

    let repeated = scratch.write(
        "repeated.tsv",
        "room\tRoom\troom\ta.wav\nroom\tHall\thall\tb.wav\n",
    );
    let problems =
        run(&options(repeated, plugin.clone()), &mut |_| {}).expect_err("a repeated id refuses");
    assert!(problems[0].contains("repeats line 1"), "{problems:?}");

    let problems = run(&options(missing.clone(), scratch.0.clone()), &mut |_| {})
        .expect_err("a folder that is not the plugin crate refuses");
    assert!(problems[0].contains("Cargo.toml"), "{problems:?}");

    let problems = run(
        &options(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
            plugin.clone(),
        ),
        &mut |_| {},
    )
    .expect_err("a manifest inside the repository refuses");
    assert!(
        problems[0].contains("inside the repository"),
        "{problems:?}"
    );

    assert_eq!(
        snapshot(&plugin),
        before,
        "a refused run changed the plugin"
    );
}

/// **One whole run**: a synthetic response in a folder and under a file name the reports must not
/// carry, a stale report the run removes, and a file beside the reports it leaves alone.
#[test]
fn a_run_writes_each_report_by_id_and_the_module_and_names_no_file() {
    let scratch = Scratch::new("whole");
    let response = render(&Known {
        space: Space::CHAMBER,
        decay_s: 0.8,
        size_s: 0.02,
        diffusion: 0.7,
        pre_delay_s: 0.006,
        rate: 44_100.0,
        mono: true,
        seconds: 1.5,
    });
    let wav = scratch.0.join("Response Folder").join("Some Response.wav");
    std::fs::create_dir_all(wav.parent().expect("a folder")).expect("its folder");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).expect("a WAV writer");
    for &sample in &response[0] {
        writer.write_sample(sample).expect("a sample");
    }
    writer.finalize().expect("a finished WAV");

    let manifest = scratch.write(
        "manifest.tsv",
        &format!(
            "# id\tname\tfamily\tresponse\nsmall-room\tSmall \"room\"\troom\t{}\n",
            wav.display()
        ),
    );
    let plugin = scratch.0.join("plugin");
    scratch.write("plugin/Cargo.toml", PLUGIN_TOML);
    scratch.write("plugin/spaces/stale.json", "{}\n");
    scratch.write("plugin/spaces/notes.txt", "kept\n");

    let mut log = Vec::new();
    let written = run(
        &Options {
            manifest,
            plugin: plugin.clone(),
            repository: repository(),
        },
        &mut |line| log.push(line),
    )
    .unwrap_or_else(|problems| panic!("the run was refused: {problems:?}"));
    assert_eq!(written.ids, ["small-room"]);
    assert_eq!(written.removed, ["stale.json"]);
    assert!(log[0].starts_with("small-room: confidence"), "{log:?}");

    let report = std::fs::read_to_string(plugin.join("spaces").join("small-room.json"))
        .expect("the report is named by its id");
    for absent in [
        "Some Response",
        "Response Folder",
        "mxm-classic-verb-generate",
        "/",
        "\\",
    ] {
        assert!(!report.contains(absent), "the report carries `{absent}`");
    }
    for present in [
        "\"source\": \"small-room\"",
        "\"verification_refusal\": null",
        "\"render\": {",
        "\"lead_frames\": 2205",
        "\"channels\": 1",
        "\"texture_a\": {",
        "\"texture_b\": ",
        "\"periodicity_lag_s\": ",
    ] {
        assert!(report.contains(present), "the report lacks `{present}`");
    }
    // Segment A's texture in the target and the verification, and its four errors: each reading at
    // least three times.
    for reading in ["peakiness", "kurtosis", "periodicity"] {
        let count = report.matches(&format!("\"{reading}\": ")).count();
        assert!(count >= 3, "`{reading}` appears {count} times");
    }

    let module = std::fs::read_to_string(plugin.join("src").join("spaces").join("generated.rs"))
        .expect("the module is written");
    assert!(!module.contains("Some Response") && !module.contains("Response Folder"));
    let enum_part = module
        .split("pub enum SpaceChoice {")
        .nth(1)
        .expect("the enum");
    assert!(
        enum_part.starts_with(
            "\n    #[id = \"small-room\"]\n    #[name = \"Small \\\"room\\\"\"]\n    SmallRoom,\n    #[id = \"loaded\"]\n    #[name = \"Loaded\"]\n    Loaded,\n}\n"
        ),
        "{enum_part}"
    );
    for present in [
        "pub const FACTORY: &[FactorySpace] = &[",
        "        id: \"small-room\",",
        "        family: SpaceFamily::Room,",
        "            early: [\n                tap(",
        "        pre_delay_s: ",
        "Generated by `classic_verb_generate`. Do not edit by hand.",
    ] {
        assert!(module.contains(present), "the module lacks `{present}`");
    }

    let left: Vec<PathBuf> = snapshot(&plugin).into_iter().map(|(p, _)| p).collect();
    assert_eq!(
        left,
        [
            PathBuf::from("Cargo.toml"),
            Path::new("spaces").join("notes.txt"),
            Path::new("spaces").join("small-room.json"),
            Path::new("src").join("spaces").join("generated.rs"),
        ]
    );
}
