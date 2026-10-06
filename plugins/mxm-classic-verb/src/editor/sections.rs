//! The Space, Decay, Texture and Output cards, in signal-flow order, the decay display, and the
//! Space card's drop target and load line.
//!
//! [`all_parameters`] is the one table of what the editor binds: every parameter once, each with the
//! one-sentence description design system §7.1 puts in its tooltip. CLAP carries no such field, so
//! only the plugin can supply it.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Rect, Ui};
use mxm_classic_verb_dsp::{Controls, DecayShape, SHAPE_MAX_S, SHAPE_MIN_S, predicted_decay_s};
use mxm_ui::control::{Size, Wave};
use mxm_ui::space::{MIN_TARGET, SPACE_3};
use mxm_ui::theme::Tokens;
use mxm_ui::tree::{self, Height, Kind, Node};
use mxm_ui::visual::{
    AXIS_STROKE, CANVAS_RADIUS, INNER_GUTTER, METER_STROKE, REFERENCE_STROKE, TALL_PLOT_HEIGHT,
};
use nice_plug::prelude::{Enum, ParamSetter};

use super::binding::{Bound, segmented_waves_named, selector};
use crate::loaded::Status;
use crate::params::{MxmClassicVerbParams, ShapeChoice, SpaceChoice};

const KNOB: Size = Size::Standard;

/// The relative controls: each is an offset on the space with a neutral point that is *the space as
/// fitted*, and each is drawn bipolar so the shared knob shows that point as its detent.
pub const RELATIVE_IDS: [&str; 6] = [
    "earlylate",
    "bass",
    "treble",
    "width",
    "lowtone",
    "hightone",
];

pub fn all_parameters(params: &MxmClassicVerbParams) -> Vec<Bound<'_>> {
    vec![
        Bound::new(
            "space",
            &params.space,
            "Chooses the space: its early reflections, how its bands decay against each other, its tone and its width.",
        ),
        Bound::new(
            "predelay",
            &params.pre_delay,
            "Sets the gap between the dry sound and the space's first answer.",
        ),
        Bound::new(
            "predelaysync",
            &params.pre_delay_sync,
            super::binding::SYNC_DESCRIPTION,
        ),
        Bound::new(
            "size",
            &params.size,
            "The size of the space, from a boxy room to past any building; Decay keeps its time.",
        ),
        Bound::new(
            "earlylate",
            &params.early_late,
            "The balance of early reflections against the reverb tail; 0 dB is the space as it comes.",
        )
        .bipolar(),
        Bound::new(
            "decay",
            &params.decay,
            "How long the reverb lasts.",
        ),
        Bound::new(
            "bass",
            &params.bass,
            "How long the bass rings compared with the rest; ×1 is the space as it comes.",
        )
        .bipolar()
        // The Decay card says what the three are of (B1); a host, the tooltip and a screen reader
        // read *Bass decay*, *Treble decay* and *Decay shape*.
        .labelled("Bass"),
        Bound::new(
            "treble",
            &params.treble,
            "How long the treble rings compared with the rest; ×1 is the space as it comes.",
        )
        .bipolar()
        .labelled("Treble"),
        Bound::new(
            "shape",
            &params.shape,
            "The reverb's shape over time.",
        )
        .labelled("Shape"),
        Bound::new(
            "diffusion",
            &params.diffusion,
            "How smooth the reverb is, from distinct repeats to a smooth wash.",
        ),
        Bound::new(
            "moddepth",
            &params.mod_depth,
            "How much the reverb tail drifts, which thickens it and blurs its pitch.",
        ),
        Bound::new(
            "modrate",
            &params.mod_rate,
            "How fast the reverb tail drifts.",
        ),
        Bound::new("modsync", &params.mod_sync, super::binding::SYNC_DESCRIPTION),
        Bound::new(
            "mix",
            &params.mix,
            "The balance of dry sound and reverb; at zero the effect is off.",
        ),
        Bound::new(
            "width",
            &params.width,
            "The reverb's stereo width; 0 % is mono, and below zero left and right swap.",
        )
        .bipolar(),
        Bound::new(
            "lowtone",
            &params.low_tone,
            "Raises or lowers the reverb's bass.",
        )
        .bipolar(),
        Bound::new(
            "hightone",
            &params.high_tone,
            "Raises or lowers the reverb's treble.",
        )
        .bipolar(),
        Bound::new(
            "duck",
            &params.duck,
            "Lowers the wet sound while the input plays, so the reverb opens up in the gaps.",
        ),
    ]
}

fn bound<'a>(id: &str, params: &'a MxmClassicVerbParams) -> Bound<'a> {
    all_parameters(params)
        .into_iter()
        .find(|binding| binding.id == id)
        .expect("every drawn control is bound")
}

/// The cards' titles, in signal-flow order.
pub const TITLES: [&str; 4] = ["Space", "Decay", "Texture", "Output"];

