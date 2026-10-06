# NOTES.md — plugins/mxm-classic-verb/

The detail behind this folder's AGENTS.md: history, measurements, rationale and worked examples.
AGENTS.md is the contract; this file is the reference it links to.

## Files

`Cargo.toml`, `README.md`, `control-map.json`, `presets/`, `spaces/` (the factory spaces'
committed fit reports), `examples/` (`classic_verb_space_audit`), `tests/` (`space_audit.rs` and its
shared `audit/mod.rs`) and `src/` (no `LICENSE` here since the split: the repository's root `LICENSE`
covers it):

- `lib.rs` — identity, layouts, activity, the background task, `filter_state`, and the block-start
  read of the loaded space.
- `params.rs` — parameters, `SpaceChoice::INIT` with the selector's `factory` and `sounding`, and the
  persisted `loaded` field.
- `preset.rs` — factory sounds and the durable-content seam, with §2.2's event tests.
- `spaces.rs` — the factory table's types (`FactorySpace`, `SpaceFamily`), the assertion that `Loaded`
  is one past the table, and the checks that hold the table to its reports.
- `spaces/generated.rs` — **generated** by `classic_verb_generate`: the `SpaceChoice` enum and the
  `FACTORY` table. Never edited by hand.
- `decode.rs` — WAV and AIFF decoding behind the header preflight.
- `fitting.rs` — **the one module in `src/` that names `mxm-classic-verb-fit`**; the space audit's
  `tests/audit/mod.rs` names its analyser too.
- `loaded.rs` — the held space, the audio thread's sequence lock, the commit barrier and fit
  generations.
- `loading.rs` — refusals, the background job, landing in the commit order, and the preset surface's
  gesture watch.
- `payload.rs` — the versioned space-or-absence payload and the fit report's summary.
- `telemetry.rs`; `editor.rs`, and `editor/` — `binding.rs` and `sections.rs`.
- `testing.rs` — test-only: planted responses, WAV and AIFF writers, an applying host.

## Identity

### Two tempo syncs

