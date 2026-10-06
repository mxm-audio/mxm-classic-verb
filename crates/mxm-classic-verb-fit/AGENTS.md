# AGENTS.md — crates/mxm-classic-verb-fit/

Parent: [`../../AGENTS.md`](../../AGENTS.md)

# Purpose

Impulse-response analysis and fitting for `mxm-classic-verb` (`plans/plan-mxm-classic-verb.md` §4).

- **The analyser (P2).** One or two channels of `f32` samples and a sample rate become the
  descriptors a space is fitted from — onset, direct sound and DRR, pre-delay, early reflections,
  per-octave decay with its noise floor and straightness, echo density and mixing time, late-field
  width, the tail's texture over two segments, and a confidence — or a `Refusal` naming the check
  that failed.
- **The fit (P3).** `fit` and `fit_with` turn a response into a `Space` and the four absolute
  controls (Decay, Size, Diffusion, Pre-delay) — Size calculated as the network's density floor,
  Diffusion and the first arrival searched against the early echo density profile and envelope —
  render the result through `mxm-classic-verb-dsp`, analyse that render with the same analyser, and
  return the error for every descriptor — or a `FitRefusal`.
- **The offline tools (plan §5.1)**, as examples: `classic_verb_fit` and `classic_verb_audition`, and
  **the factory-space generator**, `classic_verb_generate`, with `classic_verb_synthetic_responses` to
  prove it without anybody's response files.

