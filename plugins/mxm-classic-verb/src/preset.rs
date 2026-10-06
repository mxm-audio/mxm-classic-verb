//! Factory sounds and the shared preset-system seam.
//!
//! **The loaded space rides the shared durable-content seam** (plan §2.2): a user preset carries it in
//! `state` as a space or an explicit absence, a factory preset's `state = null` preserves it, and a
//! recall stages it before any gesture and commits it after the last. `loaded.rs` holds the barrier;
//! the event table's tests are in `events` below.
//!
//! **The factory set is provisional.** Its twelve sounds were designed on the four hand-authored
//! spaces and now select the fitted factory space nearest each one's intent; redesigning them by ear
//! on the hundred is still open (plan §9). Nothing is released, so the set may still change; after
//! release, a preset's selector position is permanent.

use std::sync::RwLock;

pub use mxm_preset::{
    Category, Entry, INIT_NAME, Library, Loaded, Origin, Preset, PresetIdentity, Refused, Value,
    factory, loaded, mark_loaded, mark_none, read_favourites, snapshot, write_favourites,
};

use crate::loaded::LoadedField;
use crate::params::MxmClassicVerbParams;

/// **The tempo syncs this plugin gained on 2026-09-25** (`plans/plan-tempo-sync-controls.md`). A
/// preset file written before them was written unsynced, so each loads off rather than keeping the
/// instance's sync, and without reporting a missing control.
pub(crate) const TEMPO_SYNC_IDS: &[&str] = &["predelaysync", "modsync"];

impl mxm_preset::Instrument for MxmClassicVerbParams {
    fn clap_id(&self) -> &'static str {
        crate::CLAP_ID
    }

    fn parameters(&self) -> Vec<(&'static str, &dyn mxm_preset::ErasedParam)> {
        // Declaration order is the preset and Init write order: the selector before the controls,
        // so a recalled preset's controls land on the space they were designed against.
        vec![
            ("space", &self.space),
            ("mix", &self.mix),
            ("decay", &self.decay),
            ("bass", &self.bass),
            ("treble", &self.treble),
            ("size", &self.size),
            ("diffusion", &self.diffusion),
            ("predelay", &self.pre_delay),
            ("predelaysync", &self.pre_delay_sync),
            ("earlylate", &self.early_late),
            ("shape", &self.shape),
            ("moddepth", &self.mod_depth),
            ("modrate", &self.mod_rate),
            ("modsync", &self.mod_sync),
            ("width", &self.width),
            ("lowtone", &self.low_tone),
            ("hightone", &self.high_tone),
            ("duck", &self.duck),
        ]
    }

    fn identity(&self) -> &RwLock<PresetIdentity> {
        &self.preset
    }

    fn factory_files(&self) -> &'static [(&'static str, &'static str)] {
        FACTORY_FILES
    }

    fn default_missing_legacy_parameter(&self, id: &str) -> bool {
        TEMPO_SYNC_IDS.contains(&id)
    }

    /// **A user preset always carries a payload**: the loaded space when one is held, whichever
    /// position is selected, and an explicit absence when none is.
    fn capture_preset_state(&self) -> Option<serde_json::Value> {
        Some(self.loaded.payload().to_value())
    }

    /// Always `Some`: absence has a fingerprint of its own, so clearing a space marks a loaded
    /// preset modified exactly as loading one does.
    fn preset_state_fingerprint(&self) -> Option<u64> {
        Some(self.loaded.fingerprint())
    }

    /// `None` is a factory recipe and preserves the loaded space. A payload is parsed whole; a
    /// version this build does not know is refused before any gesture.
    fn validate_preset_state(&self, state: Option<&serde_json::Value>) -> Result<(), String> {
        LoadedField::validate_preset_state(state)
    }

    /// Before the gestures: supersedes a running fit and stages the recalled space or absence.
    fn apply_preset_state(&self, state: Option<&serde_json::Value>) -> Result<(), String> {
        self.loaded.apply_preset_state(state)
    }

    /// After the last gesture: the staged space becomes audible at the next process boundary.
    fn commit_preset_state(&self) {
        self.loaded.commit();
    }
}

