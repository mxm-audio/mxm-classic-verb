# AGENTS.md — crates/mxm-classic-verb-dsp/

Parent: [`../../AGENTS.md`](../../AGENTS.md). The measurements, rationale and history behind these
rules are in [NOTES.md](NOTES.md).

# Purpose

Framework-free, zero-dependency DSP for `mxm-classic-verb`: a feedback delay network reverberator
whose **space** — the shape a fit produces — is separate from its **controls**, the performed
quantities that act on it. The plan is `plans/plan-mxm-classic-verb.md` in the private archive;
every method's source is `research:effects/feedback-delay-network-reverb.md`.

This is P1. The spaces here are hand-authored; `crates/mxm-classic-verb-fit` produces fitted ones.

# Ownership

This crate owns its `src/` (`lib.rs` the engine, controls, laws, transitions, parking and every public
bound; `space.rs`, `network.rs`, `filter.rs`, `shaped.rs`, `delay.rs`), `tests/engine.rs` and
`examples/classic_verb_render_demo.rs`; what each file holds: [NOTES.md § Files](NOTES.md#files).
No plugin-framework, parameter, preset, editor or host type belongs here.

# Local Contracts

## A space is shape; the controls are scale

`Space` carries the early pattern normalised to Size, the low-, high- and top-band decay ratios to the
mid band, the tone correction — two shelves and a high cut, 0 dB at mid — and the measured width, all
in rate-independent units, all bounded by `Space::sanitised` because a fitted space is
user-generated input. Every space has the same form, so `Space::lerp` always exists.
`high_cut_hz` runs from `MIN_HIGH_CUT_HZ` to `HIGH_CUT_OPEN_HZ`, where (or non-finite) it is **open**.

- **`SPACE_VERSION` moves when a field is added, removed or reinterpreted — from release on.** Before
  release nothing is stored, so it stays 1 ([NOTES.md § SPACE_VERSION](NOTES.md#why-space_version-is-still-1)).

The composition laws are plan §2's, implemented once in `Engine::apply_targets` and
`Engine::composed_decay_s`:

| Control | Law as implemented |
|---|---|
| Decay, bass and treble | Band decay = Decay × space ratio × multiplier, each clamped to `MIN_DECAY_S..=MAX_DECAY_S`; treble multiplies both upper bands |
| Tone | Shelf gains = space dB + offset dB, into the space's high cut, then the whole path **divided by its magnitude at `TONE_REFERENCE_HZ`**, so tone never moves the wet level at mid. The cut has no control: High tone offsets the shelf ahead of it |
| Width | The wet side signal × space width × signed share; the mid signal untouched |
| Early and late | Early × 10^(+offset/40), late × 10^(−offset/40); at `±EARLY_LATE_SILENCE_DB` (24 dB) one part is silent outright |

- **The tone normalisation is not optional:** first-order shelves leak, which makes a tone control a
  mix control (`the_tilt_normalised_at_its_reference_leaves_the_mid_band_at_unity`).
- **`tone_magnitude(...)` is that normalised curve, public, beside `predicted_decay_s`**, for
  anything that predicts the tone. The fit's `the_tone_model_is_the_engines_public_tone_curve` holds
  the fit to it, so a change here fails there; `the_public_tone_curve_is_the_engines_normalised_tilt`
  holds it to the render ([NOTES.md § The public tone curve](NOTES.md#the-public-tone-curve-and-its-tests)).

## The space's high cut

- **A second-order Butterworth lowpass** (bilinear, corner prewarped, Q = 1/√2) after the shelves, in
  transposed direct form II with f64-computed coefficients; 3 dB down at the corner at every rate,
  never above unity. Its order and floor are measured ([NOTES.md § The space's high cut](NOTES.md#the-spaces-high-cut)).
- **Open is exact:** bypassed, reference magnitude exactly one (`an_open_high_cut_is_bypassed_to_the_bit`);
  the four hand-authored spaces are open. Closed, it is normalised with the tone, lifting at most 3 dB.
- **A space change reaches the cut only where the space in force changes** — at the fade's silent
  point, or while parked — and a cut closing from open starts from rest.
- **Rate and block independent:** the corner is in hertz, held below 0.45 of the rate; the same
  settings sent again at a block boundary render the same samples.
- Realtime: coefficients are computed in `apply_targets`, on a settings change, and allocate nothing.

## Sixteen lines, allpasses in the loop

Plan D6, taken by measurement at P3.5 ([NOTES.md § Sixteen lines](NOTES.md#sixteen-lines-allpasses-in-the-loop)).

- **Output taps:** each line is read at `OUTPUT_TAPS` points inside it besides its end
  (`TAP_FRACTION`), outside the loop, as Dattorro's tank reads its output; `late_onset_s` is the
  earliest a sound reaches the output, for the fit's pre-delay.
- **The Size floor is the fit's** (`crates/mxm-classic-verb-fit/AGENTS.md`). The engine's Size range
  is unchanged: below the floor is the boxy, metallic territory plan §2.1 names on purpose.
- Character is not settled by measurement: the owner's listening is on this network.

## Four decay bands

Plan D5, taken by measurement at P3.5 ([NOTES.md § Four decay bands](NOTES.md#four-decay-bands)).

- **Bilinear first-order upper shelves** at `DECAY_HIGH_CROSSOVER_HZ` and `DECAY_TOP_CROSSOVER_HZ`,
  each the geometric mean of the calibration octaves either side of it. Not one-pole: its
  impulse-invariant form still passes `(1 − p)/(1 + p)` at Nyquist. Not second order: no better, and
  a biquad a line.
- **Neither half is enough alone:** the fit's half is `crates/mxm-classic-verb-fit/AGENTS.md`'s
  *Decay is solved across each band*.

## The network, and why it is bounded by construction

- Sixteen lines at `RATIOS` × Size, each followed inside its loop by two Schroeder allpasses
  (`LOOP_ALLPASS_S`, `LOOP_ALLPASS_GAIN`), mixed by an orthogonal order-16 Walsh–Hadamard transform
  and a rotation by one line: lossless for any delays (Schlecht and Habets, 2017). Every output is
  read outside the loop; `total_delay_s` and `size_for_total_delay_s` give its modal density.
- **The loop allpasses' coefficient is fixed:** a time-varying one inside a lossless loop can diverge.
- **The decay filter's bound is proved, not measured:** the one-pole stage is at most
  `max(low, mid)`, each bilinear shelf at most `max(1, r)`, and `BandGains::limited` holds the product
  under `LOOP_GAIN_CEILING`, taking a lift off the top shelf first. The loop is linear and carries no
  saturator.
- `every_corner_of_the_domain_stays_finite_and_under_the_loop_ceiling` claims finiteness and the
  bound, not a level: its ceiling of 1,000 catches only a broken bound
  ([NOTES.md § The network](NOTES.md#the-network-in-full)).

## The decay rule, and what the calibration measured

- Per trip, each band's gain is `10^(−3·(m + τ̄)/(f_s·T₆₀))`: `m` the line, τ̄ its loop allpasses'
  mean group delay over that band's calibration octave (`CALIBRATION_HZ`). Calibrate on the group
  delay, never the plain delay; tests read T₃₀, never T₂₀.
- Where a decay spans at least 25 trips round the mean line, the formula is the calibration
  (`the_realised_decay_follows_the_composed_decay_across_size_decay_and_rate`); below that the decay
  is a staircase of returns, not an error.
- **The composed band targets are not the realised octave decays.** The fit (P3) accounts for this
  mapping; it does not assume the targets are realised.
- **`Engine::predicted_decay_s` is that mapping in closed form**, `predicted_decays_s` the same at many
  frequencies (`the_closed_form_predicts_the_realised_octave_decays`; equal to the bit by
  `the_closed_form_over_many_frequencies_is_the_closed_form_at_each`).
- **Measure a strongly tilted decay with a steep band filter:** the tests' is three cascaded passes;
  one section per octave leaks ([NOTES.md § The decay calibration](NOTES.md#the-decay-calibration-measured)).

## Modulation and shaped decays

- A modulated or Size-gliding line is read through a first-order Thiran allpass (plan §3.1, measured:
  `modulation_costs_less_high_band_decay_through_the_allpass_than_through_linear_reads`). Linear
  reads stay behind `Engine::set_interpolation` as the measurement seam and do not ship. Line rates
  are spread ±50 %; depth is clamped to a quarter of the shortest line.
- **Shaped decays live outside the loop:** gated and reverse are feed-forward taps whose gains are
  the envelope, into two short diffusers (`the_shaped_decays_have_their_envelopes`). Detail:
  [NOTES.md § Modulation and shaped decays](NOTES.md#modulation-and-shaped-decays).

## Transitions

- **A space change leaves the paths running:** the wet fades out over `SPACE_FADE_S`, the space
  swaps, and it fades back in ([NOTES.md § Transitions and parking](NOTES.md#transitions-and-parking-measured)).
- **A shape change empties the paths** and ramps the new path's input in over `PATH_ONSET_S`.
- **A change asked for during a fade becomes the destination**; the fade does not restart.
- **Size glides** with `SIZE_GLIDE_S` (plan D8's default); per-line gains are recomputed every
  `CONTROL_TICK` samples from an internal counter, so no host block boundary enters the output.

## Off, parking and the tail

- **Mix at zero is Off:** dry to the bit in both channels, the engine emptied once, then parked. Mix
  ramps linearly over `MIX_RAMP_S`, so Off arrives in bounded time. A parked engine that has heard no
  input since it emptied settles its mix at once, as a fresh engine does; one passing dry audio
  ramps.
- **Emptying is constant-time:** delay lines forget by resetting a written count.
- **Quiet parks only when** input and wet are below `QUIET_LEVEL`, the input history holds nothing
  being read, and quiet has outlasted `QUIET_HOLD_S` plus a trip round the longest line — or a sparse
  network parks between its own returns. The decision is rate-independent.
- **The tail declaration may overestimate and must not underestimate.** While the history still
  holds input it assumes the wet may reach full scale; it adds the level follower's own fall time.
  `the_tail_declaration_never_underestimates` checks every 10 ms of three tails.

## Realtime and numeric rules

- Rust MSRV 1.87, no runtime dependencies; `mxm-measure` (the tests' rulers) and `mxm-audio-file` (the
  demo's WAV writer) are dev-dependencies only.
- Allocation only in `Engine::new`. `process` and everything it calls allocate nothing and lock
  nothing. The API is one stereo sample at a time, so output cannot depend on host block partitioning.
- Recursive state is flushed below `1e-20`. Non-finite input is read as zero.
- Sample rates are clamped to `MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE` (8–384 kHz); the validated set is
  8–192 kHz. Memory scales with the rate ([NOTES.md § Memory](NOTES.md#memory)).
- `reset()` is complete and reproducible: rendering after it repeats the samples.

## Chosen, not measured

Every constant and every value in the four hand-authored spaces is chosen, never read from a reference
product, except what was measured: the high cut's order and `MIN_HIGH_CUT_HZ`; the line count, the loop
allpasses and `LOOP_ALLPASS_GAIN`; the four decay bands, their bilinear upper shelves and crossovers
([NOTES.md § Chosen, not measured](NOTES.md#chosen-not-measured-the-full-list)).

# Work Guidance

- Measure a closed-loop change; compiling proves nothing about decay, density or stability.
- **Do not open existing implementations**, and do not extract a shared reverb primitive with
  `crates/mxm-shimmer-dsp` — its lessons are read, its code is not reused. That crate is in the
  `mxm-shimmer` repository.
- A threshold in a test comes from a measured value with headroom, written beside it.
- A new control or space field is a composition law first: say which kind it is (plan §2) before
  writing the DSP.

# Verification

```bash
cargo test -p mxm-classic-verb-dsp
cargo test -p mxm-classic-verb-dsp --release -- --nocapture   # prints the figures in NOTES.md
cargo clippy -p mxm-classic-verb-dsp --all-targets
cargo fmt -p mxm-classic-verb-dsp --check
cargo +1.87.0 test -p mxm-classic-verb-dsp
cargo tree -e normal,build -p mxm-classic-verb-dsp            # no runtime dependency
cargo run -p mxm-classic-verb-dsp --release --example classic_verb_render_demo
```

Automated checks establish bounds, decay calibration, transitions and parking. They do not
establish character: owner listening at P3.5 does.

# Child DOX Index

None.
