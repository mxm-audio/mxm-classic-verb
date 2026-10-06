//! The factory-space audit (plan §5.1): every factory space rendered **through the plugin** — its
//! selector position, its fitted Decay, Size, Diffusion and Pre-delay, every relative control neutral —
//! as its fit's verification was rendered, analysed by the fit crate's analyser, and compared with the
//! verification descriptors its committed report recorded when the space was generated. **A DSP,
//! parameter or table change that moves a fitted space fails here**, without the response it was
//! fitted from.
//!
//! Shared by `examples/classic_verb_space_audit.rs` and `tests/space_audit.rs`. The tolerances and the
//! measurement they were chosen from are in `AGENTS.md`, *The space audit*.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use mxm_classic_verb::MxmClassicVerb;
use mxm_classic_verb::params::{ShapeChoice, SpaceChoice};
use mxm_classic_verb::payload::Payload;
use mxm_classic_verb::spaces::{FACTORY, FactorySpace};
use mxm_classic_verb_dsp::{EARLY_TAPS, EarlyTap, SPACE_VERSION, Space};
use mxm_classic_verb_fit::{
    Analysis, BandReading, Segment, TailTexture, TextureSegments, analyse_with_segments,
};
use nice_plug::params::InternalParamMut;
use nice_plug::params::persist::PersistentField;
use nice_plug::prelude::{Enum, Param};
use serde_json::Value;

/// How far a descriptor may move from what its report recorded. Chosen in `AGENTS.md`, *The space
/// audit*, against the measured effect of small changes to a space and its controls.
#[derive(Clone, Copy, Debug)]
pub struct Tolerances {
    /// A band's T30 (T20 where neither side has a T30), percent of the recorded time.
    pub decay_percent: f32,
    /// A band's level against 1 kHz, dB.
    pub tone_db: f32,
    pub drr_db: f32,
    pub pre_delay_s: f32,
    pub mixing_time_s: f32,
    pub iacc: f32,
    /// Root-mean-square echo density difference over the points both sides measured.
    pub profile: f32,
    /// Echo density points measured on one side only.
    pub profile_one_sided: usize,
    /// A recorded reflection (of its strongest twelve) against the render's nearest one.
    pub reflection_s: f32,
    pub reflection_db: f32,
    pub confidence: f32,
    /// A tail texture's spectral peakiness, on either segment.
    pub peakiness: f32,
    pub kurtosis: f32,
    /// A tail texture's mean echo density, on either segment.
    pub texture_density: f32,
    pub periodicity: f32,
}

/// **Measured** (`measure_what_small_changes_move`, release, Windows, the four provisional spaces).
/// Each sits above the most rendering through the plugin moves a descriptor as generated across the
/// hundred factory spaces, and under what the small change named beside it moves on the space the
/// moved-space test renders; each change is a small fraction of the just-noticeable differences the
/// Space card ranks errors by (`AGENTS.md`, *The space audit*).
pub const TOLERANCES: Tolerances = Tolerances {
    // Floors are the worst of the owner's hundred as generated; each movement is tight-live-room's,
    // the space `a_moved_space_is_caught_and_the_descriptor_named` renders.
    // Floor 0.013 % (frozen-drone); the top decay ratio ×1.01 moves it 0.45 %, Decay ×1.01 0.90 %.
    decay_percent: 0.3,
    // Floor 0.0028 dB; the high shelf +0.1 dB moves it 0.064 dB.
    tone_db: 0.02,
    // Floor 0.0005 dB; the high shelf +0.1 dB moves it 0.062 dB.
    drr_db: 0.02,
    // Floor 0; Pre-delay +0.1 ms moves it 0.091 ms. Under three samples at 48 kHz.
    pre_delay_s: 0.00005,
    // Floor 0 — a first passage, which jumps (crates/mxm-classic-verb-fit/AGENTS.md); Size ×1.01
    // moves it 6 ms. The profile below carries the same information steadily.
    mixing_time_s: 0.003,
    // Floor 0.0001; Width ×0.99 moves it 0.0015. Lowered from 0.004 on the hundred, where that move
    // went unnamed.
    iacc: 0.001,
    // Floor 0.0021 RMS (frozen-drone); Diffusion −0.01 moves it 0.0087, Size ×1.01 0.033.
    profile: 0.005,
    // Floor 0; Pre-delay +0.1 ms moves three points onto one side.
    profile_one_sided: 2,
    // Floor 0; every tap 0.01 Size later moves it 0.57 ms.
    reflection_s: 0.00005,
    // Floor 0.0026 dB; the early level ×1.02 moves it 0.17 dB.
    reflection_db: 0.05,
    // Floor 0.228 (bass-bloom, whose 8 kHz noise margin sits on the confidence ramp, so a render through
    // the plugin crosses it); small changes elsewhere move it at most 0.057, and each such move is named
    // by another descriptor too. Only a collapse is caught here.
    confidence: 0.25,
    // Floor 0.0023 (frozen-drone, a 20 s decay); Diffusion −0.01 moves it 0.0046, Size ×1.01 0.012.
    peakiness: 0.003,
    // Floor 0.0037 (tight-live-room itself); Decay ×1.01 moves it 0.064.
    kurtosis: 0.005,
    // Floor 0.00046 (glass-plate); Diffusion −0.01 moves it 0.011.
    texture_density: 0.001,
    // Floor 0; Diffusion −0.01 moves it 0.0024, Size ×1.01 0.046.
    periodicity: 0.001,
};