**Consumers:** `mxm-classic-verb`, and `mxm-listening`, an unshipped tool that takes the analyser as a
normal dependency to read an impulse response as a space (`analyse` only; it never fits). The library
does no file I/O; decoding is the examples'. Method: `research:effects/feedback-delay-
network-reverb.md` §9, §10 and §13 step 7, implemented from the page's prose and equations. No
existing implementation was opened.

# Ownership

- `src/lib.rs` — the public seams (`preflight`, `analyse`, `analyse_with_method`, and the fit's
  re-exports), **every analyser constant with its reason**, the band loop, the required-band rule,
  the order refusals are decided in, and the crate-private non-strict `analyse_render`.
- `src/analysis.rs` — the `Analysis` types; `src/refusal.rs` — `Refusal` and its messages.
- `src/band.rs` — the octave Butterworth band-pass, run time-reversed then forward; its magnitude
  (`gain_at`) is also the tone model's band shape.
- `src/decay.rs` — Lundeby's method, Schroeder integration by Guski and Vorländer's methods E, C and
  A, the T20/T30/EDT fits, straightness and the initial level; Lundeby's step constants.
- `src/early.rs` — combined energy, onset, prominence, DRR, pre-delay, early reflections.
- `src/density.rs` — the echo density profile, its per-window statistic, and mixing time;
  `src/width.rs` — late-field IACC.
- `src/texture.rs` — the tail texture: where its segments sit, decay compensation, spectral
  peakiness, kurtosis, a segment's echo density, envelope periodicity, the centred moving mean and the
  radix-2 FFT.
- `src/validity.rs` — the checks and the confidence; `src/math.rs` — levels, conversions, least
  squares; `src/buffer.rs` — the one seam every buffer the response's length sizes is reserved through,
  fallibly, and its test-only watch.
- `src/fit.rs` — **the fit's public types and constants with their reasons**, `fit`/`fit_with`,
  the density floor that sets Size (`problem`), clamping, the verification render and
  `DescriptorErrors`.
- `src/solve.rs` — the closed-form parts: Levenberg–Marquardt, decay through `predicted_decays_s` read
  across each band (`DecayReading`), the tone solve and its high-cut search, arrivals and taps, the
  early/late energy balance, the width bisection.
- `src/search.rs` — the Diffusion search at the density floor: the model a candidate gives, its
  render, the profile and envelope distances, the grid over Diffusion and the first arrival, the
  pattern refinement over Diffusion and the selection.
- `src/render.rs` — the engine driven as an impulse response (dry removed, mono fold).
- `tests/support/mod.rs` — the planted synthetic responses, the planted tails of *The tail texture*
  (`TexturePlan`), and `assert_finite`; `tests/known/mod.rs`
  — the engine's own renders of a known space and controls, which the round trip and the synthetic
  factory responses both render.
- `tests/planted.rs`, `tests/refusals.rs`, `tests/robustness.rs`, `tests/texture.rs` (the analyser);
  `tests/memory.rs` (a response the machine cannot hold, under an allocator of its own);
  `tests/fit_contract.rs`, `tests/fit_round_trip.rs` and `tests/fit_measure.rs` (the fit; the last
  holds only the ignored measurements *Measured accuracy — the fit* prints); `tests/generate.rs` (the
  factory-space generator, including `examples/classic_verb_io` by path).
- `examples/classic_verb_analyse_synthetic.rs` — prints a planted room's analysis beside its plant.
- `examples/classic_verb_fit.rs`, `examples/classic_verb_audition.rs`,
  `examples/classic_verb_generate.rs`, `examples/classic_verb_synthetic_responses.rs`, and their
  shared `examples/classic_verb_io/`: `mod.rs` (WAV via `hound`, AIFF and AIFF-C parsed there,
  logical names, a small JSON writer), `report.rs` (the fit report `classic_verb_fit` and the
  generator both write), `manifest.rs` (the factory-space manifest and its refusals) and
  `generate.rs` (the generator's pipeline).

# Local Contracts

## One runtime dependency, MSRV 1.87

`[dependencies]` is `mxm-classic-verb-dsp` alone (`cargo tree -e normal,build`). Dev-dependencies:
`mxm-measure` (its transform plants test responses and convolves for the audition) and
`hound = "=3.5.1"` (WAV reading in the examples; no AIFF crate). `rust-version = "1.87"`, compiled
against (*Verification*): no `let` chains, nothing stabilised after 1.87. The DSP crate is read-only
from here: an API this crate lacks is reported, not added. **No transform crate either**: the tail
texture's FFT is this crate's own (`src/texture.rs`, the iterative bit-reversed form of Cormen et al.'s
*Introduction to Algorithms*, ch. 30), because a transform dependency would be a second runtime
dependency and `mxm-measure`'s transform is dev-only (`the_fft_is_the_discrete_fourier_transform`).

## Never NaN, and absent rather than zero

Every numeric field of an `Analysis` and of a `Fit` is finite or `None`; `tests/support` scans every
field of every analysis the tests produce, and the fit test scans the whole fit's text. Two floors
are representational, not measurements: an energy of exactly zero reads −300 dB, and onset
prominence over exact digital silence reads 300 dB.

## Deterministic within one build

The same samples give a bit-identical analysis and a bit-identical fit
(`analyses_of_the_same_samples_are_bit_identical`, `a_fit_is_deterministic_and_every_number_in_it_
is_finite`). Nothing is seeded from the clock; the search visits candidates in a fixed order and
breaks ties by that order. Byte identity across platforms is not claimed (plan §4.3).

## The input domain

| Bound | Value | Why |
|---|---|---|
| Channels | 1–2 | Mono or stereo; true-stereo and multichannel are plan D10 |
| Sample rate | 22,050–192,000 Hz | The lowest common production rate, where six of seven bands exist; the top of the collection's `RATES` |
| Duration | 0.3–30 s | *Derived*: a 0.2 s T60 plus the last tenth Lundeby takes its noise from. *Measured*: a planted 0.2 s decay's T30 spread in 0.3 s is no wider than in 1.0 s. The ceiling is three times a large hall's T60. *Measured* at the ceiling, stereo at 192 kHz in release: 1.02 s, and a peak working set of 137 MB for the whole test process, 46 MB of it the input |

`preflight` decides all three from a header before anything proportional to the file exists;
`analyse` checks them again, then every sample's finiteness, naming the first non-finite sample in
frame order.

## Every buffer the response's length sizes is reserved fallibly

**The fit runs inside a plugin**, on the host's background thread, and an allocation that fails the
ordinary way aborts the process — the host with it. The domain above still admits 46 MB of `f32` and
twice that for each `f64` copy. So every such buffer goes through `buffer::filled` or
`buffer::collected`, which reserve with `try_reserve_exact`: the combined energy, each band's filtered
copy and energy, each decay curve, every render and its mono fold, the search's early and combined
excerpts, and the verification's scaled copy. What a constant bounds stays ordinary — the texture
segments (at most 800 ms each), the density profile and early envelope (0.5 s at a millisecond), the
block levels, the reflections, the width match's mid, side and trial channels (1.08 s at most) and
anything counted in bands, taps or candidates.

- **The refusal**: `Refusal::OutOfMemory { bytes }` from the analyser, and `FitRefusal::OutOfMemory {
  bytes }` from the fit whichever stage ran out — the response's analysis, a render, or the
  verification's analysis, lifted rather than reported as the response's refusal or the render's.
  Not a judgement of the response. Values are unchanged: a reservation is filled exactly as the
  allocation it replaced was (the plugin's audit reads its committed figures).
- **A real allocator proves it**: `tests/memory.rs` installs one that refuses any single allocation over
  32 MB and analyses and fits thirty seconds at 192 kHz. An ordinary allocation there aborts the test
  process (`memory allocation of 46080000 bytes failed`, exit `0xc0000409`); a fallible one refuses.
- **Every site is proved**: `every_reservation_a_fit_asks_for_can_be_refused_by_name` records the call
  site of every reservation a fit asks for (202 on its planted response), refuses each site at its
  first and at its last reservation, and holds the sites to the seven modules named above, so a buffer
  moved off the seam is noticed. 13.6 s of the debug suite.

## Trailing digital silence is not part of the response

Analysis ends at the last non-zero sample. Past it a band holds only the analysis filter's ringing
sliding into numeric dust, and with no noise floor Lundeby's late fit lands there: a decay into exact
silence was refused (no decay at 500 Hz) until this rule. The fit's own renders end noiselessly.

## Validity is judged on the noise-compensated decay

`analyse_with_method` reports decay times by method E, C or A, but always scores straightness on
method E's curve, so a comparison sees the same confidence and refusals. An uncompensated curve bends
by construction and refused itself before this rule.

## Straightness reports; the noise margin refuses

The plan reports a decay that is not one slope rather than refusing it. Measured, straightness cannot
tell a coupled room's two-slope decay from some non-responses (a staircase reads 3.6–5.3 dB, a run
of decaying notes 3.3–4.1 dB), so its ramp lowers confidence and on its own refuses only past
15.8 dB, which nothing planted reached. What refuses by confidence is a required band whose margin is
too small to give a decay time, and an onset with little prominence.

## The tail texture

**Why it exists.** The owner listened to fitted spaces beside their responses and heard them as "not
diffuse enough", "like a lot of delays" and some "completely off" (2026-09-15); the fit's report had
passed them. The echo density profile cannot see either failure: a network of short lines rings as a
sparse comb of high-Q modes whose amplitudes are Gaussian enough to read a density of one, and separate
echoes and flutter hide inside a 250 ms profile. `Analysis::texture_a` and `texture_b` read four things
over two segments of the tail (`src/texture.rs`), restating a measurement made outside the repository
that day:

| Reading | What it is | Decaying Gaussian noise reads | What reads otherwise |
|---|---|---|---|
| `peakiness` | The median over sixth octaves — from 150 Hz while an octave's upper edge is at most min(10 kHz, 0.4 fs), each holding at least 10 bins — of mean(P²) / mean(P)², P the Hann-windowed periodogram of the decay-compensated segment; the mean over channels | 1.80–2.01 | Modes 30 Hz apart, over 341–372 ms: 4.82–5.57 |
| `kurtosis` | The fourth standardised moment of the decay-compensated segment, its mean removed; the mean over channels | 2.91–3.07 | Clicks 50 a second: 517–1,111 |
| `echo_density` | Abel and Huang's normalised count (`density::window`, the window's mean removed) in 20 ms windows centred every 5 ms in the segment, on the samples as they are; the mean over windows and channels | 0.977–1.015 | Clicks: 0.06–0.57 |
| `periodicity`, `periodicity_lag_s` | The largest normalised autocorrelation of the log envelope — log10 of the compensated segment's 1 ms centred mean square, less its 50 ms centred mean, less its mean — **past the main lobe**: from the later of 3 ms and the first local minimum (walking from lag one while it falls, no further than the search's end) to the earlier of 250 ms and half the segment; the larger channel's. Zero, with no lag, where that range is empty | at most 0.203 | A 2 ms burst every 20 ms: 0.66–0.95, at 20.000 ms every time |

**Decay compensation** divides each sample by √(its 30 ms centred mean square + 10⁻³⁰), the mean read
over the whole channel. Every moving mean is centred on `i − w/2 ..= i + (w − 1)/2` and counts what lies
outside its signal as zero, as numpy's `convolve(…, "same")` does, because the reference is written
that way (`centred_means_are_numpys_same_mode_convolution`). The running sum is summed afresh every
window, so rounding cannot carry from a loud start into a quiet end.

**Where the segments sit**, in frames after the direct sound:

- **Origin**: the direct sound plus `TEXTURE_ORIGIN_S` (3 ms).
- **T60**: the mean T30, else T20, of the required bands from 500 Hz to 2 kHz — `solve::decay_targets`
  over `solve::is_mid`, the bands Decay is fitted from. Without one there is no texture.
- **Usable end**: the last frame whose centred 20 ms mean square, summed over channels and counted from
  the origin, stands `NOISE_MARGIN_DB` over the broadband noise floor.
- **A**: from clamp(0.15 · T60, 20 ms, 100 ms) after the origin for clamp(0.5 · T60, 100 ms, 600 ms),
  cut at the usable end; absent under one echo density window (`TEXTURE_MIN_SEGMENT_S`).
- **B**: from A's end as cut, for up to 800 ms, to the same cut; absent under 150 ms.
- **Each is then cut to the largest power of two of frames it holds** when it holds at least 2048, so
  both transforms are radix 2. A shorter one is read at its own length, its periodogram transformed bin
  by bin, which is exact and under 2048 × 1024 products.

`segments_are_placed_by_the_mid_band_decay_and_cut_to_powers_of_two` holds a 3 s decay's segments to the
frame.

**Where this reads differently from the reference, on purpose:**

- **T60 is the mid bands'**, not the broadband energy decay's −5 to −25 dB slope, so the texture is
  placed by the decay the fit matches.
- **The noise floor is Lundeby's** broadband floor, not the mean of the 20 ms envelope's last tenth.
- **A has a floor**, one echo density window; the reference has none and would read an empty segment.
- **The echo density hop is rounded** to whole frames as every window here is: 221 frames at
  44.1 kHz, where the reference truncates to 220; the same at 48 kHz.
- **A reading is the mean over the channels that have it**, where the reference's mean is NaN if one
  lacks it. An empty periodicity range reads zero with no lag, where the reference returns a lag of
  zero.
- **A mean square rounded below zero** where a signal falls into digital silence is held at zero before
  its root or logarithm. Found on an exported render ending in silence inside segment B, where the
  root of a negative made every reading but echo density absent.
- **The samples are read as given, trailing digital silence included**, as the reference reads a file —
  unlike every other reading here (*Trailing digital silence is not part of the response*).
- **A stated segment is shortened by the same rule** (`analyse_with_segments`), which leaves every
  segment an analysis records as it is.

**Resolution follows the segment's length.** A Hann window's main lobe is ±2 bins, so a segment of
85 ms smears lines within ±23 Hz of one another: measured, modes 30 Hz apart read 1.41–1.55 over
85–93 ms — under noise — and 2.43–2.80 over 171–186 ms, where from 341 ms they read 4.8 and more (*The
tail texture — measured*). A short B on a short decay cannot see a 30 Hz comb.

**Reported, never refused.** No texture reading is a validity check, refuses a response or a fit, or
enters confidence; `the_verification_reads_the_tail_texture_on_the_responses_segments_and_reports_every_error`
holds a response read with no texture at all to the same validity. The fit's search does not read them.

## Refusals

**The analyser**, always: a bound, a non-finite sample, digital silence, an onset whose direct sound
stands under 20 dB over what precedes it, and no decay above the noise floor in a **required** band
(loudest block within 30 dB of the loudest band's). By confidence: under 0.25, the weakest check
named. **These rules are not the fit's to change.**

**The fit** refuses with:

- `FitRefusal::Analysis(Refusal)` — every analyser refusal of the response, unchanged.
- `FitRefusal::NoMidBandDecay` — no required band from 500 Hz to 2 kHz has a T30 or T20, so Decay
  has nothing to be read from. *Measured*: a broadband direct sound makes every band required, so
  only a response whose direct sound itself lacks the mid octaves reaches it
  (`a_response_with_no_mid_band_decay_is_refused`).
- `FitRefusal::Verification(Refusal)` — the render could not be analysed at all. Only the analyser's
  input bounds, silence or onset can do that, and the render is built inside them (below); nothing
  measured has reached it.
- `FitRefusal::OutOfMemory { bytes }` — a buffer could not be reserved at any stage (*Every buffer the
  response's length sizes is reserved fallibly*). The analyser's own is `Refusal::OutOfMemory`.

## The fit's parameter contract (plan §2)

A fit returns a `Space` and `FittedControls` {Decay, Size, Diffusion, Pre-delay}.
`FittedControls::apply` writes those four onto a player's controls, returns the bass and treble
multipliers, early/late offset, width share and both tone offsets to neutral, modulation depth to
zero and the decay shape to Natural, and **leaves Mix, Ducking and modulation rate bit-identical**.
Every value outside its range (`ControlRanges`, default the engine's bounds, which are the plugin's
ranges today) or its space bound is clamped and listed in `FitReport::clamps` with the fitted and the
applied value.

## How each part is fitted

- **Pre-delay** is the analyser's. The **first arrival** is a hypothesis the search tries both ways,
  because the analyser reports the late field's first peaks as reflections too: `Reflection` puts
  the first tap at the pre-delay; `LateField` takes `late_onset_s(Size)` off it — the DSP crate's
  earliest output tap, 1.25 ms at the density floor (*derived* from its `TAP_FRACTION` and `RATIOS`) —
  since the late field's first energy follows the pre-delay by that much.
- **Early taps**: the strongest 12 reflections within the engine's reach (three Size units after the
  pre-delay), times in Size units, gains normalised so the loudest is one. What lies beyond reach is
  counted in `EarlyReport::beyond_reach`, which at the density floor only a range holding Size under
  the floor can make non-zero (*derived*; *Findings P3.5 should know*).
- **Decay and band ratios** — low (125 and 250 Hz), high (4 kHz) and top (8 kHz): least squares in log
  seconds over the required bands' T30 (weight 1), else T20 (`T20_WEIGHT`), at the fitted Size, each
  band read across its octave (*Decay is solved across each band*, below). The low ratio stays one
  with no measured band on its side; an upper ratio with none follows the other upper ratio, or stays
  one if neither was measured.
- **Tone**: least squares through an f64 copy of the space's tone — first-order shelves at 250 Hz
  and 4 kHz into the second-order Butterworth high cut, normalised at 1 kHz — each band's level
  modelled as a white late field seen through the analyser's own band shape. **The copy is held to
  the DSP crate's public `tone_magnitude`** within 0.01 dB by
  `the_tone_model_is_the_engines_public_tone_curve`: at 44.1, 48 and 96 kHz, shelves up to ±48 dB,
  and the cut open, at its 1 kHz floor, at 2.5 and 7 kHz, and held below Nyquist. A change to the
  engine's tone fails there rather than skewing every fitted space. It **excludes 1 kHz and every
  band that failed the single-slope check**. The shelves are held within ±`TONE_SEARCH_DB` (48 dB,
  twice the space bound) so a clamp can say how far past the bound the curve asked.
- **The high cut**: a one-dimensional search around that least squares. The shelves are solved for
  each corner — `HIGH_CUT_STEPS_PER_OCTAVE` an octave from `MIN_HIGH_CUT_HZ` (1 kHz) to just below
  open, then `HIGH_CUT_REFINE_STEPS` golden-section steps, every corner rounded to the `f32` the space
  stores — and a corner is **scored on the tone the space will carry**, its shelves clamped to
  ±24 dB, so no corner wins on a match that only a shelf past its bound could give. **The cut stays
  open unless it brings that curve more than `HIGH_CUT_MIN_IMPROVEMENT_DB` (0.1 dB RMS) closer**
  than the shelves alone, and whenever no usable band lies above 1 kHz; an open cut solves exactly
  the shelves the fit solved before the cut existed. `ToneReport` carries the corner
  (`high_cut_hz`), the residual of the tone as carried (`residual_db`, shelves clamped) and the
  residual with the cut open (`open_residual_db`); `classic_verb_fit` writes the corner into the
  space's JSON and the open residual beside the residual. The corner is searched inside its space
  bound, as Diffusion is inside its range, so its clamp check (`FitValue::HighCut`)
  reports only a corner the bound moved; a corner at 1 kHz sits on the floor, and the curve may have
  asked for lower.
- **Width**: bisection on the render's late IACC (mid ± w·side is linear in w, so one render at
  width one serves). Not measured for mono.
- **Early/late balance**: the early level that gives the response's energy ratio, from the end of the
  direct-sound window to `BALANCE_WINDOW_S` after the pre-delay against the next window. The engine
  is linear, so early = render(level 1) − render(level 0), and the level is a quadratic root.
- **Size** is the density floor, calculated before anything is searched (*Size is the density floor*,
  below).
- **Diffusion and the first arrival** — the search, at that Size: every first-arrival hypothesis (only
  `LateField` when there are no reflections) × `DIFFUSION_GRID`, then a pattern search over Diffusion
  alone from each of the best `REFINE_STARTS` grid points. A candidate's score is its echo density
  profile distance plus `ENVELOPE_WEIGHT_PER_DB` times its early energy envelope distance, both from
  `PROFILE_START_S` to `PROFILE_SPAN_S` after the pre-delay. **Selection**: the lowest score of every
  candidate evaluated, the first of equal scores. Candidates render at the Size the fit applies, so a
  range that clamps the floor is heard in the search too.

## Decay is solved across each band

**The rule** (`solve.rs`, `DecayReading`). A band's modelled T30 is read as the analyser reads one:
`DECAY_POINTS` (31) frequencies spaced evenly in octaves across `DECAY_SPAN_OCTAVES` (1.5) either side
of its centre, each decaying at the DSP crate's closed form (`predicted_decays_s`) and weighted by the
analyser's band filtered twice, the tone the space carries (`ToneSolution::carried`) and the frequency —
a late field flat per hertz; their energy decays summed into a Schroeder curve, whose least-squares
line from −5 to −35 dB is read at `DECAY_CURVE_SAMPLES` (32) instants. A point more than
`DECAY_WEIGHT_FLOOR` (80 dB) under its band's heaviest is left out. `DecayBand` carries both readings:
`predicted_s`, the closed form at the centre, and `modelled_s`, what the solve matched.

**Why.** The slower-decaying frequencies in an octave own its late energy, so where the decay changes
steeply inside it the measured T30 reads longer than the closed form at the centre. Measured outside the
repository on the pool's 345 fits with the three-band filter: against the renders, this model missed
4 kHz by p90 2.4 % and 8 kHz by 4.7 %, where the centre missed by 9.4 % and 7.5 %. Solving across each
band was half of plan D5; the fourth decay band was the other, and neither was enough alone (the DSP
crate's *Four decay bands*).

**Measured.** `the_decay_reading_is_as_fine_as_it_needs_to_be`: on a dark space at 44.1 and 96 kHz, 31
points agree with 61 within 0.01 % in every band; on a flat space a band read across its octave is within
1.1 % of the closed form at its centre (at 125 Hz, where the low shelf's leak changes the decay inside the
octave). On the pool, the render's T30 against `modelled_s`, p90 from 125 Hz to 8 kHz: 6.4, 4.1, 5.7, 3.2,
1.9, 2.6 and 6.0 %. **Cost:** the pool's median fit 0.47 → 0.54 s, the slowest 1.55 → 1.66 s.

## Size is the density floor

**The rule** (`fit.rs`, `problem`). The network's total delay is `DENSITY_FLOOR_DECAY_SHARE` (0.15)
times the longest decay target — the required bands' T30, else T20 — and at least `DENSITY_FLOOR_MIN_S`
(1 s): the recipe's modal density floor, Σm ≥ 0.15·T₆₀·f_s (`research:effects/feedback-delay-network-reverb.md`
§3.4, §13 step 1). The DSP crate's `size_for_total_delay_s` turns that into Size: **56.3 ms for any decay
under 6.7 s**. Size is not searched. A `ControlRanges::size_s` that moves it is reported as a
`FitValue::Size` clamp, the floor as the fitted value.

**Why it is not searched**, measured at P3.5 on twenty of the owner's responses (aggregates only),
**before the output taps**, when the network was read at its lines' ends alone. The excess measures and
the network they chose are `crates/mxm-classic-verb-dsp/AGENTS.md`'s *Sixteen lines, allpasses in the
loop*. Each response was fitted with Size searched from the floor up to a ceiling, a multiple of the
floor; the limits are ringing excess 0.5, spikiness excess 2, echo density deficit 0.1 and periodicity
excess 0.1:

| Ceiling | Ringing excess, p90 / worst | Spikiness excess, p90 / worst | Periodicity excess, p90 / worst | Pairs within all limits |
|---|---|---|---|---|
| **1× (Size is the floor)** | **0.10 / 0.31** | **0.59 / 10.5** | **0.05 / 0.24** | **17** |
| 1.25× | 0.10 / 0.35 | 0.82 / 19.2 | 0.14 / 0.15 | 15 |
| 1.5× | 0.09 / 0.36 | 4.05 / 101 | 0.13 / 0.20 | 15 |
| 2× | 0.06 / 0.31 | 4.25 / 107 | 0.13 / 0.20 | 14 |
| None (up to 300 ms) | — (one render too sparse to measure) | — | 0.12 / 0.13 | 10 |

Searched upward from the floor, Size ran to its 300 ms top for most responses: a huge network is sparse
early, which matches an echoey onset in the 250 ms the score reads, and clicks later, where the score does
not look. Searched from 2 ms on the eight-line network it went the other way, to short lines that rang.
**With the output taps**, Size at the floor, 19 of the twenty are within every limit: ringing excess p90
0.35, worst 0.45; spikiness 0.16, 0.45; periodicity 0.03, 0.14. The sweep was not repeated with them;
the early-energy gap they close is the DSP crate's *Output taps inside the lines*.

**The pool**: 345 of the owner's responses fitted, the springs folder excluded, and 154 refused by every
fitter.

| Fitter | Size median | Size under 20 ms | Fits with a verification refusal |
|---|---|---|---|
| Eight lines, Size searched from 2 ms | 13.9 ms | 220 | 28 |
| Sixteen lines before the output taps, Size searched upward from the floor | 272.8 ms | 0 | 144 |
| Sixteen lines with the output taps, Size the floor, three decay bands | 56.3 ms | 0 | 10 |
| **The same with four decay bands, solved across each band (this crate)** | **56.3 ms** | **0** | **8** |

This crate's eight are straightness at 8 kHz in three and a noise margin in five (1 kHz in three, 125 Hz
in two); three bands' ten were a noise margin in eight (five at 125 Hz) and straightness at 8 kHz in two.

**What the floor and plan D5 did to decay** over the same pool, each cell the median / p90 of |%|: the
render's T30 against the response's for the eight-line fitter, the three-band one at the floor, and this
crate's; this crate's residual, the closed form against the target, at the band's centre and as the solve
reads it (`modelled_s`); and the render's T30 against each.

| Band | T30 error, eight lines | T30 error, three bands | T30 error, now | Residual now, centre; modelled | Miss now, centre; modelled |
|---|---|---|---|---|---|
| 125 Hz | 2.4 / 9.0 | 3.5 / 9.3 | 3.2 / 8.7 | 2.3 / 6.2; 1.4 / 5.1 | 1.9 / 6.1; 2.1 / 6.4 |
| 250 Hz | 2.8 / 10.4 | 3.3 / 11.2 | 2.8 / 9.3 | 2.6 / 8.2; 2.4 / 7.7 | 1.7 / 5.2; 1.3 / 4.1 |
| 500 Hz | 3.3 / 10.6 | 3.4 / 13.6 | 1.9 / 8.7 | 1.3 / 6.0; 1.3 / 5.7 | 1.2 / 7.1; 1.1 / 5.7 |
| 1 kHz | 1.7 / 6.9 | 1.8 / 6.0 | 1.9 / 6.7 | 1.6 / 6.0; 1.4 / 5.4 | 0.9 / 4.0; 0.8 / 3.2 |
| 2 kHz | 2.9 / 11.8 | 2.9 / 11.4 | 1.3 / 7.7 | 1.3 / 7.2; 1.1 / 5.3 | 0.7 / 4.3; 0.5 / 1.9 |
| 4 kHz | 4.3 / 13.1 | 3.7 / 13.4 | 1.8 / 6.9 | 1.9 / 13.2; 1.3 / 4.8 | 2.1 / 9.9; 0.7 / 2.6 |
| 8 kHz | 4.7 / 19.6 | 4.5 / 17.7 | 0.9 / 9.3 | 2.9 / 12.6; 0.4 / 2.8 | 3.9 / 14.7; 0.7 / 6.0 |

The floor cost most at 500 Hz (p90 10.6 → 13.6 % on three bands). Every band's p90 is now under the
eight-line fitter's, and under three bands' in every band but 1 kHz (6.0 → 6.7 %). The worst band per
fit reads a median of 5.4 % and a p90 of 19.8 %, against 8.1 % and 25.3 % on three bands; 35 fits have a
band over 20 %, against 50. At 4 and 8 kHz the render reads longer than the closed form at the centre, a
signed median miss of +1.8 and +3.3 %, and within +0.4 and +0.1 % of `modelled_s`. **Clamps** on the
pool, eight lines: ToneHigh 74, ToneLow 28, DecayRatioHigh 13, Decay 4, Width 3, DecayRatioLow 2,
EarlyLevel 1. Three bands: ToneHigh 74, ToneLow 28, DecayRatioHigh 13, DecayRatioLow 5, Decay 4, Size 2,
EarlyLevel 2, Width 1. Now: ToneHigh 74, DecayRatioTop 29 — every one a top ratio under the 0.1 bound, a
median of 0.069 — ToneLow 28, DecayRatioHigh 6, Decay 4, DecayRatioLow 2, Size 2, EarlyLevel 2, Width 1.

**The tail texture on four bands**, the twenty pairs refitted and rendered again: 19 within every limit;
ringing excess p90 0.36, worst 0.47; spikiness 0.16, 0.51; periodicity 0.02, 0.11 — within a few
hundredths of three bands.

## Verification is measured, never assumed, and never refused as a response

The fitted space and controls are rendered at the response's rate: `VERIFY_LEAD_S` of silence, a
unit direct impulse, and the wet scaled to the response's DRR, held under `WET_PEAK_CEILING` of the
impulse; its length is the response's after the direct sound, kept inside the input bounds. It is
analysed by **`analyse_render`**, which measures exactly what `analyse` measures but **reports rather
than refuses** a required band without a decay and a confidence under the floor: the first such
refusal is `FitReport::verification_refusal`, naming its check and band, the band reads
`BandReading::Empty`, and its errors are absent. A fit never claims a match it did not measure and
never leaves a failed verification unexplained.

**The render's tail texture is read on the response's segments**, not on segments its own decay would
place, so a render cannot move its own ruler. The least invasive route: `analyse_render` takes the
response's `TextureSegments` (`target.texture_segments()`), everything else in the analysis unchanged,
and the render's length after its direct sound always holds them. `analyse_with_segments` is the same
analysis, strict and public: `classic_verb_generate` checks a recorded render with it, and the plugin's
space audit measures its renders with it. `FitReport::verification` therefore differs from `analyse` of
the same render in its two textures alone.

## When an error is absent

`DescriptorErrors` holds render against response. A field is `None` exactly when:

| Field | Absent when |
|---|---|
| `bands[].t30_percent` | Either side has no T30 in that band: the band is above Nyquist or `Empty`, or its decay curve did not reach −35 dB before the noise intersection. Also when the response's T30 is not positive |
| `bands[].t20_percent` | The same for T20 (−25 dB) |
| `bands[].tone_db` | Either side lacks the band's level against 1 kHz (no decay in that band or in 1 kHz) |
| `pre_delay_s` | Either analysis has no pre-delay: nothing after the direct-sound window rose above both the early level floor and the noise margin |
| `mixing_time_s` | Either echo density profile never reaches one within `ECHO_DENSITY_SPAN_S` (0.5 s) |
| `profile_distance` | No profile point in the compared span has a density on both sides (each window must stand `NOISE_MARGIN_DB` over the noise floor) |
| `envelope_db` | No envelope point in the compared span is defined on both sides (the response's gate is its noise floor plus `NOISE_MARGIN_DB`) |
| `iacc` | Either side's width is not measured (mono, or no late field) |
| `early_to_late_db` | Either side's early or late balance window holds no energy above its noise |
| `reflection_time_rms_s`, `reflection_level_rms_db` | No response reflection among its strongest 12 has a render reflection within `REFLECTION_MATCH_S` |
| `texture_a`, `texture_b`: `peakiness`, `kurtosis`, `echo_density`, `periodicity` | Either side lacks that reading on the response's segment. The response has no such segment: A where its usable end leaves less than `TEXTURE_MIN_SEGMENT_S`, B less than `TEXTURE_B_MIN_S` (a fitted response always has a mid-band T60, or the fit refused). `peakiness`: no sixth octave holds `PEAKINESS_MIN_BINS` bins with power in any channel. `kurtosis`: no channel varies over the segment. `echo_density`: no window varies. `periodicity`: no channel's log envelope varies. A periodicity of zero is a reading, not an absence |

## The input domain as the examples read it

WAV through `hound` (integer PCM and IEEE float); AIFF (big-endian PCM) and AIFF-C (`NONE`, `twos`,
`sowt`, `fl32`/`FL32`, `fl64`/`FL64`) parsed from Apple's AIFF 1.3 and AIFF-C chunk layouts, with the
80-bit extended sample rate. The header goes to `preflight` before any sample buffer is allocated,
and float samples are checked finite as they are decoded. A report names its source only by a
logical name (the stem as a slug, or `--name`), never a path, and **there is no default output
folder**, so nothing is written anywhere unless asked. Checked on a handful of third-party files
for decoding only: 24-bit and float WAV, plain AIFF and AIFF-C `fl32` decoded; a file under 0.3 s and
one over 30 s refused naming the bound.

## The factory-space generator

`classic_verb_generate` turns a manifest of impulse responses into `mxm-classic-verb`'s factory spaces
(plan §2.2, §5.1). **Regenerating is a deliberate run by hand**, as the collection's preset writers
are; no build or test reads a manifest or a response.

**The manifest lives outside the repository**, because it names response files. One space per line,
four tab-separated fields; blank lines and lines starting with `#` are ignored. The order of the lines
is the selector's order, and it is permanent once released (plan §2.2):