**Two tempo syncs** (2026-09-25, `plans/plan-tempo-sync-controls.md`), each the collection's quarter
note beside its knob: `predelaysync` on `params::PRE_DELAY_SYNC` (1/64 to a quarter note, the top the
longest) and `modsync` on `params::MOD_SYNC` (every LFO's ladder, the top the fastest). `process`
resolves both once a block from the modulated positions into `MxmClassicVerb::synced`, which
`controls` hands the engine in place of the free values. The decay display keeps the free values
(`sections::controls`): neither moves the decay it draws. A loaded space still writes Pre-delay's free
value; synced, the division is what sounds. `Telemetry::tempo` lets the knobs read their divisions.

### Nothing is released

**Nothing is released.** The factory positions are the hundred spaces fitted from the owner's impulse
responses under the names the owner approved, and Init is the owner's choice: `vocal-plate` at Mix
11.7 % (plan revisions 19 and 21). **After release every id above, the selector's position list and
the `loaded` field's name are permanent**: a preset stores the selector's normalised value, so
inserting or appending a position moves every stored selection.

## Parameters

**No parameter carries a smoother.** The DSP smooths every control itself, so the shell hands it
plain `value()`s once per block, and only when a value changed. An unchanged setting is never sent
again, which is why a constant setting renders bit-identically at any host block size —
`a_constant_setting_renders_identically_whatever_the_block_size`.

**Changing the space never moves a control** (plan §2): `space` maps to a `Space` and nothing else.
`loaded` sounds the loaded space, and **the Init space** — `SpaceChoice::INIT`, the position the
defaults select — while nothing is loaded (`loaded_plays_the_init_space_until_a_space_is_loaded`,
`loaded_with_nothing_loaded_plays_the_init_space`). A fit landing writes several parameters, and that
is a player's drop, not a parameter writing another.

### Parameter text

**Parameter text is idempotent through the host's normalized conversion**: formatted with the unit,
parsed, normalized and formatted again, every parameter reads the same string.
`every_parameter_text_is_idempotent_through_the_hosts_conversion` holds it over `param_map` at the
twenty-step grid, `clap-validator` 0.4.1's own grid, both sides of every formatter branch point and a
hair either side of zero. Two rules follow from what it found:

- **A reading never prints a negative zero.** Width's range crosses zero, so `percent` printed
  `-0 %` a hair below it, which parses to zero and prints `0 %`; it prints `0 %` there now. The
  validator's grid never landed in that sliver.
- **A formatter chooses its branch from the rounding its finer branch prints.** `milliseconds` and
  `decibels` do. `seconds` does not — it picks hundredths or tenths from the rounded *tenth* — and is
  left as it is because the defect does not reproduce: the one `f32` under 9.95 that would print
  `9.9 s` is not a value Decay's normalized conversion can produce, which a sweep of every
  normalized neighbour confirmed. The test probes it, so a change to Decay's range or skew that made
  it reachable fails there.

## Presets are provisional

Twelve factory sounds, designed in `preset.rs` in the parameters' own units and generated by the
`#[ignore]`d `write_the_factory_presets`. `the_factory_files_match_the_design` compares the shipped
files with the design. Every sound is categorised and complete, none is Init under another name, and
every pair differs on at least three axes. Init is generated from the defaults
(`the_init_preset_is_the_parameter_defaults`). Declaration order — the selector first — is the write
order, so a recalled preset's controls land on the space they were designed against. Every factory
file is `state = null`, so it preserves the loaded space (*Loading*, below). **Each design names its space
by generated variant** (`SpaceChoice::VocalPlate as usize`), so a regenerated selector that renames the
space fails to compile and one that moves it carries the design along; a design's normalised value
still follows the position count, so any regeneration reruns `write_the_factory_presets`. **The twelve
were designed on the hand-authored spaces** and now select the fitted space nearest each one's intent —
Natural room, Echo chamber, Medium hall, Vocal plate, Warm concert hall, Glittering hall, Big drum room,
Lively lounge, Satin plate and Clean air. Redesigning them by ear on the hundred is open.

## Factory spaces are generated

`src/spaces/generated.rs` and `spaces/<id>.json` are written by the fit crate's `classic_verb_generate`
from a manifest that lives outside the repository (`crates/mxm-classic-verb-fit/AGENTS.md`, *The
factory-space generator*), **never by hand**.

- **The selector is the generated enum.** `SpaceChoice` is every manifest row's variant in manifest
  order, then `Loaded`; `params.rs` re-exports it and keeps `INIT`, `factory` and `sounding`. `FACTORY`
  holds each position's id, display name, family, fitted `Space` and fitted Decay, Size, Diffusion and
  Pre-delay. `factory()` reads the space from it and the editor lists the same rows' names
  (`the_selector_is_the_factory_table_then_loaded`); a compile-time assertion holds `Loaded` one past
  the table. `Loaded` behaves exactly as *Loading* describes.
- **Each position's fit report is `spaces/<id>.json`**: the response's and the verification's
  descriptors, the errors and the recipe the verification was rendered with. It names its source by the
  id alone and carries no path separator, and no report outlives its position
  (`every_factory_space_has_its_report_and_no_report_is_orphaned`).
- **The fitted controls are data nothing writes yet**: no preset is generated per space, and Init does
  not read them.
- **Nothing on the audio thread changed**: `factory()` indexes a constant table, and `synchronise` still
  compares spaces by value once a block.

**The positions are the owner's hundred** (plan revision 21): fitted from the owner's impulse responses
by the four-band fitter, named from a drafted proposal the owner approved unchanged, and ordered by
family, then by decay. **The manifest names the response files, so it lives outside the repository with
them**; regenerating needs both, and the committed reports are the record the plugin keeps. Every
position with a decay under 6.7 s fits at Size 56.3 ms, the density floor; the longest experimental
spaces fit up to 224 ms (the fit crate's *Size is the density floor*). `classic_verb_synthetic_responses`
still proves the generator without anybody's responses (the fit crate's *The factory-space generator*).
Regenerating from the owner's manifest reproduces the committed files byte for byte (checked 2026-09-15:
all 101 files checksum-identical before and after a second run).

### The final manifest run

**The final manifest run, measured** (release, Windows, 2026-09-15): 65.8 s for the hundred, 4.3 MB of
reports and a 3,634-line, 133 KB module. What it changed beside the table:

1. **Init.** `SpaceChoice::INIT` is `VocalPlate`, and Mix's default 0.117 — the owner's choice and its
   measurement (plan revision 19). A manifest without `vocal-plate` fails to compile in `params.rs`.
2. **The presets**, remapped by variant, with `write_the_factory_presets` rerun (*Presets are
   provisional*).
3. **One editor test named the Init space**: `both_themes_paint_every_card_and_control_inside_the_reference_window`
   now reads its name from the table. Nothing else names a position.
4. **The control map needs nothing.** `fx.verb_space` is a stepped role; the Player takes the step count
   from the plugin at run time, and neither `control-map.json` nor a Player test counts positions. A
   7-bit controller's 128 values still reach each of 101 steps.
5. **The space audit's tolerances were re-measured on the hundred** (*The space audit*).

