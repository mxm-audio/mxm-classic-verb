# AGENTS.md — plugins/mxm-classic-verb/

Parent: [`../AGENTS.md`](../AGENTS.md). The measurements, rationale, tables of tests and history
behind these rules are in [NOTES.md](NOTES.md).

# Purpose

The nice-plug shell for **mxm-classic-verb**, an everyday algorithmic reverb whose spaces can be
fitted from impulse responses. Identity, parameters, layouts, activity and tail, presets, loading a
space from a dropped impulse response, the editor and the controller map. The DSP is
[`crates/mxm-classic-verb-dsp`](../../crates/mxm-classic-verb-dsp/AGENTS.md); the fitter is
[`crates/mxm-classic-verb-fit`](../../crates/mxm-classic-verb-fit/AGENTS.md); the plan is
`plans/plan-mxm-classic-verb.md` in the private archive; the brief is
[`docs/briefs/mxm-classic-verb.md`](../../docs/briefs/mxm-classic-verb.md). Shared conventions — nice-plug's API, the preset rules, `process()` realtime rules, the editor
contract — live in the parent and are not restated here.

# Ownership

`Cargo.toml`, `README.md`, `control-map.json`, `presets/`, `spaces/` (the factory spaces'
committed fit reports), `examples/` (`classic_verb_space_audit`), `tests/` (`space_audit.rs` and its
shared `audit/mod.rs`) and `src/`; what each source file holds: [NOTES.md § Files](NOTES.md#files).
`src/spaces/generated.rs` is **generated** and never edited by hand; `src/testing.rs` is test-only.
No per-plugin `LICENSE` since the split (2026-10-06): the repository's root `LICENSE` covers it.

# Local Contracts

## Identity

| What | Value |
|---|---|
| CLAP id | `dk.mxm.mxm-classic-verb`, assembled from `plugin_name!` |
| Display name | `mxm-classic-verb` |
| Parameter ids | `space`, `mix`, `decay`, `bass`, `treble`, `size`, `diffusion`, `predelay`, `predelaysync`, `earlylate`, `shape`, `moddepth`, `modrate`, `modsync`, `width`, `lowtone`, `hightone`, `duck` |
| Space position ids | The hundred ids of `src/spaces/generated.rs`, in the owner's approved order from `tight-live-room` to `ice-field`, then `loaded` (*Factory spaces are generated*) |
| Shape ids | `natural`, `gated`, `reverse` |
| Persistent fields | `preset` (the loaded preset's identity), `loaded` (the loaded space, payload version 1) |

`the_bundle_is_named_after_this_plugin` holds `bundler.toml` to the same name.

- **Nothing is released.** Init is the owner's choice (`SpaceChoice::INIT`). **After release every id
  above, the selector's position list and the `loaded` field's name are permanent**: a preset stores
  the selector's normalised value, so inserting or appending a position moves every stored selection
  ([NOTES.md § Nothing is released](NOTES.md#nothing-is-released)).
- **Two tempo syncs** (`predelaysync`, `modsync`) resolve once a block into `MxmClassicVerb::synced`,
  which the engine gets in place of the free values; the decay display and a loaded space keep the
  free values ([NOTES.md § Two tempo syncs](NOTES.md#two-tempo-syncs)).

## Parameters are the plan's two kinds

Each control is either **absolute** — Mix, Decay, Size, Diffusion, Pre-delay, Mod depth, Mod rate,
Ducking — or **relative to the space** with a neutral point that is the space as fitted: Bass and
Treble decay (×1), Early and late (0 dB), Width (100 %), Low and High tone (0 dB). The composition
laws are the DSP crate's. The multipliers' ranges are skewed so ×1 sits at the middle of the travel,
and `the_multipliers_put_unity_at_the_middle_of_their_travel` holds it.

- **No parameter carries a smoother:** the DSP smooths every control, so the shell sends plain
  `value()`s once per block, only when one changed
  (`a_constant_setting_renders_identically_whatever_the_block_size`).
- **Parameter text is idempotent through the host's normalized conversion**
  (`every_parameter_text_is_idempotent_through_the_hosts_conversion`). A reading never prints a
  negative zero, and a formatter chooses its branch from the rounding its finer branch prints
  (`seconds` is a probed exception: [NOTES.md § Parameter text](NOTES.md#parameter-text)).
- **Changing the space never moves a control** (plan §2). `loaded` sounds the loaded space, or the
  Init space (`SpaceChoice::INIT`) while nothing is loaded. A fit landing is a player's drop, not a
  parameter writing another ([NOTES.md § Parameters](NOTES.md#parameters)).

## Layouts, Off and activity

- Mono in → stereo out, and stereo in → stereo out. Mono input feeds both wet channels.
- **Mix at zero is Off:** dry to the bit in both layouts, the engine emptied and parked
  (`mix_zero_is_the_dry_to_the_bit_in_both_layouts`).
- Input present, or a parked engine: `Normal`. A decaying tail: `Tail(n)` from the engine's
  declaration, recomputed every block. Nothing sustains, so there is no third state
  (`a_tail_is_reported_and_then_the_effect_sleeps`).
- `activate` rebuilds the engine at the host rate and settles it on the current parameters before
  resetting it, so state restored before activation takes effect without a transition.

## Presets are provisional

- Twelve factory sounds, designed in `preset.rs` in the parameters' own units, generated by the
  `#[ignore]`d `write_the_factory_presets` and held to the design by `the_factory_files_match_the_design`.
  Init is the defaults. Declaration order — the selector first — is the write order. Every factory
  file is `state = null`, so it preserves the loaded space.
- **Each design names its space by generated variant** (`SpaceChoice::VocalPlate as usize`), so any
  regeneration reruns `write_the_factory_presets` ([NOTES.md § Presets](NOTES.md#presets-are-provisional)).

## Factory spaces are generated

`src/spaces/generated.rs` and `spaces/<id>.json` are written by the fit crate's `classic_verb_generate`
([its generator](../../crates/mxm-classic-verb-fit/NOTES.md#the-factory-space-generator)) from a
manifest that lives outside the repository with the owner's responses, **never by hand**.
`SpaceChoice` is every manifest row in order, then `Loaded`, held one past the table at compile time;
each position's fit report is `spaces/<id>.json`, with no orphans. Regenerating reproduces the
committed files byte for byte ([NOTES.md § Factory spaces](NOTES.md#factory-spaces-are-generated)):

```bash
cargo run -p mxm-classic-verb-fit --release --example classic_verb_generate -- --manifest <the manifest, outside the repository>
cargo test -p mxm-classic-verb -- --ignored write_the_factory_presets
cargo test -p mxm-classic-verb
cargo run -p mxm-classic-verb --release --example classic_verb_space_audit
```

## The space audit

`classic_verb_space_audit` (plan §5.1, §9.1; `tests/space_audit.rs` by default) renders every factory
space **through the plugin** as its verification was rendered. **The recorded render is the target, not
the response's descriptors; any miss exits non-zero, naming the space and the descriptor.** Each
tolerance sits between what the plugin's path moves and the smallest change that moves the
descriptor; **the floors are Windows figures** ([NOTES.md § The space audit](NOTES.md#the-space-audit)).

## Loading a space from an impulse response

Plan §5.2, built at P6: a WAV or AIFF dropped on the Space card is checked, decoded and fitted on the
background task, and lands through the sampler's commit order. Only the fitted space is stored; no
path enters state or presets ([NOTES.md § Loading](NOTES.md#loading-a-space-from-an-impulse-response)).

- **Refuse early, by name, writing nothing:** the header preflight (the fit crate's own bounds) runs
  before anything proportional to the file is allocated; every decoded sample is checked finite; a
  fit under `CONFIDENCE_FLOOR` names its weakest check. The current space keeps sounding. The fit
  runs on nice-plug's background thread, never in `process()` or on the editor's thread.
- **Every buffer the file sizes is reserved fallibly** (`decode::reserve`, else `Refusal::OutOfMemory`).
  Never `vec![Vec::with_capacity(n); k]`: a clone keeps the contents, not the capacity.
- **The commit order:** staged; then the selector to `Loaded` and the fitted centre as bracketed
  gestures — **never Mix, Ducking or Mod rate**, and beyond the selector and the four fitted controls
  only what is not already at the centre; then committed. The audio thread reads only a sequence lock
  over the committed space and never waits, so a commit is heard from the next block.
- **Fit generations:** a newer drop, a recall, Init, a host state restore and the editor closing each
  supersede a running fit, which is discarded without a trace; **Reset does not supersede**. Races are
  stepped by hand, never by sleeping.
- **The payload** (`payload.rs`, version 1, JSON) travels in a user preset's `state` and the host's
  `loaded` field. **An unknown payload version or space form is refused whole**; values pass
  `Space::sanitised`; absence is explicit. The load line says every state in words, never by hue.

## Control map

Mix fills the existing `fx.reverb` role; page 13 maps the seven performed controls in the brief's slot
order, the eighth slot empty on purpose, the rest unmapped. The `effect_chain` host test holds it
([NOTES.md § Control map](NOTES.md#control-map)).

## Editor

Built to the approved brief: four Effects cards — **Space**, **Decay**, **Texture**, **Output** — in
signal-flow order ([NOTES.md § Editor](NOTES.md#editor)). Every parameter is bound once, in
`editor/sections.rs::all_parameters`, with a one-sentence tooltip. Nothing on the audio thread reads
editor state; an effect has no developer channel.

- **Every card is a `mxm_ui::tree` whose floor is computed**; `REFERENCE` and `MINIMUM` are derived.
- **Relative controls are drawn bipolar**, neutral at normalised 0.5 and as the default.
- **The decay display** draws `predicted_decay_s` on the space the selector sounds, with the controls
  built exactly as `MxmClassicVerb::controls` builds them. **Off** and **Nothing loaded** are words.
- The headless tests do not prove native windows, a native drag, physical DPI or zoom.

# Work Guidance

- A new parameter is a composition law first: say which of the two kinds it is before adding it.
- Regenerating presets: `cargo test -p mxm-classic-verb -- --ignored write_the_factory_presets`,
  then run the default suite.
- A new control changes a card's content width, and its floor follows from the tree: let
  `the_opening_size_is_the_budget_hugged` say the new opening size.
- **A change to the fit crate's API is a change to `fitting.rs` alone** in `src/`, and to
  `tests/audit/mod.rs` where the audit reads an analysis. Keep every other module on the plugin's own
  types.
- **Regenerating the factory spaces** is *Factory spaces are generated*'s commands, never a hand edit
  of `src/spaces/generated.rs` or `spaces/`. Run the audit after, and redesign the presets if the
  positions moved.
- **No test or build reads a real impulse response.** Responses are planted in `testing.rs` and
  written to memory or a temporary file; the owner's files are licensed third-party material.
- A field added to the payload that an older build would read wrongly moves `PAYLOAD_VERSION`; one
  an older build can ignore does not.

# Verification

```bash
cargo test -p mxm-classic-verb                      # the space audit over every committed space included
cargo run -p mxm-classic-verb --release --example classic_verb_space_audit
cargo clippy -p mxm-classic-verb --all-targets
# Every page, light and dark, for review -> target/layout-tree/mxm-classic-verb/<MXM_PICTURES tag>/
MXM_PICTURES=after cargo test -p mxm-classic-verb --lib tree_pictures -- --ignored
cargo fmt -p mxm-classic-verb --check
cargo test -p mxm-preset                            # the durable-content seam this plugin now uses
cargo test -p mxm-player control_map --lib          # after changing control-map.json or the page
cargo test -p mxm-player --test t5_control_map
cargo xtask bundle mxm-classic-verb
clap-validator validate "target/bundled/mxm-classic-verb.clap"
cargo xtask bundle mxm-classic-verb --release
clap-validator validate "target/bundled/mxm-classic-verb.clap"
cargo test -p mxm-classic-verb-host-tests --test effect_chain   # skips without the bundles
```

`mxm-preset` is mxm-kit's and `mxm-player` is MXM Player's: run those two in their own repositories.

Manual gates remain: owner listening; **a native drop of the owner's WAV and AIFF responses** onto the
Space card in the Player and in Bitwig, with a preset and a project saved and reopened; the editor's
native §15 review in both themes, with 100 %, 150 % and 200 % zoom at a fixed physical window;
Player audition; and Bitwig. Linux and macOS are not verified on the Windows development machine. *Since the split (2026-10-06):* Linux and macOS are checked later, together, and by CI on `v*` tags.

# Child DOX Index

No child `AGENTS.md` files.
