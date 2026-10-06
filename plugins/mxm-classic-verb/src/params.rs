//! Permanent parameter definitions for `mxm-classic-verb`.
//!
//! **Ids are permanent once a release exists** (`plugins/AGENTS.md`). Nothing is released yet. The
//! selector's positions are generated (`spaces.rs`) and provisional until P3.5's final manifest
//! (plan §2.2); after release the position list is permanent too.

use mxm_classic_verb_dsp::{
    DecayShape, MAX_BAND_MULT, MAX_DECAY_S, MAX_MOD_DEPTH_S, MAX_MOD_RATE_HZ, MAX_PRE_DELAY_S,
    MAX_SIZE_S, MAX_TONE_DB, MAX_WIDTH, MIN_BAND_MULT, MIN_DECAY_S, MIN_SIZE_S, MIN_WIDTH, Space,
};
use mxm_preset::PresetIdentity;
use nice_plug::prelude::*;
use std::sync::{Arc, RwLock};

use crate::loaded::LoadedField;
use crate::spaces::FACTORY;

/// The selector: the factory positions plus exactly one `Loaded` position, last (plan §2.2).
/// **Generated** from the factory-space manifest with the table its positions sound
/// (`spaces/generated.rs`).
pub use crate::spaces::SpaceChoice;

impl SpaceChoice {
    /// The position the defaults select. Its space is the **Init space**, which `Loaded` plays while
    /// nothing is loaded (plan §2.2). **Chosen by the owner** (plan revision 19): a vocal plate at a
    /// mix level, the digital plate fitted at 1.96 s. A manifest without it fails to compile here.
    pub const INIT: SpaceChoice = SpaceChoice::VocalPlate;

    /// The factory space at this position, from the generated table; for `Loaded`, the Init space.
    pub const fn factory(self) -> Space {
        let index = self as usize;
        if index < FACTORY.len() {
            FACTORY[index].space
        } else {
            FACTORY[Self::INIT as usize].space
        }
    }

