//! `mxm-classic-verb` — an everyday algorithmic reverb whose spaces can be fitted from impulse
//! responses.
//!
//! The plan is `plans/plan-mxm-classic-verb.md`. The DSP is `crates/mxm-classic-verb-dsp`, which
//! smooths every control itself; this shell hands it plain values and a space, and reports its
//! activity to the host. A space fitted from a dropped impulse response is `loading.rs`'s, fitted on
//! the background task and read here, lock-free, at the start of a block.

macro_rules! plugin_name {
    () => {
        "mxm-classic-verb"
    };
}

pub const NAME: &str = plugin_name!();
pub const CLAP_ID: &str = concat!("dk.mxm.", plugin_name!());

pub mod decode;
pub mod editor;
pub mod fitting;
pub mod loaded;
pub mod loading;
pub mod params;
pub mod payload;
pub mod preset;
pub mod spaces;
pub mod telemetry;
#[cfg(test)]
pub(crate) mod testing;

use mxm_classic_verb_dsp::{Controls, Engine, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE, Space};
use nice_plug::prelude::*;
use params::MxmClassicVerbParams;
use std::sync::Arc;
use telemetry::Telemetry;

/// The persistent field the loaded space is kept under in host state.
pub const LOADED_FIELD: &str = "loaded";

pub struct MxmClassicVerb {
    pub params: Arc<MxmClassicVerbParams>,
    telemetry: Arc<Telemetry>,
    engine: Engine,
    sample_rate: f32,
    input_channels: usize,
    applied_controls: Controls,
    /// The committed loaded space as this thread last read it, and the sequence it was read at.
    loaded_space: Option<Space>,
    loaded_sequence: u64,
    /// Pre-delay and mod rate as their syncs resolved them for this block, or `None` for their
    /// free values.
    synced: [Option<f32>; 2],
}

impl Default for MxmClassicVerb {
    fn default() -> Self {
        let params = Arc::new(MxmClassicVerbParams::default());
        let mut plugin = Self {
            applied_controls: Controls::default(),
            loaded_space: None,
            // Never a real sequence, so the first block reads the committed space.
            loaded_sequence: u64::MAX,
            params,
            telemetry: Telemetry::shared(),
            engine: Engine::new(48_000.0),
            sample_rate: 48_000.0,
            input_channels: 1,
            synced: [None; 2],
        };
        plugin.synchronise();
        plugin
    }
}

impl MxmClassicVerb {
    fn prepare(&mut self, sample_rate: f32, input_channels: usize) {
        // Forget the last activation's tempo too: nice-plug resets right after activating, and a
        // division resolved from a tempo the host may since have changed would seed the engine.
        self.telemetry.tempo.publish(None);
        // A restored state is resolved afresh by the next block: activation must not seed the
        // engine with the division the previous state was synced to.
        self.synced = [None; 2];
        self.sample_rate = valid_sample_rate(sample_rate);
        self.input_channels = input_channels.clamp(1, 2);
        self.engine = Engine::new(self.sample_rate);
        // State may have been restored before activation: the engine starts at it, not at the Rust
        // defaults followed by an audible transition.
        self.synchronise();
        self.engine.reset();
    }

    fn controls(&self) -> Controls {
        let p = &self.params;
        Controls {
            mix: p.mix.value(),
            decay_s: p.decay.value(),
            bass_mult: p.bass.value(),
            treble_mult: p.treble.value(),
            size_s: p.size.value(),
            diffusion: p.diffusion.value(),
            // Synced, pre-delay and mod rate are their divisions (`plans/plan-tempo-sync-controls.md`).
            pre_delay_s: self.synced[0].unwrap_or_else(|| p.pre_delay.value()),
            early_late_db: p.early_late.value(),
            shape: p.shape.value().dsp(),
            mod_depth_s: p.mod_depth.value(),
            mod_rate_hz: self.synced[1].unwrap_or_else(|| p.mod_rate.value()),
            width: p.width.value(),
            tone_low_db: p.low_tone.value(),
            tone_high_db: p.high_tone.value(),
            duck: p.duck.value(),
        }
    }

