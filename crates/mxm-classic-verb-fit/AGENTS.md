# AGENTS.md — crates/mxm-classic-verb-fit/

Parent: [`../../AGENTS.md`](../../AGENTS.md). Measurements, reasons and past runs: [NOTES.md](NOTES.md).

# Purpose

Impulse-response analysis and fitting for `mxm-classic-verb` (`plans/plan-mxm-classic-verb.md` §4;
in full: [NOTES.md § What the crate does](NOTES.md#what-the-crate-does)).

- **The analyser (P2)**: `f32` samples (one or two channels) and a rate become the descriptors a space
  is fitted from, with a confidence — or a `Refusal` naming the check that failed.
- **The fit (P3)**: `fit` and `fit_with` turn a response into a `Space` and Decay, Size, Diffusion
  and Pre-delay, render it through `mxm-classic-verb-dsp`, analyse the render with the same analyser
  and return the error for every descriptor — or a `FitRefusal`.
- **The offline tools (plan §5.1)**, as examples: `classic_verb_fit`, `classic_verb_audition`, and the
  factory-space generator `classic_verb_generate` with `classic_verb_synthetic_responses`.

Consumers: `mxm-classic-verb`, and the unshipped `mxm-listening` (`analyse` only). The library does no
file I/O; decoding is the examples'. Method: `research:effects/feedback-delay-network-reverb.md` §9,
§10 and §13 step 7, from the page's prose and equations; no existing implementation was opened.

# Ownership

Every file and what it holds: [NOTES.md § Files](NOTES.md#files). The ones that carry rules:

- `src/lib.rs` — the public seams (`preflight`, `analyse`, `analyse_with_method`, the fit's
  re-exports), **every analyser and texture constant with its reason**, the required-band rule, the
  order refusals are decided in, and the crate-private non-strict `analyse_render`.
- `src/fit.rs` — **the fit's public types and constants with their reasons**, `fit`/`fit_with`, the
  density floor that sets Size (`problem`), clamping, the verification render, `DescriptorErrors`.
- `src/buffer.rs` — the one seam every buffer the response's length sizes is reserved through.

# Local Contracts

## Dependencies and MSRV

- `[dependencies]` is `mxm-classic-verb-dsp` alone; dev-only `mxm-measure` and `hound = "=3.5.1"`.
  **No transform crate**: the texture's FFT is this crate's own
  (`the_fft_is_the_discrete_fourier_transform`). Why: [NOTES.md](NOTES.md#one-runtime-dependency-msrv-187).
- `rust-version = "1.87"`, compiled against: no `let` chains, nothing stabilised after 1.87.
- The DSP crate is read-only from here: an API this crate lacks is reported, not added.

## Invariants

- **Never NaN, and absent rather than zero**: every numeric field of an `Analysis` and a `Fit` is
  finite or `None` (`tests/support` and the fit test scan for it). Only two floors are
  representational: zero energy reads −300 dB, prominence over digital silence 300 dB.
- **Deterministic within one build**: the same samples give a bit-identical analysis and fit; nothing
  is seeded from the clock, and the search breaks ties by its fixed order. Not across platforms.
- **The input domain**: 1–2 channels, 22,050–192,000 Hz, 0.3–30 s ([why](NOTES.md#the-input-domain)).
  `preflight` decides all three from a header before anything proportional to the file exists;
  `analyse` checks them again, then every sample's finiteness, naming the first non-finite one.
- **Trailing digital silence is not part of the response**: analysis ends at the last non-zero sample
  (the tail texture excepted).
- **Validity is judged on the noise-compensated decay**: straightness is always scored on method E's
  curve, whichever method `analyse_with_method` reports.
- **Straightness reports; the noise margin refuses**: straightness lowers confidence and alone refuses
  only past 15.8 dB; a required band's too-small margin and a weak onset refuse by confidence. Why
  each rule: [NOTES.md § The analysis rules](NOTES.md#the-analysis-rules).

## Every buffer the response's length sizes is reserved fallibly

The fit runs inside a plugin on the host's background thread, where an ordinary failed allocation
aborts the host. Every such buffer goes through `buffer::filled` or `buffer::collected`
(`try_reserve_exact`); what a constant bounds stays ordinary. Running out is `Refusal::OutOfMemory`
or `FitRefusal::OutOfMemory { bytes }` at any stage — not a judgement of the response — and values do
not change. `tests/memory.rs` and `every_reservation_a_fit_asks_for_can_be_refused_by_name` catch a
buffer moved off the seam. Which buffers, and the figures:
[NOTES.md](NOTES.md#every-buffer-the-responses-length-sizes-is-reserved-fallibly).

## Refusals

**The analyser**, always: a bound, a non-finite sample, digital silence, an onset whose direct sound
stands under 20 dB over what precedes it, and no decay above the noise floor in a **required** band
(loudest block within 30 dB of the loudest band's). By confidence: under 0.25, the weakest check
named. **These rules are not the fit's to change.**

**The fit**: `FitRefusal::Analysis(Refusal)` (every analyser refusal, unchanged), `NoMidBandDecay` (no
required band from 500 Hz to 2 kHz has a T30 or T20), `Verification(Refusal)` (the render could not be
analysed at all) and `OutOfMemory { bytes }`. When each is reached: [NOTES.md § Refusals](NOTES.md#refusals).

## The tail texture

`Analysis::texture_a` and `texture_b` read peakiness, kurtosis, echo density and periodicity over two
segments of the tail (`src/texture.rs`), restating a measurement made outside the repository.

- **Reported, never refused**: no texture reading is a validity check, refuses a response or a fit,
  or enters confidence; the fit's search does not read them.
- Segments are placed by the mid-band T60 (none, no texture) and cut to a power of two of frames
  (`segments_are_placed_by_the_mid_band_decay_and_cut_to_powers_of_two`); moving means are centred
  as numpy's `convolve(…, "same")` (`centred_means_are_numpys_same_mode_convolution`).
- Where it differs from the reference is deliberate: [NOTES.md § The tail texture](NOTES.md#the-tail-texture).

## The fit's parameter contract (plan §2)

A fit returns a `Space` and `FittedControls` {Decay, Size, Diffusion, Pre-delay}.
`FittedControls::apply` writes those four onto a player's controls, returns the bass and treble
multipliers, early/late offset, width share and both tone offsets to neutral, modulation depth to
zero and the decay shape to Natural, and **leaves Mix, Ducking and modulation rate bit-identical**.
Every value outside its range (`ControlRanges`, default the engine's bounds, which are the plugin's
ranges today) or its space bound is clamped and listed in `FitReport::clamps` with the fitted and the
applied value.

## How each part is fitted

The method in full: [NOTES.md § How each part is fitted](NOTES.md#how-each-part-is-fitted). Keep:

- **The tone model is held to the DSP crate's public `tone_magnitude`** within 0.01 dB
  (`the_tone_model_is_the_engines_public_tone_curve`), so an engine tone change fails there. The tone
  solve excludes 1 kHz and every band that failed the single-slope check.
- **The high cut** is scored on the tone the space will carry (shelves clamped) and **stays open
  unless it brings that curve more than `HIGH_CUT_MIN_IMPROVEMENT_DB` closer**, and whenever no usable
  band lies above 1 kHz.
- **Diffusion and the first arrival** are searched at the fitted Size. **Selection**: the lowest score
  of every candidate evaluated, the first of equal scores.

## Decay is solved across each band

A band's modelled T30 is read as the analyser reads one, across its octave, not at its centre
(`solve.rs`, `DecayReading`; `the_decay_reading_is_as_fine_as_it_needs_to_be`). `DecayBand` carries
`predicted_s` (the centre) and `modelled_s` (what the solve matched). The rule in full, why and
measured: [NOTES.md](NOTES.md#decay-is-solved-across-each-band).

## Size is the density floor

The network's total delay is `DENSITY_FLOOR_DECAY_SHARE` times the longest decay target, at least
`DENSITY_FLOOR_MIN_S`, turned into Size by `size_for_total_delay_s` (`fit.rs`, `problem`). **Size is
not searched.** A `ControlRanges::size_s` that moves it is a `FitValue::Size` clamp, the floor as the
fitted value. Why, and the pool: [NOTES.md](NOTES.md#size-is-the-density-floor).

## Verification is measured, never assumed, and never refused as a response

- The fitted result is rendered at the response's rate and analysed by `analyse_render`, which
  **reports rather than refuses** a required band without a decay and a confidence under the floor,
  as `FitReport::verification_refusal`. A fit never claims a match it did not measure and never leaves
  a failed verification unexplained. The render: [NOTES.md](NOTES.md#the-verification-render).
- **The render's tail texture is read on the response's segments**, so a render cannot move its own
  ruler. `analyse_with_segments` is the strict public form.
- A `DescriptorErrors` field is `None` exactly when either side lacks the reading:
  [NOTES.md § When an error is absent](NOTES.md#when-an-error-is-absent).

## The examples and the factory-space generator

- The header goes to `preflight` before any sample buffer is allocated ([formats](NOTES.md#the-input-domain-as-the-examples-read-it)).
  A report names its source only by a logical name, never a path. **There is no default output folder.**
- `classic_verb_generate` turns a manifest into the plugin's factory spaces: **a deliberate run by
  hand**; no build or test reads a manifest or a response. **The manifest lives outside the
  repository**; its line order is the selector's order and each id is permanent once released.
- **The whole run is refused, and nothing is written**, on any manifest problem, a manifest or
  response inside the repository, a refused fit or `verification_refusal`, a recorded render that does
  not reproduce the fit's verification analysis bit for bit, or an output that would carry a path, file
  name or folder name. Format, outputs and runs: [NOTES.md](NOTES.md#the-factory-space-generator).

# Work Guidance

- **Every constant is declared with its reason beside it** (`src/lib.rs`, `src/decay.rs`,
  `src/fit.rs`, `solve.rs`). Values: [analyser](NOTES.md#chosen-constants--the-analyser),
  [fit](NOTES.md#chosen-constants--the-fit), [texture](NOTES.md#chosen-constants--the-tail-texture);
  where the page was interpreted: [NOTES.md](NOTES.md#how-the-page-was-read-where-it-had-to-be-interpreted).
- **Do not open an implementation** — pyrato, ITA-Toolbox, Faust, zita or any acoustics code. The
  research page's prose and equations are the source (root *Research citations*).
- **A threshold moves only with a new measurement**, and the comment beside its assertion quotes the
  figure. The planted generator stays independent of the analyser: brick-wall octaves from the
  transform, never this crate's Butterworth filters. The round trips render with the engine, never
  with the fit's own model.
- Tolerances are argued from populations of realisations or cases, not from the seeds asserted, so a
  changed seed is not a changed claim. The populations: [analyser](NOTES.md#measured-accuracy--the-analyser),
  [texture](NOTES.md#the-tail-texture--measured), [fit](NOTES.md#measured-accuracy--the-fit),
  [findings](NOTES.md#findings-p35-should-know).
- **Third-party impulse responses never enter either repository**, nor anything derived from them;
  run the examples on them only to check decoding, write output outside the tree, report only
  aggregates. The one exception is the generated factory spaces, which the plugin ships
  ([NOTES.md](NOTES.md#third-party-impulse-responses)).

# Verification

```bash
cargo test -p mxm-classic-verb-fit
cargo clippy -p mxm-classic-verb-fit --all-targets -- -D warnings
cargo fmt -p mxm-classic-verb-fit --check
cargo +1.87.0 test -p mxm-classic-verb-fit          # the declared floor, compiled against
cargo tree -p mxm-classic-verb-fit -e normal,build  # mxm-classic-verb-dsp alone
cargo run -p mxm-classic-verb-fit --release --example classic_verb_analyse_synthetic
cargo test -p mxm-classic-verb-fit --release --test robustness -- --ignored   # the ceiling's cost
cargo test -p mxm-classic-verb-fit --release --test texture -- --ignored --nocapture   # the texture populations
cargo test -p mxm-classic-verb-fit --release --test fit_round_trip -- --ignored --nocapture the_round_trip_population
cargo test -p mxm-classic-verb-fit --release --test fit_measure -- --ignored --nocapture --test-threads=1
cargo run -p mxm-classic-verb-fit --release --example classic_verb_fit -- --out <folder outside the tree> <files or folders>
cargo run -p mxm-classic-verb-fit --release --example classic_verb_audition -- --out <folder outside the tree> <file>
cargo run -p mxm-classic-verb-fit --release --example classic_verb_synthetic_responses -- --out <folder outside the tree>
cargo run -p mxm-classic-verb-fit --release --example classic_verb_generate -- --manifest <that folder>/manifest.tsv
```

Never `--workspace` from this crate while the workspace's other members (the collection's, before the split) are being written beside it.
Linux and macOS are not verified by anything here ([last runs](NOTES.md#last-run)); *since the split (2026-10-06)* the tests run on Linux in WSL before a push and on macOS by CI on `v*` tags.

# Child DOX Index

No child AGENTS.md files.