**At a hundred positions, measured** on the owner's hundred (Windows, 2026-09-15; release unless named):
the release bundle is 9,424,384 bytes; `clap-validator` passed 33 and failed 0 on the debug and the
release bundle; the audit took 1.1 s, and `tests/space_audit.rs` ran in 8.3 s in debug;
`cargo clippy --all-targets -- -D warnings` found nothing in the generated module; the
`effect_chain` host test (`cargo test -p mxm-classic-verb-host-tests`), the Player's `t5_control_map`
and control-map tests pass. A nice-plug `Enum` of 101 variants
needed nothing: the derive writes two arrays and one match each way. The generator's own figures are the
fit crate's (*The factory-space generator*).

## The space audit

`classic_verb_space_audit` (plan §5.1, §9.1) renders every factory space **through the plugin** — its own
selector position, its fitted controls, every relative control neutral, modulation depth zero, the decay
natural, ducking off and Mix one — exactly as its fit's verification was rendered (the report's `render`
recipe), analyses the render with the fit crate's analyser as the fit did — `analyse_with_segments`, its
tail textures on the segments the report recorded, which are the response's — and compares it with the
verification descriptors the report recorded. **It exits non-zero on any miss, naming the space and the descriptor.**
`tests/space_audit.rs` runs the same audit by default; the code is `tests/audit/mod.rs`. It also refuses
a report whose source is not its id, whose space or controls are not the table's to the bit, which
carries a verification refusal or lacks its recipe, and a file in `spaces/` no position owns.

**The recorded render is the target, not the response's descriptors.** No fit reproduces its response
exactly, and its errors are in the report; what a fitted space can be held to is what it measured when
it was generated. A render within tolerance of that is within the committed error plus the tolerance of
the response's own descriptors.

### The tolerances, measured

**The tolerances, measured** with `measure_what_small_changes_move` (release, Windows, the hundred
factory spaces). *As generated* is what rendering through the plugin moves: through `Loaded` it
measures the same, so none of it is the selector's path — *derived*, not measured separately: the four
fitted controls' normalised round trip. Each tolerance sits above that and under the smallest change
that moves the descriptor:

| Descriptor | Tolerance | As generated, worst of the hundred | A small change, and how far it moves the descriptor on tight-live-room |
|---|---|---|---|
| T30 per band, T20 where neither side has one | 0.3 % | 0.013 % (frozen-drone) | the top decay ratio ×1.01: 0.45 %; Decay ×1.01: 0.90 % |
| Level against 1 kHz per band | 0.02 dB | 0.0028 dB | the high shelf +0.1 dB: 0.064 dB |
| DRR | 0.02 dB | 0.0005 dB | the high shelf +0.1 dB: 0.062 dB |
| Pre-delay | 0.05 ms | 0 | Pre-delay +0.1 ms: 0.091 ms |
| Mixing time | 3 ms | 0 — a first passage, which jumps | Size ×1.01: 6 ms |
| IACC | 0.001 | 0.0001 | Width ×0.99: 0.0015 |
| Echo density profile, RMS | 0.005 | 0.0021 (frozen-drone) | Diffusion −0.01: 0.0087; Size ×1.01: 0.033 |
| Profile points measured on one side only | 2 | 0 | Pre-delay +0.1 ms: 3 |
| The strongest 12 reflections: delay; level | 0.05 ms; 0.05 dB | 0; 0.0026 dB | every tap 0.01 Size later: 0.57 ms; the early level ×1.02: 0.17 dB |
| A reflection missing within 1 ms | none | none | none moved one out on this space |
| Confidence | 0.25 | 0.228 (bass-bloom) | the low shelf +0.1 dB: 0.014; on any space, at most 0.057 |
| Tail texture, either segment: spectral peakiness | 0.003 | 0.0023 (frozen-drone) | Diffusion −0.01: 0.0046; Size ×1.01: 0.012 |
| Tail texture: kurtosis | 0.005 | 0.0037 (tight-live-room) | Decay ×1.01: 0.064 |
| Tail texture: echo density | 0.001 | 0.00046 (glass-plate) | Diffusion −0.01: 0.011 |
| Tail texture: periodicity | 0.001 | 0 | Diffusion −0.01: 0.0024; Size ×1.01: 0.046 |
| A segment's texture read on one side only | none | none | nothing measured |

Every row was re-measured on 2026-09-15 on the owner's hundred, as generated on all of them and each
small change on tight-live-room, the space the moved-space test renders. **Five tolerances moved with
it.** IACC down to 0.001, so Width ×0.99 is named on that space. Confidence up to 0.25: rendered through
the plugin, bass-bloom's 8 kHz noise margin crosses the confidence ramp and its confidence jumps 0.228,
so only a collapse is caught; a small change that moves confidence also moves a descriptor that names
it. Peakiness to 0.003, kurtosis to 0.005 and echo density to 0.001, each above its worst as generated
with room for another platform. T30 stays at 0.3 %. **A change a space cannot express moves nothing** —
a shelf already at ±24 dB, no early taps, zero width or Diffusion, a ratio at its bound — and among the
hundred the high decay ratio ×1.01 goes unnamed on six spaces and the top ratio on eight, those at a
bound among them. The texture is read on the segments the report recorded, so a render cannot move
where it is read.