    /// Hands the engine the parameters' current values, each only when it changed. The engine smooths
    /// every control itself, so a block-rate update is the whole contract; an unchanged value is not
    /// sent again, so a constant setting cannot make the output depend on the host's block size.
    ///
    /// **The loaded space is read here, once a block, and only through atomics** (`loaded.rs`): a
    /// commit that lands mid-block is heard from the next one, so the barrier is the process boundary.
    /// A read that meets a write in progress keeps the space it has and looks again next block. A space
    /// change, a loaded space replacing the one sounding included, is the engine's fade (plan §3.4).
    fn synchronise(&mut self) {
        let loaded = &self.params.loaded;
        if loaded.committed_sequence() != self.loaded_sequence
            && let Some((sequence, space)) = loaded.read_committed()
        {
            self.loaded_sequence = sequence;
            self.loaded_space = space;
        }
        let space = self.params.space.value().sounding(self.loaded_space);
        if space != self.engine.space() {
            self.engine.set_space(space);
        }
        let controls = self.controls();
        if controls != self.applied_controls || controls != self.engine.controls() {
            self.engine.set_controls(controls);
            self.applied_controls = controls;
        }
    }

    fn tail_samples(&self) -> u32 {
        (self.engine.remaining_tail_seconds() * self.sample_rate).clamp(0.0, u32::MAX as f32) as u32
    }

    pub fn prepare_for_test(&mut self, sample_rate: f32, input_channels: usize) {
        self.prepare(sample_rate, input_channels);
    }

    pub fn process_block_for_test(&mut self, channels: &mut [&mut [f32]]) -> ProcessStatus {
        self.process_block(channels)
    }

    pub fn engine_for_test(&self) -> &Engine {
        &self.engine
    }

    fn process_block(&mut self, channels: &mut [&mut [f32]]) -> ProcessStatus {
        let Some(first) = channels.first() else {
            return ProcessStatus::Normal;
        };
        let samples = first.len();
        let mono = self.input_channels == 1 || channels.len() < 2;
        let inputs = self.input_channels.min(channels.len());
        let mut has_input = false;
        for channel in &mut channels[..inputs] {
            for sample in &mut channel[..samples] {
                if sample.abs() < f32::MIN_POSITIVE || !sample.is_finite() {
                    *sample = 0.0;
                } else {
                    has_input = true;
                }
            }
        }

        self.synchronise();
        if !has_input && self.engine.is_parked() {
            if mono && channels.len() > 1 {
                let (left, rest) = channels.split_at_mut(1);
                rest[0][..samples].copy_from_slice(&left[0][..samples]);
            }
            return ProcessStatus::Normal;
        }

        let mut peak = 0.0f32;
        let mut wet = 0.0f32;
        for index in 0..samples {
            let dry_l = channels[0][index];
            let dry_r = if mono { dry_l } else { channels[1][index] };
            let (out_l, out_r) = self.engine.process(dry_l, dry_r);
            peak = peak.max(out_l.abs()).max(out_r.abs());
            wet = wet.max((out_l - dry_l).abs()).max((out_r - dry_r).abs());
            channels[0][index] = out_l;
            if channels.len() > 1 {
                channels[1][index] = out_r;
            }
        }
        if channels.len() > 2 {
            let (left, rest) = channels.split_at_mut(1);
            for channel in rest.iter_mut().skip(1) {
                channel[..samples].copy_from_slice(&left[0][..samples]);
            }
        }
        self.telemetry.publish(peak, wet);

        if has_input || self.engine.is_parked() {
            ProcessStatus::Normal
        } else {
            ProcessStatus::Tail(self.tail_samples())
        }
    }
}

impl Plugin for MxmClassicVerb {
    const NAME: &'static str = NAME;
    const VENDOR: &'static str = "mxm";
    const URL: &'static str = "https://mxm.dk";
    const EMAIL: &'static str = "plugins@mxm.dk";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type Editor = editor::MxmClassicVerbEditor;
    type SysExMessage = ();
    type BackgroundTask = loading::LoadTask;

