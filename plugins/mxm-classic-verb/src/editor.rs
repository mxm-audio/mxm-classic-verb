//! The dynamically paged, four-card MXM editor for `mxm-classic-verb`.
//!
//! Built to `docs/briefs/mxm-classic-verb.md`, the owner-approved gating document, which this module
//! implements and does not re-decide: four Effects cards — Space, Decay, Texture, Output — in
//! signal-flow order, **no view bar**, the collection accent, the decay display on the Decay card, and
//! loading a space from a file on the Space card.
//!
//! # It is a panel, not a window
//!
//! [`panel`] takes a `Ui` and draws into it. It does not create a window, run an event loop, or own
//! a swapchain.
//!
//! # Gestures
//!
//! Every edit is bracketed — `begin_set_parameter`, `set_parameter_normalized`,
//! `end_set_parameter` — in `binding.rs` for the controls, and in `loading.rs` for a fit landing.
//!
//! # Loading
//!
//! A drop on the Space card takes a fit generation and schedules the background task
//! (`loading.rs`). Once a frame, before anything is drawn, a finished fit for the newest drop lands
//! in the commit order. The preset surface is drawn through a setter that notices its gestures, so a
//! recall or Init supersedes a running fit; closing the editor supersedes it too.
//!
//! # No developer channel
//!
//! It arrives as MIDI CC and an effect has no note port (`plugins/AGENTS.md`).

pub mod binding;
pub mod sections;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use egui::Ui;
use mxm_ui::space::SPACE_5;
use mxm_ui::theme::Tokens;
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use nice_plug_egui::{EguiEditorState, NiceEguiApp, create_egui_editor};

use crate::loading::{self, GestureWatch, LoadTask};
use crate::params::MxmClassicVerbParams;
use crate::telemetry::Telemetry;

/// The opening size: the quarter-4K budget hugged (design system §4.3) — all four cards on one row,
/// each as wide as its controls, as tall as the Decay card — which
/// `the_opening_size_is_the_budget_hugged` derives and holds.
const REFERENCE: (u32, u32) = (1076, 408);
/// The narrowest and shortest the window may be: the widest card at its computed floor plus the
/// gutters, and tall enough that the Decay card — the tallest — paged on its own under the page bar
/// that width produces needs no scrolling. `the_minimum_holds_the_widest_card` and
/// `the_minimum_window_shows_each_card_without_scrolling` hold it.
const MINIMUM: (u32, u32) = (316, 476);

pub type MxmClassicVerbEditor = nice_plug_egui::EguiEditor<MxmClassicVerbApp>;
pub use mxm_preset::PresetUi;

/// How the editor schedules a fit: the host's background executor in a plugin, a recorder in a test.
pub type Spawn = Arc<dyn Fn(LoadTask) + Send + Sync>;

pub fn create(
    params: Arc<MxmClassicVerbParams>,
    telemetry: Arc<Telemetry>,
    executor: AsyncExecutor<crate::MxmClassicVerb>,
) -> Option<MxmClassicVerbEditor> {
    let state = EguiEditorState::from_size(
        nice_plug::editor::dpi::LogicalSize::new(REFERENCE.0, REFERENCE.1),
        1.0,
    );
    let spawn: Spawn = Arc::new(move |task| executor.execute_background(task));
    create_egui_editor(
        state,
        nice_plug_egui::RepaintNotifier::new(),
        nice_plug_egui::EguiNiceSettings {
            title: crate::NAME.to_owned(),
            resize_hint: ResizeHint {
                size_constraints: nice_plug::editor::SizeConstraints::min_logical_size(
                    nice_plug::editor::dpi::LogicalSize::new(MINIMUM.0 as f32, MINIMUM.1 as f32),
                ),
                ..ResizeHint::RESIZABLE
            },
            ..Default::default()
        },
        MxmClassicVerbApp::new(params, telemetry, spawn),
    )
}

pub struct MxmClassicVerbApp {
    params: Arc<MxmClassicVerbParams>,
    telemetry: Arc<Telemetry>,
    /// The host, wrapped so the preset surface's gestures supersede a running fit. `None` while no
    /// window is open.
    host: Option<GestureWatch>,
    spawn: Spawn,
    text_entry: HashMap<&'static str, Option<String>>,
    presets: PresetUi,
    /// Where the keyboard is: a card, and a parameter inside it. Transient, like the text
    /// buffers — it is not a parameter and nothing durable reads it.
    nav: mxm_ui::navigation::State,
}

