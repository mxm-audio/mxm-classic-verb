//! T15 — `mxm-classic-verb` through MXM Player's ordinary effect chain.
//!
//! Product-specific DSP stays in its own crate. This proves only the host boundary: normal bundle
//! discovery, an audible stereo effect whose tail outlives its source, and exact dry restoration
//! when the chain bypasses it.

use mxm_player_harness::harness;

use harness::{CHANNELS, Harness, mxm_mono_01};
use mxm_player::events::input::Payload;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

const SOURCE_ID: &str = "dk.mxm.mxm-mono-01";
const EFFECT_ID: &str = "dk.mxm.mxm-classic-verb";
const BLOCK: usize = 256;
const BLOCKS: usize = 700;

fn classic_verb() -> Option<PathBuf> {
    let path = mxm_player_harness::workspace_root().join("target/bundled/mxm-classic-verb.clap");
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "skipping: {} is missing — run `cargo xtask bundle mxm-classic-verb --release`",
            path.display()
        );
        None
    }
}

fn render(with_effect: bool, bypassed: bool) -> Option<Vec<f32>> {
    let source = mxm_mono_01()?;
    let effect = classic_verb()?;
    let chain: Vec<(&Path, &str)> = if with_effect {
        vec![(effect.as_path(), EFFECT_ID)]
    } else {
        vec![]
    };
    let mut harness = Harness::with_fx(&source, SOURCE_ID, 1, &chain).expect("the chain builds");
    if let Some(effect) = harness.fx.first() {
        effect.bypassed.store(bypassed, Ordering::Relaxed);
    }

    let mut output = Vec::with_capacity(BLOCKS * BLOCK * CHANNELS);
    for block in 0..BLOCKS {
        if block == 2 {
            assert!(harness.push_at(
                0,
                0,
                Payload::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 100.0 / 127.0
                },
            ));
        }
        if block == 28 {
            assert!(harness.push_at(
                0,
                0,
                Payload::NoteOff {
                    channel: 0,
                    key: 60,
                    velocity: 0.0
                },
            ));
        }
        output.extend_from_slice(harness.render(BLOCK));
    }
    harness.shutdown();
    Some(output)
}

fn last_sounding_frame(interleaved: &[f32]) -> Option<usize> {
    interleaved
        .chunks(CHANNELS)
        .enumerate()
        .rev()
        .find(|(_, frame)| frame.iter().any(|sample| *sample != 0.0))
        .map(|(frame, _)| frame)
}

#[test]
fn the_classic_verb_is_discovered_is_audible_and_outlives_the_source() {
    let Some(dry) = render(false, false) else {
        return;
    };
    let wet = render(true, false).expect("the bundle existed above");
    assert_ne!(
        wet, dry,
        "the loaded effect must change the rendered signal"
    );

    let dry_end = last_sounding_frame(&dry).expect("the source sounded");
    let wet_end = last_sounding_frame(&wet).expect("the effect sounded");
    assert!(
        wet_end > dry_end,
        "the reverb tail ended at {wet_end}, dry at {dry_end}"
    );

    let first = dry
        .chunks(CHANNELS)
        .position(|frame| frame.iter().any(|sample| *sample != 0.0))
        .expect("the source sounded");
    assert!(
        wet[..first * CHANNELS].iter().all(|sample| *sample == 0.0),
        "the effect produced sound before its input"
    );
}

#[test]
fn player_bypass_restores_the_dry_render_to_the_bit() {
    let Some(dry) = render(false, false) else {
        return;
    };
    let bypassed = render(true, true).expect("the bundle existed above");
    assert!(
        dry.iter().any(|sample| *sample != 0.0),
        "the source must be audible"
    );
    assert_eq!(bypassed, dry);
}

/// The effect's own control map, read through the player's `ControlMap` as a host reads it. These
/// were MXM Player's (`tests/t5_control_map.rs`) until the collection was split into one repository
/// per product (2026-10-06); the player names no product any more.
mod control_map {
    use mxm_player::control_map::ControlMap;
    use mxm_player::control_map::schema;
    use std::path::PathBuf;

    #[test]
    fn the_classic_verb_map_claims_its_new_page_and_reuses_the_existing_reverb_role() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../control-map.json");
        let mut map = ControlMap::shipped();
        map.load_instrument_map(&path)
            .expect("mxm-classic-verb's map loads");

        let id = "dk.mxm.mxm-classic-verb";
        assert_eq!(
            map.param_for(id, "fx.reverb"),
            Some(schema::hash_param_id("mix"))
        );
        let page_roles = [
            ("fx.verb_space", "space"),
            ("fx.verb_decay", "decay"),
            ("fx.verb_size", "size"),
            ("fx.verb_predelay", "predelay"),
            ("fx.verb_bass", "bass"),
            ("fx.verb_treble", "treble"),
            ("fx.verb_shape", "shape"),
        ];
        for (role, parameter) in page_roles {
            assert_eq!(
                map.param_for(id, role),
                Some(schema::hash_param_id(parameter)),
                "{role}"
            );
        }

        // The brief's slot order, with the eighth slot left free on purpose.
        let standard = schema::shipped();
        let page = standard
            .pages
            .iter()
            .find(|page| page.name == "Classic verb")
            .expect("the standard has the Classic verb page");
        assert_eq!(page.section, "Effects");
        let mut slots: Vec<Option<String>> = page_roles
            .iter()
            .map(|(role, _)| Some((*role).to_owned()))
            .collect();
        slots.push(None);
        assert_eq!(page.slots, slots);

        // The shape controls stay off the controller: Mix and the seven page roles, nothing else.
        let claimed: Vec<_> = standard
            .roles
            .keys()
            .filter_map(|role| map.param_for(id, role))
            .collect();
        assert_eq!(claimed.len(), 8, "{claimed:?}");
        for parameter in [
            "diffusion",
            "moddepth",
            "modrate",
            "width",
            "lowtone",
            "hightone",
            "earlylate",
            "duck",
        ] {
            assert!(
                !claimed.contains(&schema::hash_param_id(parameter)),
                "{parameter} is a shape control and must stay unmapped"
            );
        }
    }
}