/// A recorded reflection is looked for this far either side in the render; past it, it is missing.
/// The fit crate's own `REFLECTION_MATCH_S`.
pub const REFLECTION_SEARCH_S: f32 = 0.001;
/// The strongest recorded reflections compared, as the fit compares them.
pub const REFLECTIONS_COMPARED: usize = EARLY_TAPS;
const BLOCK: usize = 512;

/// How a fit's verification was rendered, as its report records it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recipe {
    pub sample_rate: u32,
    pub channels: usize,
    pub lead_frames: usize,
    pub frames: usize,
    pub wet_gain: f32,
}

/// Which way a render reaches the space: its own selector position, or the loaded space under
/// `Loaded`, which is the path a dropped response takes and the only one a changed space has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Through {
    Position,
    Loaded,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BandDescriptors {
    pub centre_hz: f32,
    pub t20_s: Option<f32>,
    pub t30_s: Option<f32>,
    pub level_re_1k_db: Option<f32>,
}

/// A tail texture as the audit compares it: the segment it was read on and its four readings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureDescriptors {
    pub segment: Segment,
    pub peakiness: Option<f32>,
    pub kurtosis: Option<f32>,
    pub echo_density: Option<f32>,
    pub periodicity: Option<f32>,
}

impl TextureDescriptors {
    fn of(t: &TailTexture) -> TextureDescriptors {
        TextureDescriptors {
            segment: t.segment,
            peakiness: t.peakiness,
            kurtosis: t.kurtosis,
            echo_density: t.echo_density,
            periodicity: t.periodicity,
        }
    }

    /// `Some(None)` for a recorded `null`, `None` for a texture missing its segment.
    fn from_json(v: &Value) -> Option<Option<TextureDescriptors>> {
        if v.is_null() {
            return Some(None);
        }
        Some(Some(TextureDescriptors {
            segment: Segment {
                start_s: number(&v["start_s"])?,
                end_s: number(&v["end_s"])?,
            },
            peakiness: number(&v["peakiness"]),
            kurtosis: number(&v["kurtosis"]),
            echo_density: number(&v["echo_density"]),
            periodicity: number(&v["periodicity"]),
        }))
    }
}

/// The descriptors the audit compares, from a report's JSON or from a fresh analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct Descriptors {
    pub bands: Vec<BandDescriptors>,
    pub drr_db: Option<f32>,
    pub pre_delay_s: Option<f32>,
    pub mixing_time_s: Option<f32>,
    pub iacc: Option<f32>,
    /// Delay and level.
    pub reflections: Vec<(f32, f32)>,
    pub echo_density: Vec<Option<f32>>,
    /// The tail texture over segments A and B.
    pub textures: [Option<TextureDescriptors>; 2],
    pub confidence: f32,
}

impl Descriptors {
    /// The segments the textures were read on, which a render is read on again.
    pub fn texture_segments(&self) -> TextureSegments {
        TextureSegments {
            a: self.textures[0].map(|t| t.segment),
            b: self.textures[1].map(|t| t.segment),
        }
    }