Every change is a small fraction of the scales the Space card ranks errors by (5 % of a decay time,
1 dB, 10 ms, 0.075 of IACC), and `a_moved_space_is_caught_and_the_descriptor_named` holds five to being
named: Decay +1 %, Size +1 %, the high shelf +0.1 dB, Pre-delay +0.1 ms and Width −1 %. **These floors
are Windows figures**: another platform's maths library may move the *as generated* column, and Linux
and macOS are not measured.

**Cost.** The hundred factory spaces: 1.1 s in release, and the default test file 8.3 s in debug. The
audit renders each space once, on as many threads as there are cores.

## Loading a space from an impulse response

Plan §5.2, built at P6. **A WAV or AIFF dropped on the Space card is checked, decoded and fitted on
the background task, and lands through the sampler's commit order.** Nothing is stored of the
response but the space fitted to it; no path enters state or presets.

### The path, and what refuses where

1. **Drop.** Native drag-and-drop through the already-patched `egui-baseview`; no Browse, because the
   brief asks for none. A drop anywhere on the Space card's controls takes a new fit generation
   (`LoadedField::begin`) and schedules `LoadTask::Fit`; it writes no gesture
   (`a_drop_on_the_space_card_starts_a_fit_and_writes_no_gesture`). Measurement passes take no drop.
2. **Extension**: `wav`, `wave`, `aif`, `aiff`, `aifc`, any case; anything else is refused by name.
3. **Header preflight** (plan §4.5 stage 1), before anything proportional to the file is allocated:
   `fitting::preflight`, the fit crate's own bounds — one or two channels, 22.05–192 kHz, 0.3–30 s
   (`the_header_is_judged_before_any_sample_is_read`, `the_header_stage_is_the_fit_crates_own_preflight`).
4. **Decode** (stage 2), every sample checked finite, stopping at the first that is not
   (`decoding_stops_at_the_first_non_finite_sample`). **Accepted:** WAV 8–32-bit integer PCM and
   32-bit float, through `hound`; AIFF 8–32-bit PCM; AIFF-C `NONE`, `twos`, `sowt`, `fl32` and `fl64`
   (`every_accepted_encoding_decodes_to_the_planted_samples`). **Refused by name:** a float WAV that is
   not 32-bit, any other WAV encoding `hound` does not read, compressed AIFF-C (`ulaw`, `alaw`,
   `ima4`…), a file cut off in its header or its audio, a missing COMM or SSND chunk, an unreadable
   file (`every_decoder_failure_is_a_named_refusal`). No prefix of a valid file panics the decoder
   (`no_prefix_of_a_valid_file_panics_the_decoder`). The decoder is a port of the logic in the fit
   crate's `examples/classic_verb_io`; the plugin does not depend on example code.
   **Every buffer the file sizes is reserved fallibly**, through `decode::reserve`: each channel once,
   up front, for the samples the header promises, and room made before each run is decoded, so no
   `push` grows a buffer. The preflight still admits 46 MB of stereo, and on this thread an ordinary
   allocation that fails aborts the host. Memory that cannot be had is `Refusal::OutOfMemory`, shown
   as *not enough memory for this response (N MB could not be reserved)*
   (`a_reservation_that_cannot_be_made_is_refused_by_name`,
   `every_channel_is_reserved_once_for_what_the_file_holds`). Never `vec![Vec::with_capacity(n); k]`:
   a clone keeps the contents, not the capacity, so all but one channel grow. nice-plug installs a
   global allocator of its own, so no test here can refuse a real allocation; the seam's test hook
   stands in.
5. **Fit**: `mxm_classic_verb_fit::fit` — the analyser's refusals (silence, no identifiable onset, no
   decay above the noise floor in a required band), then **confidence under `CONFIDENCE_FLOOR`
   (0.25), refused with the weakest check named**, and checked again in `fitting::judged`
   (`a_low_confidence_analysis_names_its_weakest_check`, `a_fit_under_the_floor_is_refused_here_too`,
   `digital_silence_is_refused_by_the_analysis_and_named`). The fit crate's `FitRefusal::OutOfMemory`,
   from whichever of its stages ran out, is the same `Refusal::OutOfMemory`
   (`a_fit_that_runs_out_of_memory_is_refused_as_that`). It runs on nice-plug's background thread,
   never in `process()` and never on the editor's thread. **The fit crate measures 0.47–0.56 s**
   for a planted 2–6 s stereo room at 44.1 kHz and 1.14–1.15 s at 96 kHz in release, and a median 0.54 s
   per response over its pool (its *Measured accuracy — the fit*); nothing here measured it again,
   and the busy line needs no progress.