```text
# id	name	family	response
small-room	Small room	room	D:\responses\a response.aif
```

| Field | Rule |
|---|---|
| id | Permanent. Lowercase ASCII letters and digits in hyphen-separated words, the first starting with a letter. It names the report, and capitalised word by word the Rust variant (`small-room` → `SmallRoom`) |
| name | The selector's display name: not empty, no path separator, no control character |
| family | `room`, `ambience`, `chamber`, `hall`, `large space`, `drum room`, `plate`, `spring`, `strange` or `experimental` — the plugin's `SpaceFamily` |
| response | A WAV or AIFF file outside the repository, read as `classic_verb_fit` reads one. A relative path resolves against the working directory |

**What a run writes**, into `plugins/mxm-classic-verb`, or `--plugin <folder>`, whose `Cargo.toml`
must name `mxm-classic-verb`:

- `spaces/<id>.json` for every row: `classic_verb_fit`'s report with `source` the id, plus `render` —
  the sample rate, channel count, lead, length and wet gain of the fit's verification render — so the
  plugin's `classic_verb_space_audit` can make that render again without the response.
- `src/spaces/generated.rs`: the `SpaceChoice` enum, every row's variant in manifest order and
  `Loaded` last, and `FACTORY` — each row's id, name, family, fitted `Space` and fitted Decay, Size,
  Diffusion and Pre-delay. Floats are written with `{:?}`, which round-trips every `f32`; the text is
  written as `rustfmt` leaves it, and a `clippy::approx_constant` allowance covers a fitted number
  landing on a named constant's digits.
