//! In-plugin loading (plan §5.2): an impulse response dropped on the editor is checked, decoded and
//! fitted on the background task, then lands through the sampler's commit order.
//!
//! ```text
//! editor: drop ──► begin (new generation) ──► background task
//! task:   extension ─► header ─► preflight ─► decode, every sample finite ─► fit ─► complete
//!                    (each arrow a cancellation point: a superseded job stops, and says nothing)
//! editor: take_ready ─► stage ─► selector and the fit's controls as bracketed gestures ─► commit
//! audio:  the next block reads the committed space
//! ```
//!
//! A refusal at any stage leaves the current space sounding, names the reason on the Space card and
//! emits no gesture. The response itself is never stored, and neither is its path.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use mxm_classic_verb_dsp::Space;
use mxm_preset::ErasedParam;
use nice_plug::context::gui::{GuiContext, GuiContextInner};
use nice_plug::params::internals::ParamPtr;
use nice_plug::prelude::{Param, ParamSetter, PluginApi, PluginState};

use crate::decode::{self, Decoded};
use crate::fitting;
use crate::loaded::LoadedField;
use crate::params::{MxmClassicVerbParams, ShapeChoice, SpaceChoice};
use crate::payload::{Payload, Report};

/// The plugin's background task.
#[derive(Debug)]
pub enum LoadTask {
    /// Fit a space to the file at `path`, for the drop that took `generation`.
    Fit { generation: u64, path: PathBuf },
}

/// Why a dropped file did not become a space. Every decoder and fitter failure is one of these,
/// never a panic.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// Not a WAV or AIFF file, by its extension.
    Unsupported { extension: String },
    /// The file could not be opened or read.
    Unreadable(String),
    /// It claims to be WAV or AIFF and is not a readable one.
    Malformed(String),
    /// A readable file in an encoding this plugin does not decode.
    Encoding(String),
    /// Decoding stopped at the first non-finite sample (plan §4.5, stage 2).
    NonFinite { channel: usize, frame: usize },
    /// The fit crate's own refusal — the header stage, or the analysis — in its own words.
    Input(String),
    /// The response's confidence is under the floor; the weakest check is named.
    LowConfidence {
        confidence: f32,
        floor: f32,
        check: String,
    },
    /// A buffer the file sizes — while decoding, or inside the fit — could not be reserved; `bytes` is
    /// what was asked for. Refused rather than allocated the ordinary way, which would abort the host.
    OutOfMemory { bytes: usize },
    /// The job was superseded. Never shown: a superseded job's outcome is discarded unread.
    Stopped,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Unsupported { extension } if extension.is_empty() => {
                write!(f, "a file without an extension; drop a WAV or AIFF file")
            }
            Refusal::Unsupported { extension } => {
                write!(f, "a .{extension} file; drop a WAV or AIFF file")
            }
            Refusal::Unreadable(why) => write!(f, "the file could not be read: {why}"),
            Refusal::Malformed(why) => write!(f, "{why}"),
            Refusal::Encoding(what) => write!(f, "{what} is not an encoding this plugin reads"),
            Refusal::NonFinite { channel, frame } => write!(
                f,
                "sample {frame} of channel {} is not a number",
                channel + 1
            ),
            Refusal::Input(why) => write!(f, "{why}"),
            Refusal::LowConfidence {
                confidence,
                floor,
                check,
            } => write!(
                f,
                "confidence {confidence:.2} is under {floor:.2}; the weakest check is {check}"
            ),
            Refusal::OutOfMemory { bytes } => write!(
                f,
                "not enough memory for this response ({:.1} MB could not be reserved)",
                *bytes as f64 / 1_048_576.0
            ),
            Refusal::Stopped => write!(f, "superseded"),
        }
    }
}

/// The four absolute controls a fit measured or searched (plan §2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FittedControls {
    pub decay_s: f32,
    pub size_s: f32,
    pub diffusion: f32,
    pub pre_delay_s: f32,
}