/// What a leaf of this editor's cards draws. Hashed by what it names, which keeps its widget ids
/// stable when a card is re-paged.
#[derive(Clone, Copy, Debug, Hash)]
pub enum Leaf {
    Knob(&'static str),
    /// The Space selector.
    Space,
    /// The drop target's visible face.
    LoadLine,
    DecayDisplay,
    /// The Decay shape switch, drawn as the three envelopes (C3).
    Shape,
    /// A control's tempo sync, the quarter note beside it.
    Picture(&'static str),
}

/// A control's tempo sync, the quarter note on the grid of the knob beside it
/// (`plans/plan-tempo-sync-controls.md`).
fn sync_beside(id: &'static str) -> Node<Leaf> {
    mxm_ui::tree::switch_beside_knob(KNOB, tree::leaf(Leaf::Picture(id), Kind::SyncToggle))
}

/// The ladder of the control `id`, if it has a tempo sync.
fn ladder_of(id: &str) -> Option<mxm_tempo::Ladder> {
    match id {
        "predelay" => Some(crate::params::PRE_DELAY_SYNC),
        "modrate" => Some(crate::params::MOD_SYNC),
        _ => None,
    }
}

/// Decay shape's pictures, in the parameter's order: the envelope each option puts on the tail.
const SHAPES: [Wave; 3] = [Wave::Decay, Wave::Gated, Wave::Swell];
/// What each shape does, in [`SHAPES`]' order (design system §7.3; the owner, 2026-09-27: the cells
/// of a row do not share one sentence).
const SHAPE_DETAILS: [&str; 3] = [
    "The reverb fades away naturally.",
    "A flat block of reverb that stops dead.",
    "The reverb swells up into a sudden stop.",
];

/// A row of knobs, each in a `KNOB_COLUMN`, the row's spacing apart.
fn knob_row(ui: &Ui, params: &MxmClassicVerbParams, ids: &[&'static str]) -> Node<Leaf> {
    mxm_ui::tree::knob_row(
        ui,
        ids.iter()
            .map(|&id| {
                let bound = bound(id, params);
                let param = bound.param;
                // A syncable control's column holds its free readings and its divisions.
                let widest = match ladder_of(id) {
                    Some(ladder) => super::binding::synced_widest(param, ladder.span),
                    None => mxm_ui::control::widest_value(|n| param.format(n as f32)),
                };
                let knob = tree::leaf(
                    Leaf::Knob(id),
                    Kind::Knob {
                        name: bound.painted().to_owned(),
                        widest,
                        size: KNOB,
                        // In the collection's knob row, which sizes the columns.
                        column: 0.0,
                    },
                );
                (KNOB, knob)
            })
            .collect(),
    )
}

/// A display that fills the width it is given at a fixed height, at least `min_width`.
fn display(key: Leaf, min_width: f32, height: f32) -> Node<Leaf> {
    tree::leaf(
        key,
        Kind::Custom {
            min_width,
            height: Height::Fixed(height),
            fills: true,
        },
    )
}

/// Card `index`'s body, as a tree (plans/plan-layout-tree.md): described once, and that one
/// description is both measured — the card's floor and height — and drawn, leaf by leaf, through
/// the bindings ([`paint`]).
pub fn card(ui: &Ui, index: usize, params: &MxmClassicVerbParams) -> Node<Leaf> {
    match index {
        // **A list, not a segmented switch.** About a hundred positions, so the options are the
        // enum's own names in declaration order — nothing here counts them — through the
        // collection's shared caret selector (§7.4). **Loading a space from a file belongs on this
        // card** (brief §5): the selector's `Loaded` position, the drop target and its one line.
        0 => tree::stack(vec![
            tree::leaf(
                Leaf::Space,
                Kind::Selector {
                    label: bound("space", params).param.name().to_owned(),
                    options: SpaceChoice::variants()
                        .iter()
                        .map(|&o| o.to_owned())
                        .collect(),
                    caption: false,
                    width: None,
                },
            ),
            display(Leaf::LoadLine, load_line_min_width(ui), MIN_TARGET),
            // Pre-delay with its tempo sync beside it, one flat row with what follows.
            tree::row_gap(
                ui.spacing().item_spacing.x,
                vec![
                    knob_row(ui, params, &["predelay"]),
                    sync_beside("predelaysync"),
                    knob_row(ui, params, &["size", "earlylate"]),
                ],
            ),
        ]),
        1 => tree::stack(vec![
            display(Leaf::DecayDisplay, DISPLAY_MIN_WIDTH, TALL_PLOT_HEIGHT),
            knob_row(ui, params, &["decay", "bass", "treble"]),
            tree::leaf(
                Leaf::Shape,
                Kind::Waves {
                    label: Some(bound("shape", params).painted().to_owned()),
                    count: SHAPES.len(),
                    marks: Vec::new(),
                    beside: None,
                },
            ),
        ]),
        // Mod rate with its tempo sync beside it.
        2 => tree::stack(vec![tree::row_gap(
            ui.spacing().item_spacing.x,
            vec![
                knob_row(ui, params, &["diffusion", "moddepth", "modrate"]),
                sync_beside("modsync"),
            ],
        )]),
        _ => tree::stack(vec![
            knob_row(ui, params, &["mix", "width"]),
            knob_row(ui, params, &["lowtone", "hightone", "duck"]),
        ]),
    }
}

/// The authored cards, each floor computed from its tree in `ui`'s fonts. Also what the keyboard
/// cursor is given: `mxm_ui::navigation::paged` reads the plan's own order from the last frame's
/// report and falls back to these before one exists.
pub fn page_items(ui: &Ui, params: &MxmClassicVerbParams) -> Vec<mxm_ui::paging::Item<'static>> {
    use mxm_ui::paging::{Category, Item, Key};
    TITLES
        .iter()
        .enumerate()
        .map(|(index, title)| Item {
            key: Key(index as u64),
            // As wide as its controls and no wider (`plans/plan-editor-standard.md` A1).
            card: {
                let floor = tree::card_floor(ui, title, &card(ui, index, params));
                mxm_ui::flow::Card::new(title, floor).capped(floor)
            },
            category: Category::Effects,
            kind: title,
        })
        .collect()
}

/// Everything a leaf draws with, and what the frame read once before anything was drawn: the wet
/// level (mxm-kit's `docs/plugin-conventions.md`, *Editor contract*: destructive telemetry is read
/// once) and whether a file is held over the Space card.
pub struct Live<'a, 'b> {
    pub params: &'a MxmClassicVerbParams,
    pub setter: &'a ParamSetter<'b>,
    pub text: &'a mut HashMap<&'static str, Option<String>>,
    pub wet: f32,
    pub files_over: bool,
    /// The host tempo in force: a synced knob reads its division with one.
    pub tempo: Option<f64>,
}

/// Draws one leaf, in the `Ui` the tree bounded to `rect`, through the bindings — so the controls,
/// their gestures and their names are exactly what they were.
pub fn paint(ui: &mut Ui, tokens: &Tokens, leaf: &Leaf, rect: Rect, live: &mut Live<'_, '_>) {
    let params = live.params;
    match *leaf {
        // Synced to a tempo, a knob reads its division; the host still reads its value.
        Leaf::Knob(id) => {
            let size = KNOB;
            let bound = bound(id, params);
            let division = {
                use nice_plug::prelude::Param as _;
                let synced: Option<(bool, &nice_plug::prelude::FloatParam, mxm_tempo::Ladder)> =
                    match id {
                        "predelay" => Some((
                            params.pre_delay_sync.value(),
                            &params.pre_delay,
                            crate::params::PRE_DELAY_SYNC,
                        )),
                        "modrate" => Some((
                            params.mod_sync.value(),
                            &params.mod_rate,
                            crate::params::MOD_SYNC,
                        )),
                        _ => None,
                    };
                synced
                    .filter(|(on, _, _)| *on)
                    .and_then(|(_, param, ladder)| {
                        ladder.shown(
                            param.unmodulated_normalized_value(),
                            live.tempo,
                            f64::from(param.preview_plain(0.0)),
                            f64::from(param.preview_plain(1.0)),
                        )
                    })
            };
            match division {
                Some(division) => bound.knob_with_reading(
                    ui,
                    tokens,
                    live.setter,
                    size,
                    rect.width(),
                    live.text,
                    division.label(),
                ),
                None => bound.knob(ui, tokens, live.setter, size, rect.width(), live.text),
            }
        }
        Leaf::Picture(id) => {
            super::binding::sync_picture(ui, tokens, id, bound(id, params).param, live.setter);
        }
        Leaf::Space => selector(
            ui,
            tokens,
            "space",
            &params.space,
            SpaceChoice::variants(),
            None,
            bound("space", params).description,
            live.setter,
        ),
        Leaf::LoadLine => load_line(ui, tokens, params, live.files_over),
        Leaf::DecayDisplay => decay_display(ui, tokens, params, live.wet),
        Leaf::Shape => {
            let binding = bound("shape", params);
            // Each picture announces the option's own name (B4).
            let options: Vec<(Wave, &str)> = SHAPES
                .into_iter()
                .zip(ShapeChoice::variants().iter().copied())
                .collect();
            segmented_waves_named(
                ui,
                tokens,
                "shape",
                binding.param,
                binding.panel.as_deref(),
                &options,
                None,
                &SHAPE_DETAILS,
                live.setter,
            );
        }
    }
}

/// The cards, through the shared paging renderer. `wet` is this frame's one read of the telemetry.
/// `load` is what a drop on the Space card calls.
#[allow(clippy::too_many_arguments)]
pub fn cards(
    ui: &mut Ui,
    tokens: &Tokens,
    params: &MxmClassicVerbParams,
    setter: &ParamSetter<'_>,
    text: &mut HashMap<&'static str, Option<String>>,
    wet: f32,
    tempo: Option<f64>,
    load: &mut dyn FnMut(PathBuf),
) -> f32 {
    let target = space_target();
    let items = page_items(ui, params);
    let text_editing = text.values().any(Option::is_some);
    let mut live = Live {
        params,
        setter,
        text,
        wet,
        files_over: files_over(ui, target),
        tempo,
    };
    // **The Space card's body is the drop target**: every leaf its tree laid out, which is what
    // `ui.min_rect()` stood in for when the card was drawing code. Empty when the card is not on
    // the page this frame, and then nothing is taken.
    let mut space = egui::Rect::NOTHING;
    let report = mxm_ui::paging::editor::show(
        ui,
        tokens,
        &items,
        &[],
        text_editing,
        &mut |ui, index| card(ui, index, params),
        &mut |ui, index, leaf, rect| {
            if index == 0 {
                space = space.union(rect);
            }
            paint(ui, tokens, leaf, rect, &mut live);
        },
    );
    if space.is_positive() {
        ui.ctx().data_mut(|data| data.insert_temp(target, space));
        accept_drop(ui, space, target, load);
    }
    report
        .visible
        .iter()
        .map(|(_, rect)| rect.bottom())
        .fold(ui.min_rect().bottom(), f32::max)
}

/// Where the Space card's drop target is remembered between frames.
fn space_target() -> egui::Id {
    egui::Id::new("mxm-classic-verb-space-card")
}

/// What the load line says while nothing is held and nothing is happening.
pub const DROP_HERE: &str = "Drop a WAV or AIFF impulse response here";
/// What it says while a file is held over the card.
pub const RELEASE_TO_FIT: &str = "Release to fit a space to this file";
/// The load line's narrowest before its words are counted. It is `MIN_TARGET` tall whatever it says,
/// and fills its card's width.
const LOAD_LINE_MIN_WIDTH: f32 = 224.0;

/// The load line's narrowest: wide enough for its fixed words — [`DROP_HERE`] and
/// [`RELEASE_TO_FIT`] — so an instruction is never elided (E1). A file's name, a refusal or a fit
/// report still elides to the card, with the whole of it in the tooltip.
fn load_line_min_width(ui: &Ui) -> f32 {
    let font = mxm_ui::typography::caption_style(ui.style()).resolve(ui.style());
    [DROP_HERE, RELEASE_TO_FIT]
        .into_iter()
        .map(|text| {
            ui.ctx().fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
                    .size()
                    .x
            }) + 2.0 * SPACE_3
        })
        .fold(LOAD_LINE_MIN_WIDTH, f32::max)
}