- The reports of ids the manifest no longer lists are removed. Nothing else in `spaces/` is touched.

**The whole run is refused, and nothing is written**, when:

- the manifest has any problem, each named with its line: not four fields; an id or name that breaks
  the rules above or holds a path separator; an unknown family; no response file; a repeated id; a
  name repeated in any case; two ids naming one variant (`a1`, `a-1`); the id `loaded` or `self`, or
  the name `Loaded`; no row at all. Nothing is fitted from a manifest with a problem;
- the manifest or a response lies inside the repository (compared folder by folder after resolving
  the path), or the target folder is not the plugin crate;
- a response cannot be read or its header is refused, the fit refuses, or the fit's report carries a
  `verification_refusal`. Every row is still tried, so one run names every refusal;
- **the recorded render does not reproduce the fit's verification analysis bit for bit.** The
  generator renders the recipe with the engine and analyses it as the fit does — `analyse_with_segments`
  on the response's texture segments — so a change to `fit.rs`'s verification that the generator has
  not followed refuses here rather than committing reports the audit would misread;
- an output would carry the response's path as given or resolved, its file name, or — in a report —
  its folder's name or any path separator.

Every refusal is decided before anything is written. The outputs then go to `.tmp` files beside their
destinations and are renamed into place once all are written; a failed write removes them and changes
nothing. **A rename failing partway cannot be undone across files**: the run says how far it got.