    /// The space this position sounds: its factory space, or for `Loaded` the loaded space, and the
    /// Init space while nothing is loaded.
    pub fn sounding(self, loaded: Option<Space>) -> Space {
        match (self, loaded) {
            (Self::Loaded, Some(space)) => space,
            (choice, _) => choice.factory(),
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeChoice {
    #[id = "natural"]
    #[name = "Natural"]
    Natural,
    #[id = "gated"]
    #[name = "Gated"]
    Gated,
    #[id = "reverse"]
    #[name = "Reverse"]
    Reverse,
}

impl ShapeChoice {
    pub const fn dsp(self) -> DecayShape {
        match self {
            Self::Natural => DecayShape::Natural,
            Self::Gated => DecayShape::Gated,
            Self::Reverse => DecayShape::Reverse,
        }
    }
}

type ValueToString = Arc<dyn Fn(f32) -> String + Send + Sync>;
type StringToValue = Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>;

fn number(text: &str, suffixes: &[&str]) -> Option<f32> {
    let mut t = text.trim().to_ascii_lowercase();
    for suffix in suffixes {
        if let Some(stripped) = t.strip_suffix(suffix) {
            t = stripped.trim().to_owned();
            break;
        }
    }
    t.trim_start_matches(['x', '×']).trim().parse::<f32>().ok()
}

/// A whole percentage — **never a negative zero**. Width's range crosses zero, and a hair below it
/// printed `-0 %`, which parses to zero and prints `0 %`: text a host cannot round-trip.
fn percent() -> (ValueToString, StringToValue) {
    (
        Arc::new(|v| {
            let text = format!("{:.0}", v * 100.0);
            format!("{} %", if text == "-0" { "0" } else { &text })
        }),
        Arc::new(|t| number(t, &["%"]).map(|v| v / 100.0)),
    )
}

/// Seconds, with the precision chosen from the rounded display value so a host round-trip is
/// text-idempotent.
fn seconds() -> (ValueToString, StringToValue) {
    (
        Arc::new(|v| {
            if (v * 10.0).round() >= 100.0 {
                format!("{v:.1} s")
            } else {
                format!("{v:.2} s")
            }
        }),
        Arc::new(|t| {
            let lower = t.trim().to_ascii_lowercase();
            if let Some(ms) = lower.strip_suffix("ms") {
                ms.trim().parse::<f32>().ok().map(|v| v / 1_000.0)
            } else {
                number(&lower, &["s"])
            }
        }),
    )
}

/// Milliseconds, one decimal below 10 ms.
fn milliseconds() -> (ValueToString, StringToValue) {
    (
        Arc::new(|v| {
            let ms = v * 1_000.0;
            if (ms * 10.0).round() >= 100.0 {
                format!("{ms:.0} ms")
            } else {
                format!("{ms:.1} ms")
            }
        }),
        Arc::new(|t| number(t, &["ms"]).map(|v| v / 1_000.0)),
    )
}

fn multiplier() -> (ValueToString, StringToValue) {
    (
        Arc::new(|v| format!("×{v:.2}")),
        Arc::new(|t| number(t, &[])),
    )
}

fn decibels() -> (ValueToString, StringToValue) {
    (
        Arc::new(|v| {
            if (v * 10.0).round() == 0.0 {
                "0.0 dB".to_owned()
            } else {
                format!("{v:+.1} dB")
            }
        }),
        Arc::new(|t| number(t, &["db"])),
    )
}

fn hertz() -> (ValueToString, StringToValue) {
    (
        Arc::new(|v| format!("{v:.2} Hz")),
        Arc::new(|t| number(t, &["hz"])),
    )
}

/// The skew that puts `centre` at the middle of a `min..max` travel.
fn centred_skew(min: f32, max: f32, centre: f32) -> f32 {
    0.5f32.ln() / ((centre - min) / (max - min)).ln()
}

/// **Pre-delay's tempo sync** (`plans/plan-tempo-sync-controls.md`): 1/64 to a quarter note, the
/// slice of the ladder the pre-delay's 0 – 500 ms holds at 120 bpm, the top the longest.
pub const PRE_DELAY_SYNC: mxm_tempo::Ladder = mxm_tempo::Ladder::new(
    mxm_tempo::Span::new(
        mxm_tempo::Division::SixtyFourth,
        mxm_tempo::Division::Quarter,
    ),
    mxm_tempo::Direction::Time,
);

/// **Mod rate's tempo sync**: every LFO's ladder, 1/32 to four bars, the top the fastest.
pub const MOD_SYNC: mxm_tempo::Ladder =
    mxm_tempo::Ladder::new(mxm_tempo::Span::LFO, mxm_tempo::Direction::Rate);

#[derive(Params)]
pub struct MxmClassicVerbParams {
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "space"]
    pub space: EnumParam<SpaceChoice>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "size"]
    pub size: FloatParam,
    #[id = "diffusion"]
    pub diffusion: FloatParam,
    #[id = "predelay"]
    pub pre_delay: FloatParam,
    /// Pre-delay's tempo sync: its position picks a division of the host's tempo.
    #[id = "predelaysync"]
    pub pre_delay_sync: BoolParam,
    #[id = "earlylate"]
    pub early_late: FloatParam,
    #[id = "shape"]
    pub shape: EnumParam<ShapeChoice>,
    #[id = "moddepth"]
    pub mod_depth: FloatParam,
    #[id = "modrate"]
    pub mod_rate: FloatParam,
    /// Mod rate's tempo sync.
    #[id = "modsync"]
    pub mod_sync: BoolParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "lowtone"]
    pub low_tone: FloatParam,
    #[id = "hightone"]
    pub high_tone: FloatParam,
    #[id = "duck"]
    pub duck: FloatParam,

    #[persist = "preset"]
    pub preset: RwLock<PresetIdentity>,

    /// The loaded space, persisted with the patch as a versioned payload holding a space or an
    /// explicit absence (plan §2.2). Never the response, never a path. See `loaded.rs`.
    #[persist = "loaded"]
    pub loaded: LoadedField,
}