/// Whether a file is held over the Space card: files hovering, with the pointer inside the rectangle
/// the card's leaves were laid out in last frame.
fn files_over(ui: &Ui, target: egui::Id) -> bool {
    let Some(rect) = ui.ctx().data(|data| data.get_temp::<egui::Rect>(target)) else {
        return false;
    };
    ui.input(|input| {
        !input.raw.hovered_files.is_empty()
            && input
                .pointer
                .hover_pos()
                .is_some_and(|at| rect.contains(at))
    })
}

/// **A drop anywhere on the Space card's body is taken** — the first file of it, once a frame.
/// Nothing else happens here: `drop` takes a fit generation and schedules the background task, and no
/// parameter is touched until a fit lands.
fn accept_drop(ui: &Ui, rect: egui::Rect, target: egui::Id, drop: &mut dyn FnMut(PathBuf)) {
    let pass = ui.ctx().cumulative_pass_nr();
    let taken = target.with("taken");
    if ui.ctx().data(|data| data.get_temp::<u64>(taken)) == Some(pass) {
        return;
    }
    let dropped = ui.input(|input| {
        let inside = input
            .pointer
            .hover_pos()
            .is_some_and(|at| rect.contains(at));
        if inside {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        } else {
            None
        }
    });
    if let Some(path) = dropped {
        ui.ctx().data_mut(|data| data.insert_temp(taken, pass));
        drop(path);
    }
}