    /// **The fit runs here, on nice-plug's background thread** — never in `process()` and never on
    /// the editor's thread (plan §5.2). It takes most of a second in release, which is why the editor
    /// shows a busy line and no progress.
    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = Arc::clone(&self.params);
        Box::new(move |task| match task {
            loading::LoadTask::Fit { generation, path } => {
                loading::run(&params.loaded, generation, &path, &fitting::fit);
            }
        })
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        editor::create(self.params.clone(), self.telemetry.clone(), async_executor)
    }

    /// **A loaded space this build cannot read is refused whole** (plan §2.2): nice-plug 0.3 cannot
    /// reject state from this hook, so the whole state becomes a no-op — parameters and fields
    /// cleared before either is written — which is the sampler's answer to the same limit. **State
    /// from before loading existed carries no field**, and gets an explicit absence, so a restore
    /// always settles the loaded space and always supersedes a running fit.
    fn filter_state(state: &mut PluginState) {
        // First, so a state the payload check below clears stays a complete no-op.
        mxm_preset::add_switches_off(state, crate::preset::TEMPO_SYNC_IDS);
        match state.fields.get(LOADED_FIELD) {
            None => {
                state.fields.insert(
                    LOADED_FIELD.to_owned(),
                    nice_plug::params::persist::serialize_field(&payload::Payload::absence())
                        .expect("an absence serialises"),
                );
            }
            Some(serialized) if payload::parse_serialized(serialized).is_err() => {
                state.params.clear();
                state.fields.clear();
            }
            Some(_) => {}
        }
    }

    fn activate(
        &mut self,
        layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.prepare(
            config.sample_rate,
            layout
                .main_input_channels
                .map_or(1, |channels| channels.get() as usize),
        );
        true
    }

    fn reset(&mut self) {
        // Parameter flushes can arrive while a host has the effect bypassed; settle on them first so
        // the next excitation starts in the current setting. **Reset does not supersede a running
        // fit** (plan §5.2): it clears DSP state, not the patch.
        // A host resets without a callback between (a bypass, a transport restart), and a parameter
        // flush may have moved a sync meanwhile: re-resolve every sync from the parameters as they
        // stand and the last tempo seen, so nothing is seeded from the previous division.
        let tempo = self.telemetry.tempo.get();
        self.synced = [
            self.params.synced_pre_delay(tempo),
            self.params.synced_mod_rate(tempo),
        ];
        self.synchronise();
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // The tempo syncs, once a block, and the tempo in force for the editor's readings.
        let tempo = context.transport().tempo;
        self.synced = [
            self.params.synced_pre_delay(tempo),
            self.params.synced_mod_rate(tempo),
        ];
        self.telemetry.tempo.publish(tempo);
        self.process_block(buffer.as_slice())
    }
}

impl ClapPlugin for MxmClassicVerb {
    const CLAP_ID: &'static str = CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Rooms, chambers, halls and plates, with controls that reach past any real space");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Reverb,
        ClapFeature::Stereo,
    ];
}

nice_export_clap!(MxmClassicVerb);