6. **Land**, in the editor's frame, before anything is drawn (`loading::settle`). A refusal names its
   reason on the Space card and writes nothing; the current space keeps sounding
   (`a_refusal_is_named_and_writes_nothing`,
   `a_fit_under_the_confidence_floor_leaves_the_current_space_and_writes_nothing`, and for memory
   refused while decoding or fitting, `a_refused_import_leaves_the_space_that_was_sounding`).

### The commit order, and a fit's parameter contract

**The fitted space is staged; the selector moves to `Loaded` and the fit's controls are written as
bracketed gestures; then the space commits**, and the next block hears it
(`a_fit_lands_staged_then_as_bracketed_gestures_then_committed`). The whole path runs on the real
fitter against a planted response in `a_dropped_wav_is_fitted_by_the_real_fitter_and_lands`.

**The gestures are plan §2's whole fitted centre**, which is also
`mxm_classic_verb_fit::FittedControls::apply`: Decay, Size, Diffusion and Pre-delay to the fitted
values; Bass and Treble decay to ×1, Early and late to 0 dB, Width to 100 %, both tones to 0 dB,
Mod depth to 0 and Decay shape to Natural. **Mix, Ducking and Mod rate are never written.** The
selector and the four fitted controls are always written; the rest **only where the patch is not
already at the centre**, so an untouched patch sees exactly five gestures and a host's undo list
carries no edits that changed nothing
(`a_load_returns_the_patch_to_the_fitted_centre_and_leaves_the_mix_alone`).

### The audio thread's side: a sequence lock, and the process boundary

`loaded.rs`. Off the audio thread, one mutex holds the payload, a staged space, the status line and a
finished fit. **The audio thread reads a sequence lock over the committed space's 45 words, and
nothing else**: a space is copied by value, so there is nothing to allocate, retire or reclaim — the
sampler's reader-counted slots exist because its snapshots are heap allocations. A reader that meets
a write in progress keeps the space it has and reads again next block; it never waits
(`a_write_in_progress_is_never_read`, `a_write_in_progress_is_left_for_the_next_block`,
`concurrent_readers_only_see_whole_spaces`). `process` reads it once at the start of a block, so
**the barrier is the process boundary**: a staged space is not heard, and a committed one is heard
from the next block (`a_staged_space_is_not_committed_until_it_is_asked_for`,
`a_committed_space_reaches_the_engine_at_the_next_block_and_a_staged_one_does_not`).

**A replacement while `Loaded` sounds is the engine's fade** (plan §3.4): the wet goes through
silence, swaps and comes back (`replacing_the_loaded_space_while_it_sounds_is_the_engines_fade`).

**One gap the barrier cannot close, derived and not measured.** nice-plug's CLAP wrapper queues an
editor's parameter writes and applies them when that processing cycle's output events are written,
so a commit made just after the gestures can be seen one block before the values it goes with. It is
not heard as such: a space change fades out for `SPACE_FADE_S` (30 ms) before the new space is
swapped in, which is longer than any ordinary block, and a move from a factory position to `Loaded`
changes nothing until the selector's own value arrives. The sampler's barrier has the same exposure.

### Fit generations, and every race

Every drop takes a new generation. **A newer drop, a preset or bank recall, Init, a host state
restore and the editor closing each supersede the running fit**; a superseded result is discarded
without staging, gestures or a message. A result is kept only if its generation is newest when it
completes, taken only if newest when the editor looks, and committed only if newest after its
gestures. **Reset does not supersede**: it clears DSP state, not the patch. An effect has no MIDI,
so there is no panic to test.

| Race or rule | Test |
|---|---|
| The editor closing during a fit | `closing_the_editor_supersedes_a_running_fit` (through `NiceEguiApp::editor_closed`) |
| A second drop | `a_second_drop_supersedes_the_first` |
| A recall during a fit | `a_recall_during_a_fit_supersedes_it` |
| Init during a fit | `init_during_a_fit_supersedes_it` |
| A host state restore during a fit | `a_host_state_restore_during_a_fit_supersedes_it` |
| A completion at the same boundary as a supersession: while fitting, after completing, and between the gestures and the commit | `a_completion_at_the_boundary_of_a_supersession_is_discarded` |
| A superseded job stops at its cancellation points: before decoding, inside it, before the fit | `a_superseded_job_stops_before_it_decodes_or_fits`, `a_superseded_decode_stops` |
| Reset does not supersede | `reset_does_not_supersede_a_fit` |
| Save, Rename, Delete and favourites do not supersede | `a_preset_action_without_a_gesture_does_not_supersede` |
| The bookkeeping | `a_generation_decides_whether_a_result_may_land` |