pub const FACTORY_FILES: &[(&str, &str)] = &[
    ("Room", include_str!("../presets/room.json")),
    ("Chamber", include_str!("../presets/chamber.json")),
    ("Hall", include_str!("../presets/hall.json")),
    ("Plate", include_str!("../presets/plate.json")),
    ("Lush hall", include_str!("../presets/lush-hall.json")),
    ("Glass", include_str!("../presets/glass.json")),
    ("Resonator", include_str!("../presets/resonator.json")),
    ("Gated drums", include_str!("../presets/gated-drums.json")),
    (
        "Reverse swell",
        include_str!("../presets/reverse-swell.json"),
    ),
    (
        "Slapback room",
        include_str!("../presets/slapback-room.json"),
    ),
    ("Ducked plate", include_str!("../presets/ducked-plate.json")),
    (
        "Wide ambience",
        include_str!("../presets/wide-ambience.json"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// **A project saved before the tempo syncs restores them Off** (`mxm_preset::add_switches_off`),
    /// whatever this instance had.
    #[test]
    fn an_older_state_restores_the_tempo_syncs_off() {
        use nice_plug::prelude::Plugin as _;
        let mut state = nice_plug::prelude::PluginState {
            version: String::new(),
            params: Default::default(),
            fields: Default::default(),
        };
        crate::MxmClassicVerb::filter_state(&mut state);
        for id in TEMPO_SYNC_IDS {
            assert!(
                matches!(
                    state.params.get(*id),
                    Some(nice_plug::plugin::ParamValue::Bool(false))
                ),
                "{{id}} was not restored off"
            );
        }
    }

    /// **A preset saved before the tempo syncs loads them off, and cleanly** ([`TEMPO_SYNC_IDS`]).
    #[test]
    fn a_preset_from_before_the_tempo_syncs_loads_them_off() {
        let params = crate::params::MxmClassicVerbParams::default();
        let mut old = mxm_preset::Preset::init(&params);
        for id in TEMPO_SYNC_IDS {
            old.params.remove(*id);
        }
        let (writes, problems) = old.resolve(&params);
        assert!(problems.is_empty(), "{{problems:?}}");
        for id in TEMPO_SYNC_IDS {
            assert!(
                writes.iter().any(|(w, _, v)| w == id && *v == 0.0),
                "{{id}} was not written off"
            );
        }
    }

    use crate::params::{ShapeChoice, SpaceChoice};
    use mxm_preset::Instrument;
    use nice_plug::prelude::{Enum, Param};

    // Selector positions and shapes, by index, in the parameters' own order. Each space is named by
    // its generated variant (`spaces/generated.rs`), so a regenerated selector that renames it fails
    // to compile here and one that moves it carries the design with it.
    const NATURAL_ROOM: f32 = SpaceChoice::NaturalRoom as usize as f32;
    const LIVELY_LOUNGE: f32 = SpaceChoice::LivelyLounge as usize as f32;
    const CLEAN_AIR: f32 = SpaceChoice::CleanAir as usize as f32;
    const ECHO_CHAMBER: f32 = SpaceChoice::EchoChamber as usize as f32;
    const GLITTERING_HALL: f32 = SpaceChoice::GlitteringHall as usize as f32;
    const WARM_CONCERT_HALL: f32 = SpaceChoice::WarmConcertHall as usize as f32;
    const MEDIUM_HALL: f32 = SpaceChoice::MediumHall as usize as f32;
    const BIG_DRUM_ROOM: f32 = SpaceChoice::BigDrumRoom as usize as f32;
    const SATIN_PLATE: f32 = SpaceChoice::SatinPlate as usize as f32;
    const VOCAL_PLATE: f32 = SpaceChoice::VocalPlate as usize as f32;
    const GATED: f32 = 1.0;
    const REVERSE: f32 = 2.0;

    type Design = (&'static str, Category, &'static [(&'static str, f32)]);

    /// Designs in the parameters' own units: seconds, multipliers, decibels, fractions, indices.
    pub const DESIGNS: &[Design] = &[
        (
            "Room",
            Category::Fx,
            &[
                ("space", NATURAL_ROOM),
                ("mix", 0.25),
                ("decay", 0.9),
                ("size", 0.018),
                ("diffusion", 0.7),
                ("predelay", 0.004),
            ],
        ),
        (
            "Chamber",
            Category::Fx,
            &[
                ("space", ECHO_CHAMBER),
                ("mix", 0.28),
                ("decay", 1.6),
                ("size", 0.030),
                ("predelay", 0.012),
            ],
        ),
        (
            "Hall",
            Category::Fx,
            &[
                ("space", MEDIUM_HALL),
                ("mix", 0.30),
                ("decay", 2.8),
                ("size", 0.070),
                ("predelay", 0.025),
            ],
        ),
        (
            "Plate",
            Category::Fx,
            &[
                ("space", VOCAL_PLATE),
                ("mix", 0.30),
                ("decay", 2.2),
                ("size", 0.035),
                ("diffusion", 1.0),
                ("predelay", 0.0),
            ],
        ),
        (
            "Lush hall",
            Category::Pad,
            &[
                ("space", WARM_CONCERT_HALL),
                ("mix", 0.32),
                ("decay", 3.5),
                ("size", 0.080),
                ("moddepth", 0.0015),
                ("modrate", 0.7),
            ],
        ),
        (
            "Glass",
            Category::Pad,
            &[
                ("space", GLITTERING_HALL),
                ("mix", 0.30),
                ("decay", 2.5),
                ("bass", 0.5),
                ("treble", 3.0),
                ("hightone", 3.0),
            ],
        ),
        (
            "Resonator",
            Category::Fx,
            &[
                ("space", NATURAL_ROOM),
                ("mix", 0.25),
                ("decay", 3.0),
                ("size", 0.003),
                ("diffusion", 0.2),
            ],
        ),
        (
            "Gated drums",
            Category::Percussion,
            &[
                ("space", BIG_DRUM_ROOM),
                ("mix", 0.40),
                ("shape", GATED),
                ("decay", 0.35),
                ("size", 0.030),
            ],
        ),
        (
            "Reverse swell",
            Category::Fx,
            &[
                ("space", MEDIUM_HALL),
                ("mix", 0.45),
                ("shape", REVERSE),
                ("decay", 0.8),
                ("predelay", 0.0),
            ],
        ),
        (
            "Slapback room",
            Category::Fx,
            &[
                ("space", LIVELY_LOUNGE),
                ("mix", 0.30),
                ("decay", 1.2),
                ("size", 0.090),
                ("earlylate", 18.0),
                ("diffusion", 0.1),
            ],
        ),
        (
            "Ducked plate",
            Category::Fx,
            &[
                ("space", SATIN_PLATE),
                ("mix", 0.35),
                ("decay", 2.4),
                ("duck", 0.7),
                ("predelay", 0.060),
            ],
        ),
        (
            "Wide ambience",
            Category::Pad,
            &[
                ("space", CLEAN_AIR),
                ("mix", 0.35),
                ("decay", 1.2),
                ("size", 0.020),
                ("width", 2.2),
            ],
        ),
    ];

    fn normalised_for(params: &MxmClassicVerbParams, id: &str, plain: f32) -> f32 {
        let floats = [
            ("mix", &params.mix),
            ("decay", &params.decay),
            ("bass", &params.bass),
            ("treble", &params.treble),
            ("size", &params.size),
            ("diffusion", &params.diffusion),
            ("predelay", &params.pre_delay),
            ("earlylate", &params.early_late),
            ("moddepth", &params.mod_depth),
            ("modrate", &params.mod_rate),
            ("width", &params.width),
            ("lowtone", &params.low_tone),
            ("hightone", &params.high_tone),
            ("duck", &params.duck),
        ];
        if let Some((_, param)) = floats.iter().find(|(found, _)| *found == id) {
            return param.preview_normalized(plain);
        }
        // The position counts are the enums' own, so a regenerated selector cannot leave a design
        // normalised against the old count.
        let (positions, name) = match id {
            "space" => (SpaceChoice::variants().len() as f32, "space"),
            "shape" => (ShapeChoice::variants().len() as f32, "shape"),
            other => panic!("`{other}` is not a parameter"),
        };
        assert!(
            plain.fract() == 0.0 && plain >= 0.0 && plain < positions,
            "{name} index {plain}"
        );
        plain / (positions - 1.0)
    }

    pub fn generated(
        params: &MxmClassicVerbParams,
        name: &str,
        category: Category,
        overrides: &[(&str, f32)],
    ) -> Preset {
        let bindings = params.parameters();
        let mut preset = Preset::init(params);
        preset.name = name.to_owned();
        preset.category = category;
        for (id, plain) in overrides {
            let (_, param) = bindings
                .iter()
                .find(|(found, _)| found == id)
                .expect("real parameter");
            let value = normalised_for(params, id, *plain).clamp(0.0, 1.0);
            preset.params.insert(
                (*id).to_owned(),
                Value {
                    v: value,
                    text: param.format(value),
                },
            );
        }
        preset
    }

    fn file_name(name: &str) -> String {
        name.to_ascii_lowercase().replace(' ', "-") + ".json"
    }

    #[test]
    #[ignore = "writes the factory preset files"]
    fn write_the_factory_presets() {
        let params = MxmClassicVerbParams::default();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
        std::fs::create_dir_all(&dir).expect("presets folder");
        for (name, category, overrides) in DESIGNS {
            let preset = generated(&params, name, *category, overrides);
            std::fs::write(dir.join(file_name(name)), preset.to_json()).expect("write preset");
        }
    }

    #[test]
    fn preset_writes_follow_parameter_declaration_order() {
        let params = MxmClassicVerbParams::default();
        let ids: Vec<_> = params.parameters().into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ids,
            [
                "space",
                "mix",
                "decay",
                "bass",
                "treble",
                "size",
                "diffusion",
                "predelay",
                "predelaysync",
                "earlylate",
                "shape",
                "moddepth",
                "modrate",
                "modsync",
                "width",
                "lowtone",
                "hightone",
                "duck"
            ]
        );
    }

    #[test]
    fn the_init_preset_is_the_parameter_defaults() {
        let params = MxmClassicVerbParams::default();
        for (id, param) in params.parameters() {
            assert_eq!(
                param.normalised(),
                param.default_normalised(),
                "{id} does not start at its default"
            );
        }
    }

    #[test]
    fn the_factory_files_match_the_design() {
        let params = MxmClassicVerbParams::default();
        assert_eq!(FACTORY_FILES.len(), DESIGNS.len());
        for (name, category, overrides) in DESIGNS {
            let text = FACTORY_FILES
                .iter()
                .find(|(found, _)| found == name)
                .expect("listed")
                .1;
            let shipped = Preset::parse(text, crate::CLAP_ID).expect("factory preset parses");
            assert_eq!(
                shipped,
                generated(&params, name, *category, overrides),
                "{name}"
            );
        }
    }

    #[test]
    fn every_factory_preset_is_complete_categorised_and_not_init() {
        let params = MxmClassicVerbParams::default();
        let init = Preset::init(&params);
        for (name, category, overrides) in DESIGNS {
            assert_ne!(*category, Category::Uncategorised, "{name}");
            let preset = generated(&params, name, *category, overrides);
            assert_eq!(
                preset.params.len(),
                params.parameters().len(),
                "{name} is complete"
            );
            let moved = preset
                .params
                .iter()
                .filter(|(id, v)| (v.v - init.params[*id].v).abs() >= 0.02)
                .count();
            assert!(moved >= 3, "{name} is Init under another name");
        }
        assert_eq!(
            factory(&params).first().map(|entry| entry.name.as_str()),
            Some(INIT_NAME)
        );
    }

    #[test]
    fn every_factory_pair_differs_on_at_least_three_axes() {
        let params = MxmClassicVerbParams::default();
        let presets: Vec<_> = DESIGNS
            .iter()
            .map(|(n, c, o)| generated(&params, n, *c, o))
            .collect();
        for (i, a) in presets.iter().enumerate() {
            for b in &presets[i + 1..] {
                let axes = a
                    .params
                    .iter()
                    .filter(|(id, v)| (v.v - b.params[*id].v).abs() >= 0.02)
                    .count();
                assert!(axes >= 3, "{} and {} differ on {axes} axes", a.name, b.name);
            }
        }
    }
}

/// **Plan §2.2's event table**, one test per row the durable-content seam decides. Loading itself is
/// `loading.rs`'s, and `Loaded` with nothing loaded is `lib.rs`'s and the editor's.
#[cfg(test)]
mod events {
    use std::sync::Arc;

    use mxm_classic_verb_dsp::Space;
    use mxm_preset::ui::{apply_preset_checked, init_patch};
    use mxm_preset::{Instrument, Preset};
    use nice_plug::params::persist::{PersistentField, deserialize_field, serialize_field};
    use nice_plug::prelude::{Param, ParamSetter};

    use super::{Category, FACTORY_FILES, Loaded, Origin, loaded, mark_loaded};
    use crate::params::{MxmClassicVerbParams, SpaceChoice};
    use crate::payload::{self, Payload};
    use crate::testing::{self, ApplyingHost};

    fn holding(space: Option<Space>) -> MxmClassicVerbParams {
        let params = MxmClassicVerbParams::default();
        params.loaded.set(match space {
            Some(space) => Payload::holding(&space, None),
            None => Payload::absence(),
        });
        params
    }

    fn factory(name: &str) -> Preset {
        let text = FACTORY_FILES
            .iter()
            .find(|(found, _)| *found == name)
            .expect("a factory preset")
            .1;
        Preset::parse(text, crate::CLAP_ID).expect("it parses")
    }

    #[test]
    fn a_factory_preset_preserves_the_loaded_space() {
        let params = holding(Some(Space::ROOM));
        testing::set(&params.space, SpaceChoice::Loaded);
        let fingerprint = params.loaded.fingerprint();
        let recipe = factory("Hall");
        assert_eq!(recipe.state, None, "a factory preset carries state = null");
        let host = ApplyingHost::default();
        let (applied, problems) = apply_preset_checked(&params, &ParamSetter::new(&host), &recipe);
        assert!(applied, "{problems:?}");
        let position = params.space.preview_plain(recipe.params["space"].v);
        assert_ne!(position, SpaceChoice::Loaded);
        assert_eq!(params.space.value(), position);
        assert_eq!(params.loaded.space(), Some(Space::ROOM));
        assert_eq!(params.loaded.fingerprint(), fingerprint);
        testing::set(&params.space, SpaceChoice::Loaded);
        assert_eq!(params.sounding_space(), Space::ROOM);
    }

    #[test]
    fn a_user_preset_always_carries_a_payload() {
        let empty = holding(None);
        testing::set(&empty.space, SpaceChoice::Loaded);
        let captured = Preset::capture("Nothing loaded", Category::Fx, &empty);
        assert_eq!(
            captured.state,
            Some(serde_json::json!({ "version": 1, "space": null })),
            "nothing held is an explicit absence, never a missing state"
        );

        // A space held while a factory position is selected is still carried.
        let held = holding(Some(Space::CHAMBER));
        assert_eq!(held.space.value(), SpaceChoice::INIT);
        let captured = Preset::capture("Chamber held", Category::Fx, &held);
        let text = captured.to_json();
        assert!(text.len() < 8_192, "{} bytes", text.len());
        let back = Preset::parse(&text, crate::CLAP_ID).expect("a user preset parses");
        let (_, space) = payload::parse(back.state.as_ref().expect("a payload")).expect("it reads");
        assert_eq!(space, Some(Space::CHAMBER));
    }

    #[test]
    fn a_user_preset_recall_applies_before_any_gesture_and_commits_after_the_last() {
        let source = holding(Some(Space::PLATE));
        testing::set(&source.space, SpaceChoice::Loaded);
        let preset = Preset::capture("Plate loaded", Category::Fx, &source);

        let params = Arc::new(holding(None));
        let sequence = params.loaded.committed_sequence();
        let host = ApplyingHost::default();
        {
            let params = Arc::clone(&params);
            host.on_gesture(move || {
                assert!(
                    params.loaded.is_staged(),
                    "a gesture was written before the payload was applied"
                );
                assert_eq!(
                    params.loaded.committed_sequence(),
                    sequence,
                    "the recalled space was audible before the last gesture"
                );
            });
        }
        let (applied, problems) =
            apply_preset_checked(params.as_ref(), &ParamSetter::new(&host), &preset);
        assert!(applied && problems.is_empty(), "{problems:?}");
        assert_eq!(host.begins(), params.parameters().len());
        assert_eq!(host.begins(), host.ends());
        assert!(!params.loaded.is_staged());
        assert_eq!(params.loaded.space(), Some(Space::PLATE));
        assert_eq!(params.sounding_space(), Space::PLATE);
    }

    #[test]
    fn a_recalled_absence_clears_a_stale_loaded_space() {
        let source = holding(None);
        testing::set(&source.space, SpaceChoice::Loaded);
        let preset = Preset::capture("Saved with nothing loaded", Category::Fx, &source);

        let params = holding(Some(Space::ROOM));
        let host = ApplyingHost::default();
        assert!(apply_preset_checked(&params, &ParamSetter::new(&host), &preset).0);
        assert_eq!(params.loaded.space(), None);
        assert_eq!(params.space.value(), SpaceChoice::Loaded);
        assert_eq!(params.sounding_space(), SpaceChoice::INIT.factory());
    }

    #[test]
    fn init_writes_defaults_and_the_loaded_space_survives_it_unselected() {
        let params = holding(Some(Space::ROOM));
        testing::set(&params.space, SpaceChoice::Loaded);
        testing::set(&params.mix, 0.8);
        let fingerprint = params.loaded.fingerprint();
        let host = ApplyingHost::default();
        init_patch(&params, &ParamSetter::new(&host));
        assert_eq!(host.begins(), host.ends());
        for (id, param) in params.parameters() {
            assert_eq!(param.normalised(), param.default_normalised(), "{id}");
        }
        assert_eq!(params.space.value(), SpaceChoice::INIT);
        assert_eq!(params.loaded.space(), Some(Space::ROOM));
        assert_eq!(params.loaded.fingerprint(), fingerprint);
        testing::set(&params.space, SpaceChoice::Loaded);
        assert_eq!(params.sounding_space(), Space::ROOM);
    }

    #[test]
    fn host_state_persists_the_payload_and_its_fingerprint_joins_the_dirty_comparison() {
        let params = holding(Some(Space::PLATE));
        let serialized = params
            .loaded
            .map(serialize_field)
            .expect("the field serialises");
        assert!(serialized.len() < 4_096, "{} bytes", serialized.len());
        let restored = MxmClassicVerbParams::default();
        restored
            .loaded
            .set(deserialize_field::<Payload>(&serialized).expect("the field reads back"));
        assert_eq!(restored.loaded.space(), Some(Space::PLATE));
        assert_eq!(restored.loaded.fingerprint(), params.loaded.fingerprint());

        let params = holding(None);
        let absence = params.preset_state_fingerprint();
        assert!(absence.is_some(), "absence has a fingerprint of its own");
        mark_loaded(&params, "Mine", Origin::User);
        assert!(matches!(loaded(&params), Loaded::Clean { .. }));
        params.loaded.set(Payload::holding(&Space::ROOM, None));
        assert!(
            matches!(loaded(&params), Loaded::Modified { .. }),
            "loading a space did not mark the loaded preset modified"
        );
        assert_ne!(params.preset_state_fingerprint(), absence);
        params.loaded.set(Payload::absence());
        assert!(
            matches!(loaded(&params), Loaded::Clean { .. }),
            "clearing the space did not return to the preset's own absence"
        );
    }

    #[test]
    fn a_payload_version_this_build_does_not_know_is_refused_whole_before_any_gesture() {
        let params = holding(Some(Space::ROOM));
        let running = params.loaded.begin("running.wav");
        let mut preset = Preset::capture("From a later build", Category::Fx, &params);
        preset.state.as_mut().expect("a payload")["version"] = serde_json::json!(2);
        let host = ApplyingHost::default();
        let (applied, problems) = apply_preset_checked(&params, &ParamSetter::new(&host), &preset);
        assert!(!applied);
        assert!(
            problems.iter().any(|problem| problem.contains("version 2")),
            "{problems:?}"
        );
        assert_eq!(host.begins(), 0, "a refused payload wrote a gesture");
        assert_eq!(params.loaded.space(), Some(Space::ROOM));
        assert!(!params.loaded.is_staged());
        assert!(
            params.loaded.is_current(running),
            "a refused payload cancelled a fit"
        );
    }
}