fn valid_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() {
        sample_rate.clamp(MIN_SAMPLE_RATE, MAX_SAMPLE_RATE)
    } else {
        48_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Activation does not seed the engine with a previous state's divisions**: a host restoring
    /// an unsynced state reactivates the same object, and the controls it starts at are the knobs'.
    #[test]
    fn activation_forgets_the_previous_states_synced_values() {
        let mut plugin = MxmClassicVerb {
            synced: [Some(0.25), Some(8.0)],
            ..Default::default()
        };
        plugin.prepare(48_000.0, 2);
        let controls = plugin.controls();
        assert_eq!(controls.pre_delay_s, plugin.params.pre_delay.value());
        assert_eq!(controls.mod_rate_hz, plugin.params.mod_rate.value());
    }
    use crate::params::SpaceChoice;
    use nice_plug::params::persist::PersistentField;
    use nice_plug::params::{InternalParamMut, Param};

    fn set_float(param: &FloatParam, value: f32) {
        unsafe {
            let _ = param._internal_set_normalized_value(param.preview_normalized(value));
        }
    }

    fn set_space(param: &EnumParam<SpaceChoice>, value: SpaceChoice) {
        unsafe {
            let _ = param._internal_set_normalized_value(param.preview_normalized(value));
        }
    }

    fn render(plugin: &mut MxmClassicVerb, left: &mut [f32], right: &mut [f32]) -> ProcessStatus {
        let mut channels: Vec<&mut [f32]> = vec![left, right];
        plugin.process_block(&mut channels)
    }

    #[test]
    fn mix_zero_is_the_dry_to_the_bit_in_both_layouts() {
        let left_input: Vec<f32> = (0..4096).map(|n| (n as f32 * 0.13).sin() * 0.4).collect();
        let right_input: Vec<f32> = (0..4096).map(|n| (n as f32 * 0.07).cos() * 0.3).collect();
        for input_channels in [1, 2] {
            let mut plugin = MxmClassicVerb::default();
            set_float(&plugin.params.mix, 0.0);
            plugin.prepare(48_000.0, input_channels);
            let (mut left, mut right) = (left_input.clone(), right_input.clone());
            render(&mut plugin, &mut left, &mut right);
            assert_eq!(left, left_input);
            let expected_right = if input_channels == 1 {
                &left_input
            } else {
                &right_input
            };
            assert_eq!(&right, expected_right, "{input_channels} input channel(s)");
        }
    }

    #[test]
    fn a_tail_is_reported_and_then_the_effect_sleeps() {
        let mut plugin = MxmClassicVerb::default();
        set_float(&plugin.params.decay, 0.5);
        plugin.prepare(48_000.0, 2);
        let (mut left, mut right) = (vec![0.0; 4096], vec![0.0; 4096]);
        left[0] = 0.8;
        right[0] = 0.8;
        assert_eq!(
            render(&mut plugin, &mut left, &mut right),
            ProcessStatus::Normal
        );
        left.fill(0.0);
        right.fill(0.0);
        assert!(matches!(
            render(&mut plugin, &mut left, &mut right),
            ProcessStatus::Tail(n) if n > 0
        ));
        let mut blocks = 0;
        loop {
            left.fill(0.0);
            right.fill(0.0);
            if render(&mut plugin, &mut left, &mut right) == ProcessStatus::Normal {
                break;
            }
            blocks += 1;
            assert!(blocks < 200, "the effect never went back to sleep");
        }
        assert!(plugin.engine.is_parked());
    }

    #[test]
    fn a_parameter_change_reaches_the_engine_at_the_next_block() {
        let mut plugin = MxmClassicVerb::default();
        plugin.prepare(48_000.0, 2);
        let other = SpaceChoice::factory_positions()
            .find(|choice| *choice != SpaceChoice::INIT)
            .expect("more than one factory space");
        set_space(&plugin.params.space, other);
        set_float(&plugin.params.decay, 5.0);
        let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
        render(&mut plugin, &mut left, &mut right);
        assert_eq!(plugin.params.space.value(), other);
        assert_eq!(plugin.engine.space(), other.factory());
        // The parameter's own value, not the literal: a skewed range's normalised round trip is
        // not exact in f32.
        assert_eq!(
            plugin.engine.controls().decay_s,
            plugin.params.decay.value()
        );
        assert!((plugin.engine.controls().decay_s - 5.0).abs() < 1e-3);
    }

    #[test]
    fn a_constant_setting_renders_identically_whatever_the_block_size() {
        let input: Vec<f32> = (0..9_600)
            .map(|n| ((n * 7919 % 1000) as f32 / 500.0 - 1.0) * 0.3)
            .collect();
        let run = |block: usize| {
            let mut plugin = MxmClassicVerb::default();
            plugin.prepare(48_000.0, 2);
            let (mut l, mut r) = (input.clone(), input.clone());
            for (bl, br) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
                render(&mut plugin, bl, br);
            }
            (l, r)
        };
        assert_eq!(run(64), run(333));
    }

    fn commit(params: &MxmClassicVerbParams, space: Space) {
        let generation = params.loaded.begin("test.wav");
        assert!(
            params
                .loaded
                .stage_for(generation, payload::Payload::holding(&space, None))
        );
        assert!(params.loaded.commit_for(generation));
    }

    /// Plan §2.2: `Loaded` with nothing loaded plays the Init space; a committed space replaces it;
    /// an absence restores it.
    #[test]
    fn loaded_with_nothing_loaded_plays_the_init_space() {
        let mut plugin = MxmClassicVerb::default();
        set_space(&plugin.params.space, SpaceChoice::Loaded);
        plugin.prepare(48_000.0, 2);
        assert_eq!(plugin.engine.space(), SpaceChoice::INIT.factory());
        let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
        commit(&plugin.params, Space::ROOM);
        render(&mut plugin, &mut left, &mut right);
        assert_eq!(plugin.engine.space(), Space::ROOM);
        plugin.params.loaded.set(payload::Payload::absence());
        render(&mut plugin, &mut left, &mut right);
        assert_eq!(plugin.engine.space(), SpaceChoice::INIT.factory());
    }

    /// **A staged space is not heard; a committed one is heard from the next block.**
    #[test]
    fn a_committed_space_reaches_the_engine_at_the_next_block_and_a_staged_one_does_not() {
        let mut plugin = MxmClassicVerb::default();
        set_space(&plugin.params.space, SpaceChoice::Loaded);
        plugin.prepare(48_000.0, 2);
        let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
        let generation = plugin.params.loaded.begin("chamber.wav");
        assert!(
            plugin
                .params
                .loaded
                .stage_for(generation, payload::Payload::holding(&Space::CHAMBER, None))
        );
        render(&mut plugin, &mut left, &mut right);
        assert_eq!(
            plugin.engine.space(),
            SpaceChoice::INIT.factory(),
            "a staged space was audible"
        );
        assert!(plugin.params.loaded.commit_for(generation));
        render(&mut plugin, &mut left, &mut right);
        assert_eq!(plugin.engine.space(), Space::CHAMBER);
    }

    /// A block that meets a write in progress keeps the space it has, and the next block takes the
    /// new one: the audio thread never waits for the writer and never reads half a space.
    #[test]
    fn a_write_in_progress_is_left_for_the_next_block() {
        let mut plugin = MxmClassicVerb::default();
        set_space(&plugin.params.space, SpaceChoice::Loaded);
        plugin.prepare(48_000.0, 2);
        let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
        plugin.params.loaded.open_a_write_for_test();
        render(&mut plugin, &mut left, &mut right);
        assert_eq!(plugin.engine.space(), SpaceChoice::INIT.factory());
        plugin
            .params
            .loaded
            .close_the_write_for_test(Some(Space::PLATE));
        render(&mut plugin, &mut left, &mut right);
        assert_eq!(plugin.engine.space(), Space::PLATE);
    }

    fn noise_block(plugin: &mut MxmClassicVerb, rng: &mut testing::XorShift) -> f32 {
        let mut left: Vec<f32> = (0..48).map(|_| (0.3 * rng.bipolar()) as f32).collect();
        let mut right = left.clone();
        render(plugin, &mut left, &mut right);
        (left.iter().chain(&right).map(|v| v * v).sum::<f32>() / 96.0).sqrt()
    }

    /// Plan §3.4: **a loaded space replacing the one sounding is a space change**, so the wet fades
    /// through silence, swaps and fades back rather than jumping. At Mix 1 the output is the wet.
    #[test]
    fn replacing_the_loaded_space_while_it_sounds_is_the_engines_fade() {
        let mut plugin = MxmClassicVerb::default();
        set_space(&plugin.params.space, SpaceChoice::Loaded);
        set_float(&plugin.params.mix, 1.0);
        commit(&plugin.params, Space::ROOM);
        plugin.prepare(48_000.0, 2);
        let mut rng = testing::XorShift::new(4);
        let steady: Vec<f32> = (0..1_000)
            .map(|_| noise_block(&mut plugin, &mut rng))
            .collect();
        let level = steady[500..].iter().sum::<f32>() / 500.0;
        commit(&plugin.params, Space::CHAMBER);
        let fade: Vec<f32> = (0..90)
            .map(|_| noise_block(&mut plugin, &mut rng))
            .collect();
        let quietest = fade.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            quietest < 0.1 * level,
            "the wet never went quiet: {quietest} against a steady {level}"
        );
        assert_eq!(plugin.engine.space(), Space::CHAMBER);
        let after = fade[80..].iter().sum::<f32>() / 10.0;
        assert!(
            after > 0.3 * level,
            "the new space never faded in: {after} against {level}"
        );
    }

    /// Plan §5.2: **reset does not supersede a fit** — it clears DSP state, not the patch. An effect
    /// has no MIDI, so there is no panic to test beside it.
    #[test]
    fn reset_does_not_supersede_a_fit() {
        let mut plugin = MxmClassicVerb::default();
        plugin.prepare(48_000.0, 2);
        let generation = plugin.params.loaded.begin("room.wav");
        plugin.reset();
        assert!(plugin.params.loaded.is_current(generation));
        loading::run_with(
            &plugin.params.loaded,
            generation,
            |_| {
                Ok(decode::Decoded {
                    channels: vec![vec![0.0; 8]],
                    sample_rate: 48_000,
                })
            },
            &|_, _| Ok(testing::fitted(Space::ROOM, 0.9)),
        );
        plugin.reset();
        assert!(
            plugin.params.loaded.take_ready().is_some(),
            "a reset discarded a finished fit"
        );
    }

    #[test]
    fn host_state_with_a_loaded_space_this_build_cannot_read_is_refused_whole() {
        let mut state = PluginState {
            version: "0.1.0".to_owned(),
            params: [("mix".to_owned(), nice_plug::plugin::ParamValue::F32(0.9))]
                .into_iter()
                .collect(),
            fields: [(
                LOADED_FIELD.to_owned(),
                r#"{"version":2,"space":null}"#.to_owned(),
            )]
            .into_iter()
            .collect(),
        };
        MxmClassicVerb::filter_state(&mut state);
        assert!(state.params.is_empty());
        assert!(state.fields.is_empty());
    }

    #[test]
    fn host_state_from_before_loading_existed_restores_an_absence() {
        let mut state = PluginState {
            version: "0.1.0".to_owned(),
            params: [("mix".to_owned(), nice_plug::plugin::ParamValue::F32(0.9))]
                .into_iter()
                .collect(),
            fields: Default::default(),
        };
        MxmClassicVerb::filter_state(&mut state);
        // Its one parameter, and the tempo syncs it predates, restored Off.
        assert_eq!(state.params.len(), 1 + crate::preset::TEMPO_SYNC_IDS.len());
        let supplied = state
            .fields
            .get(LOADED_FIELD)
            .expect("an absence was supplied")
            .clone();
        assert_eq!(
            payload::parse_serialized(&supplied).expect("it reads").0,
            payload::Payload::absence()
        );
        MxmClassicVerb::filter_state(&mut state);
        assert_eq!(
            state.fields.get(LOADED_FIELD),
            Some(&supplied),
            "a readable field was changed"
        );
    }

    #[test]
    fn the_name_and_id_have_one_source() {
        assert_eq!(NAME, "mxm-classic-verb");
        assert_eq!(CLAP_ID, format!("dk.mxm.{NAME}"));
    }

    #[test]
    fn the_bundle_is_named_after_this_plugin() {
        mxm_plugin_test::bundle::is_named(env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"), NAME);
    }
}