The races are stepped by hand: the fitter is injected (`loading::Fitter`), and a host hook
supersedes at an exact gesture. Nothing sleeps.

**The fit crate has no cancellation point inside `fit`**, so a job superseded while fitting runs to
its end and is discarded on completion.

**Init has no hook in the shared preset seam**, so the preset row and its overlays are drawn through
`loading::GestureWatch`, a setter that notices gestures: every gesture the preset surface writes is a
recall or Init, and any one supersedes. A recall also supersedes in `apply_preset_state`, before its
gestures.

**Where supersession is decided, and the one case the plugin cannot make atomic.** A result taken by
the editor and then superseded before its commit is not committed. That can happen only when a host
restores state on a thread other than the editor's, and then the landing's gestures have already
reached the host while the restored space stands. Which hosts do that is not established here; the
editor's own supersessions — a drop, a recall, Init — run on its thread and cannot.

### The payload, and §2.2's events

`payload.rs`. **Version 1**, JSON, carried in a user preset's `state` and the host's `loaded` field
alike:

```json
{ "version": 1, "space": null }
{ "version": 1,
  "space": { "form": 1, "early": [[0.18, 0.82, 0.55], "… twelve taps"], "early_level": 0.7,
             "decay_ratio_low": 1.1, "decay_ratio_high": 0.77, "decay_ratio_top": 0.6,
             "tone_low_db": 0.0, "tone_high_db": -3.0, "width": 0.8, "high_cut_hz": 20000.0 },
  "report": { "confidence": 0.82, "errors": [{ "what": "decay", "hz": 1000.0, "value": 3.1 }],
              "clamps": [{ "what": "size", "fitted": 0.41, "applied": 0.3 }] } }
```

`form` is `mxm_classic_verb_dsp::SPACE_VERSION`. **A payload version or space form this build does not
know is refused whole**; a space's values are bounded by the DSP crate's `Space::sanitised` on the way
in (`an_unknown_version_is_refused_whole`, `a_hostile_space_is_bounded_on_the_way_in`). The report is
kept so the Space card can say how good a held space is after a reload; an absence carries none. A
full payload is under 4 KiB (`the_payload_is_kilobytes`). The fingerprint is FNV-1a over the canonical
JSON, and **absence has one of its own** (`absence_is_explicit_and_has_a_fingerprint_of_its_own`).

**`high_cut_hz` and `decay_ratio_top` joined the space before anything was released**: the first when
P3.5 found that the tone's shelves cannot follow real responses' high-frequency rolloff (plan revision
13), the second with plan D5's fourth decay band (revision 20), which also moved `decay_ratio_high` from
the 8 kHz octave to the 4 kHz one. No payload existed, so neither moved `PAYLOAD_VERSION` or
`SPACE_VERSION`. The sequence lock carries the space in 45 words. **After release, a field like this
moves a version.**

| §2.2 event | What happens | Test |
|---|---|---|
| A response is loaded | Staged, gestures, committed; a replacement fades | *The commit order*, above |
| `Loaded` with nothing loaded | The Init space sounds; the display says **Nothing loaded** | `loaded_with_nothing_loaded_plays_the_init_space`, `off_and_nothing_loaded_are_painted_as_words` |
| A factory preset | `state = null` preserves the loaded space | `a_factory_preset_preserves_the_loaded_space` |
| A user preset saved | Always a payload: the held space whichever position is selected, or an explicit absence | `a_user_preset_always_carries_a_payload` |
| A user preset recalled | Applied before any gesture, committed after the last; an absence clears the loaded space | `a_user_preset_recall_applies_before_any_gesture_and_commits_after_the_last`, `a_recalled_absence_clears_a_stale_loaded_space` |
| Init | Parameter defaults only; the loaded space survives, unselected | `init_writes_defaults_and_the_loaded_space_survives_it_unselected` |
| Host state | The same payload through the `loaded` field; its fingerprint joins dirty comparison | `host_state_persists_the_payload_and_its_fingerprint_joins_the_dirty_comparison` |
| A version this build does not know | Refused whole, before any gesture, and a running fit is not cancelled | `a_payload_version_this_build_does_not_know_is_refused_whole_before_any_gesture`, `host_state_with_a_loaded_space_this_build_cannot_read_is_refused_whole` |