impl MxmClassicVerbParams {
    /// Pre-delay while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`PRE_DELAY_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_pre_delay(&self, tempo: Option<f64>) -> Option<f32> {
        let param = &self.pre_delay;
        PRE_DELAY_SYNC
            .resolve(
                self.pre_delay_sync.value(),
                tempo,
                param.modulated_normalized_value(),
                f64::from(param.preview_plain(0.0)),
                f64::from(param.preview_plain(1.0)),
            )
            .map(|seconds| seconds as f32)
    }

    /// Mod rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`MOD_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_mod_rate(&self, tempo: Option<f64>) -> Option<f32> {
        let param = &self.mod_rate;
        MOD_SYNC
            .resolve(
                self.mod_sync.value(),
                tempo,
                param.modulated_normalized_value(),
                f64::from(param.preview_plain(0.0)),
                f64::from(param.preview_plain(1.0)),
            )
            .map(|hz| hz as f32)
    }

    /// The space the selector's position sounds, for the editor. **Not for the audio thread**, which
    /// reads the loaded space through its own lock-free copy (`MxmClassicVerb::synchronise`).
    pub fn sounding_space(&self) -> Space {
        self.space.value().sounding(self.loaded.space())
    }
}

impl Default for MxmClassicVerbParams {
    /// The defaults are Init, and Init is one audible sound (plan §2.3): a factory space at its
    /// natural settings, every relative control neutral, the decay natural, modulation and ducking
    /// off, and Mix audibly above zero. **Provisional** until P3.5 chooses the space and mix by ear.
    fn default() -> Self {
        let with = |p: FloatParam, (v2s, s2v): (ValueToString, StringToValue)| {
            p.with_value_to_string(v2s).with_string_to_value(s2v)
        };
        let skewed = |min: f32, max: f32, centre: f32| FloatRange::Skewed {
            min,
            max,
            factor: centred_skew(min, max, centre),
        };
        // The DSP smooths every control itself (crates/mxm-classic-verb-dsp), so no parameter here
        // carries a smoother of its own.
        Self {
            // Init's mix (plan revision 19): the Init space's wet signal 16 dB under a sung phrase,
            // measured at 11.7 % on the four-band build.
            mix: with(
                FloatParam::new("Mix", 0.117, FloatRange::Linear { min: 0.0, max: 1.0 }),
                percent(),
            ),
            space: EnumParam::new("Space", SpaceChoice::INIT),
            decay: with(
                FloatParam::new("Decay", 2.0, skewed(MIN_DECAY_S, MAX_DECAY_S, 2.0)),
                seconds(),
            ),
            bass: with(
                FloatParam::new("Bass decay", 1.0, skewed(MIN_BAND_MULT, MAX_BAND_MULT, 1.0)),
                multiplier(),
            ),
            treble: with(
                FloatParam::new(
                    "Treble decay",
                    1.0,
                    skewed(MIN_BAND_MULT, MAX_BAND_MULT, 1.0),
                ),
                multiplier(),
            ),
            size: with(
                FloatParam::new("Size", 0.060, skewed(MIN_SIZE_S, MAX_SIZE_S, 0.050)),
                milliseconds(),
            ),
            diffusion: with(
                FloatParam::new("Diffusion", 0.75, FloatRange::Linear { min: 0.0, max: 1.0 }),
                percent(),
            ),
            pre_delay: with(
                FloatParam::new("Pre-delay", 0.020, skewed(0.0, MAX_PRE_DELAY_S, 0.050)),
                milliseconds(),
            ),
            pre_delay_sync: BoolParam::new("Pre-delay sync", false),
            early_late: with(
                FloatParam::new(
                    "Early and late",
                    0.0,
                    FloatRange::Linear {
                        min: -mxm_classic_verb_dsp::EARLY_LATE_SILENCE_DB,
                        max: mxm_classic_verb_dsp::EARLY_LATE_SILENCE_DB,
                    },
                ),
                decibels(),
            ),
            shape: EnumParam::new("Decay shape", ShapeChoice::Natural),
            mod_depth: with(
                FloatParam::new("Mod depth", 0.0, skewed(0.0, MAX_MOD_DEPTH_S, 0.001)),
                milliseconds(),
            ),
            mod_rate: with(
                FloatParam::new("Mod rate", 0.5, skewed(0.05, MAX_MOD_RATE_HZ, 0.7)),
                hertz(),
            ),
            mod_sync: BoolParam::new("Mod rate sync", false),
            width: with(
                FloatParam::new(
                    "Width",
                    1.0,
                    FloatRange::Linear {
                        min: MIN_WIDTH,
                        max: MAX_WIDTH,
                    },
                ),
                percent(),
            ),
            low_tone: with(
                FloatParam::new(
                    "Low tone",
                    0.0,
                    FloatRange::Linear {
                        min: -MAX_TONE_DB,
                        max: MAX_TONE_DB,
                    },
                ),
                decibels(),
            ),
            high_tone: with(
                FloatParam::new(
                    "High tone",
                    0.0,
                    FloatRange::Linear {
                        min: -MAX_TONE_DB,
                        max: MAX_TONE_DB,
                    },
                ),
                decibels(),
            ),
            duck: with(
                FloatParam::new("Ducking", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
                percent(),
            ),
            preset: RwLock::new(PresetIdentity::none()),
            loaded: LoadedField::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Pre-delay's sync picks a division and is inert without a tempo**
    /// (`plans/plan-tempo-sync-controls.md`): off, or with no tempo, the knob's own time stands; on
    /// at 120 bpm the ends are the ladder's ends that the range can hold, the top the longest.
    #[test]
    fn pre_delay_sync_picks_a_division_and_is_inert_without_a_tempo() {
        use nice_plug::params::InternalParamMut;
        fn set<P: InternalParamMut>(param: &P, normalized: f32) {
            unsafe {
                let _ = param._internal_set_normalized_value(normalized);
            }
        }
        let p = MxmClassicVerbParams::default();
        set(&p.pre_delay, 1.0);
        assert_eq!(
            p.synced_pre_delay(Some(120.0)),
            None,
            "off is the free time"
        );
        set(&p.pre_delay_sync, 1.0);
        assert_eq!(p.synced_pre_delay(None), None, "no tempo is the free time");

        let top = p.synced_pre_delay(Some(120.0)).expect("synced at a tempo");
        set(&p.pre_delay, 0.0);
        let bottom = p.synced_pre_delay(Some(120.0)).expect("synced at a tempo");
        let (lo, hi) = (
            f64::from(p.pre_delay.preview_plain(0.0)),
            f64::from(p.pre_delay.preview_plain(1.0)),
        );
        assert!(
            top > bottom,
            "the top of a time is the longest: {bottom} to {top}"
        );
        let reach = PRE_DELAY_SYNC.reachable(120.0, lo, hi).divisions();
        let shortest = reach[0].seconds(120.0) as f32;
        let longest = reach[reach.len() - 1].seconds(120.0) as f32;
        assert!(
            (bottom - shortest).abs() < 1e-5,
            "{bottom} against {shortest}"
        );
        assert!((top - longest).abs() < 1e-5, "{top} against {longest}");
    }

    /// **Mod rate's sync picks a division and is inert without a tempo**
    /// (`plans/plan-tempo-sync-controls.md`): off, or with no tempo, the knob's own hertz stand; on
    /// at 120 bpm the ends are the ladder's ends that the range can hold, the top the fastest.
    #[test]
    fn mod_sync_picks_a_division_and_is_inert_without_a_tempo() {
        use nice_plug::params::InternalParamMut;
        fn set<P: InternalParamMut>(param: &P, normalized: f32) {
            unsafe {
                let _ = param._internal_set_normalized_value(normalized);
            }
        }
        let p = MxmClassicVerbParams::default();
        set(&p.mod_rate, 1.0);
        assert_eq!(p.synced_mod_rate(Some(120.0)), None, "off is the free rate");
        set(&p.mod_sync, 1.0);
        assert_eq!(p.synced_mod_rate(None), None, "no tempo is the free rate");

        let top = p.synced_mod_rate(Some(120.0)).expect("synced at a tempo");
        set(&p.mod_rate, 0.0);
        let bottom = p.synced_mod_rate(Some(120.0)).expect("synced at a tempo");
        let (lo, hi) = (
            f64::from(p.mod_rate.preview_plain(0.0)),
            f64::from(p.mod_rate.preview_plain(1.0)),
        );
        assert!(
            top > bottom,
            "the top of a rate is the fastest: {bottom} to {top}"
        );
        let reach = MOD_SYNC.reachable(120.0, lo, hi).divisions();
        let fastest = reach[0].hz(120.0) as f32;
        let slowest = reach[reach.len() - 1].hz(120.0) as f32;
        assert!((top - fastest).abs() < 1e-4, "{top} against {fastest}");
        assert!(
            (bottom - slowest).abs() < 1e-4,
            "{bottom} against {slowest}"
        );
    }

    #[test]
    fn the_effect_opens_engaged_with_every_relative_control_neutral() {
        let p = MxmClassicVerbParams::default();
        assert!(
            p.mix.value() > 0.0,
            "Mix at zero is Off and must never be the default"
        );
        assert_eq!(p.bass.value(), 1.0);
        assert_eq!(p.treble.value(), 1.0);
        assert_eq!(p.early_late.value(), 0.0);
        assert_eq!(p.width.value(), 1.0);
        assert_eq!(p.low_tone.value(), 0.0);
        assert_eq!(p.high_tone.value(), 0.0);
        assert_eq!(p.mod_depth.value(), 0.0);
        assert_eq!(p.duck.value(), 0.0);
        assert_eq!(p.shape.value(), ShapeChoice::Natural);
        assert_ne!(p.space.value(), SpaceChoice::Loaded);
    }

    /// `Loaded` with nothing loaded sounds the Init space, and with a space loaded sounds that space.
    #[test]
    fn loaded_plays_the_init_space_until_a_space_is_loaded() {
        assert_eq!(
            SpaceChoice::Loaded.sounding(None),
            SpaceChoice::INIT.factory()
        );
        assert_eq!(
            SpaceChoice::Loaded.sounding(Some(Space::PLATE)),
            Space::PLATE
        );
        let first = SpaceChoice::from_index(0);
        assert_eq!(first.sounding(Some(Space::PLATE)), FACTORY[0].space);
        assert_eq!(
            MxmClassicVerbParams::default().space.value(),
            SpaceChoice::INIT
        );
    }

    #[test]
    fn the_multipliers_put_unity_at_the_middle_of_their_travel() {
        let p = MxmClassicVerbParams::default();
        for param in [&p.bass, &p.treble] {
            assert!((param.preview_normalized(1.0) - 0.5).abs() < 1e-4);
        }
    }

    /// Plain values either side of a formatter's branch point: the point, and fractions of the
    /// **finer** branch's printed step around it, both halves of its rounding included.
    fn around(point: f32, step: f32) -> impl Iterator<Item = f32> {
        [
            -1.0, -0.6, -0.5, -0.49, -0.4, -0.1, 0.0, 0.1, 0.4, 0.49, 0.5, 0.6, 1.0,
        ]
        .into_iter()
        .map(move |k| point + k * step)
    }

    /// Every place a formatter here changes its precision or its words, in the parameter's plain
    /// units, with the finer branch's printed step. A point past a range's end is clamped by the
    /// parameter and costs nothing.
    fn branch_points(id: &str) -> Vec<(f32, f32)> {
        match id {
            // `seconds`: hundredths below ten seconds, tenths from ten. Both where the hundredths
            // round to ten and where the tenths do, 9.95, which is where it used to switch.
            "decay" => vec![(9.95, 0.01), (10.0, 0.01)],
            // `milliseconds`: tenths below ten milliseconds, whole milliseconds from ten.
            "size" | "predelay" | "moddepth" => vec![(0.010, 0.000_1)],
            // `decibels`: "0.0 dB" where a tenth rounds to zero, a signed tenth either side.
            "earlylate" | "lowtone" | "hightone" => vec![(-0.05, 0.1), (0.05, 0.1)],
            _ => Vec::new(),
        }
    }

    /// One trip through the host, as nice-plug's CLAP wrapper makes it (`vendor/nice-plug` before
    /// the split; the mxm-audio/nice-plug fork since 2026-10-06): the CLAP value is the
    /// normalized value times the step count, the text carries the unit, and the parsed text
    /// comes back through the parameter's normalized conversion before it is formatted again.
    /// Returns the failure, if the text changed or did not parse.
    ///
    /// # Safety
    ///
    /// `ptr` must point at a parameter that outlives the call.
    unsafe fn host_round_trip(id: &str, ptr: ParamPtr, clap_value: f64) -> Option<String> {
        unsafe {
            let steps = ptr.step_count().unwrap_or(1);
            let normalised = clap_value as f32 / steps as f32;
            let first = ptr.normalized_value_to_string(normalised, true);
            let Some(parsed) = ptr.string_to_normalized_value(&first) else {
                return Some(format!("{id}: {first:?} does not parse"));
            };
            let back = parsed as f64 * steps as f64;
            let second = ptr.normalized_value_to_string(back as f32 / steps as f32, true);
            (second != first).then(|| {
                format!(
                    "{id} at plain {}: {first:?} came back {second:?}",
                    ptr.preview_plain(normalised)
                )
            })
        }
    }

    /// **Every parameter's text survives the host's normalized conversion unchanged.** A host
    /// formats a value, parses the text, normalizes the result and formats that again, and
    /// `clap-validator`'s `param-conversions` checks that the text came back identical — on a grid
    /// of its own, so a clean validator run proves nothing about a sliver between its points.
    ///
    /// **Branch points and zero, not only a grid.** The 98-point grid this replaced passed Width, the
    /// one percentage whose range crosses zero, printing `-0 %` a hair below zero and `0 %` on the
    /// way back.
    ///
    /// **Decay is probed where it switches, and is a guard there rather than a reproduction.**
    /// `seconds` picks hundredths or tenths from the rounded *tenth*, so the one `f32` just under
    /// 9.95 whose tenth rounds up would print `9.9 s` and come back `9.90 s`. This range's
    /// normalized conversion cannot produce that value — 9.95 itself comes back a ULP lower, and a
    /// sweep of every normalized neighbour around it found no failure — so nothing here fails today;
    /// a change to the range or its skew that made it reachable would.
    ///
    /// Probes, for every parameter in `param_map` — the selector and Decay shape included: the
    /// twenty-step grid; `clap-validator` 0.4.1's own grid, whose size follows the parameter count;
    /// both sides of every branch point in [`branch_points`]; and a hair either side of zero on a
    /// range that crosses it.
    #[test]
    fn every_parameter_text_is_idempotent_through_the_hosts_conversion() {
        let params = MxmClassicVerbParams::default();
        let map = params.param_map();
        let validator_points = 4000usize.div_ceil(map.len()).clamp(5, 100);
        let mut failures = Vec::new();
        for (id, ptr, _) in map {
            // SAFETY: `params` owns every parameter these pointers refer to and outlives the loop;
            // this is the access `param-conversions` makes through CLAP.
            unsafe {
                let steps = ptr.step_count().unwrap_or(1) as f64;
                let mut probes: Vec<f64> = (0..=19).map(|i| steps * i as f64 / 19.0).collect();
                probes.extend(
                    (0..validator_points)
                        .map(|i| steps * (i as f64 / (validator_points - 1) as f64)),
                );
                let mut plains: Vec<f32> = branch_points(&id)
                    .into_iter()
                    .flat_map(|(point, step)| around(point, step))
                    .collect();
                let (low, high) = (ptr.preview_plain(0.0), ptr.preview_plain(1.0));
                if low.min(high) < 0.0 && low.max(high) > 0.0 {
                    plains.extend([-1.0e-3, -1.0e-4, 0.0, 1.0e-4, 1.0e-3]);
                }
                probes.extend(
                    plains
                        .into_iter()
                        .map(|plain| ptr.preview_normalized(plain) as f64 * steps),
                );
                failures.extend(
                    probes
                        .into_iter()
                        .filter_map(|value| host_round_trip(&id, ptr, value)),
                );
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