    pub fn of(a: &Analysis) -> Descriptors {
        Descriptors {
            bands: a
                .bands
                .iter()
                .map(|b| {
                    let d = match &b.reading {
                        BandReading::Measured(d) => Some(d),
                        _ => None,
                    };
                    BandDescriptors {
                        centre_hz: b.centre_hz,
                        t20_s: d.and_then(|d| d.t20_s),
                        t30_s: d.and_then(|d| d.t30_s),
                        level_re_1k_db: d.and_then(|d| d.level_re_1k_db),
                    }
                })
                .collect(),
            drr_db: a.onset.drr_db,
            pre_delay_s: a.pre_delay_s,
            mixing_time_s: a.echo_density.mixing_time_s,
            iacc: a.width.measured().map(|c| c.iacc),
            reflections: a
                .early
                .reflections
                .iter()
                .map(|r| (r.delay_s, r.level_db))
                .collect(),
            echo_density: a.echo_density.points.iter().map(|p| p.density).collect(),
            textures: [
                a.texture_a.as_ref().map(TextureDescriptors::of),
                a.texture_b.as_ref().map(TextureDescriptors::of),
            ],
            confidence: a.validity.confidence,
        }
    }

    fn from_json(v: &Value) -> Option<Descriptors> {
        Some(Descriptors {
            bands: v["bands"]
                .as_array()?
                .iter()
                .map(|b| {
                    Some(BandDescriptors {
                        centre_hz: number(&b["centre_hz"])?,
                        t20_s: number(&b["t20_s"]),
                        t30_s: number(&b["t30_s"]),
                        level_re_1k_db: number(&b["level_re_1k_db"]),
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            drr_db: number(&v["drr_db"]),
            pre_delay_s: number(&v["pre_delay_s"]),
            mixing_time_s: number(&v["mixing_time_s"]),
            iacc: number(&v["width"]["iacc"]),
            reflections: v["reflections"]
                .as_array()?
                .iter()
                .map(|r| Some((number(&r["delay_s"])?, number(&r["level_db"])?)))
                .collect::<Option<Vec<_>>>()?,
            echo_density: v["echo_density"].as_array()?.iter().map(number).collect(),
            textures: [
                TextureDescriptors::from_json(&v["texture_a"])?,
                TextureDescriptors::from_json(&v["texture_b"])?,
            ],
            confidence: number(&v["confidence"])?,
        })
    }
}

/// The worst movement of each descriptor, for one space or across several.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Figures {
    pub decay_percent: f32,
    pub tone_db: f32,
    pub drr_db: f32,
    pub pre_delay_s: f32,
    pub mixing_time_s: f32,
    pub iacc: f32,
    pub profile: f32,
    pub profile_one_sided: usize,
    pub reflection_s: f32,
    pub reflection_db: f32,
    pub unmatched_reflections: usize,
    pub confidence: f32,
    pub peakiness: f32,
    pub kurtosis: f32,
    pub texture_density: f32,
    pub periodicity: f32,
}

impl Figures {
    pub fn worst(self, other: Figures) -> Figures {
        Figures {
            peakiness: self.peakiness.max(other.peakiness),
            kurtosis: self.kurtosis.max(other.kurtosis),
            texture_density: self.texture_density.max(other.texture_density),
            periodicity: self.periodicity.max(other.periodicity),
            decay_percent: self.decay_percent.max(other.decay_percent),
            tone_db: self.tone_db.max(other.tone_db),
            drr_db: self.drr_db.max(other.drr_db),
            pre_delay_s: self.pre_delay_s.max(other.pre_delay_s),
            mixing_time_s: self.mixing_time_s.max(other.mixing_time_s),
            iacc: self.iacc.max(other.iacc),
            profile: self.profile.max(other.profile),
            profile_one_sided: self.profile_one_sided.max(other.profile_one_sided),
            reflection_s: self.reflection_s.max(other.reflection_s),
            reflection_db: self.reflection_db.max(other.reflection_db),
            unmatched_reflections: self.unmatched_reflections.max(other.unmatched_reflections),
            confidence: self.confidence.max(other.confidence),
        }
    }

    pub const HEADER: &str = "decay %  tone dB  DRR dB  pre-delay ms  mixing ms  IACC     profile  one-sided  refl ms  refl dB  unmatched  confidence  peaky    kurtosis  density  periodic";

    pub fn row(&self) -> String {
        format!(
            "{:<8.4} {:<8.4} {:<7.4} {:<13.4} {:<10.3} {:<8.5} {:<8.5} {:<10} {:<8.4} {:<8.4} {:<10} {:<11.4} {:<8.4} {:<9.4} {:<8.5} {:.4}",
            self.decay_percent,
            self.tone_db,
            self.drr_db,
            1000.0 * self.pre_delay_s,
            1000.0 * self.mixing_time_s,
            self.iacc,
            self.profile,
            self.profile_one_sided,
            1000.0 * self.reflection_s,
            self.reflection_db,
            self.unmatched_reflections,
            self.confidence,
            self.peakiness,
            self.kurtosis,
            self.texture_density,
            self.periodicity,
        )
    }
}

/// One space's audit.
#[derive(Clone, Debug)]
pub struct Audited {
    pub index: usize,
    pub id: &'static str,
    pub seconds: f32,
    pub figures: Figures,
    pub misses: Vec<String>,
}

fn number(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

fn count(v: &Value) -> Option<usize> {
    v.as_u64().and_then(|x| usize::try_from(x).ok())
}

pub fn reports_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("spaces")
}

fn space_from_json(v: &Value) -> Option<Space> {
    if v["version"].as_u64()? != u64::from(SPACE_VERSION) {
        return None;
    }
    let early = v["early"].as_array()?;
    if early.len() != EARLY_TAPS {
        return None;
    }
    let mut taps = [EarlyTap {
        time: 0.0,
        gain_l: 0.0,
        gain_r: 0.0,
    }; EARLY_TAPS];
    for (tap, json) in taps.iter_mut().zip(early) {
        *tap = EarlyTap {
            time: number(&json["time_units"])?,
            gain_l: number(&json["gain_l"])?,
            gain_r: number(&json["gain_r"])?,
        };
    }
    Some(Space {
        early: taps,
        early_level: number(&v["early_level"])?,
        decay_ratio_low: number(&v["decay_ratio_low"])?,
        decay_ratio_high: number(&v["decay_ratio_high"])?,
        decay_ratio_top: number(&v["decay_ratio_top"])?,
        tone_low_db: number(&v["tone_low_db"])?,
        tone_high_db: number(&v["tone_high_db"])?,
        high_cut_hz: number(&v["high_cut_hz"])?,
        width: number(&v["width"])?,
    })
}

/// What a committed report holds for the audit: the recipe and the recorded descriptors, once the
/// report is shown to be this table entry's.
pub struct Committed {
    pub recipe: Recipe,
    /// Its tail textures were read on the response's segments, and a render is read on them again.
    pub verification: Descriptors,
}

/// Reads and checks a space's report: its source is the id, its space and controls are the table's
/// to the bit, it carries no verification refusal, and its recipe and descriptors are complete.
pub fn committed(entry: &FactorySpace) -> Result<Committed, String> {
    let id = entry.id;
    let text = std::fs::read_to_string(reports_folder().join(format!("{id}.json")))
        .map_err(|e| format!("{id}: its report could not be read: {e}"))?;
    let report: Value =
        serde_json::from_str(&text).map_err(|e| format!("{id}: its report does not parse: {e}"))?;
    if report["source"] != id {
        return Err(format!("{id}: its report names another source"));
    }
    if space_from_json(&report["space"]) != Some(entry.space) {
        return Err(format!(
            "{id}: the table's space is not the space its report recorded"
        ));
    }
    let controls = &report["controls"];
    if [
        number(&controls["decay_s"]),
        number(&controls["size_s"]),
        number(&controls["diffusion"]),
        number(&controls["pre_delay_s"]),
    ] != [
        Some(entry.decay_s),
        Some(entry.size_s),
        Some(entry.diffusion),
        Some(entry.pre_delay_s),
    ] {
        return Err(format!(
            "{id}: the table's controls are not the controls its report recorded"
        ));
    }
    if !report["verification_refusal"].is_null() {
        return Err(format!("{id}: its report carries a verification refusal"));
    }
    let render = &report["render"];
    let recipe = (|| {
        Some(Recipe {
            sample_rate: u32::try_from(render["sample_rate"].as_u64()?).ok()?,
            channels: count(&render["channels"])?,
            lead_frames: count(&render["lead_frames"])?,
            frames: count(&render["frames"])?,
            wet_gain: number(&render["wet_gain"])?,
        })
    })()
    .filter(|r| (1..=2).contains(&r.channels) && r.lead_frames < r.frames)
    .ok_or_else(|| format!("{id}: its report has no complete render recipe"))?;
    let verification = Descriptors::from_json(&report["verification"])
        .ok_or_else(|| format!("{id}: its report's verification descriptors are incomplete"))?;
    Ok(Committed {
        recipe,
        verification,
    })
}

fn set<P: Param + InternalParamMut>(param: &P, value: P::Plain) {
    // SAFETY: the plugin belongs to this thread and nothing else reads its parameters.
    unsafe {
        let _ = param._internal_set_normalized_value(param.preview_normalized(value));
    }
}

/// The verification render made again through the plugin: `entry`'s space at its selector
/// `position` — or through `Loaded` when asked, or when `entry`'s space is not the position's — its
/// fitted controls, every relative control neutral, modulation depth zero, the decay natural, ducking
/// off and Mix one; a unit impulse into both inputs after the lead; the dry leak at the impulse's frame
/// taken off, a mono response folded, the wet scaled and the unit direct sound added. The fit's own
/// recipe (`crates/mxm-classic-verb-fit/examples/classic_verb_io/generate.rs`).
pub fn render(
    position: SpaceChoice,
    entry: &FactorySpace,
    through: Through,
    recipe: &Recipe,
) -> Vec<Vec<f32>> {
    let mut plugin = MxmClassicVerb::default();
    {
        let p = &plugin.params;
        let own = position.factory_entry().map(|table| table.space) == Some(entry.space);
        if through == Through::Position && own {
            set(&p.space, position);
        } else {
            p.loaded.set(Payload::holding(&entry.space, None));
            set(&p.space, SpaceChoice::Loaded);
        }
        set(&p.mix, 1.0);
        set(&p.decay, entry.decay_s);
        set(&p.size, entry.size_s);
        set(&p.diffusion, entry.diffusion);
        set(&p.pre_delay, entry.pre_delay_s);
        set(&p.bass, 1.0);
        set(&p.treble, 1.0);
        set(&p.early_late, 0.0);
        set(&p.shape, ShapeChoice::Natural);
        set(&p.mod_depth, 0.0);
        set(&p.mod_rate, 0.5);
        set(&p.width, 1.0);
        set(&p.low_tone, 0.0);
        set(&p.high_tone, 0.0);
        set(&p.duck, 0.0);
    }
    plugin.prepare_for_test(recipe.sample_rate as f32, 2);
    let (lead, frames) = (recipe.lead_frames, recipe.frames);
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[lead] = 1.0;
    right[lead] = 1.0;
    // The first block ends at the impulse, so the engine, parked on silence until then, starts where
    // the fit's renderer starts.
    let mut start = 0;
    while start < frames {
        let end = if start < lead {
            lead
        } else {
            (start + BLOCK).min(frames)
        };
        let mut channels: Vec<&mut [f32]> = vec![&mut left[start..end], &mut right[start..end]];
        plugin.process_block_for_test(&mut channels);
        start = end;
    }
    let dry = core::f32::consts::FRAC_PI_2.cos();
    left[lead] -= dry;
    right[lead] -= dry;
    let wet = if recipe.channels == 1 {
        vec![
            left.iter()
                .zip(&right)
                .map(|(l, r)| 0.5 * (l + r))
                .collect::<Vec<f32>>(),
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

fn scalar(
    misses: &mut Vec<String>,
    id: &str,
    what: &str,
    (recorded, now): (Option<f32>, Option<f32>),
    tolerance: f32,
    relative: bool,
    worst: &mut f32,
) {
    match (recorded, now) {
        (None, None) => {}
        (Some(r), Some(n)) => {
            let moved = if relative {
                100.0 * (n / r - 1.0)
            } else {
                n - r
            };
            if moved.is_finite() {
                *worst = worst.max(moved.abs());
            }
            if moved.is_nan() || moved.abs() > tolerance {
                misses.push(format!(
                    "{id}: {what} moved from {r} to {n} ({moved:+}), past ±{tolerance}"
                ));
            }
        }
        (r, n) => misses.push(format!(
            "{id}: {what} was {r:?} when generated and is {n:?} now"
        )),
    }
}

/// Compares a render's descriptors with the recorded ones.
pub fn compare(
    id: &str,
    recorded: &Descriptors,
    now: &Descriptors,
    tolerances: &Tolerances,
) -> (Figures, Vec<String>) {
    let mut f = Figures::default();
    let mut misses = Vec::new();
    let t = tolerances;
    if recorded.bands.len() != now.bands.len() {
        misses.push(format!("{id}: the analysis has a different set of bands"));
    }
    for (r, n) in recorded.bands.iter().zip(&now.bands) {
        let hz = r.centre_hz;
        let (what, pair) = if r.t30_s.is_some() || n.t30_s.is_some() {
            (format!("T30 at {hz} Hz"), (r.t30_s, n.t30_s))
        } else {
            (format!("T20 at {hz} Hz"), (r.t20_s, n.t20_s))
        };
        scalar(
            &mut misses,
            id,
            &what,
            pair,
            t.decay_percent,
            true,
            &mut f.decay_percent,
        );
        scalar(
            &mut misses,
            id,
            &format!("the level against 1 kHz at {hz} Hz"),
            (r.level_re_1k_db, n.level_re_1k_db),
            t.tone_db,
            false,
            &mut f.tone_db,
        );
    }
    let scalars = [
        (
            "the DRR",
            (recorded.drr_db, now.drr_db),
            t.drr_db,
            &mut f.drr_db,
        ),
        (
            "the pre-delay",
            (recorded.pre_delay_s, now.pre_delay_s),
            t.pre_delay_s,
            &mut f.pre_delay_s,
        ),
        (
            "the mixing time",
            (recorded.mixing_time_s, now.mixing_time_s),
            t.mixing_time_s,
            &mut f.mixing_time_s,
        ),
        ("the IACC", (recorded.iacc, now.iacc), t.iacc, &mut f.iacc),
        (
            "the confidence",
            (Some(recorded.confidence), Some(now.confidence)),
            t.confidence,
            &mut f.confidence,
        ),
    ];
    for (what, pair, tolerance, worst) in scalars {
        scalar(&mut misses, id, what, pair, tolerance, false, worst);
    }

    if recorded.echo_density.len() != now.echo_density.len() {
        misses.push(format!(
            "{id}: the echo density profile has {} points, and had {} when generated",
            now.echo_density.len(),
            recorded.echo_density.len()
        ));
    }
    let (mut sum, mut both) = (0.0f64, 0usize);
    for (r, n) in recorded.echo_density.iter().zip(&now.echo_density) {
        match (r, n) {
            (Some(r), Some(n)) => {
                sum += (f64::from(*r) - f64::from(*n)).powi(2);
                both += 1;
            }
            (None, None) => {}
            _ => f.profile_one_sided += 1,
        }
    }
    if both > 0 {
        f.profile = (sum / both as f64).sqrt() as f32;
    }
    if f.profile.is_nan() || f.profile > t.profile {
        misses.push(format!(
            "{id}: the echo density profile moved by {} RMS, past {}",
            f.profile, t.profile
        ));
    }
    if f.profile_one_sided > t.profile_one_sided {
        misses.push(format!(
            "{id}: {} echo density points are measured on one side only, past {}",
            f.profile_one_sided, t.profile_one_sided
        ));
    }

    // The tail textures, both read on the segments the report recorded.
    for (name, (r, n)) in ["A", "B"]
        .iter()
        .zip(recorded.textures.iter().zip(&now.textures))
    {
        match (r, n) {
            (None, None) => {}
            (Some(r), Some(n)) => {
                if r.segment != n.segment {
                    misses.push(format!(
                        "{id}: segment {name} was read on {:?} when generated and on {:?} now",
                        r.segment, n.segment
                    ));
                }
                let readings = [
                    (
                        "spectral peakiness",
                        (r.peakiness, n.peakiness),
                        t.peakiness,
                        &mut f.peakiness,
                    ),
                    (
                        "kurtosis",
                        (r.kurtosis, n.kurtosis),
                        t.kurtosis,
                        &mut f.kurtosis,
                    ),
                    (
                        "echo density",
                        (r.echo_density, n.echo_density),
                        t.texture_density,
                        &mut f.texture_density,
                    ),
                    (
                        "periodicity",
                        (r.periodicity, n.periodicity),
                        t.periodicity,
                        &mut f.periodicity,
                    ),
                ];
                for (what, pair, tolerance, worst) in readings {
                    scalar(
                        &mut misses,
                        id,
                        &format!("segment {name}'s {what}"),
                        pair,
                        tolerance,
                        false,
                        worst,
                    );
                }
            }
            (r, n) => misses.push(format!(
                "{id}: segment {name}'s texture was {} when generated and is {} now",
                if r.is_some() { "read" } else { "absent" },
                if n.is_some() { "read" } else { "absent" },
            )),
        }
    }

    let mut strongest = recorded.reflections.clone();
    strongest.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.total_cmp(&b.0)));
    strongest.truncate(REFLECTIONS_COMPARED);
    for (delay, level) in strongest {
        let nearest = now
            .reflections
            .iter()
            .filter(|(d, _)| (d - delay).abs() <= REFLECTION_SEARCH_S)
            .min_by(|a, b| (a.0 - delay).abs().total_cmp(&(b.0 - delay).abs()));
        let at = format!("the reflection at {:.3} ms", 1000.0 * delay);
        match nearest {
            Some(&(d, l)) => {
                scalar(
                    &mut misses,
                    id,
                    &format!("{at}'s delay"),
                    (Some(delay), Some(d)),
                    t.reflection_s,
                    false,
                    &mut f.reflection_s,
                );
                scalar(
                    &mut misses,
                    id,
                    &format!("{at}'s level"),
                    (Some(level), Some(l)),
                    t.reflection_db,
                    false,
                    &mut f.reflection_db,
                );
            }
            None => {
                f.unmatched_reflections += 1;
                misses.push(format!(
                    "{id}: {at} is no longer within {} ms",
                    1000.0 * REFLECTION_SEARCH_S
                ));
            }
        }
    }
    (f, misses)
}

/// Audits factory position `index`, rendering `entry` — the table's own, or a changed copy of it to
/// show what the audit catches — against the table entry's committed report.
pub fn audit_with(
    index: usize,
    entry: &FactorySpace,
    through: Through,
    tolerances: &Tolerances,
) -> Audited {
    let table = &FACTORY[index];
    let mut audited = Audited {
        index,
        id: table.id,
        seconds: 0.0,
        figures: Figures::default(),
        misses: Vec::new(),
    };
    let committed = match committed(table) {
        Ok(committed) => committed,
        Err(miss) => {
            audited.misses.push(miss);
            return audited;
        }
    };
    let recipe = committed.recipe;
    audited.seconds = recipe.frames as f32 / recipe.sample_rate as f32;
    let rendered = render(SpaceChoice::from_index(index), entry, through, &recipe);
    let slices: Vec<&[f32]> = rendered.iter().map(Vec::as_slice).collect();
    // As the fit read its verification: the tail textures on the response's segments.
    match analyse_with_segments(
        &slices,
        recipe.sample_rate as f32,
        &committed.verification.texture_segments(),
    ) {
        Ok(analysis) => {
            let (figures, misses) = compare(
                table.id,
                &committed.verification,
                &Descriptors::of(&analysis),
                tolerances,
            );
            audited.figures = figures;
            audited.misses = misses;
        }
        Err(refusal) => audited
            .misses
            .push(format!("{}: the render was refused: {refusal}", table.id)),
    }
    audited
}

/// Every factory space at its own position, on as many threads as there are cores, in the
/// selector's order.
pub fn audit_all(tolerances: &Tolerances) -> Vec<Audited> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, FACTORY.len().max(1));
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(FACTORY.len()));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(entry) = FACTORY.get(index) else {
                        break;
                    };
                    let audited = audit_with(index, entry, Through::Position, tolerances);
                    results.lock().expect("no audit panicked").push(audited);
                }
            });
        }
    });
    let mut results = results.into_inner().expect("no audit panicked");
    results.sort_by_key(|a| a.index);
    results
}

/// Files in the reports folder that no factory space owns.
pub fn orphaned_reports() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(reports_folder()) else {
        return vec!["the spaces folder could not be read".to_owned()];
    };
    let mut orphans: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !FACTORY.iter().any(|e| format!("{}.json", e.id) == *name))
        .map(|name| format!("`{name}` belongs to no factory space"))
        .collect();
    orphans.sort();
    orphans
}