impl MxmClassicVerbApp {
    pub fn new(params: Arc<MxmClassicVerbParams>, telemetry: Arc<Telemetry>, spawn: Spawn) -> Self {
        let presets = PresetUi::new(params.as_ref());
        Self {
            params,
            telemetry,
            host: None,
            spawn,
            text_entry: HashMap::new(),
            presets,
            nav: mxm_ui::navigation::State::default(),
        }
    }
}

impl NiceEguiApp for MxmClassicVerbApp {
    fn build(
        &mut self,
        context: egui::Context,
        gui_context: GuiContext,
        _frame: &mut nice_plug_egui::Frame,
    ) -> Result<(), nice_plug_egui::baseview::HandlerError> {
        mxm_ui::theme::apply(&context);
        mxm_ui::typography::apply(&context);
        // Light by default, overridable with `MXM_EDITOR_THEME`; see `mxm_ui::theme::preference`.
        context.set_theme(mxm_ui::theme::preference());
        self.host = Some(GestureWatch::new(gui_context));
        Ok(())
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut nice_plug_egui::Frame) {
        let Some(host) = self.host.as_ref() else {
            return;
        };
        // **A finished fit lands before anything is drawn**, so this frame's controls show the values
        // it wrote, and a drop or a recall in this frame supersedes whatever lands after it.
        loading::settle(&self.params, &host.setter());
        let params = &self.params;
        let spawn = &self.spawn;
        let mut load = |path: PathBuf| spawn(loading::begin(&params.loaded, &path));
        panel(
            ui,
            params,
            &self.telemetry,
            host,
            &mut self.text_entry,
            &mut self.presets,
            &mut self.nav,
            &mut load,
        );
    }

    /// **Closing the editor supersedes a running fit** (plan §5.2). Its gestures belong to an open
    /// editor, and committing a result whose editor has closed would replace the space a `Loaded`
    /// selection plays with no gesture a host could undo — which the plan rejects.
    fn editor_closed(&mut self) {
        self.params.loaded.supersede();
        self.host = None;
    }
}

/// The whole editor surface. `load` is what a drop on the Space card calls with the file's path.
#[allow(clippy::too_many_arguments)]
pub fn panel(
    ui: &mut Ui,
    params: &MxmClassicVerbParams,
    telemetry: &Telemetry,
    host: &GestureWatch,
    text_entry: &mut HashMap<&'static str, Option<String>>,
    presets: &mut PresetUi,
    nav: &mut mxm_ui::navigation::State,
    load: &mut dyn FnMut(PathBuf),
) {
    let tokens = tokens_for(ui);
    let setter = host.setter();
    // A tail decays without input, so the frames have to come without input too — and a fit
    // finishing in the background has to be seen without it.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(50));
    // Destructive reads, once per frame and before anything is drawn.
    let peak = telemetry.take_peak();
    let wet = telemetry.take_wet();
    let clipped = telemetry.clipped();

    // One question, and both layers suspend on it: the paging renderer's `hold` and the cursor's
    // `inert` both ask whether another surface owns this frame's keyboard.
    let busy = presets.holds_the_keyboard() || text_entry.values().any(Option::is_some);
    mxm_ui::paging::editor::hold(ui.ctx(), busy);
    // **The cursor moves before anything is drawn**, so a navigation arrow is consumed here rather
    // than also walking egui's own focus ring.
    mxm_ui::navigation::paged(ui.ctx(), nav, busy);

    mxm_ui::AppBar::new(crate::NAME).show_with(
        ui,
        &tokens,
        |ui| {
            loading::through_the_preset_surface(&params.loaded, host, |setter| {
                mxm_preset::ui::preset_row(ui, &tokens, params, setter, presets)
            })
        },
        |ui| {
            if mxm_ui::shell::level_meter(ui, &tokens, peak, clipped) {
                telemetry.clear_clip();
            }
            mxm_ui::shell::zoom_control(ui);
            mxm_ui::shell::editor_theme_control(ui);
        },
    );
    loading::through_the_preset_surface(&params.loaded, host, |setter| {
        mxm_preset::ui::overlays(ui, &tokens, params, setter, presets)
    });
    // **No view bar** (brief §4): one set of Effects cards, paged by the shared renderer.
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(tokens.canvas)
                .inner_margin(egui::Margin::same(SPACE_5 as i8)),
        )
        .show(ui, |ui| {
            sections::cards(
                ui,
                &tokens,
                params,
                &setter,
                text_entry,
                wet,
                telemetry.tempo.get(),
                load,
            );
        });
}

/// The collection's tokens, unchanged: an effect is told apart by its name in a chain (brief §7).
fn tokens_for(ui: &Ui) -> Tokens {
    if ui.visuals().dark_mode {
        mxm_ui::DARK
    } else {
        mxm_ui::LIGHT
    }
}