/// How a load line's words are inked. Every state has words, so none is carried by hue alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Quiet,
    Normal,
    Accent,
    Warning,
    Danger,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoadLine {
    pub text: String,
    pub tone: Tone,
    pub hover: String,
}

/// **The brief's one line**: the fit report's summary — confidence and the largest descriptor error —
/// with the full report on hover; a refusal replaces it with its reason; a running fit says so; and a
/// file held over the card says what releasing it will do.
pub(crate) fn load_line_for(params: &MxmClassicVerbParams, files_over: bool) -> LoadLine {
    if files_over {
        return LoadLine {
            text: RELEASE_TO_FIT.to_owned(),
            tone: Tone::Accent,
            hover: "The file is being analysed; the current space keeps playing until it is ready."
                .to_owned(),
        };
    }
    match params.loaded.status() {
        Status::Fitting { name } => LoadLine {
            text: format!("Fitting a space to {name}…"),
            tone: Tone::Normal,
            hover: format!(
                "A space is being fitted to {name}. The space sounding now keeps sounding until it lands, and a new drop, a preset, Init or closing the editor cancels it."
            ),
        },
        Status::Refused { reason } => LoadLine {
            text: format!("Refused: {reason}"),
            tone: Tone::Danger,
            hover: format!(
                "Refused: {reason}.\nThe space that was sounding still sounds, and no control was changed."
            ),
        },
        Status::Idle => match (params.loaded.holds_space(), params.loaded.report()) {
            (true, Some(report)) => LoadLine {
                tone: if report.is_low_confidence() {
                    Tone::Warning
                } else {
                    Tone::Normal
                },
                text: report.summary(),
                hover: format!(
                    "{}\nSelect Loaded to hear it; drop another response to replace it.",
                    report.details()
                ),
            },
            (true, None) => LoadLine {
                text: "A space is loaded; its fit report was not kept".to_owned(),
                tone: Tone::Normal,
                hover: "Select Loaded to hear it; drop another response to replace it.".to_owned(),
            },
            (false, _) => LoadLine {
                text: DROP_HERE.to_owned(),
                tone: Tone::Quiet,
                hover: "Drop an impulse response, a WAV or AIFF file, on this card: it becomes the Loaded space, and Decay, Size, Diffusion and Pre-delay are set to match it."
                    .to_owned(),
            },
        },
    }
}

/// The drop target and its one line: a fixed-height box whatever it says, so no state moves the
/// knobs under it, and words elided to its width, never extending it.
fn load_line(ui: &mut Ui, tokens: &Tokens, params: &MxmClassicVerbParams, files_over: bool) {
    let line = load_line_for(params, files_over);
    let width = ui.available_width().max(load_line_min_width(ui));
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, MIN_TARGET), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    // Held over: the selection fill and a border twice as heavy in the accent, beside the words.
    let (fill, stroke) = if files_over {
        (
            tokens.selection,
            egui::Stroke::new(2.0 * AXIS_STROKE, tokens.accent),
        )
    } else {
        (
            tokens.surface_2,
            egui::Stroke::new(AXIS_STROKE, tokens.border),
        )
    };
    painter.rect_filled(rect, CANVAS_RADIUS, fill);
    painter.rect_stroke(rect, CANVAS_RADIUS, stroke, egui::StrokeKind::Inside);
    let ink = match line.tone {
        Tone::Quiet => tokens.text_secondary,
        Tone::Normal => tokens.text_primary,
        Tone::Accent => tokens.accent,
        Tone::Warning => tokens.warning,
        Tone::Danger => tokens.danger,
    };
    let font = mxm_ui::typography::caption_style(ui.style()).resolve(ui.style());
    let mut job = egui::text::LayoutJob::single_section(
        line.text.clone(),
        egui::TextFormat::simple(font, ink),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: (rect.width() - 2.0 * SPACE_3).max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = painter.layout_job(job);
    let at = egui::pos2(
        rect.left() + SPACE_3,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(at, galley, ink);
    let label = line.text;
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, label.clone()));
    response.on_hover_text(line.hover);
}