**Proved without anybody's responses.** `classic_verb_synthetic_responses` renders the DSP crate's
four hand-authored spaces with `tests/known` — stereo at 48 kHz, 2.5 to 6 s, at the controls of the
plugin's provisional presets of the same names — and writes them with a manifest to a folder outside
the repository. The plugin's committed spaces are that run (`plugins/mxm-classic-verb/AGENTS.md`,
*Factory spaces are generated*). `tests/generate.rs` holds the manifest and every refusal but the
fit's, a refused run leaving a plugin folder byte-identical, and one whole run on a mono synthetic
response filed under a name and folder the outputs are checked not to carry.

**Measured**, release, Windows, with four decay bands: the four synthetic responses fitted in 0.55–0.76 s
each, the whole run took 2.7 s, and each report is about 44 KB (1,650–1,666 lines). A hundred rows over
the same four responses, on the eight-line network with Size searched, took 88 s and wrote 4.25 MB of
reports and a 3,534-line, 130 KB module, still as `rustfmt` leaves it; that run was not repeated. **The
owner's hundred** (2026-09-15, four decay bands): 65.8 s, 4.3 MB of reports and a 3,634-line, 133 KB
module, and a second run from the same manifest reproduced all 101 files byte for byte.

## How the page was read, where it had to be interpreted

- **Band filter**: "second-order Butterworth" read as a second-order low-pass prototype, a four-pole
  band-pass. Time-reversed then forward, as §9.4 says, which makes it zero-phase with a squared
  magnitude (−6 dB at the octave edges).
- **Onset prominence**: the direct sound's mean square over ±2.5 ms against the mean square before
  the onset, less a 1 ms guard. The 20 dB rule is applied to that ratio; applied to single samples it
  could never fail, because every sample before the onset is under the threshold by definition.
- **DRR**: the ACE window with pre-direct energy counted as reverberant. The equalising filter the
  ACE challenge applies before picking the peak is not implemented; the peak is the broadband energy
  summed over channels.
- **Lundeby**: step 3's "last block above noise + 10 dB" is read over the blocks after the maximum;
  aborting a fit with fewer than two blocks is ours; the thirty-iteration cap reports non-convergence
  rather than refusing. **The truncation correction** — the late line's energy at the limit times its
  energy time constant — is *derived*: the page names a correction without its formula.
- **Initial level** is the T30 (else T20) fit of the decay curve, converted from integrated to
  instantaneous energy and read at the direct sound, so the tone curve includes each band's decay
  over the pre-delay. Anchoring it where the fit range begins was measured and was no better.
- **Echo density**: σ is the window's standard deviation with its mean removed; the profile is
  averaged over channels; a window not 20 dB above the broadband noise floor has no density (ours:
  noise alone is Gaussian and would read as mixed). "First within σ_l of one" is not implemented;
  the plan chose "first reaches one".
- **Width**: IACC is the largest |IACF|, with the sign kept so an inverted channel reads −1. The late
  window's end is ours (ISO's is *unverified* on the page).
- **Fit search**: §13 step 7 names a search over Size and Diffusion against the profile. Here Size is
  step 1's modal density floor instead (*Size is the density floor*), and only Diffusion and the first
  arrival are searched. The early envelope term and the first-arrival hypothesis are ours.
- **Not implemented**: the energy decay relief (§9.5) — how Jot turns it into filters is *unverified*
  on the page — and C7. The per-band initial level carries the tone instead.
- **The tail texture is not the page's.** It restates the measurement *The tail texture* names; the
  periodogram's normalised second moment, kurtosis and the envelope autocorrelation are textbook
  statistics, and only its echo density is §9.6's.

## Chosen constants — the analyser

Every one is a `pub const` in `src/lib.rs` or a step constant in `src/decay.rs`, with its reason
beside it.

| Constant | Value | Source or measured reason |
|---|---|---|
| `BAND_PROTOTYPE_ORDER` | 2 | §9.4 (IEC 61260) |
| `BAND_UPPER_EDGE_MAX_NYQUIST_FRACTION` | 0.9 | Chosen; decides only non-standard rates under 25.1 kHz |
| `BAND_REQUIRED_WITHIN_DB` | 30 dB | Chosen, for P3.5 to revisit |
| `ONSET_BELOW_PEAK_DB` | 20 dB | §9.1, ISO 3382-1 (*secondary*) |
| `ONSET_GUARD_S` | 1 ms | Chosen |
| `DIRECT_HALF_WINDOW_S` | 2.5 ms | §9.1, ACE challenge |
| `LUNDEBY_BROADBAND_BLOCK_S`; band block | 30 ms; (800 / f + 10) ms | §9.3 step 1 |
| Lundeby steps 2–9 | last 10 %; noise + 10 dB, 5 dB range; 5 blocks per 10 dB; 90 % or 10 dB past the crossing; noise + 30 to + 10 dB; 0.01 s; 30 iterations | §9.3 (*secondary*) |
| T20, T30, EDT ranges | −5 to −25, −5 to −35, 0 to −10 dB | §9.4 |
| `ONE_SLOPE_MAX_DEVIATION_DB`, `STRAIGHTNESS_FULL_DB` | 3.3 dB | Measured: single slope ≤ 3.05 dB (24), two-slope ≥ 3.61 dB (8) |
| `STRAIGHTNESS_ZERO_DB` | 20 dB | Measured: the strongest planted two-slope, 11.6 dB, scores 0.5 |
| `ECHO_DENSITY_WINDOW_S` | 20 ms rectangular (effective 20 ms) | §9.6 range; measured steadiest of rect 20/25/30 and Hann 20/30 effective |
| `ECHO_DENSITY_HOP_S`, `ECHO_DENSITY_SPAN_S` | 1 ms, 0.5 s | Chosen; the span covers §10's 300 ms and §9.6's 150 ms |
| `NOISE_MARGIN_DB` | 20 dB | *Derived*: noise under 1 % of a window |
| `EARLY_MIN_LEVEL_DB` | −40 dB | Chosen, under §5's weakest listed tap (−17.5 dB) |
| `PRE_DELAY_WINDOW_S`, `REFLECTION_HALF_WIDTH_S` | 1 ms, ±0.5 ms | Chosen |
| `EARLY_WINDOW_MAX_S`, `MAX_EARLY_REFLECTIONS` | 100 ms after the pre-delay, 24 | §5 |
| `WIDTH_LATE_START_S`, `WIDTH_MAX_LAG_S` | 80 ms, 1 ms | §9.7 |
| `WIDTH_LATE_SPAN_MAX_S`, `WIDTH_LATE_MIN_S` | 1 s, 20 ms | A cost bound (*derived*: a 3 s T60 leaves 1 % past it); one density window |
| `PROMINENCE_FULL_DB` | 35 dB | §9.3, ISO's recommended PNR via Guski and Vorländer |
| `MARGIN_ZERO_DB` | 25 dB | Measured: no T20 under a 25 dB margin |
| `MARGIN_FULL_DB` | 45 dB | §9.3, Guski and Vorländer's method C; measured T30 ≤ 7.4 % above it |
| `CONFIDENCE_FLOOR` | 0.25 | Measured: refuses under a 30 dB margin, where a T20 was present only 25 times in 28 |

## Chosen constants — the fit

Public ones are in `src/fit.rs` with their reasons beside them.

| Constant | Value | Source or measured reason |
|---|---|---|
| `T20_WEIGHT` | 0.5 | Measured: T20's worst error was 1.1–1.7 × T30's band by band, about twice the variance |
| `TONE_POINTS` | 17 over two octaves | Chosen: follows the band shape's skirts into the neighbouring octaves |
| `TONE_SEARCH_DB` (private, `solve.rs`) | 48 dB | Measured: unbounded, steep tilts ran the solve to −234 dB; twice the space bound lets a clamp say how far. **Kept with the high cut**: the cut takes the fall a shelf could not, and where a shelf still clamps — the dense onsets ask −28 dB, the low-cut plant −48 dB — the range still says how far |
| `DECAY_POINTS`, `DECAY_SPAN_OCTAVES` (private, `solve.rs`) | 31 points across 1.5 octaves either side | Chosen: a tenth of an octave apart; measured against 61 points within 0.01 % (*Decay is solved across each band*) |
| `DECAY_WEIGHT_FLOOR`, `DECAY_CURVE_SAMPLES` (private, `solve.rs`) | 80 dB under the band's heaviest point; 32 instants | Chosen: no slower neighbour that could own −35 dB is dropped; the curve is exact, so the line needs few instants |
| `HIGH_CUT_STEPS_PER_OCTAVE`, `HIGH_CUT_REFINE_STEPS` | 8; 12 | Chosen: an eighth of an octave, refined to under a thousandth of one; the tone solve is then about 50 least squares |
| `HIGH_CUT_MIN_IMPROVEMENT_DB` | 0.1 dB RMS | Measured on the eight-line network while Size was searched: the ten round trips, whose spaces have no cut, would take one for at most 0.039 dB, and taking it moved the search — the mono room's Size error +22.5 → +72.4 %, its 8 kHz tone error +1.37 → −3.05 dB. Not re-measured at the floor. The cuts taken now improve the carried tone by 0.23 dB (both dense onsets), 0.89 dB (the low-cut plant), 2.0–2.1 dB (the pack's clamped median) and 7.95 dB (the dark plant) |
| `BALANCE_WINDOW_S` | 100 ms (`EARLY_WINDOW_MAX_S`) | Chosen: the window the taps' reflections are read in |
| `PROFILE_START_S` | 11 ms | *Derived*: past half a density window, so no compared window holds the direct sound, which a search render lacks |
| `PROFILE_SPAN_S` | 250 ms after the pre-delay | Chosen: planted mixing times were 45–60 ms; past that a dense field reads about one in any candidate |
| `ENVELOPE_WINDOW_S`, `ENVELOPE_CLIP_DB` | 5 ms; ±20 dB | Chosen: averages a late field's fine structure at 22.05 kHz; a silent gap counts as a clear miss without swamping |
| `ENVELOPE_WEIGHT_PER_DB` | 0.02 | Measured over the ten round trips on the eight-line network while Size was searched: mean \|ln Size ratio\| 0.549 with the profile alone, 0.224 at 0.005 and 0.01, 0.170 at 0.02 and 0.04; worst Diffusion error 0.81 → 0.20, worst pre-delay 16.7 → 8.0 ms. The smallest weight on the plateau. Kept, not re-measured, for Diffusion and the first arrival alone |
| `DENSITY_FLOOR_DECAY_SHARE`, `DENSITY_FLOOR_MIN_S` | 0.15 of the longest band decay; at least 1 s of total delay | §3.4 and §13 step 1's modal density floor. Measured at P3.5, before the output taps: at the floor 17 of twenty responses were within every tail-texture limit, against 14–15 with Size searched up to 1.25–2 times it and 10 with no ceiling; with the taps, 19 at the floor (*Size is the density floor*). The 1 s minimum over 0.5 s is `crates/mxm-classic-verb-dsp/AGENTS.md`'s |
| `DIFFUSION_GRID` | 0 to 1 in 0.2 | Chosen; refined by the pattern search |
| `REFINE_STARTS` | 4 | Measured while Size was searched: from the best grid point alone a plate stopped at 0.0253 against its truth's 0.0089, 41 % short on Size. Kept, not re-measured, for Diffusion alone |
| `REFINE_MAX_EVALUATIONS`; `REFINE_MIN_DIFFUSION_STEP` | 24; 0.02 | Chosen: a cost bound; the step is inside the round trips' measured spread |
| `WIDTH_BISECTIONS` | 20 | Chosen |
| `VERIFY_LEAD_S` | 50 ms | Chosen: a lead for the onset rules, well past `ONSET_GUARD_S` |
| `WET_PEAK_CEILING` | 0.5 | Chosen: no wet sample may pass for the direct sound |
| `REFLECTION_MATCH_S` | 1 ms | Chosen: twice the analyser's reflection half-width |
| `least_squares` (private) | 60 iterations, difference step 1e-3 | Chosen; fixed, so deterministic |