/// A fit, in this plugin's own types. `fitting.rs` is the one place that builds one from the fit
/// crate.
#[derive(Clone, Debug, PartialEq)]
pub struct Fitted {
    pub space: Space,
    pub controls: FittedControls,
    pub report: Report,
}

pub type Outcome = Result<Fitted, Refusal>;

/// The fit, injected so a test can step a job by hand, fail it on purpose or supersede it mid-fit.
pub type Fitter<'a> = dyn Fn(&[&[f32]], u32) -> Outcome + Send + Sync + 'a;

/// A drop: takes a generation and names the job. The editor schedules the task it returns.
pub fn begin(loaded: &LoadedField, path: &Path) -> LoadTask {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("the dropped file");
    LoadTask::Fit {
        generation: loaded.begin(name),
        path: path.to_path_buf(),
    }
}

/// The background task's body: the file at `path` decoded after the fit crate's header preflight,
/// then fitted.
pub fn run(loaded: &LoadedField, generation: u64, path: &Path, fit: &Fitter<'_>) {
    run_with(
        loaded,
        generation,
        |keep_going| decode::read_file(path, &mut fitting::preflight, keep_going),
        fit,
    );
}

/// [`run`], with the decoding step given, so a test needs no file.
///
/// **Cancellation points**: before anything, inside decoding (every few thousand frames), before the
/// fit, and at completion. The fit crate offers none inside `fit`, so a job superseded while fitting
/// runs to its end and is discarded by [`LoadedField::complete`].
pub fn run_with(
    loaded: &LoadedField,
    generation: u64,
    decode: impl FnOnce(&dyn Fn() -> bool) -> Result<Decoded, Refusal>,
    fit: &Fitter<'_>,
) {
    if !loaded.is_current(generation) {
        return;
    }
    let keep_going = || loaded.is_current(generation);
    let decoded = decode(&keep_going);
    if !loaded.is_current(generation) {
        return;
    }
    let outcome = decoded.and_then(|decoded| {
        let channels: Vec<&[f32]> = decoded.channels.iter().map(Vec::as_slice).collect();
        fit(&channels, decoded.sample_rate)
    });
    loaded.complete(generation, outcome);
}

/// What a frame's landing did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    Nothing,
    Refused,
    Loaded,
    /// Superseded between being taken and being committed; nothing became audible.
    Superseded,
}

/// **The sampler's commit order**, run by the editor once a frame. A finished fit for the newest
/// drop is staged; the selector moves to `Loaded` and the fit's controls are written as bracketed
/// host gestures; then the space commits, and the next block hears it. A refusal is named and
/// writes nothing.
pub fn settle(params: &MxmClassicVerbParams, setter: &ParamSetter<'_>) -> Settled {
    let loaded = &params.loaded;
    let Some((generation, outcome)) = loaded.take_ready() else {
        return Settled::Nothing;
    };
    match outcome {
        Err(refusal) => {
            if loaded.refuse(generation, refusal.to_string()) {
                Settled::Refused
            } else {
                Settled::Superseded
            }
        }
        Ok(fitted) => {
            if !loaded.stage_for(
                generation,
                Payload::holding(&fitted.space, Some(fitted.report)),
            ) {
                return Settled::Superseded;
            }
            write_the_fit(params, setter, &fitted.controls);
            if loaded.commit_for(generation) {
                Settled::Loaded
            } else {
                Settled::Superseded
            }
        }
    }
}

