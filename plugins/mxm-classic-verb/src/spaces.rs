//! The factory spaces (plan §2.2): the Space selector's positions and the space each one sounds, as
//! `classic_verb_generate` fitted them. **`spaces/generated.rs` is written by that tool, never by
//! hand**; this module holds the types it fills and the checks that hold it to its reports.
//!
//! **Provisional until P3.5's final manifest.** Today's four positions are fitted from synthetic
//! responses of the DSP crate's four hand-authored spaces (`AGENTS.md`, *Factory spaces are
//! generated*).

mod generated;

pub use generated::{FACTORY, SpaceChoice};

use mxm_classic_verb_dsp::{EarlyTap, Space};
use nice_plug::prelude::Enum;

/// What kind of space a factory position is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceFamily {
    Room,
    Ambience,
    Chamber,
    Hall,
    LargeSpace,
    DrumRoom,
    Plate,
    Spring,
    Strange,
    Experimental,
}

impl SpaceFamily {
    /// The family as a manifest writes it.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Room => "room",
            Self::Ambience => "ambience",
            Self::Chamber => "chamber",
            Self::Hall => "hall",
            Self::LargeSpace => "large space",
            Self::DrumRoom => "drum room",
            Self::Plate => "plate",
            Self::Spring => "spring",
            Self::Strange => "strange",
            Self::Experimental => "experimental",
        }
    }
}

/// One factory space as fitted: its selector position's permanent id and display name, its family,
/// the space, and the absolute controls its fit returned (plan §2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FactorySpace {
    pub id: &'static str,
    pub name: &'static str,
    pub family: SpaceFamily,
    pub space: Space,
    pub decay_s: f32,
    pub size_s: f32,
    pub diffusion: f32,
    pub pre_delay_s: f32,
}

/// An early tap, as the generated table writes one.
const fn tap(time: f32, gain_l: f32, gain_r: f32) -> EarlyTap {
    EarlyTap {
        time,
        gain_l,
        gain_r,
    }
}

// `Loaded` is the last position, one past the factory table.
const _: () = assert!(SpaceChoice::Loaded as usize == FACTORY.len());

impl SpaceChoice {
    /// Every factory position, in the selector's order. `Loaded` is not one.
    pub fn factory_positions() -> impl Iterator<Item = SpaceChoice> {
        (0..FACTORY.len()).map(SpaceChoice::from_index)
    }

    /// This position's factory space as fitted; `None` for `Loaded`.
    pub fn factory_entry(self) -> Option<&'static FactorySpace> {
        FACTORY.get(self.to_index())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mxm_classic_verb_dsp::{MAX_DECAY_S, MAX_PRE_DELAY_S, MAX_SIZE_S, MIN_DECAY_S, MIN_SIZE_S};
    use std::path::Path;

    /// **The display names and ids come from the table**: the enum and the table are written from the
    /// same manifest rows, and this holds them together position by position.
    #[test]
    fn the_selector_is_the_factory_table_then_loaded() {
        let names = SpaceChoice::variants();
        let ids = SpaceChoice::ids().expect("every position has a permanent id");
        assert_eq!(names.len(), FACTORY.len() + 1);
        for (index, entry) in FACTORY.iter().enumerate() {
            let choice = SpaceChoice::from_index(index);
            assert_eq!(names[index], entry.name);
            assert_eq!(ids[index], entry.id);
            assert_eq!(choice.factory_entry(), Some(entry));
            assert_eq!(choice.factory(), entry.space);
        }
        assert_eq!(
            (names[FACTORY.len()], ids[FACTORY.len()]),
            ("Loaded", "loaded")
        );
        assert_eq!(SpaceChoice::Loaded.factory_entry(), None);
        assert_eq!(SpaceChoice::factory_positions().count(), FACTORY.len());
    }

    #[test]
    fn every_id_and_name_is_permanent_shaped_and_unique() {
        for (index, entry) in FACTORY.iter().enumerate() {
            assert!(
                entry.id.split('-').all(|word| !word.is_empty()
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())),
                "{}",
                entry.id
            );
            assert!(!entry.name.contains(['/', '\\']), "{}", entry.name);
            for other in &FACTORY[..index] {
                assert_ne!(entry.id, other.id);
                assert_ne!(entry.name.to_lowercase(), other.name.to_lowercase());
            }
        }
    }

    #[test]
    fn every_factory_space_and_its_controls_are_inside_their_bounds() {
        for entry in FACTORY {
            assert_eq!(entry.space.sanitised(), entry.space, "{}", entry.id);
            assert!(
                (MIN_DECAY_S..=MAX_DECAY_S).contains(&entry.decay_s)
                    && (MIN_SIZE_S..=MAX_SIZE_S).contains(&entry.size_s)
                    && (0.0..=1.0).contains(&entry.diffusion)
                    && (0.0..=MAX_PRE_DELAY_S).contains(&entry.pre_delay_s),
                "{entry:?}"
            );
        }
    }

    /// **Every factory space has its report beside the crate, and no report outlives its space.** A
    /// report names its source by the space's id and carries no path separator, so nothing committed
    /// points at a disk (plan §5.1). The descriptors themselves are `classic_verb_space_audit`'s.
    #[test]
    fn every_factory_space_has_its_report_and_no_report_is_orphaned() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("spaces");
        let mut found: Vec<String> = std::fs::read_dir(&folder)
            .expect("the spaces folder is readable")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        let mut expected: Vec<String> = FACTORY.iter().map(|e| format!("{}.json", e.id)).collect();
        expected.sort();
        assert_eq!(found, expected);
        for entry in FACTORY {
            let text = std::fs::read_to_string(folder.join(format!("{}.json", entry.id)))
                .expect("the report reads");
            assert!(!text.contains(['/', '\\']), "{} names a path", entry.id);
            let report: serde_json::Value = serde_json::from_str(&text).expect("the report parses");
            assert_eq!(report["source"], entry.id);
        }
    }
}