/// The engine's controls from the parameters: the same fields, the same way, as
/// `MxmClassicVerb::controls` hands them to the engine — `the_display_is_given_the_engines_controls`
/// holds the two together. Pre-delay and mod rate are their free values here even when synced: the
/// engine's are the division, and neither moves the decay the display draws.
pub(crate) fn controls(p: &MxmClassicVerbParams) -> Controls {
    Controls {
        mix: p.mix.value(),
        decay_s: p.decay.value(),
        bass_mult: p.bass.value(),
        treble_mult: p.treble.value(),
        size_s: p.size.value(),
        diffusion: p.diffusion.value(),
        pre_delay_s: p.pre_delay.value(),
        early_late_db: p.early_late.value(),
        shape: p.shape.value().dsp(),
        mod_depth_s: p.mod_depth.value(),
        mod_rate_hz: p.mod_rate.value(),
        width: p.width.value(),
        tone_low_db: p.low_tone.value(),
        tone_high_db: p.high_tone.value(),
        duck: p.duck.value(),
    }
}

/// The frequencies the display's three lines stand for: the low, mid and high octave bands the DSP
/// crate measured its prediction against (`crates/mxm-classic-verb-dsp/NOTES.md`).
pub(crate) const BANDS_HZ: [f32; 3] = [125.0, 1_000.0, 8_000.0];
const BAND_LABELS: [&str; 3] = ["125 Hz", "1 kHz", "8 kHz"];
/// The rate the prediction is evaluated at. The editor is never told the host's rate; the decay
/// rule is stated in seconds, so the picture keeps its shape at another rate, and the exact figures
/// there are not measured here.
const DISPLAY_RATE_HZ: f32 = 48_000.0;

/// The full-scale times the natural display's axis steps between.
///
/// **Stepped, not fitted to the lines and not fixed.** Fitted, every knob move would rescale the
/// axis and a longer decay would look the same length as a shorter one. Fixed at the longest decay
/// the engine allows, the everyday one-to-three-second decays would be a few pixels. A 1–2–5 ladder
/// moves only when the longest line crosses a step, so a Decay drag or a bass multiplier visibly
/// changes lengths against a scale that holds still, and the scale's end is written on the axis.
pub(crate) const AXIS_STEPS_S: [f32; 7] = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 60.0];
/// Room past the longest line, so its end marker never sits on the axis's edge.
const AXIS_HEADROOM: f32 = 1.2;
/// A narrowest useful display: the band labels, a readable span, and the times beside it. It is
/// `TALL_PLOT_HEIGHT` tall and fills its card's width.
const DISPLAY_MIN_WIDTH: f32 = 224.0;
/// Vertical positions inside the display, from its top: three band rows, then the axis.
const FIRST_ROW_Y: f32 = 14.0;
const ROW_GAP: f32 = 16.0;
const AXIS_Y: f32 = 60.0;
const TICK: f32 = 3.0;
/// Points along a reverse envelope's curve.
const ENVELOPE_POINTS: usize = 24;

/// The axis's full scale for a picture whose longest line is `longest_s`.
pub(crate) fn axis_s(longest_s: f32) -> f32 {
    let last = AXIS_STEPS_S[AXIS_STEPS_S.len() - 1];
    if !longest_s.is_finite() {
        return last;
    }
    AXIS_STEPS_S
        .iter()
        .copied()
        .find(|step| longest_s * AXIS_HEADROOM <= *step)
        .unwrap_or(last)
}

/// What the display draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Picture {
    /// The network's predicted decay at each of [`BANDS_HZ`], in seconds.
    Bands([f32; 3]),
    /// A gated or reverse decay: its envelope, over its length in seconds.
    Shaped { shape: DecayShape, length_s: f32 },
}

/// **Geometry comes from the DSP.** A natural decay is `predicted_decay_s` — the realised octave
/// decay in closed form, not the composed band targets, which shelving filters only approach. A
/// shaped decay has no network decay: Decay is its length, clamped where the engine clamps it.
pub(crate) fn picture(params: &MxmClassicVerbParams) -> Picture {
    let controls = controls(params);
    match controls.shape {
        DecayShape::Natural => {
            // The space the selector sounds: the loaded one under `Loaded`, when one is held.
            let space = params.sounding_space();
            Picture::Bands(
                BANDS_HZ.map(|hz| predicted_decay_s(&space, &controls, DISPLAY_RATE_HZ, hz)),
            )
        }
        shape => Picture::Shaped {
            shape,
            length_s: controls.decay_s.clamp(SHAPE_MIN_S, SHAPE_MAX_S),
        },
    }
}

/// A shaped decay's envelope at a fraction `u` of its length: flat for gated, rising as u² for
/// reverse. The DSP crate's shapes (`crates/mxm-classic-verb-dsp/NOTES.md`, *Shaped decays live
/// outside the loop*), restated here because that crate keeps its tap gains private.
fn envelope(shape: DecayShape, u: f32) -> f32 {
    match shape {
        DecayShape::Natural => 0.0,
        DecayShape::Gated => 1.0,
        DecayShape::Reverse => u * u,
    }
}

fn shape_name(shape: DecayShape) -> &'static str {
    match shape {
        DecayShape::Natural => "Natural",
        DecayShape::Gated => "Gated",
        DecayShape::Reverse => "Reverse",
    }
}

/// Seconds, as the Decay parameter writes them.
fn seconds(value: f32) -> String {
    if !value.is_finite() {
        "> 60 s".to_owned()
    } else if (value * 10.0).round() >= 100.0 {
        format!("{value:.1} s")
    } else {
        format!("{value:.2} s")
    }
}

/// An axis end, in the fewest digits that say it.
fn axis_label(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0} s")
    } else {
        format!("{value} s")
    }
}