**Host state.** nice-plug writes every parameter before handing over the fields, so a restore arrives
in the barrier's order and publishes in one step, superseding a running fit first.
`Plugin::filter_state` cannot reject state in nice-plug 0.3, so a `loaded` field this build cannot
read makes the whole state a no-op — parameters and fields cleared — which is the sampler's answer to
the same limit. **State from before P6 has no field and is given an explicit absence**, so a restore
always settles the loaded space (`host_state_from_before_loading_existed_restores_an_absence`).

### Refusals and confidence on the Space card

Plan §4.5 and the brief. The load line is one fixed-height line under the selector:

| State | Line | Ink |
|---|---|---|
| Nothing held | *Drop a WAV or AIFF impulse response here* | secondary |
| A file held over the card | *Release to fit a space to this file*, the selection fill and a 2-point accent border | accent |
| Fitting | *Fitting a space to ‹file›…* | primary |
| Refused | *Refused: ‹reason›*, the reason in full on hover | danger |
| A space held | the fit report's summary: *Confidence 0.82 · largest error 125 Hz tone -1.5 dB*, the full report on hover | primary |
| Held, confidence under `LOW_CONFIDENCE` (0.5, chosen) | *Low confidence 0.31 · …* | warning |

Every state has words, so none is carried by hue alone (`the_load_line_says_what_loading_is_doing`,
`the_space_card_says_what_loading_did`, `a_file_held_over_the_space_card_lights_its_drop_target`).
A file's name, a refusal or a report is elided to the line, never extending it; the two fixed
lines, *Drop a WAV or AIFF impulse response here* and *Release to fit a space to this file*, are
never elided — the load line is at least as wide as they are (`load_line_min_width`). **The largest error is judged against a scale
per descriptor**, chosen on the scale of the just-noticeable differences commonly quoted from
ISO 3382-1 — 5 % of a decay time, 1 dB of a level or energy ratio, 10 ms of a time — and otherwise
from the fit crate's own matching: 0.075 of IACC, 1 ms of reflection timing, 3 dB of reflection
level. None is a pass mark; they only choose which error the line names
(`the_summary_names_confidence_and_the_largest_error_against_its_scale`). A descriptor a later build
reports is shown and never ranked (`a_later_builds_descriptor_is_shown_and_never_ranked`).

### The fit crate's API stays in one file

`fitting.rs` is the only module that names `mxm-classic-verb-fit`; everything else sees
`loading::{Fitted, FittedControls, Refusal}` and `payload::Report`. **Written against the fit crate
at `fae3577`, and merged onto its P3 delivery and the high cut**; the only change the merge needed
was `FitValue::HighCut`, which maps to `Clamped::HighCut` (*the high cut*). The P3 additions not yet
taken (`envelope_db`, `verification_refusal`, and the tail texture's `texture_a` and
`texture_b` errors), and what taking each would be, are listed at the top of that file.

## Control map

Mix fills the existing `fx.reverb` role. Page 13, Effects / Classic verb, maps the seven performed
controls in the brief's slot order: Space (`fx.verb_space`, stepped), Decay (`fx.verb_decay`, log),
Size (`fx.verb_size`, log), Pre-delay (`fx.verb_predelay`, log), Bass decay (`fx.verb_bass`,
bipolar), Treble decay (`fx.verb_treble`, bipolar) and Decay shape (`fx.verb_shape`, stepped). The
eighth slot is empty on purpose. The multipliers are bipolar because ×1 is the middle of their
travel and the curve's centre detent is how a controller knob returns one to the space as fitted.

Diffusion, Mod depth, Mod rate, Width, Low tone, High tone, Early and late and Ducking are shape
controls: unmapped, automatable and on the panel. The roles entered mxm-kit's `docs/MXM_CONTROL_MAP.md`
with this plugin, so the map claims them.
`host-tests/tests/effect_chain.rs::control_map::the_classic_verb_map_claims_its_new_page_and_reuses_the_existing_reverb_role`
holds the roles, the slot order and the unmapped set; `cargo xtask bundle` stages the file beside
both debug and release bundles.

## Editor

Built to the approved brief. Four Effects cards through `mxm_ui::paging::editor`, in signal-flow
order: **Space** (the Space selector, the load line and drop target, Pre-delay, Size, Early and
late), **Decay** (the decay display, Decay, Bass decay, Treble decay, Decay shape — painted *Bass*,
*Treble* and *Shape*, the card saying whose, with the shape's three options drawn as the envelopes
they put on the tail: `Wave::Decay`, `Wave::Gated`, `Wave::Swell`), **Texture**
(Diffusion, Mod depth, Mod rate) and **Output** (Mix and Width; Low tone, High tone and Ducking). No
view bar; the app bar carries the preset row, the peak and clip meter, zoom and theme. Every
parameter is bound once, in `editor/sections.rs::all_parameters`, with a one-sentence tooltip; a
control's gestures are bracketed in `editor/binding.rs`, a fit's in `loading.rs`. Nothing on the audio
thread reads editor state; telemetry stays atomics. An effect has no developer channel.