/// The paging items as the editor computes them, from a context set up as an editor's is — three
/// passes in, so the weighted font cuts are bound — for tests, which have no editor `Ui` to hand.
#[cfg(test)]
pub(crate) fn test_items(params: &MxmClassicVerbParams) -> Vec<mxm_ui::paging::Item<'static>> {
    let ctx = egui::Context::default();
    mxm_ui::typography::apply(&ctx);
    mxm_ui::theme::apply(&ctx);
    let mut items = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            items = sections::page_items(ui, params);
        });
        output.textures_delta.clear();
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mxm_classic_verb_dsp::Space;
    use nice_plug::params::internals::ParamPtr;
    use nice_plug::params::{InternalParamMut, Param};
    use nice_plug::prelude::{PluginApi, PluginState};

    use crate::loaded::Status;
    use crate::params::SpaceChoice;
    use crate::payload::Payload;
    use crate::testing;

    struct NoHost;

    impl nice_plug::context::gui::GuiContextInner for NoHost {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, _param: ParamPtr, _normalized: f32) {}
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            PluginState {
                version: String::new(),
                params: Default::default(),
                fields: Default::default(),
            }
        }
        fn set_state(&self, _state: PluginState) {}
    }

    /// A host that remembers what it was sent, so a test can ask where a gesture left a value.
    #[derive(Default)]
    struct Recording {
        begins: AtomicUsize,
        ends: AtomicUsize,
        sets: Mutex<Vec<f32>>,
    }

    impl nice_plug::context::gui::GuiContextInner for Recording {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {
            self.begins.fetch_add(1, Ordering::Relaxed);
        }
        unsafe fn raw_set_parameter_normalized(&self, _param: ParamPtr, normalized: f32) {
            self.sets.lock().unwrap().push(normalized);
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {
            self.ends.fetch_add(1, Ordering::Relaxed);
        }
        fn get_state(&self) -> PluginState {
            PluginState {
                version: String::new(),
                params: Default::default(),
                fields: Default::default(),
            }
        }
        fn set_state(&self, _state: PluginState) {}
    }

    fn quiet_host() -> GestureWatch {
        GestureWatch::new(GuiContext::new(Arc::new(NoHost)))
    }

    fn context(theme: egui::ThemePreference) -> egui::Context {
        let context = egui::Context::default();
        mxm_ui::theme::apply(&context);
        mxm_ui::typography::apply(&context);
        context.set_theme(theme);
        context.all_styles_mut(|style| style.animation_time = 0.0);
        context
    }

    fn window(size: (u32, u32)) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.0 as f32, size.1 as f32),
            )),
            ..Default::default()
        }
    }

    fn painted_boxes(
        theme: egui::ThemePreference,
        params: &MxmClassicVerbParams,
    ) -> Vec<(String, egui::Rect)> {
        let context = context(theme);
        let telemetry = Telemetry::default();
        let host = quiet_host();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), params);
        let mut nav = mxm_ui::navigation::State::default();
        let input = window(REFERENCE);

        let mut boxes = Vec::new();
        for pass in 0..3 {
            let mut output = context.run_ui(input.clone(), |ui| {
                panel(
                    ui,
                    params,
                    &telemetry,
                    &host,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut |_| {},
                );
            });
            if pass == 2 {
                for clipped in &output.shapes {
                    collect_boxes(&clipped.shape, &mut boxes);
                }
            }
            output.textures_delta.clear();
        }
        boxes
    }

    fn collect_boxes(shape: &egui::epaint::Shape, boxes: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::epaint::Shape::Text(text) => boxes.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_boxes(shape, boxes);
                }
            }
            _ => {}
        }
    }

    use mxm_plugin_test::keyboard_checks;
    use mxm_plugin_test::{opening_size, paging_checks};

    /// What this editor keeps behind a disclosure, opened so the checks see it. Nothing here:
    /// every control is on a card (brief §5).
    const REVEAL: fn(&egui::Context) = |_| {};

    /// The rollout's own failure mode: a control whose `navigation::at` scope was forgotten paints
    /// exactly as before and is simply unreachable from the keyboard. Nothing else would say so.
    #[test]
    fn the_keyboard_cursor_reaches_and_operates_every_parameter() {
        let params = MxmClassicVerbParams::default();
        let telemetry = Telemetry::default();
        let recorder = Arc::new(keyboard_checks::Recorder::default());
        let host = GestureWatch::new(GuiContext::new(recorder.clone()));
        let ids: Vec<&str> = sections::all_parameters(&params)
            .iter()
            .map(|bound| bound.id)
            .collect();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        keyboard_checks::the_cursor_reaches_and_operates(
            egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            &test_items(&params),
            keyboard_checks::Coverage::Exactly(&ids),
            &REVEAL,
            &recorder,
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &host,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut |_| {},
                );
            },
        );
    }

    /// **The editor opens at the quarter-4K budget, hugged** — the owner's rule, 2026-09-09.
    #[test]
    fn the_opening_size_is_the_budget_hugged() {
        let params = MxmClassicVerbParams::default();
        let telemetry = Telemetry::default();
        let host = quiet_host();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        opening_size::is_the_budget_hugged(
            egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            &REVEAL,
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &host,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut |_| {},
                );
            },
        );
    }

    /// **The app bar holds in the narrowest window**: its `…` menu whole and nothing drawn over
    /// anything else, from `MINIMUM` up (`opening_size::bar_holds_from_the_minimum`).
    #[test]
    fn the_app_bar_holds_in_the_minimum_window() {
        let params = MxmClassicVerbParams::default();
        let telemetry = Telemetry::default();
        let host = quiet_host();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        opening_size::bar_holds_from_the_minimum(
            egui::vec2(MINIMUM.0 as f32, MINIMUM.1 as f32),
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &host,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut |_| {},
                );
            },
        );
    }

    #[test]
    fn every_dynamic_page_fits_and_every_card_is_reachable() {
        let params = MxmClassicVerbParams::default();
        let telemetry = Telemetry::default();
        let host = quiet_host();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        paging_checks::verify(
            &test_items(&params),
            &[
                egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
                egui::vec2(MINIMUM.0 as f32, MINIMUM.1 as f32),
            ],
            |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &host,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut |_| {},
                )
            },
        );
    }

    #[test]
    fn the_minimum_holds_the_widest_card() {
        let cards: Vec<_> = test_items(&MxmClassicVerbParams::default())
            .iter()
            .map(|item| item.card)
            .collect();
        let needed = mxm_ui::flow::minimum_width(&cards) + 2.0 * SPACE_5;
        assert!(
            MINIMUM.0 as f32 >= needed,
            "the widest card needs {needed} points"
        );
    }

    /// At the minimum window every card, requested in turn, is drawn whole: nothing scrolls.
    #[test]
    fn the_minimum_window_shows_each_card_without_scrolling() {
        let params = MxmClassicVerbParams::default();
        let telemetry = Telemetry::default();
        let host = quiet_host();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        let context = context(egui::ThemePreference::Light);
        for item in test_items(&params) {
            mxm_ui::paging::editor::request_card(&context, item.key);
            for _ in 0..4 {
                let mut output = context.run_ui(window(MINIMUM), |ui| {
                    panel(
                        ui,
                        &params,
                        &telemetry,
                        &host,
                        &mut text_entry,
                        &mut presets,
                        &mut nav,
                        &mut |_| {},
                    );
                });
                output.textures_delta.clear();
            }
            let report = mxm_ui::paging::editor::report(&context).expect("a paged panel");
            assert!(
                report.visible.iter().any(|(key, _)| *key == item.key),
                "{} is not on the page it was requested on",
                item.card.title
            );
            assert!(
                !report.scrolling,
                "{} scrolls in the minimum window: {report:?}",
                item.card.title
            );
        }
    }

    /// Straight into a parameter: a check has no host.
    fn set<P: Param + InternalParamMut>(param: &P, value: P::Plain) {
        unsafe {
            let _ = param._internal_set_normalized_value(param.preview_normalized(value));
        }
    }

    /// Takes `params` to one state of the structural-state matrix, and says whether a file is held
    /// over the Space card in it.
    fn enter(params: &MxmClassicVerbParams, state: &str) -> bool {
        use crate::params::ShapeChoice;
        use crate::payload::Report;
        use nice_plug::params::persist::PersistentField;
        let floats = [
            &params.pre_delay,
            &params.size,
            &params.early_late,
            &params.decay,
            &params.bass,
            &params.treble,
            &params.diffusion,
            &params.mod_depth,
            &params.mod_rate,
            &params.mix,
            &params.width,
            &params.low_tone,
            &params.high_tone,
            &params.duck,
        ];
        let at = |value: f32| {
            for param in floats {
                unsafe {
                    let _ = param._internal_set_normalized_value(value);
                }
            }
        };
        match state {
            "a file held over the Space card" => return true,
            "fitting a long file name" => {
                params.loaded.begin(
                    "a response recorded at the far end of a very long stone nave, second take.wav",
                );
            }
            "refused" => {
                let generation = params.loaded.begin("a.aif");
                assert!(
                    params.loaded.refuse(
                        generation,
                        "the file holds more channels than a stereo response and cannot be fitted"
                            .into()
                    )
                );
            }
            "a space held at low confidence" => {
                set(&params.space, SpaceChoice::Loaded);
                params.loaded.set(Payload::holding(
                    &Space::ROOM,
                    Some(Report {
                        confidence: 0.31,
                        errors: Vec::new(),
                        clamps: Vec::new(),
                    }),
                ));
            }
            "a space held with no report" => {
                set(&params.space, SpaceChoice::Loaded);
                params.loaded.set(Payload::holding(&Space::ROOM, None));
            }
            "off, and Loaded holding nothing" => {
                set(&params.mix, 0.0);
                set(&params.space, SpaceChoice::Loaded);
            }
            "the longest space name" => {
                let names = SpaceChoice::variants();
                let (index, _) = names
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, name)| name.chars().count())
                    .expect("spaces");
                unsafe {
                    let _ = params
                        .space
                        ._internal_set_normalized_value(index as f32 / (names.len() - 1) as f32);
                }
            }
            "both syncs on, no tempo" | "both syncs on at a tempo" => {
                set(&params.pre_delay_sync, true);
                set(&params.mod_sync, true);
            }
            "every control at its top" => at(1.0),
            "every control at its bottom" => at(0.0),
            "gated at its longest" => {
                at(1.0);
                set(&params.shape, ShapeChoice::Gated);
            }
            "reverse at its shortest" => {
                at(0.0);
                set(&params.shape, ShapeChoice::Reverse);
            }
            _ => {}
        }
        false
    }

    /// Every card, in every state that changes what it holds or paints, passes the layout tree's
    /// checks (plans/plan-layout-tree.md §4.3, `tree_checks::card`): its computed floor holds its
    /// content with nothing painted outside the card, the content floor is exact, the height its
    /// tree states is the height it draws, and every leaf stays in the room it was given.
    ///
    /// The states are this editor's structural-state matrix. The cards have no route, disclosure or
    /// reserved alternative; what changes is what is painted into the room they state: the load
    /// line in each of its states — nothing held, a file held over the card, fitting, refused, a
    /// space held at low confidence and with no report — the Space selector at its longest name,
    /// the decay display Off with Loaded holding nothing, at every control's top and bottom, and
    /// gated and reversed. The wet level is overdriven throughout, so the lines are at their
    /// brightest.
    #[test]
    fn every_card_passes_the_tree_checks_in_every_state() {
        for state in [
            "init",
            "a file held over the Space card",
            "fitting a long file name",
            "refused",
            "a space held at low confidence",
            "a space held with no report",
            "off, and Loaded holding nothing",
            "the longest space name",
            "every control at its top",
            "every control at its bottom",
            "gated at its longest",
            "reverse at its shortest",
            "both syncs on, no tempo",
            "both syncs on at a tempo",
        ] {
            let params = MxmClassicVerbParams::default();
            let files_over = enter(&params, state);
            let tempo = (state == "both syncs on at a tempo").then_some(120.0);
            let floors: Vec<f32> = test_items(&params)
                .iter()
                .map(|item| item.card.floor)
                .collect();
            let host = NoHost;
            let setter = ParamSetter::new(&host);
            for (index, floor) in floors.into_iter().enumerate() {
                let mut text = HashMap::new();
                let mut live = sections::Live {
                    params: &params,
                    setter: &setter,
                    text: &mut text,
                    wet: 4.0,
                    files_over,
                    tempo,
                };
                tree_checks::card(
                    &|_| {},
                    state,
                    sections::TITLES[index],
                    floor,
                    &|ui| sections::card(ui, index, &params),
                    &mut |ui, leaf, rect| {
                        sections::paint(ui, &mxm_ui::LIGHT, leaf, rect, &mut live);
                    },
                );
            }
        }
    }

    /// At the opening size the four cards share one row in the brief's signal-flow order — Space,
    /// Decay, Texture, Output — and start and end on one line (§3.3).
    #[test]
    fn the_opening_page_is_one_row_in_signal_flow_order() {
        let params = MxmClassicVerbParams::default();
        let telemetry = Telemetry::default();
        let host = quiet_host();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        let context = context(egui::ThemePreference::Light);
        for _ in 0..4 {
            let mut output = context.run_ui(window(REFERENCE), |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &host,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut |_| {},
                );
            });
            output.textures_delta.clear();
        }
        let rects = paging_checks::all_rects(&context, sections::TITLES.len());
        for pair in rects.windows(2) {
            assert!(pair[0].right() <= pair[1].left(), "out of order: {rects:?}");
            assert!(
                (pair[0].top() - pair[1].top()).abs() < 0.75
                    && (pair[0].bottom() - pair[1].bottom()).abs() < 0.75,
                "not one row: {rects:?}"
            );
        }
    }

    #[test]
    fn both_themes_paint_every_card_and_control_inside_the_reference_window() {
        let params = MxmClassicVerbParams::default();
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            let boxes = painted_boxes(theme, &params);
            let words: Vec<_> = boxes.iter().map(|(word, _)| word.as_str()).collect();
            for expected in [
                "Space",
                crate::spaces::FACTORY[SpaceChoice::INIT as usize].name,
                "Pre-delay",
                "Size",
                "Early and late",
                "Decay",
                // The Decay card's own names (B1); the shape's options are pictures (C3).
                "Bass",
                "Treble",
                "Shape",
                "125 Hz",
                "1 kHz",
                "8 kHz",
                "Texture",
                "Diffusion",
                "Mod depth",
                "Mod rate",
                "Output",
                "Mix",
                "Width",
                "Low tone",
                "High tone",
                "Ducking",
                sections::DROP_HERE,
            ] {
                assert!(
                    words.contains(&expected),
                    "{theme:?} never painted {expected:?}; it painted {words:?}"
                );
            }
            for (text, rect) in boxes {
                assert!(
                    rect.left() >= 0.0
                        && rect.right() <= REFERENCE.0 as f32
                        && rect.bottom() <= REFERENCE.1 as f32,
                    "{theme:?} painted {text:?} at {rect:?} outside the reference window"
                );
                if text.chars().count() > 1 {
                    assert!(
                        rect.height() <= 3.0 * rect.width().max(1.0),
                        "{theme:?} squeezed {text:?} into a {:.1} x {:.1} column",
                        rect.width(),
                        rect.height()
                    );
                }
            }
        }
    }

    /// Off and Nothing loaded are words on the display, not a dimmer line alone (brief §8). **Nothing
    /// loaded is the real state now**: it is painted while `Loaded` holds no space, and not once a
    /// space is held.
    #[test]
    fn off_and_nothing_loaded_are_painted_as_words() {
        use nice_plug::params::persist::PersistentField;
        let params = MxmClassicVerbParams::default();
        let words = |params: &MxmClassicVerbParams| -> Vec<String> {
            painted_boxes(egui::ThemePreference::Light, params)
                .into_iter()
                .map(|(word, _)| word)
                .collect()
        };
        let engaged = words(&params);
        assert!(!engaged.iter().any(|w| w == "Off" || w == "Nothing loaded"));

        unsafe {
            let _ = params
                .mix
                ._internal_set_normalized_value(params.mix.preview_normalized(0.0));
            let _ = params.space._internal_set_normalized_value(
                params.space.preview_normalized(SpaceChoice::Loaded),
            );
        }
        let parked = words(&params);
        for state in ["Off", "Nothing loaded"] {
            assert!(
                parked.iter().any(|w| w == state),
                "{state:?} was not painted: {parked:?}"
            );
        }

        params.loaded.set(Payload::holding(&Space::ROOM, None));
        let held = words(&params);
        assert!(
            !held.iter().any(|w| w == "Nothing loaded"),
            "Nothing loaded was painted over a held space: {held:?}"
        );
    }

    /// **A double-click returns a relative control to the space as fitted, as one host gesture.**
    /// Driven through the real panel with pointer events on the knob the cursor registry says is
    /// there, from well off neutral.
    #[test]
    fn a_double_click_returns_every_relative_control_to_its_neutral_as_one_gesture() {
        for id in sections::RELATIVE_IDS {
            let params = MxmClassicVerbParams::default();
            let param = match id {
                "earlylate" => &params.early_late,
                "bass" => &params.bass,
                "treble" => &params.treble,
                "width" => &params.width,
                "lowtone" => &params.low_tone,
                _ => &params.high_tone,
            };
            unsafe {
                let _ = param._internal_set_normalized_value(0.85);
            }
            let telemetry = Telemetry::default();
            let recording = Arc::new(Recording::default());
            let host = GestureWatch::new(GuiContext::new(recording.clone()));
            let mut text_entry = HashMap::new();
            let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
            let mut nav = mxm_ui::navigation::State::default();
            let context = context(egui::ThemePreference::Light);
            let mut time = 0.0;
            let mut frame = |events: Vec<egui::Event>| {
                time += 0.02;
                let input = egui::RawInput {
                    time: Some(time),
                    events,
                    ..window(REFERENCE)
                };
                let mut output = context.run_ui(input, |ui| {
                    panel(
                        ui,
                        &params,
                        &telemetry,
                        &host,
                        &mut text_entry,
                        &mut presets,
                        &mut nav,
                        &mut |_| {},
                    );
                });
                output.textures_delta.clear();
            };
            for _ in 0..4 {
                frame(Vec::new());
            }
            let at = mxm_ui::navigation::spots(&context)
                .into_iter()
                .find(|spot| spot.key == id)
                .unwrap_or_else(|| panic!("{id} is not on the opening page"))
                .rect
                .center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false, true, false] {
                frame(vec![button(pressed)]);
            }
            frame(Vec::new());

            let begins = recording.begins.load(Ordering::Relaxed);
            assert!(begins >= 1, "{id}: the double-click sent nothing");
            assert_eq!(
                begins,
                recording.ends.load(Ordering::Relaxed),
                "{id}: the reset left an unbalanced gesture"
            );
            let last = *recording
                .sets
                .lock()
                .unwrap()
                .last()
                .expect("a value was set");
            assert_eq!(last, param.default_normalized_value(), "{id}");
            assert!(
                (last - 0.5).abs() < 1e-4,
                "{id} did not return to the detent"
            );
        }
    }

    // ── Loading ─────────────────────────────────────────────────────────────────────────────────

    /// A file handle as the patched `egui-baseview` hands one over.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("not read by the editor".to_owned())
        }
    }

    /// Paints the real panel at the opening size and returns the four cards' rectangles and every
    /// text it painted on the frame given `input`.
    struct Surface {
        params: Arc<MxmClassicVerbParams>,
        recording: Arc<Recording>,
        host: GestureWatch,
        context: egui::Context,
        telemetry: Telemetry,
        text_entry: HashMap<&'static str, Option<String>>,
        presets: PresetUi,
        nav: mxm_ui::navigation::State,
        loads: Vec<PathBuf>,
    }

    impl Surface {
        fn new(params: Arc<MxmClassicVerbParams>) -> Self {
            let recording = Arc::new(Recording::default());
            let presets = PresetUi::at(crate::preset::Library::at(None), params.as_ref());
            let mut surface = Self {
                host: GestureWatch::new(GuiContext::new(recording.clone())),
                recording,
                context: context(egui::ThemePreference::Light),
                telemetry: Telemetry::default(),
                text_entry: HashMap::new(),
                presets,
                nav: mxm_ui::navigation::State::default(),
                loads: Vec::new(),
                params,
            };
            for _ in 0..4 {
                surface.frame(window(REFERENCE));
            }
            surface
        }

        fn frame(&mut self, input: egui::RawInput) -> Vec<String> {
            let loads = &mut self.loads;
            let mut output = self.context.run_ui(input, |ui| {
                panel(
                    ui,
                    &self.params,
                    &self.telemetry,
                    &self.host,
                    &mut self.text_entry,
                    &mut self.presets,
                    &mut self.nav,
                    &mut |path| loads.push(path),
                );
            });
            let mut boxes = Vec::new();
            for clipped in &output.shapes {
                collect_boxes(&clipped.shape, &mut boxes);
            }
            output.textures_delta.clear();
            boxes.into_iter().map(|(text, _)| text).collect()
        }

        fn card(&self, index: usize) -> egui::Rect {
            paging_checks::all_rects(&self.context, sections::TITLES.len())[index]
        }
    }

    #[test]
    fn a_drop_on_the_space_card_starts_a_fit_and_writes_no_gesture() {
        let mut surface = Surface::new(Arc::new(MxmClassicVerbParams::default()));
        let path = PathBuf::from("responses/room.aif");
        let drop_at = |surface: &mut Surface, at: egui::Pos2| {
            surface.frame(egui::RawInput {
                events: vec![egui::Event::PointerMoved(at)],
                dropped_files: vec![Arc::new(Dropped(path.clone())) as egui::DroppedFileHandle],
                ..window(REFERENCE)
            });
            surface.frame(window(REFERENCE));
        };

        // On the Output card: not a drop target.
        let output = surface.card(3).center();
        drop_at(&mut surface, output);
        assert!(
            surface.loads.is_empty(),
            "a drop on the Output card was taken"
        );

        let space = surface.card(0).center();
        drop_at(&mut surface, space);
        assert_eq!(surface.loads, vec![path.clone()], "taken exactly once");
        assert_eq!(
            surface.recording.begins.load(Ordering::Relaxed),
            0,
            "a drop wrote a gesture"
        );
    }

    #[test]
    fn a_file_held_over_the_space_card_lights_its_drop_target() {
        let mut surface = Surface::new(Arc::new(MxmClassicVerbParams::default()));
        let at = surface.card(0).center();
        let hovered = egui::RawInput {
            events: vec![egui::Event::PointerMoved(at)],
            hovered_files: vec![egui::HoveredFile {
                path: Some(PathBuf::from("hall.wav")),
                mime: String::new(),
            }],
            ..window(REFERENCE)
        };
        surface.frame(hovered.clone());
        let words = surface.frame(hovered);
        assert!(
            words.iter().any(|w| w == sections::RELEASE_TO_FIT),
            "no drop feedback was painted: {words:?}"
        );
        let words = surface.frame(window(REFERENCE));
        assert!(!words.iter().any(|w| w == sections::RELEASE_TO_FIT));
    }

    /// The Space card's load line says what loading is doing: busy, refused with its reason, and the
    /// fit report's summary once a space is held.
    #[test]
    fn the_space_card_says_what_loading_did() {
        let params = Arc::new(MxmClassicVerbParams::default());
        let mut surface = Surface::new(Arc::clone(&params));

        let generation = params.loaded.begin("cathedral.wav");
        let words = surface.frame(window(REFERENCE));
        assert!(
            words
                .iter()
                .any(|w| w.starts_with("Fitting a space to cathedral.wav")),
            "{words:?}"
        );

        assert!(params.loaded.complete(
            generation,
            Err(crate::loading::Refusal::NonFinite {
                channel: 0,
                frame: 12
            })
        ));
        // A finished job is the editor's to land: the app's frame calls `settle` before `panel`.
        assert_eq!(
            crate::loading::settle(&params, &surface.host.setter()),
            crate::loading::Settled::Refused
        );
        let words = surface.frame(window(REFERENCE));
        assert!(
            words
                .iter()
                .any(|w| w == "Refused: sample 12 of channel 1 is not a number"),
            "{words:?}"
        );
        assert_eq!(
            surface.recording.begins.load(Ordering::Relaxed),
            0,
            "a refusal wrote a gesture"
        );
        assert_eq!(
            params.loaded.status(),
            Status::Refused {
                reason: "sample 12 of channel 1 is not a number".into()
            }
        );

        // Through the app's own frame, so the landing is the editor's.
        let generation = params.loaded.begin("room.wav");
        assert!(
            params
                .loaded
                .complete(generation, Ok(testing::fitted(Space::ROOM, 0.9)))
        );
        assert_eq!(
            crate::loading::settle(&params, &surface.host.setter()),
            crate::loading::Settled::Loaded
        );
        let words = surface.frame(window(REFERENCE));
        assert!(
            words
                .iter()
                .any(|w| w == "Confidence 0.90 · largest error 1 kHz decay +3.0 %"),
            "{words:?}"
        );
    }

    #[test]
    fn closing_the_editor_supersedes_a_running_fit() {
        let params = Arc::new(MxmClassicVerbParams::default());
        let scheduled = Arc::new(Mutex::new(Vec::new()));
        let spawn: Spawn = {
            let scheduled = Arc::clone(&scheduled);
            Arc::new(move |task| scheduled.lock().unwrap().push(task))
        };
        let mut app =
            MxmClassicVerbApp::new(Arc::clone(&params), Arc::new(Telemetry::default()), spawn);
        let generation = params.loaded.begin("room.wav");
        NiceEguiApp::editor_closed(&mut app);
        assert!(!params.loaded.is_current(generation));
        crate::loading::run_with(
            &params.loaded,
            generation,
            |_| {
                Ok(crate::decode::Decoded {
                    channels: vec![vec![0.0; 8]],
                    sample_rate: 48_000,
                })
            },
            &|_, _| Ok(testing::fitted(Space::ROOM, 0.9)),
        );
        assert!(
            params.loaded.take_ready().is_none(),
            "a result outlived its editor"
        );
        assert_eq!(params.loaded.status(), Status::Idle);
        assert_eq!(params.loaded.space(), None);
    }

    use mxm_plugin_test::tree_checks;

    /// Every page at the opening size, light and dark, for the owner's review of the layout-tree
    /// conversion (plans/plan-layout-tree.md §4.3): `target/layout-tree/mxm-classic-verb/<tag>/`,
    /// where `MXM_PICTURES` names the tag — `before` on the unconverted editor, `after` on the tree.
    ///
    /// `MXM_PICTURES=after cargo test -p mxm-classic-verb --lib tree_pictures -- --ignored`
    #[test]
    #[ignore = "renders through wgpu; run by hand"]
    fn tree_pictures() {
        let tag = std::env::var("MXM_PICTURES").unwrap_or_else(|_| "after".to_owned());
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/layout-tree/mxm-classic-verb")
            .join(tag);
        let params = MxmClassicVerbParams::default();
        let telemetry = Telemetry::default();
        let host = quiet_host();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        tree_checks::pictures(
            &|_| {},
            egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            &dir,
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &host,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut |_| {},
                );
            },
        );
    }
}