/// **A fit's parameter contract, as host gestures** (plan §2): the selector to `Loaded`; Decay,
/// Size, Diffusion and Pre-delay to the fitted values; every relative control to neutral,
/// modulation depth to zero and the decay shape to natural — the centre the space was fitted at.
/// Mix, Ducking and modulation rate are never written. `mxm_classic_verb_fit::FittedControls::apply`
/// states the same contract in the fit crate.
///
/// The selector and the four fitted controls are always written. The contract's other values are
/// written **only where they are not already there**, so a patch already at the centre sees exactly
/// five gestures, and a host's undo list carries no edits that changed nothing. Declaration order,
/// so the controls land on the space they were fitted against.
pub fn write_the_fit(
    params: &MxmClassicVerbParams,
    setter: &ParamSetter<'_>,
    controls: &FittedControls,
) {
    let p = params;
    gesture(
        &p.space,
        setter,
        p.space.preview_normalized(SpaceChoice::Loaded),
    );
    gesture(
        &p.decay,
        setter,
        p.decay.preview_normalized(controls.decay_s),
    );
    to_the_centre(&p.bass, setter, p.bass.preview_normalized(1.0));
    to_the_centre(&p.treble, setter, p.treble.preview_normalized(1.0));
    gesture(&p.size, setter, p.size.preview_normalized(controls.size_s));
    gesture(
        &p.diffusion,
        setter,
        p.diffusion.preview_normalized(controls.diffusion),
    );
    gesture(
        &p.pre_delay,
        setter,
        p.pre_delay.preview_normalized(controls.pre_delay_s),
    );
    to_the_centre(&p.early_late, setter, p.early_late.preview_normalized(0.0));
    to_the_centre(
        &p.shape,
        setter,
        p.shape.preview_normalized(ShapeChoice::Natural),
    );
    to_the_centre(&p.mod_depth, setter, p.mod_depth.preview_normalized(0.0));
    to_the_centre(&p.width, setter, p.width.preview_normalized(1.0));
    to_the_centre(&p.low_tone, setter, p.low_tone.preview_normalized(0.0));
    to_the_centre(&p.high_tone, setter, p.high_tone.preview_normalized(0.0));
}

fn gesture(param: &dyn ErasedParam, setter: &ParamSetter<'_>, normalised: f32) {
    let normalised = if normalised.is_finite() {
        normalised.clamp(0.0, 1.0)
    } else {
        param.normalised()
    };
    param.begin(setter);
    param.set(setter, normalised);
    param.end(setter);
}

fn to_the_centre(param: &dyn ErasedParam, setter: &ParamSetter<'_>, normalised: f32) {
    if (param.normalised() - normalised).abs() > 1.0e-6 {
        gesture(param, setter, normalised);
    }
}

/// The editor's host, wrapped so the preset surface's gestures can be noticed.
///
/// **Init has no hook in the preset seam** — `mxm_preset::ui::init_patch` asks the instrument for
/// its parameters and writes each as a gesture, and nothing else — and Init must supersede a running
/// fit (plan §5.2). Every gesture the preset row and its overlays write is a recall or Init: Save,
/// Rename, Delete, favourites and the bank actions write none. So the preset surface is drawn through
/// a setter that counts, and a gesture from it supersedes. A recall also supersedes in
/// `Instrument::apply_preset_state`, before its gestures.
pub struct GestureWatch {
    context: GuiContext,
    gestures: AtomicUsize,
}

impl GestureWatch {
    pub fn new(context: GuiContext) -> Self {
        Self {
            context,
            gestures: AtomicUsize::new(0),
        }
    }

    /// The host itself, for every control that is not the preset surface.
    pub fn setter(&self) -> ParamSetter<'_> {
        self.context.param_setter()
    }

    fn watching(&self) -> ParamSetter<'_> {
        ParamSetter::new(self)
    }
}

impl GuiContextInner for GestureWatch {
    fn plugin_api(&self) -> PluginApi {
        self.context.plugin_api()
    }

    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        self.gestures.fetch_add(1, Ordering::Relaxed);
        unsafe { self.context.raw_begin_set_parameter(param) }
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        unsafe { self.context.raw_set_parameter_normalized(param, normalized) }
    }

    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        unsafe { self.context.raw_end_set_parameter(param) }
    }

    fn get_state(&self) -> PluginState {
        self.context.get_state()
    }

    fn set_state(&self, state: PluginState) {
        self.context.set_state(state);
    }

    fn request_restart(&self) {
        self.context.request_restart();
    }
}