## Chosen constants — the tail texture

Public ones are in `src/lib.rs`. **The reference** is the measurement made outside the repository on
2026-09-15 (*The tail texture*); its values are its own choices and were not tuned here.

| Constant | Value | Source or reason |
|---|---|---|
| `TEXTURE_ORIGIN_S` | 3 ms | The reference: just past the direct-sound window's ±2.5 ms |
| `TEXTURE_A_START_T60`, `TEXTURE_A_START_MIN_S`, `TEXTURE_A_START_MAX_S` | 0.15 T60, within 20–100 ms | The reference |
| `TEXTURE_A_LENGTH_T60`, `TEXTURE_A_LENGTH_MIN_S`, `TEXTURE_A_LENGTH_MAX_S` | 0.5 T60, within 100–600 ms | The reference |
| `TEXTURE_B_MAX_S`, `TEXTURE_B_MIN_S` | 800 ms; 150 ms | The reference |
| `TEXTURE_MIN_SEGMENT_S` | 20 ms (`ECHO_DENSITY_WINDOW_S`) | Ours: one density window, where the reference has no floor for A |
| `TEXTURE_USABLE_WINDOW_S` | 20 ms, at `NOISE_MARGIN_DB` | The reference's window and margin; the floor is Lundeby's |
| `TEXTURE_POWER_OF_TWO_FROM` | 2048 frames | The reference; lets both transforms be radix 2 |
| `TEXTURE_COMPENSATION_WINDOW_S` | 30 ms | The reference |
| `PEAKINESS_LOW_HZ`, `PEAKINESS_HIGH_HZ`, `PEAKINESS_HIGH_RATE_FRACTION` | 150 Hz; 10 kHz or 0.4 fs | The reference |
| `PEAKINESS_BANDS_PER_OCTAVE`, `PEAKINESS_MIN_BINS` | 6; 10 bins | The reference |
| `TEXTURE_DENSITY_HOP_S` | 5 ms, rounded | The reference's hop, rounded as every window here |
| `PERIODICITY_ENVELOPE_S`, `PERIODICITY_TREND_S` | 1 ms; 50 ms | The reference |
| `PERIODICITY_MIN_LAG_S`, `PERIODICITY_MAX_LAG_S` | 3 ms or the main lobe's end; 250 ms or half the segment | The reference, corrected the same day: searched from 3 ms regardless, a dark tail read its own main lobe, 0.78 at 3.0 ms |
| `COMPENSATION_FLOOR`, `LOG_FLOOR` (private, `texture.rs`) | 10⁻³⁰; 10⁻¹² | Representational, as the reference's |

## Measured accuracy — the analyser

On the planted room at 48 kHz — five reflections, a tail from 45 ms with T60 falling from 1.4 s at
125 Hz to 0.6 s at 8 kHz, a noise floor 51–57 dB under the tail — worst errors over many realisations,
per band from 125 Hz to 8 kHz. The tests assert on fixed seeds with the tolerances in the last
column.

| Reading | Worst measured | Asserted |
|---|---|---|
| T30, mono (12) | 9.7, 8.5, 4.0, 6.3, 1.8, 3.1, 5.4 % | 12 % to 250 Hz, 8 % above |
| T30, stereo (24) | 8.6, 5.9, 3.4, 4.6, 1.8, 3.1, 5.5 % | the same |
| T20, mono (12) | 16.4, 7.9, 6.8, 7.6, 3.7, 3.1, 5.7 % | 20 % at 125 Hz, 10 % above |
| EDT, tail from the direct sound (24) | 34, 31, 18, 12, 11, 8, 11 % | 40 % to 500 Hz, 15 % above |
| Tone against 1 kHz (24) | 3.2, 2.0, 1.3, —, 1.5, 1.6, 1.7 dB; mean under 0.4 dB | 4 dB at 125 Hz, 2.5 dB |
| Band noise floor (12 mono, 24 stereo) | 1.9 dB | 2.5 dB |
| Intersection time (24) | 10.7 % of T60 | 15 % |
| DRR | 0.002 dB | 0.05 dB |
| Reflection delay and level; pre-delay to a reflection | to the sample; 0.005 dB | 1 sample; 0.1 dB |
| Pre-delay to a tail with no reflection (24) | 0.52 ms late | 0.75 ms |
| Mixing time (32, at 44.1 and 48 kHz) | median 4.0 ms, 90th percentile 11.8 ms, worst 83 ms | 15 ms on seeds 1–3 (+1, −2, +1 ms) |
| IACC, decorrelated / identical (24) | ≤ 0.153 / 1.000 | < 0.25 / > 0.999 |
| T30, one response at 48 and 96 kHz (6) | 0.26 points apart | 0.5 points |
| T30, a 0.2 s decay in 0.3 s (8 each at 22.05, 48 kHz) | 27.6, 13.7 %, then ≤ 8.6 % | 35, 20, 12 % |

**Noise compensation.** With the floor at −75 dB and 3 s of record, mean |T30 error| on two seeds:
method E 3.5 % and 2.6 %, C 5.7 % and 4.8 %, A 630 % and 613 %; C read longer than E in every band.
With the floor at −85 dB, A read +63 % to +95 % at 8 kHz where E read +3.8 % to +5.2 %.

**The noise-margin sweep** (seven floors, four realisations): under 25 dB of margin no T20; 25–30 dB,
T20 in 25 of 28 bands; 30–35 dB, every T20 (≤ 16 %) and no T30; 35–40 dB, T30 in 25 of 29
(≤ 14 %); 40–45 dB, T30 ≤ 9.8 %; 45–50 dB, ≤ 7.4 %; above 50 dB, ≤ 4.0 %.

**Non-responses.** Refused: a steady sine, sustained noise, random noise bursts, a linear fade, a
gated block, a reverse swell, a single impulse, a DC step, a 50-sample burst and a Nyquist tone.
**Not refused, and recorded as the confidence's limit**: a run of decaying sine notes (three
realisations in four), a noise staircase falling 15 dB every 250 ms, and a hold-then-decay. Their
per-band decays really are exponential or nearly so; P3.5 decides against real material whether
prominence should weigh more.

## The tail texture — measured

`measure_the_texture_populations` (release, Windows): each plant at 44.1 and 48 kHz, mono and stereo,
seeds 1–8, on `tests/support`'s `TexturePlan` — a unit direct sound, the tail 20 dB under it and 70 dB
over a white Gaussian floor, recorded until a tenth of the response is floor. Ranges over every
realisation that read the segment:

| Plant | T60 | Segment | Peakiness | Kurtosis | Echo density | Periodicity |
|---|---|---|---|---|---|---|
| Decaying Gaussian noise | 0.6, 1.5, 3 s (96) | A (96) | 1.803–2.014 | 2.912–3.073 | 0.977–1.015 | 0.094–0.203 |
| | | B (64; none at 0.6 s) | 1.798–1.998 | 2.939–3.047 | 0.987–1.013 | 0.064–0.163 |
| Modes 30 Hz apart | 0.8, 1.2, 2 s (96) | A, 341–372 ms (96) | 4.817–5.572 | 2.923–3.060 | 0.987–1.015 | 0.108–0.163 |
| | | B, 85–93 ms (32) | 1.411–1.547 | 2.833–3.155 over all B | 0.957–1.028 over all B | 0.070–0.256 over all B |
| | | B, 171–186 ms (32) | 2.430–2.799 | | | |
| | | B, 683–743 ms (32) | 9.396–10.925 | | | |
| Clicks 50 a second | 0.8, 1.2, 2 s (85 of 96; the analyser refused 11 for a band's decay or margin) | A (85) | 1.701–1.923 | 532–1,019 | 0.174–0.451 | 0.166–0.430 |
| | | B (74) | 1.378–1.970 | 517–1,111 | 0.060–0.567 | 0.042–0.639 |
| A 2 ms burst every 20 ms | 0.8, 1.2, 2 s (96) | A (96) | 8.919–11.143 | 21.6–33.3 | 0.222–0.250 | 0.887–0.899, at 20.000 ms |
| | | B (89) | 2.254–20.964 | 21.6–37.4 | 0.223–0.250 | 0.656–0.947, at 20.000 ms |

`tests/texture.rs` asserts two or three realisations of each against those, with headroom: noise
peakiness 1.65–2.2, kurtosis 2.8–3.2, echo density 0.95–1.05, periodicity under 0.3; modes peakiness over
4 in A and in a B over 0.6 s; clicks kurtosis over 100 and echo density under 0.7; the burst periodicity
over 0.5 at 20 ± 0.1 ms. **Random clicks can read a periodicity of 0.64** over a short B: periodicity
alone does not tell flutter from sparse echoes, and kurtosis and echo density do.

## Measured accuracy — the fit

**Round trips** (`the_round_trip_population`, release, Windows). The engine renders a hand-authored space
(taps shifted so the first sounds at the pre-delay) at known Decay, Diffusion and Pre-delay and at the
density floor's Size, 56.3 ms — the Size the fit calculates, which every round trip recovers exactly —
everything else at the fit's neutral, as a response: 50 ms lead, a unit direct sound, the wet at 0.5,
noise at 1e-5 (`tests/known`). Diffusion, Pre-delay and Decay are fitted against true; the rest are the
verification's errors, T30 and Tone each case's worst band:

| Case | Decay, Diffusion, pre-delay | Arrival | Diffusion | Pre-delay | Decay | T30 | Tone | Profile | Envelope | Early/late |
|---|---|---|---|---|---|---|---|---|---|---|
| chamber, 48 kHz | 1.6 s, 0.7, 12 ms | Reflection | −0.02 | 0.00 ms | +1.0 % | 7.1 % | 2.20 dB | 0.037 | 0.59 dB | +1.03 dB |
| hall, 44.1 kHz | 2.4 s, 0.5, 25 ms | Reflection | 0.00 | +0.01 ms | +0.5 % | 1.5 % | 2.81 dB | 0.041 | 0.59 dB | +0.05 dB |
| room, 48 kHz | 0.8 s, 0.9, 4 ms | Reflection | +0.03 | 0.00 ms | −1.9 % | 7.0 % | 3.09 dB | 0.017 | 0.89 dB | +0.65 dB |
| plate, 48 kHz | 2.0 s, 1.0, 0 ms | LateField | −0.05 | +3.06 ms | +2.0 % | 3.3 % | 1.65 dB | 0.028 | 0.47 dB | 0.00 dB |
| room, 44.1 kHz mono | 1.0 s, 0.3, 8 ms | Reflection | 0.00 | 0.00 ms | −6.0 % | 13.2 % | 4.20 dB | 0.031 | 1.38 dB | 0.00 dB |
| hall, 96 kHz | 1.8 s, 0.8, 30 ms | Reflection | −0.02 | 0.00 ms | +0.7 % | 3.0 % | 1.28 dB | 0.031 | 0.44 dB | −0.01 dB |
| chamber, 44.1 kHz | 1.2 s, 0.4, 20 ms | Reflection | 0.00 | 0.00 ms | −0.5 % | 10.8 % | 2.69 dB | 0.019 | 0.68 dB | +1.02 dB |
| plate, 44.1 kHz | 1.4 s, 0.6, 10 ms | LateField | +0.32 | +13.26 ms | +3.5 % | 4.0 % | 2.31 dB | 0.144 | 4.39 dB | +0.04 dB |
| room, 44.1 kHz | 0.6 s, 0.6, 2 ms | LateField | +0.40 | +4.93 ms | −2.0 % | 9.1 % | 3.25 dB | 0.051 | 1.21 dB | +0.60 dB |
| hall, 48 kHz | 3.0 s, 0.9, 15 ms | Reflection | 0.00 | 0.00 ms | −0.7 % | 0.8 % | 0.60 dB | 0.029 | 0.34 dB | 0.00 dB |

Worst over the ten, with four decay bands: Decay −6.0 %, Diffusion +0.40, pre-delay +13.3 ms, band
ratios −0.091 low, −0.172 high and −0.143 top, width 0.039 (stereo), early level against the strongest
tap −42 %; verification T30 13.2 %, tone 4.20 dB, profile 0.144, envelope 4.39 dB, early-to-late
1.03 dB, IACC 0.001; reflections matched 11 or 12 of 12. The hand-authored spaces' upper ratios were
re-authored for the fourth band (the DSP crate's `space.rs`), so these responses are not the three-band
population's; there the worsts were Decay −7.3 %, T30 15.6 % and tone 2.67 dB. No clamp and no verification refusal. `tests/fit_round_trip.rs` asserts three of the
cases (chamber 44.1, plate 48, mono room) against the population's worst with headroom, and every Size
to the floor it was rendered at.

**Dense onset** (`measure_the_dense_onsets`: `Plan::room`, stereo at 48 kHz, 3.5 s, no planted
reflections, its tail starting 0.5 ms after the direct sound with T60 2.2, 2.0, 1.8, 1.6, 1.4, 1.25 and
1.1 s from 125 Hz to 8 kHz; the second column adds ten reflections of gain 0.3, evenly from 15 to 45 ms):

| | No reflections | Ten reflections at 15–45 ms |
|---|---|---|
| Fitted Size, Diffusion, arrival, pre-delay | 56.3 ms, 0.70, LateField, 1.42 ms | 56.3 ms, 0.58, Reflection, 2.67 ms |
| Taps kept; reflections beyond reach | 6; 0 | 12; 0 |
| Profile, envelope | 0.138, 2.63 dB | 0.109, 2.10 dB |
| Mixing time error | +14 ms | +10 ms |
| T30, tone, early-to-late | 1.6 %, 1.48 dB, +0.26 dB | 1.7 %, 1.86 dB, +0.26 dB |
| Reflections matched | 6 of 6 | 12 of 12 |
| High cut; clamps | 8.2 kHz; ToneHigh −28.2 → −24 dB | 8.2 kHz; ToneHigh −28.5 → −24 dB |

**Steep tilts** (`measure_the_steep_tilts`: `Plan::room`, stereo at 48 kHz, noise −95 dB; band levels
against 1 kHz from 125 Hz to 8 kHz — dark 3, 2, 1, 0, −6, −18, −30 dB; bright −12, −8, −4, 0, 6, 12, 18;
low cut −30, −18, −6, 0, 1, 2, 3):

| Plant | High cut | Clamps | Worst tone error | Worst T30 error |
|---|---|---|---|---|
| Dark | 1.67 kHz | none | 2.3 dB at 500 Hz; 8 kHz +1.3 dB | 5.2 % at 500 Hz |
| Bright | open | none | 2.5 dB at 125 Hz | 5.7 % at 1 kHz |
| Low cut | 2.81 kHz | ToneLow −48 → −24 dB | 5.4 dB at 2 kHz; 8 kHz −2.2 dB | 6.8 % at 250 Hz |

None has a verification refusal. On three decay bands, solved at band centres, the dark render's high
bands read long — +8.8, +6.0 and +6.7 % at 2, 4 and 8 kHz — a band held some 28 dB under a slower
neighbour reading that neighbour's leak (*Second-order bands leak*, below); solved across each band they
read +3.4, −1.1 and +4.7 %. `a_shelf_at_its_bound_does_not_kill_a_band_in_the_render` holds the
dark and low-cut plants to keeping every T30, and allows a straightness note and no other refusal.

**The high cut on planted rooms** (`measure_the_high_cut_on_planted_rooms`: `Plan::room` with a white
late field; 24 realisations of each plant — stereo seeds 1–8 and mono seeds 1–4, at 44.1 and 48 kHz — and
no clamp or verification refusal in any):

| Plant | Cut taken | Corner | Worst tone error at 2, 4 and 8 kHz |
|---|---|---|---|
| −12 dB/oct above 3 kHz | 24 of 24 | 2.16–3.76 kHz | 1.10, 1.71, 2.87 dB |
| White | 1 of 24 | 10.6 kHz, moving the tone below 8 kHz by 0.18 dB | 1.08, 1.74, 2.50 dB |
| +2 dB/oct above 2 kHz | 0 of 24 | — | 2.05, 2.29, 1.75 dB |

`a_steep_rolloff_is_fitted_with_a_high_cut_instead_of_a_clamped_shelf` and
`a_flat_or_bright_response_keeps_its_high_cut_open` assert one realisation of each against these.

**The pack's aggregate tone curves** (`measure_the_packs_tone_curves`: planted the same way, white below
2 kHz, seed 1, stereo) at 44.1 and 48 kHz:

| Band levels against 1 kHz at 2, 4, 8 kHz | Cut | Worst tone error | Tone residual as carried, and with the cut open | Clamps |
|---|---|---|---|---|
| Clamped median: +2.0, +1.1, −4.5 dB | 6.2; 4.4 kHz | 2.12; 1.98 dB | 0.30, 2.40; 0.56, 2.54 dB | 44.1 kHz: ToneHigh −25.3 → −24 dB |
| Clamped tenth percentile: −3.0, −10.2, −17.2 dB | 1.0; 1.2 kHz | 2.31; 2.09 dB | 0.66, 9.10; 0.83, 9.19 dB | none |
| Unclamped median: +2.8, +5.0, +6.4 dB | 11.4; 9.6 kHz | 2.23; 2.15 dB | 0.10, 0.20; 0.48, 0.62 dB | none |

**Cost** (`measure_the_cost`, release, Windows, one fit of a planted room, three runs each, with four decay
bands): 0.49 s (2 s stereo at 44.1 kHz), 0.48–0.49 s (3 s), 0.47 s (3.5 s), 0.56 s (6 s); mono 3 s 0.40 s;
96 kHz stereo 4 s 1.14–1.15 s — on three bands 0.41–0.47 s at 44.1 kHz and 1.04–1.06 s at 96 kHz. It barely
grows with length, so the search's renders dominate. **On the pool** of *Size is the density floor*, fitted
on 10 workers at once: a median 0.54 s and a slowest 1.66 s per response, and the 345 responses in 39 s
(0.47 s, 1.55 s and 35 s on three bands).

## Findings P3.5 should know

- **The mixing time has a long tail.** On a decaying tail the profile hovers just under one, so
  "first reaches one" is a first passage; one planted room in 32 read 83 ms late. The search
  compares profiles, not this one number.
- **Second-order bands leak.** A band beside a slower-decaying one reads long: +3.7 % at 8 kHz
  beside a 4 kHz band 25 % slower. Worse at the bottom: a planted 125 Hz T60 of 0.06–0.15 s reads
  0.37–0.45 s through 250 Hz leakage — an **analyser artefact** that measures a band rather than
  refusing it. And a mid band with no content of its own still reads a decay from its neighbours.
- **Low bands are noisy on one response.** An octave at 125 Hz is 88 Hz wide; a single realisation's
  T30 spreads by ±10 % and its tone by ±3 dB.
- **Short rooms pay for the floor.** Where a decay spans about ten trips round the network — the 0.6 s
  room at 56.3 ms — it is a staircase of returns, which the DSP crate's calibration does not cover
  (`crates/mxm-classic-verb-dsp/AGENTS.md`, *The decay rule*). The 1.0 s mono room carries the round
  trips' worst Decay (−6.0 %), early level (−42 %), T30 (13.2 %), tone (4.20 dB, at 8 kHz) and top band
  ratio (−0.143); the 0.6 s room the worst Diffusion (+0.40) and high and low ratios (−0.172, −0.091).
  **On the pool, short rooms gained from four bands**: under a 0.8 s mid-band decay the worst band per
  fit fell from a median of 13.0 % to 10.3 %, 25 of 81 fits better by more than 3 points and 2 worse.
- **A diffuse onset is measured late.** Where the first arrival is taken as the late field, the pre-delay
  lands late: +3.1 ms (the plate at 48 kHz), +4.9 ms (the 0.6 s room) and +13.3 ms (the plate at
  44.1 kHz), where `late_onset_s` takes 1.25 ms off. Read as the analyser placing the pre-delay where a
  diffuse onset's energy first rises clear, which is later than it starts (*not measured* separately).
  Every round trip read as `Reflection` is within 0.01 ms.
- **A response dense from its first milliseconds** fits within the round trips' spread on the planted
  dense onset (profile 0.138, envelope 2.63 dB, mixing time +14 ms; *Dense onset*). On real responses,
  three dense within 10 ms of the direct sound stay short in their first window (the DSP crate's *Output
  taps inside the lines*).
- **Every reflection is within tap reach at the floor.** *Derived*: three Size units is 169 ms, past the
  analyser's 100 ms early window even after a `LateField` shift, so the selection no longer weighs reach.
  Every fit measured here reported none beyond it.
- **A steep low cut is still past the tone's reach.** The high cut carries a high fall that no
  first-order shelf could follow (*Steep tilts*, above), but there is no low counterpart: on the
  planted low-cut room the low shelf asks −48 dB, and the least squares borrows the high shelf's leak
  into 1 kHz through the normalisation to reach further, trading the high end — 125 Hz +3.9 dB against
  2 kHz +5.4 dB and 8 kHz −2.2 dB, with a 2.81 kHz cut. A band that is not single-slope no longer drives
  a shelf; EDT against T30 is not used.
- **The top octave — plan D5, taken.** Four decay bands in the DSP crate and the solve read across each
  band (*Decay is solved across each band*). In closed form at Size 56.3 ms and a 1.5 s mid band, both
  upper ratios at their 0.1 bound give 8 kHz 0.24 s, where three bands could not get under 0.41 s.
  **Still short of some rooms:** 29 of the pool's fits ask for a top ratio under 0.1.
- **The 8 kHz tone error widened on the pool** with four bands: p90 5.2 → 6.4 dB, with the signed median
  unchanged (−0.1 dB) and the change per fit a median of 0.0 dB in every range of top ratio. Its tail is
  fits whose high shelf and cut sit at their bounds, some moving tens of dB either way between fitters;
  nothing measured ties it to the decay change (the change in 8 kHz tone against the change in 8 kHz T30
  correlates −0.14).
- **A planted room with a gap before its tail fits its early part poorly.** `Plan::room` holds five
  reflections from 7.1 ms and a tail starting abruptly 45 ms after the direct sound — the gap the output
  taps were added to fill. Its nine tone plants (*Steep tilts*, *The pack's aggregate tone curves*) fit
  with Diffusion 0–0.45 and a profile distance of 0.17–0.24, seven of them with the first reflection as
  the first arrival; six match 1–3 of the 12 reflections compared and read the mixing time 45–60 ms late.
  Why is *not measured*.
- **The tail texture is reported and not searched.** On the four synthetic factory spaces — the
  engine's own renders of its hand-authored spaces, fitted back at the floor — the responses read
  peakiness 1.97–2.49 in A, and the texture errors are small: peakiness within ±0.38, kurtosis ±2.04
  (the hall's A, where the response reads 8.3 and the render 6.2), echo density ±0.03, periodicity
  ±0.06. The Diffusion search does not read any of it; the Size rule was chosen on it (*Size is the
  density floor*).
- **A tone change moves the search.** Every candidate render carries the tone, so a cut changes the
  echo density profile and envelope Diffusion and the first arrival are chosen on, which is why a cut
  must earn `HIGH_CUT_MIN_IMPROVEMENT_DB`.
- **Pool refusals** (reported by the orchestrator, build fae3577; this crate did not re-run the pool):
  - **Render stage (23).** The cause was the tone model: it normalised each band by its own weights,
    removing a white field's +3 dB per octave, and extreme tilts ran unbounded. Both are fixed, and a
    render the analyser would refuse is reported, not refused.
  - **Response stage (179), the rule unchanged.** A 125 Hz `NoiseMargin` reproduces synthetically
    only as the response's property: a weak low band near a raised floor. `TooShort` on a 1.4 s
    response cannot be the length check; it is Lundeby's step 5 re-average block (2 / |slope| s)
    exceeding half the response. That happens when the first line falls slower than about 3 dB/s, so
    the name misleads. It is neither the band filter nor the time reversal. Onset refusals are
    consistent with wet-only responses, which have no direct sound 20 dB over what precedes. How
    `BAND_REQUIRED_WITHIN_DB` bites: with a broadband direct sound every band is required.
- **The P3.5 pool** (reported by the orchestrator, aggregates only): 154 responses outside the springs
  folder were refused by every fitter of *Size is the density floor*. Of the hundred factory candidates
  chosen on this fitter with four decay bands, 37 carry a clamp (33 on three bands), and the hundred's
  worst per-band T30 error has a median of 5.6 % and a p90 of 30.8 % (10.4 % and 33.3 %); the selection
  is the plan's.

# Work Guidance

- **Do not open an implementation** — pyrato, ITA-Toolbox, Faust, zita or any acoustics code. The
  research page's prose and equations are the source (root *Research boundary*).
- **A threshold moves only with a new measurement**, and the comment beside its assertion quotes the
  figure. The planted generator stays independent of the analyser: brick-wall octaves from the
  transform, never this crate's Butterworth filters. The round trips render with the engine, never
  with the fit's own model.
- Tolerances are argued from populations of realisations or cases, not from the seeds asserted, so a
  changed seed is not a changed claim.
- **Third-party impulse responses never enter either repository**, nor anything derived from them
  (reports, audio, figures per file). Run the examples on them only to check decoding, write output
  outside the tree, and report only aggregate figures. **The one exception is plan §5.1's and §8's**:
  the factory spaces `classic_verb_generate` writes — each fitted space, its report named by an id we
  choose, and the generated module — are committed, because the plugin ships them and its audit
  re-verifies them without the responses. The generator's refusals keep paths, file names and
  folders out of them.

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

Never `--workspace` from this crate while the collection's other members are being written beside
it.

Last run (2026-09-17, Windows, with fallible buffers): 63 tests pass and 8 are ignored (18 unit, 8
planted, 10 refusals, 2 robustness with the ceiling's timing ignored, 5 texture with the population
measurement ignored, 10 fit contract, 3 fit round trip with the population ignored, 6 generate, 1
memory, and the 5 measurements of `fit_measure`); the debug tests take about 42 s. Clippy with
`-D warnings` reports nothing; `cargo fmt -p mxm-classic-verb-fit --check` is clean; the same 63 pass on
1.87.0; `cargo tree -e normal,build` lists `mxm-classic-verb-dsp` alone; and the plugin's release space
audit reads every committed figure as before. The round-trip population, `fit_measure`, the texture
populations and the synthetic regeneration were last run on 2026-09-15 (four decay bands) and not since:
reserving fallibly changed no value. Linux and macOS are not verified by anything here.

# Child DOX Index

No child AGENTS.md files.