/// **A reset re-resolves the tempo syncs**: a sync turned off while the host held the effect
/// unprocessed does not seed the reset from the previous division, and one still on stays on it.
#[cfg(test)]
mod reset_resolves_the_syncs {
    use super::*;
    use nice_plug::params::InternalParamMut;

    #[test]
    fn a_reset_re_resolves_the_tempo_syncs() {
        let mut plugin = MxmClassicVerb {
            synced: [Some(0.25), Some(8.0)],
            ..Default::default()
        };
        plugin.reset();
        let controls = plugin.controls();
        assert_eq!(controls.pre_delay_s, plugin.params.pre_delay.value());
        assert_eq!(controls.mod_rate_hz, plugin.params.mod_rate.value());
        plugin.telemetry.tempo.publish(Some(120.0));
        unsafe {
            let _ = plugin
                .params
                .pre_delay_sync
                ._internal_set_normalized_value(1.0);
        }
        plugin.reset();
        assert_eq!(
            Some(plugin.controls().pre_delay_s),
            plugin.params.synced_pre_delay(Some(120.0))
        );
    }

    /// **Reactivation forgets the old tempo**: nice-plug resets right after activating, and that
    /// reset must not resolve from the tempo the host reported before it was deactivated — the first
    /// callback's tempo is the first one used.
    #[test]
    fn reactivation_forgets_the_previous_tempo() {
        let mut plugin = MxmClassicVerb::default();
        plugin.telemetry.tempo.publish(Some(120.0));
        unsafe {
            let _ = plugin
                .params
                .pre_delay_sync
                ._internal_set_normalized_value(1.0);
        }
        plugin.prepare(48_000.0, 2);
        plugin.reset();
        assert_eq!(plugin.synced, [None; 2]);
    }
}

/// What a player reads — on hover in the editor, and in a host's plugin browser — speaks to the
/// player about the sound, never about the machine or the code (`mxm_plugin_test::hover_text`).
#[cfg(test)]
mod speaks_to_the_player {
    #[test]
    fn hover_text() {
        mxm_plugin_test::hover_text::speaks_to_the_player(env!("CARGO_MANIFEST_DIR"));
    }

    #[test]
    fn host_description() {
        mxm_plugin_test::hover_text::host_description_speaks_to_the_player(env!(
            "CARGO_MANIFEST_DIR"
        ));
    }
}