- **Every card is a `mxm_ui::tree`, and its floor is computed** (mxm-kit's `crates/ui/AGENTS.md`, *A card
  body as data*; now its [`crates/ui/NOTES.md`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/ui/NOTES.md#a-card-body-as-data--tree)). `sections::card` describes each body once — the Space selector filling its row,
  the load line, the decay display, the collection's knob rows (`tree::knob_row`), the Decay shape pictures — and
  that description is measured for the card's floor and height and drawn leaf by leaf through the
  bindings (`sections::paint`), through `paging::editor::show`. Each floor is the tree's
  narrowest plus the card's chrome, with no usability minimum declared beside it: the row of three
  knobs, each holding its widest reading (`control::knob_size`), sets it, and the Space selector
  and the decay display (`DISPLAY_MIN_WIDTH`) fit under it; on Space the load line's fixed words
  (`load_line_min_width`) set it.
  The displays state their heights: the load line `MIN_TARGET`, the decay display
  `TALL_PLOT_HEIGHT`. `every_card_passes_the_tree_checks_in_every_state` runs the shared
  checks (`mxm_plugin_test::tree_checks`) over the load line's every state, the longest space name,
  Off with Loaded holding nothing, every control at its top and bottom, and gated and reversed
  decays. Each card is as wide as its floor. **The opening size** (`REFERENCE`) is the quarter-4K
  budget hugged, all four cards on one row (`the_opening_size_is_the_budget_hugged`,
  `the_opening_page_is_one_row_in_signal_flow_order`). **The minimum** (`MINIMUM`) is the widest
  floor plus gutters, and tall enough that the Decay card, the tallest, paged alone under the page
  bar that width produces, does not scroll (`the_minimum_holds_the_widest_card`,
  `the_minimum_window_shows_each_card_without_scrolling`).
- **The Space selector is the collection's caret selector, not a segmented switch.** Its options
  are `SpaceChoice::variants()`, so the P3.5 list of about a hundred positions needs no editor
  change, and the arrows, Home and End step it at any length
  (`the_space_list_is_every_position_the_parameter_has_in_order`). The shared menu scrolls inside
  the window, opens on the selection, and searches past `mxm_ui::control::SEARCH_ABOVE` options
  (mxm-kit's `crates/ui/AGENTS.md`).
- **Relative controls are drawn bipolar.** Each has its neutral at normalised 0.5 and as its
  default, so the shared knob's centre detent marks the space as fitted and a double-click returns
  there as one bracketed gesture
  (`every_relative_control_has_its_neutral_on_the_detent_and_as_its_default`,
  `a_double_click_returns_every_relative_control_to_its_neutral_as_one_gesture`). A range change
  that moves a neutral off the centre fails both.
- **The decay display** reserves a `TALL_PLOT_HEIGHT` canvas at rest. A natural decay is three
  lines, at 125 Hz, 1 kHz and 8 kHz, from `predicted_decay_s` on **the space the selector
  sounds** — the loaded space under `Loaded` when one is held, the Init space when not — and the
  parameters' controls at 48 kHz, built exactly as `MxmClassicVerb::controls` builds them
  (`the_display_is_given_the_engines_controls`,
  `the_display_draws_the_loaded_space_when_loaded_is_selected`). The axis steps through 0.5, 1, 2,
  5, 10, 20 and 60 s, taking the smallest step that holds the longest line with 20 % room, so it
  moves only when a line crosses a step. A gated or reverse decay draws its envelope over its length
  on a fixed axis at `SHAPE_MAX_S`. Brightness is `Telemetry::take_wet`, read once per frame before
  paging, so a parked effect goes dark. **Off** and **Nothing loaded** are painted as words, Nothing
  loaded only while `Loaded` holds no space (`off_and_nothing_loaded_are_painted_as_words`), and the
  canvas publishes an accessibility label carrying the band times and both states.
- **The load line** is *Loading*'s table above: the drop target's visible face, one line, with an
  accessibility label carrying its words. **The drop target is the Space card's body** — the union
  of the rectangles its tree gave its leaves this frame, remembered for the next frame's
  held-file test — and a drop is taken after the cards are drawn, only when the Space card was on
  the page. Whether a file is held over it is read once, before the frame, and handed to the load
  line's painting.

The headless tests prove keyboard coverage (`Coverage::Exactly`), geometry and painting in both
themes, gestures, a synthetic drop and hover, and the display's arithmetic, at logical sizes. They do
not prove native windows, a native drag from the file manager, physical DPI or zoom.