/// Draws the preset surface through the watching setter, and supersedes the running fit if it
/// wrote a gesture — a recall or Init.
pub fn through_the_preset_surface<R>(
    loaded: &LoadedField,
    watch: &GestureWatch,
    draw: impl FnOnce(&ParamSetter<'_>) -> R,
) -> R {
    watch.gestures.store(0, Ordering::Relaxed);
    let result = draw(&watch.watching());
    if watch.gestures.swap(0, Ordering::Relaxed) > 0 {
        loaded.supersede();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loaded::Status;
    use crate::testing::{self, ApplyingHost};
    use std::sync::Arc;

    fn decoded() -> Result<Decoded, Refusal> {
        Ok(Decoded {
            channels: vec![vec![0.0; 16]],
            sample_rate: 48_000,
        })
    }

    fn fits(space: Space) -> impl Fn(&[&[f32]], u32) -> Outcome + Send + Sync {
        move |_, _| Ok(testing::fitted(space, 0.9))
    }

    fn never(_: &[&[f32]], _: u32) -> Outcome {
        panic!("a superseded job reached the fit");
    }

    fn host() -> (Arc<ApplyingHost>, GestureWatch) {
        let host = Arc::new(ApplyingHost::default());
        let watch = GestureWatch::new(GuiContext::new(host.clone()));
        (host, watch)
    }

    /// **The sampler's commit order.** At every gesture the fitted space is staged and not yet
    /// committed; after the last one it is committed; every gesture is balanced; the selector moves to
    /// `Loaded` and the four fitted controls land.
    #[test]
    fn a_fit_lands_staged_then_as_bracketed_gestures_then_committed() {
        let params = Arc::new(MxmClassicVerbParams::default());
        let (host, watch) = host();
        let before = params.loaded.committed_sequence();
        {
            let params = Arc::clone(&params);
            host.on_gesture(move || {
                assert!(params.loaded.is_staged(), "a gesture came before the stage");
                assert_eq!(
                    params.loaded.committed_sequence(),
                    before,
                    "the space was audible before its gestures"
                );
            });
        }
        let generation = params.loaded.begin("room.wav");
        run_with(
            &params.loaded,
            generation,
            |_| decoded(),
            &fits(Space::ROOM),
        );
        assert_eq!(settle(&params, &watch.setter()), Settled::Loaded);

        assert_eq!(params.loaded.space(), Some(Space::ROOM));
        assert!(!params.loaded.is_staged());
        assert_eq!(params.loaded.status(), Status::Idle);
        assert_eq!(host.begins(), host.ends());
        assert_eq!(
            host.begins(),
            5,
            "the selector and the four fitted controls"
        );
        assert_eq!(params.space.value(), SpaceChoice::Loaded);
        let controls = testing::fitted(Space::ROOM, 0.9).controls;
        assert!((params.decay.value() - controls.decay_s).abs() < 1.0e-3);
        assert!((params.size.value() - controls.size_s).abs() < 1.0e-4);
        assert!((params.diffusion.value() - controls.diffusion).abs() < 1.0e-4);
        assert!((params.pre_delay.value() - controls.pre_delay_s).abs() < 1.0e-4);
        assert_eq!(
            settle(&params, &watch.setter()),
            Settled::Nothing,
            "it landed twice"
        );
    }

    /// Plan §2: a fit returns every relative control to neutral, modulation depth to zero and the
    /// shape to natural, and never touches Mix, Ducking or modulation rate.
    #[test]
    fn a_load_returns_the_patch_to_the_fitted_centre_and_leaves_the_mix_alone() {
        let params = MxmClassicVerbParams::default();
        testing::set(&params.bass, 3.0);
        testing::set(&params.width, 2.0);
        testing::set(&params.shape, ShapeChoice::Gated);
        testing::set(&params.mod_depth, 0.002);
        testing::set(&params.mix, 0.7);
        testing::set(&params.duck, 0.4);
        testing::set(&params.mod_rate, 3.0);
        let (mix, duck, rate) = (
            params.mix.value(),
            params.duck.value(),
            params.mod_rate.value(),
        );
        let (host, watch) = host();
        let generation = params.loaded.begin("hall.aif");
        run_with(
            &params.loaded,
            generation,
            |_| decoded(),
            &fits(Space::HALL),
        );
        assert_eq!(settle(&params, &watch.setter()), Settled::Loaded);
        assert!(
            (params.bass.value() - 1.0).abs() < 1.0e-4,
            "{}",
            params.bass.value()
        );
        assert_eq!(params.width.value(), 1.0);
        assert_eq!(params.shape.value(), ShapeChoice::Natural);
        assert_eq!(params.mod_depth.value(), 0.0);
        assert_eq!(
            (
                params.mix.value(),
                params.duck.value(),
                params.mod_rate.value()
            ),
            (mix, duck, rate)
        );
        assert_eq!(
            host.begins(),
            9,
            "five, plus the four controls that were off the centre"
        );
        assert_eq!(host.begins(), host.ends());
    }

    #[test]
    fn a_refusal_is_named_and_writes_nothing() {
        let params = MxmClassicVerbParams::default();
        let (host, watch) = host();
        let generation = params.loaded.begin("song.mp3");
        run(&params.loaded, generation, Path::new("song.mp3"), &never);
        assert_eq!(settle(&params, &watch.setter()), Settled::Refused);
        assert_eq!(host.begins(), 0);
        assert_eq!(params.loaded.space(), None);
        let Status::Refused { reason } = params.loaded.status() else {
            panic!("no reason was given");
        };
        assert!(reason.contains(".mp3"), "{reason}");
    }

    #[test]
    fn a_fit_under_the_confidence_floor_leaves_the_current_space_and_writes_nothing() {
        let params = MxmClassicVerbParams::default();
        let (host, watch) = host();
        let first = params.loaded.begin("good.wav");
        run_with(&params.loaded, first, |_| decoded(), &fits(Space::PLATE));
        settle(&params, &watch.setter());
        let gestures = host.begins();

        let generation = params.loaded.begin("music.wav");
        run_with(&params.loaded, generation, |_| decoded(), &|_, _| {
            // The adapter's own floor check, which every fit passes through.
            fitting::judged(testing::fitted(Space::ROOM, 0.1))
        });
        assert_eq!(settle(&params, &watch.setter()), Settled::Refused);
        assert_eq!(host.begins(), gestures, "a refusal wrote a gesture");
        assert_eq!(
            params.loaded.space(),
            Some(Space::PLATE),
            "the current space was lost"
        );
        let Status::Refused { reason } = params.loaded.status() else {
            panic!("no reason");
        };
        assert!(reason.contains("confidence 0.10"), "{reason}");
    }

    /// **A drop the machine has no memory for leaves the space that was sounding.** Refused at the
    /// decoder's reservation seam, through the real job and a real file, and then as the fit's own
    /// refusal: named on the card, no gesture, the selector and the fitted controls where they were,
    /// and nothing staged or committed.
    #[test]
    fn a_refused_import_leaves_the_space_that_was_sounding() {
        let params = MxmClassicVerbParams::default();
        let (host, watch) = host();
        let first = params.loaded.begin("hall.wav");
        run_with(&params.loaded, first, |_| decoded(), &fits(Space::HALL));
        assert_eq!(settle(&params, &watch.setter()), Settled::Loaded);
        let (gestures, sequence) = (host.begins(), params.loaded.committed_sequence());
        let controls = (
            params.space.value(),
            params.decay.value(),
            params.size.value(),
        );
        let left_alone = |stage: &str| {
            assert_eq!(
                settle(&params, &watch.setter()),
                Settled::Refused,
                "{stage}"
            );
            let Status::Refused { reason } = params.loaded.status() else {
                panic!("{stage}: no reason was given");
            };
            assert!(reason.contains("not enough memory"), "{stage}: {reason}");
            assert_eq!(params.loaded.space(), Some(Space::HALL), "{stage}");
            assert_eq!(params.loaded.committed_sequence(), sequence, "{stage}");
            assert!(!params.loaded.is_staged(), "{stage}");
            assert_eq!(
                host.begins(),
                gestures,
                "{stage}: a refusal wrote a gesture"
            );
            assert_eq!(
                (
                    params.space.value(),
                    params.decay.value(),
                    params.size.value()
                ),
                controls,
                "{stage}"
            );
        };

        let planted = testing::response(24_000, 1.2, 0.5, 2, 12);
        let file = testing::TempFile::new(
            "no-room.wav",
            &testing::wav_bytes(&planted, 24_000, testing::WavFormat::Float32),
        );
        let LoadTask::Fit { generation, path } = begin(&params.loaded, file.path());
        crate::decode::seam::refusing(|| {
            run(&params.loaded, generation, &path, &|_, _| {
                panic!("a decode refused for memory went on to the fit")
            })
        });
        left_alone("decoding");

        let generation = params.loaded.begin("no-room-to-fit.wav");
        run_with(&params.loaded, generation, |_| decoded(), &|_, _| {
            Err(Refusal::OutOfMemory { bytes: 92_160_000 })
        });
        left_alone("fitting");
    }

    // ── §5.2's races, each stepped by hand ─────────────────────────────────────────────────────

    #[test]
    fn a_superseded_job_stops_before_it_decodes_or_fits() {
        let loaded = LoadedField::new();
        let generation = loaded.begin("slow.wav");
        loaded.supersede();
        run_with(
            &loaded,
            generation,
            |_| panic!("a superseded job decoded"),
            &never,
        );
        // Superseded during decoding: the decoder is told to stop, and the fit never runs.
        let generation = loaded.begin("slow.wav");
        run_with(
            &loaded,
            generation,
            |keep_going| {
                assert!(keep_going());
                loaded.supersede();
                assert!(
                    !keep_going(),
                    "the cancellation point did not see the supersession"
                );
                Err(Refusal::Stopped)
            },
            &never,
        );
        assert!(loaded.take_ready().is_none());
        assert_eq!(loaded.status(), Status::Idle);
    }

    #[test]
    fn a_second_drop_supersedes_the_first() {
        let params = MxmClassicVerbParams::default();
        let (host, watch) = host();
        let first = params.loaded.begin("first.wav");
        let second = params.loaded.begin("second.wav");
        // The first finishes after the second was dropped, and before the second finishes.
        run_with(&params.loaded, first, |_| decoded(), &fits(Space::ROOM));
        assert_eq!(settle(&params, &watch.setter()), Settled::Nothing);
        assert_eq!(host.begins(), 0, "the superseded fit wrote a gesture");
        assert_eq!(
            params.loaded.status(),
            Status::Fitting {
                name: "second.wav".into()
            },
            "the superseded fit left a message"
        );
        run_with(&params.loaded, second, |_| decoded(), &fits(Space::CHAMBER));
        assert_eq!(settle(&params, &watch.setter()), Settled::Loaded);
        assert_eq!(params.loaded.space(), Some(Space::CHAMBER));
    }

    #[test]
    fn a_recall_during_a_fit_supersedes_it() {
        let params = MxmClassicVerbParams::default();
        let (host, watch) = host();
        let generation = params.loaded.begin("room.wav");
        let recipe = mxm_preset::Preset::parse(crate::preset::FACTORY_FILES[0].1, crate::CLAP_ID)
            .expect("a factory preset");
        let (applied, _) = through_the_preset_surface(&params.loaded, &watch, |setter| {
            mxm_preset::ui::apply_preset_checked(&params, setter, &recipe)
        });
        assert!(applied);
        let recalled = host.begins();
        run_with(
            &params.loaded,
            generation,
            |_| decoded(),
            &fits(Space::ROOM),
        );
        assert_eq!(settle(&params, &watch.setter()), Settled::Nothing);
        assert_eq!(host.begins(), recalled, "the fit wrote over the recall");
        assert_eq!(params.loaded.space(), None);
        assert_eq!(params.loaded.status(), Status::Idle);
    }

    #[test]
    fn init_during_a_fit_supersedes_it() {
        let params = MxmClassicVerbParams::default();
        let (host, watch) = host();
        let generation = params.loaded.begin("room.wav");
        through_the_preset_surface(&params.loaded, &watch, |setter| {
            mxm_preset::ui::init_patch(&params, setter)
        });
        let init = host.begins();
        assert_eq!(init, 18, "Init writes every parameter");
        run_with(
            &params.loaded,
            generation,
            |_| decoded(),
            &fits(Space::ROOM),
        );
        assert_eq!(settle(&params, &watch.setter()), Settled::Nothing);
        assert_eq!(host.begins(), init);
        assert_eq!(params.space.value(), SpaceChoice::INIT);
    }

    /// Save, Rename, Delete and favourites write no gesture, so they do not cancel a fit.
    #[test]
    fn a_preset_action_without_a_gesture_does_not_supersede() {
        let params = MxmClassicVerbParams::default();
        let (_, watch) = host();
        let generation = params.loaded.begin("room.wav");
        through_the_preset_surface(&params.loaded, &watch, |_| {
            let _ = mxm_preset::Preset::capture("Mine", mxm_preset::Category::Fx, &params);
        });
        assert!(params.loaded.is_current(generation));
    }

    #[test]
    fn a_host_state_restore_during_a_fit_supersedes_it() {
        use nice_plug::params::persist::PersistentField;
        let params = MxmClassicVerbParams::default();
        let (host, watch) = host();
        let generation = params.loaded.begin("room.wav");
        params.loaded.set(Payload::holding(&Space::PLATE, None));
        run_with(
            &params.loaded,
            generation,
            |_| decoded(),
            &fits(Space::ROOM),
        );
        assert_eq!(settle(&params, &watch.setter()), Settled::Nothing);
        assert_eq!(host.begins(), 0);
        assert_eq!(
            params.loaded.space(),
            Some(Space::PLATE),
            "the restore was overwritten"
        );
    }

    /// **A completion at the same boundary as a supersession**, at each of the boundaries there are:
    /// superseded while the fit runs (it completes into a newer generation); superseded after it
    /// completed and before the editor looked; and superseded after the editor took it, between its
    /// gestures and its commit — the one that can only happen with a host restoring state on another
    /// thread. In none of them does the superseded space become audible or leave a message.
    #[test]
    fn a_completion_at_the_boundary_of_a_supersession_is_discarded() {
        // While fitting.
        let params = Arc::new(MxmClassicVerbParams::default());
        let (host, watch) = host();
        let generation = params.loaded.begin("a.wav");
        run_with(&params.loaded, generation, |_| decoded(), &|_, _| {
            params.loaded.supersede();
            Ok(testing::fitted(Space::ROOM, 0.9))
        });
        assert_eq!(settle(&params, &watch.setter()), Settled::Nothing);

        // Completed, then superseded before the editor's frame.
        let generation = params.loaded.begin("b.wav");
        run_with(
            &params.loaded,
            generation,
            |_| decoded(),
            &fits(Space::ROOM),
        );
        params.loaded.supersede();
        assert_eq!(settle(&params, &watch.setter()), Settled::Nothing);
        assert_eq!(host.begins(), 0);

        // Taken, then superseded by a restore between the gestures and the commit.
        let generation = params.loaded.begin("c.wav");
        run_with(
            &params.loaded,
            generation,
            |_| decoded(),
            &fits(Space::ROOM),
        );
        let sequence = params.loaded.committed_sequence();
        {
            let fired = std::sync::atomic::AtomicBool::new(false);
            let restoring = Arc::clone(&params);
            host.on_gesture(move || {
                if !fired.swap(true, Ordering::Relaxed) {
                    restoring.loaded.supersede();
                }
            });
        }
        assert_eq!(settle(&params, &watch.setter()), Settled::Superseded);
        assert_eq!(
            params.loaded.committed_sequence(),
            sequence,
            "a superseded space was committed"
        );
        assert!(!params.loaded.is_staged());
        assert_eq!(params.loaded.space(), None);
        assert_eq!(params.loaded.status(), Status::Idle);
    }
}