/// The display's name for a screen reader: what it draws and the states it is in, in words.
pub(crate) fn accessible_label(picture: Picture, off: bool, unloaded: bool) -> String {
    let mut label = match picture {
        Picture::Bands(times) => format!(
            "Decay across the bands: {} {}, {} {}, {} {}",
            BAND_LABELS[0],
            seconds(times[0]),
            BAND_LABELS[1],
            seconds(times[1]),
            BAND_LABELS[2],
            seconds(times[2])
        ),
        Picture::Shaped { shape, length_s } => {
            format!("{} decay over {}", shape_name(shape), seconds(length_s))
        }
    };
    if unloaded {
        label.push_str("; Nothing loaded");
    }
    if off {
        label.push_str("; Off");
    }
    label
}

/// The decay across the bands, on the Decay card.
///
/// **Geometry is the DSP's, brightness is the audio thread's, and the states are words.** The lines
/// are [`picture`]; how lit they are is the wet the engine actually added, read once this frame from
/// lock-free telemetry, so a parked effect goes dark whatever the knobs say; and Off and Nothing
/// loaded are painted as text, because a dim line alone could mean a quiet input. The box is
/// reserved at a fixed height whatever it shows, so no state moves the controls under it.
fn decay_display(ui: &mut Ui, tokens: &Tokens, params: &MxmClassicVerbParams, wet: f32) {
    let width = ui.available_width().max(DISPLAY_MIN_WIDTH);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, TALL_PLOT_HEIGHT), egui::Sense::hover());
    let picture = picture(params);
    let off = params.mix.value() == 0.0;
    let unloaded = params.space.value() == SpaceChoice::Loaded && !params.loaded.holds_space();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            accessible_label(picture, off, unloaded),
        )
    });

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CANVAS_RADIUS, tokens.surface_2);
    painter.rect_stroke(
        rect,
        CANVAS_RADIUS,
        egui::Stroke::new(AXIS_STROKE, tokens.border),
        egui::StrokeKind::Inside,
    );

    let font = mxm_ui::typography::caption_style(ui.style()).resolve(ui.style());
    // A little compression, so the quiet end of a tail still lights the lines.
    let glow = wet.clamp(0.0, 1.0).powf(0.4);
    let ink = lerp_colour(tokens.track, tokens.accent, glow);
    let label_ink = if off {
        tokens.text_disabled
    } else {
        tokens.text_secondary
    };
    let value_ink = if off {
        tokens.text_disabled
    } else {
        tokens.text_primary
    };

    let text_width = |text: &str| {
        painter
            .layout_no_wrap(text.to_owned(), font.clone(), label_ink)
            .size()
            .x
    };
    let label_width = BAND_LABELS
        .iter()
        .copied()
        .chain(["Natural", "Gated", "Reverse"])
        .map(text_width)
        .fold(0.0, f32::max);
    let value_width = text_width("60.0 s");
    let pad = SPACE_3;
    let left = rect.left() + pad + label_width + pad;
    let right = rect.right() - pad - value_width - pad;
    let span = (right - left).max(1.0);
    let axis_y = rect.top() + AXIS_Y;
    let fraction = |time: f32, axis: f32| {
        if time.is_finite() {
            (time / axis).clamp(0.0, 1.0)
        } else {
            1.0
        }
    };

    let axis = match picture {
        Picture::Bands(times) => {
            let axis = axis_s(times.iter().copied().fold(0.0, f32::max));
            for (index, time) in times.iter().copied().enumerate() {
                let y = rect.top() + FIRST_ROW_Y + ROW_GAP * index as f32;
                painter.text(
                    egui::pos2(rect.left() + pad, y),
                    egui::Align2::LEFT_CENTER,
                    BAND_LABELS[index],
                    font.clone(),
                    label_ink,
                );
                // The rail: how long a line could be on this scale, so a short decay reads as short
                // rather than as a display that has been rescaled.
                painter.line_segment(
                    [egui::pos2(left, y), egui::pos2(right, y)],
                    egui::Stroke::new(REFERENCE_STROKE, tokens.border),
                );
                let end = left + span * fraction(time, axis);
                painter.line_segment(
                    [egui::pos2(left, y), egui::pos2(end, y)],
                    egui::Stroke::new(METER_STROKE, ink),
                );
                painter.circle_filled(egui::pos2(end, y), METER_STROKE, ink);
                painter.text(
                    egui::pos2(rect.right() - pad, y),
                    egui::Align2::RIGHT_CENTER,
                    seconds(time),
                    font.clone(),
                    value_ink,
                );
            }
            axis
        }
        Picture::Shaped { shape, length_s } => {
            // A shape's length is bounded, so its axis is that bound and never moves at all.
            let axis = SHAPE_MAX_S;
            let top = rect.top() + FIRST_ROW_Y - TICK;
            let height = axis_y - top;
            let end = left + span * fraction(length_s, axis);
            let mut points = Vec::with_capacity(ENVELOPE_POINTS + 3);
            points.push(egui::pos2(left, axis_y));
            for step in 0..=ENVELOPE_POINTS {
                let u = step as f32 / ENVELOPE_POINTS as f32;
                points.push(egui::pos2(
                    left + (end - left) * u,
                    axis_y - height * envelope(shape, u),
                ));
            }
            points.push(egui::pos2(end, axis_y));
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(METER_STROKE, ink),
            ));
            let middle = (top + axis_y) / 2.0;
            painter.text(
                egui::pos2(rect.left() + pad, middle),
                egui::Align2::LEFT_CENTER,
                shape_name(shape),
                font.clone(),
                label_ink,
            );
            painter.text(
                egui::pos2(rect.right() - pad, middle),
                egui::Align2::RIGHT_CENTER,
                seconds(length_s),
                font.clone(),
                value_ink,
            );
            axis
        }
    };

    let axis_stroke = egui::Stroke::new(AXIS_STROKE, tokens.border_strong);
    painter.line_segment(
        [egui::pos2(left, axis_y), egui::pos2(right, axis_y)],
        axis_stroke,
    );
    for tick in [0.0, 0.5, 1.0] {
        let x = left + span * tick;
        painter.line_segment(
            [egui::pos2(x, axis_y), egui::pos2(x, axis_y + TICK)],
            axis_stroke,
        );
    }
    painter.text(
        egui::pos2(left, axis_y + TICK),
        egui::Align2::CENTER_TOP,
        "0",
        font.clone(),
        label_ink,
    );
    painter.text(
        egui::pos2(right, axis_y + TICK),
        egui::Align2::CENTER_TOP,
        axis_label(axis),
        font.clone(),
        label_ink,
    );

    let state_y = rect.bottom() - INNER_GUTTER;
    if unloaded {
        painter.text(
            egui::pos2(rect.left() + pad, state_y),
            egui::Align2::LEFT_BOTTOM,
            "Nothing loaded",
            font.clone(),
            tokens.text_primary,
        );
    }
    if off {
        painter.text(
            egui::pos2(rect.right() - pad, state_y),
            egui::Align2::RIGHT_BOTTOM,
            "Off",
            font,
            tokens.text_primary,
        );
    }

    let mut hover = if off {
        "Mix is at zero, so the reverb is off.".to_owned()
    } else {
        match picture {
            Picture::Bands(_) => "How long each part of the sound rings; the brightness is how much reverb is sounding.".to_owned(),
            Picture::Shaped { .. } => "The reverb's shape over time; the brightness is how much is sounding.".to_owned(),
        }
    };
    if unloaded {
        hover.push_str(
            "
Nothing is loaded yet, so it plays the default space; drop an impulse response on the Space card to load one.",
        );
    }
    response.on_hover_text(hover);
}

/// Mixes two colours, `t` of the way from `a` to `b`.
fn lerp_colour(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
    egui::Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mxm_classic_verb_dsp::{MAX_DECAY_S, Space};
    use nice_plug::params::{InternalParamMut, Param};
    use nice_plug::prelude::Params;

    fn set<P: Param + InternalParamMut>(param: &P, value: P::Plain) {
        unsafe {
            let _ = param._internal_set_normalized_value(param.preview_normalized(value));
        }
    }

    #[test]
    fn every_parameter_is_bound_once() {
        let params = MxmClassicVerbParams::default();
        let ids: Vec<_> = all_parameters(&params)
            .iter()
            .map(|binding| binding.id)
            .collect();
        assert_eq!(ids.len(), params.param_map().len());
        for (id, _, _) in params.param_map() {
            assert_eq!(ids.iter().filter(|bound| **bound == id).count(), 1, "{id}");
        }
    }

    #[test]
    fn every_tooltip_carries_one_sentence() {
        let params = MxmClassicVerbParams::default();
        for binding in all_parameters(&params) {
            let text = binding.description;
            assert!(
                text.ends_with('.') && !text[..text.len() - 1].contains(". "),
                "{}'s description is not one sentence: {text:?}",
                binding.id
            );
        }
    }

    /// The selector lists the parameter's own positions, in its own order, however many there are:
    /// a list that named a different space than the one it selects would be a control that lies.
    #[test]
    fn the_space_list_is_every_position_the_parameter_has_in_order() {
        let params = MxmClassicVerbParams::default();
        let space = bound("space", &params);
        let names = SpaceChoice::variants();
        assert_eq!(space.param.steps().map(|s| s + 1), Some(names.len()));
        let last = (names.len() - 1) as f32;
        for (index, name) in names.iter().enumerate() {
            assert_eq!(space.param.format(index as f32 / last), *name);
        }
    }

    /// The shared knob draws a bipolar control's detent at the middle of its travel, and a
    /// double-click returns to the default. Both are the neutral point only if the neutral value
    /// sits at the middle and is the default — which this holds for every relative control.
    #[test]
    fn every_relative_control_has_its_neutral_on_the_detent_and_as_its_default() {
        let params = MxmClassicVerbParams::default();
        let neutral = [
            ("earlylate", &params.early_late, 0.0),
            ("bass", &params.bass, 1.0),
            ("treble", &params.treble, 1.0),
            ("width", &params.width, 1.0),
            ("lowtone", &params.low_tone, 0.0),
            ("hightone", &params.high_tone, 0.0),
        ];
        assert_eq!(
            neutral.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            RELATIVE_IDS
        );
        for (id, param, value) in neutral {
            assert!(bound(id, &params).bipolar, "{id} draws no detent");
            assert!(
                (param.preview_normalized(value) - 0.5).abs() < 1e-4,
                "{id}'s neutral is not at the detent"
            );
            assert_eq!(param.default_plain_value(), value, "{id}");
        }
        for binding in all_parameters(&params) {
            assert_eq!(
                binding.bipolar,
                RELATIVE_IDS.contains(&binding.id),
                "{} is absolute or relative, and its knob must say which",
                binding.id
            );
        }
    }

    #[test]
    fn the_display_is_given_the_engines_controls() {
        let plugin = crate::MxmClassicVerb::default();
        set(&plugin.params.decay, 4.5);
        set(&plugin.params.bass, 2.0);
        set(&plugin.params.width, 0.4);
        set(&plugin.params.shape, ShapeChoice::Reverse);
        set(&plugin.params.duck, 0.3);
        assert_eq!(controls(&plugin.params), plugin.controls());
    }

    #[test]
    fn the_natural_display_draws_the_engines_prediction_and_tilts_with_the_multipliers() {
        let params = MxmClassicVerbParams::default();
        let Picture::Bands(flat) = picture(&params) else {
            panic!("the default decay is natural");
        };
        assert_eq!(params.space.value(), SpaceChoice::INIT);
        let space = params.space.value().factory();
        for (index, hz) in BANDS_HZ.iter().enumerate() {
            assert_eq!(
                flat[index],
                predicted_decay_s(&space, &controls(&params), DISPLAY_RATE_HZ, *hz)
            );
        }
        set(&params.bass, 2.0);
        set(&params.treble, 0.5);
        let Picture::Bands(tilted) = picture(&params) else {
            panic!("still natural");
        };
        assert!(tilted[0] > flat[0] * 1.3, "{tilted:?} against {flat:?}");
        assert!(tilted[2] < flat[2] * 0.8, "{tilted:?} against {flat:?}");
    }

    #[test]
    fn a_shaped_decay_is_drawn_as_its_envelope_over_its_length() {
        let params = MxmClassicVerbParams::default();
        set(&params.shape, ShapeChoice::Gated);
        set(&params.decay, 0.5);
        let Picture::Shaped { shape, length_s } = picture(&params) else {
            panic!("a gated decay is an envelope");
        };
        assert_eq!(shape, DecayShape::Gated);
        assert!((length_s - 0.5).abs() < 1e-3);
        set(&params.decay, 12.0);
        assert_eq!(
            picture(&params),
            Picture::Shaped {
                shape: DecayShape::Gated,
                length_s: SHAPE_MAX_S
            },
            "the engine clamps a shape's length, so the picture does too"
        );
        assert_eq!(envelope(DecayShape::Reverse, 0.0), 0.0);
        assert_eq!(envelope(DecayShape::Reverse, 1.0), 1.0);
        assert!(envelope(DecayShape::Reverse, 0.5) < 0.5);
    }

    #[test]
    fn the_axis_holds_every_line_and_moves_only_in_steps() {
        assert!(AXIS_STEPS_S[AXIS_STEPS_S.len() - 1] >= MAX_DECAY_S);
        let mut previous = axis_s(0.1);
        let mut changes = 0;
        for step in 0..=6_000 {
            let longest = 0.1 + step as f32 * 0.01;
            let axis = axis_s(longest);
            assert!(AXIS_STEPS_S.contains(&axis));
            assert!(
                axis >= longest.min(MAX_DECAY_S),
                "{longest} s on a {axis} s axis"
            );
            if axis != previous {
                changes += 1;
                previous = axis;
            }
        }
        // Six thousand positions of the longest line move the scale six times.
        assert_eq!(changes, AXIS_STEPS_S.len() - 1);
        assert_eq!(axis_s(f32::INFINITY), MAX_DECAY_S);
    }

    #[test]
    fn the_display_draws_the_loaded_space_when_loaded_is_selected() {
        use nice_plug::params::persist::PersistentField;
        let params = MxmClassicVerbParams::default();
        let bands = |params: &MxmClassicVerbParams| {
            let Picture::Bands(times) = picture(params) else {
                panic!("the default decay is natural");
            };
            times
        };
        let predicted = |space: &Space, params: &MxmClassicVerbParams| {
            BANDS_HZ.map(|hz| predicted_decay_s(space, &controls(params), DISPLAY_RATE_HZ, hz))
        };
        set(&params.space, SpaceChoice::Loaded);
        assert_eq!(
            bands(&params),
            predicted(&SpaceChoice::INIT.factory(), &params)
        );
        params
            .loaded
            .set(crate::payload::Payload::holding(&Space::PLATE, None));
        assert_eq!(bands(&params), predicted(&Space::PLATE, &params));
        assert_ne!(
            bands(&params),
            predicted(&SpaceChoice::INIT.factory(), &params),
            "the plate and the Init space decay alike, so this proves nothing"
        );
        let factory = SpaceChoice::factory_positions()
            .next()
            .expect("a factory position");
        set(&params.space, factory);
        assert_eq!(bands(&params), predicted(&factory.factory(), &params));
    }

    #[test]
    fn the_load_line_says_what_loading_is_doing() {
        use crate::payload::{Payload, Report};
        use nice_plug::params::persist::PersistentField;
        let params = MxmClassicVerbParams::default();
        let line = load_line_for(&params, false);
        assert_eq!((line.text.as_str(), line.tone), (DROP_HERE, Tone::Quiet));
        assert_eq!(load_line_for(&params, true).tone, Tone::Accent);

        let generation = params.loaded.begin("a.aif");
        assert_eq!(
            load_line_for(&params, false).text,
            "Fitting a space to a.aif…"
        );
        assert_eq!(
            load_line_for(&params, true).text,
            RELEASE_TO_FIT,
            "a file held over the card says so even while a fit runs"
        );

        assert!(
            params
                .loaded
                .refuse(generation, "every sample is zero".into())
        );
        let line = load_line_for(&params, false);
        assert_eq!(
            (line.text.as_str(), line.tone),
            ("Refused: every sample is zero", Tone::Danger)
        );
        assert!(
            line.hover.contains("no control was changed"),
            "{}",
            line.hover
        );

        params.loaded.set(Payload::holding(
            &Space::ROOM,
            Some(Report {
                confidence: 0.3,
                errors: Vec::new(),
                clamps: Vec::new(),
            }),
        ));
        let line = load_line_for(&params, false);
        assert_eq!(line.tone, Tone::Warning);
        assert!(
            line.text.starts_with("Low confidence 0.30"),
            "{}",
            line.text
        );
    }

    #[test]
    fn the_display_names_its_states_in_words() {
        let times = Picture::Bands([2.1, 2.0, 1.5]);
        assert_eq!(
            accessible_label(times, false, false),
            "Decay across the bands: 125 Hz 2.10 s, 1 kHz 2.00 s, 8 kHz 1.50 s"
        );
        let label = accessible_label(times, true, true);
        assert!(label.ends_with("; Nothing loaded; Off"), "{label}");
        let gated = Picture::Shaped {
            shape: DecayShape::Gated,
            length_s: 0.5,
        };
        assert_eq!(
            accessible_label(gated, false, false),
            "Gated decay over 0.50 s"
        );
    }
}
