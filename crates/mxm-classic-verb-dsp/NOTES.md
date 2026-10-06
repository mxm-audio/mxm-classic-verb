# NOTES.md — crates/mxm-classic-verb-dsp/

The detail behind this folder's AGENTS.md: history, measurements, rationale and worked examples.
AGENTS.md is the contract; this file is the reference it links to.

## Files

- `src/lib.rs` — `Engine`, `Controls`, `DecayShape`, the composition laws, transitions, parking,
  activity and the tail declaration, and every public bound.
- `src/space.rs` — `Space`, `EarlyTap`, the form's version, sanitising, interpolation and the four
  hand-authored spaces.
- `src/network.rs` — the sixteen-line network, the allpasses inside its loops and their group delay,
  its output taps, its matrix, its patterns, fractional reads, total delay and late onset.
- `src/filter.rs` — the per-line decay filter, its bilinear lowpasses and its bound, the output tilt
  and the space's high cut, the diffusion allpass.
- `src/shaped.rs` — the feed-forward path for gated and reverse decays.
- `src/delay.rs` — delay storage with constant-time invalidation, the follower coefficient, the
  smoother and the ramp.
- `tests/engine.rs` — the engine's contracts, measured; `examples/classic_verb_render_demo.rs`.

## Why `SPACE_VERSION` is still 1

**`SPACE_VERSION` moves when a field is added, removed or reinterpreted — from release on.** It
stayed 1 when `high_cut_hz` was added, and when `decay_ratio_top` was added and `decay_ratio_high`
moved from the 8 kHz octave to the 4 kHz one (plan D5): nothing is released and no space is stored
anywhere, so there is no earlier form a version would have to tell apart. The in-plugin loading work
serialises `Space`, and takes both fields with it.

## The public tone curve and its tests

**The tone normalisation is not optional.** First-order shelves leak: a +12 dB low shelf at
250 Hz lifts 1 kHz by about 3 dB, which is a mix control wearing a tone label.
`the_tilt_normalised_at_its_reference_leaves_the_mid_band_at_unity` holds the closed form against
the filter and the normalised result within 1 %.

**`tone_magnitude(low_db, high_db, high_cut_hz, sample_rate, hz)` is that normalised curve, public,
beside `predicted_decay_s`.** The shelves are totals (space plus offset) and the corner is taken as
given, unbounded like them. It is for anything that has to predict the tone rather than render it.
The fit's tone solve is held to it by `mxm-classic-verb-fit`'s
`the_tone_model_is_the_engines_public_tone_curve`, so a change here fails there instead of silently
skewing fitted spaces. `the_public_tone_curve_is_the_engines_normalised_tilt` holds it against the
rendered shelves and cut within 1 % at 44.1, 48 and 96 kHz, the cut open, at its floor, at 3 kHz and
at 9 kHz — a stopband about 60 dB down included, which is why that test's sine has its phase in f64.

## The space's high cut

| Field | Bound | Non-finite reads |
|---|---|---|
| `high_cut_hz` | `MIN_HIGH_CUT_HZ` (1 kHz) to `HIGH_CUT_OPEN_HZ` (20 kHz); at the top the cut is **open** | open |

**What it is for.** A first-order shelf's transition cannot fall steeply above 4 kHz at any depth.
On the owner's 536-response pack (P3's fitter, aggregates only) 45 % of fits drove the high shelf to
its clamp and left a median 8 kHz tone error of 7.6 dB: their targets fall a median 9.1 dB an octave
faster than a white field between 4 and 8 kHz (tenth percentile 14.9), where the shelf gave 3.2.
That is air absorption and a darkened source, and the remedy is a lowpass with a corner of its own.

**The form: a second-order Butterworth lowpass** by the bilinear transform with its corner
prewarped (Butterworth, 1930; Bristow-Johnson's cookbook form at Q = 1/√2), after the shelves, in
transposed direct form II with f64-computed coefficients. Its magnitude is exactly
`1/√(1 + (tan(ω/2)/tan(ω_c/2))⁴)`: 3 dB down at the corner at every rate, never above unity.

### Why second order

**Why that order — measured on the fit's band model** (the analyser's octave shape over a white late
field, the shelves solved with it, scored with the shelves inside ±24 dB), on tone curves built from
the pack's figures:

| Form | Reach alone, 48 kHz: 4→8 kHz slope re white, and 2 kHz re white | Worst band error over eight dark curves (floor 1 kHz, 48 kHz) |
|---|---|---|
| Shelves only | −3.3 dB/oct at −24 dB | 20.3 dB |
| First order | at most −6.5 dB/oct (1 kHz corner, 2 kHz −4.0) | not a candidate: cannot reach −9 |
| Two one-poles (critically damped) | −8.4 dB/oct at 3 kHz, 2 kHz −2.3 | 4.4 dB |
| **Butterworth, second order** | **−11.4 dB/oct at 3 kHz, 2 kHz −0.9; with the shelf at −24 dB, −14.2** | **3.2 dB** (median 0.8) |
| Butterworth, third order | — | 4.7 dB (median 1.1) |
| Butterworth, fourth order | −18.2 dB/oct at 4 kHz, 2 kHz −0.1 | 7.6 dB (median 1.3) |

The eight curves are the clamped median, that median with the tenth percentile's 4→8 kHz slope, the
tenth percentile in every band, a darker 4 kHz with that slope, −12 dB/oct above 3 kHz,
−15 dB/oct above 4 kHz, and two f² air-absorption curves. Third and fourth order win on the abrupt
ones and fail the gradual ones; second order is the only one within 1 dB on the gradual curves and
within 3.2 dB on every curve.

### What the high cut's tests measured

**Open is exact.** At or above `HIGH_CUT_OPEN_HZ`, or for a non-finite corner, the cut is bypassed
and its reference magnitude is exactly one, so an open space renders sample for sample as it did
before the cut existed: `an_open_high_cut_is_bypassed_to_the_bit`. **The four hand-authored spaces
are open**, and every figure in this file was re-measured on the sixteen-line network.

- **Normalised with the tone.** Measured through the engine, two engines differing only in the cut
  (Hall at 2.5 kHz) against the closed form's ratio at 250 Hz, 1, 2.5 and 8 kHz and 44.1, 48 and
  96 kHz: within 0.001 %, and exactly 1.00000 at 1 kHz
  (`the_high_cut_follows_its_closed_form_through_the_engine_at_every_rate`).
- **The floor bounds the normalisation, and is measured.** At `MIN_HIGH_CUT_HZ` the cut is 3 dB down
  at the reference, so the division lifts the rest of the tone by at most 3 dB. On the same band
  model at 48 kHz, with every band at the tenth percentile of the pack's clamped fits (2, 4 and 8 kHz
  at −3.0, −10.2 and −17.2 dB against 1 kHz), the worst band came within 0.77 dB with the floor at
  1 kHz, 1.29 dB at 1.5 kHz and 2.96 dB at 2 kHz.
- **A space change reaches the cut through the fade.** Its coefficients change only where the space
  in force does — at the fade's silent point, or while parked — and a cut closing from open starts
  from rest. Measured on the sustained 220 Hz tone: output identical to a shelf-only change up to the
  silent point, and worst step 0.0133 across a change to a cut at its floor, 0.0133 without one
  (`a_space_change_reaches_the_high_cut_at_the_fades_silent_point`).
- **Rate and block independence.** The corner is in hertz and held below 0.45 of the rate as every
  corner is; `the_high_cut_stays_finite_at_its_floor_and_past_nyquist_at_every_rate` runs 1 kHz,
  5 kHz and 19.999 kHz at 8–192 kHz with full-scale noise and both shelves at +24 dB. Sending the same
  settings again at any block boundary renders the same samples
  (`settings_sent_again_at_block_boundaries_leave_the_high_cut_untouched`).

## Sixteen lines, allpasses in the loop

Plan D6, taken by measurement at P3.5.

**Why.** The owner heard the first fitted spaces as not diffuse enough, like a lot of delays, and
some as completely off (2026-09-15). Measured against their responses with the fit crate's tail
measures, the eight-line network rang: the fit had chosen lines of 2–14 ms, a sparse comb of high-Q
modes that the echo density profile it scored cannot see. Raising Size to fix the ringing turned the
first half-second into separate echoes instead.

**What decided it.** Twenty of the owner's responses, each fitted once, rendered through each variant
with the fitted early reflections held at their times, and measured against the response on two
segments of the tail. *Excess* is the variant's reading less the response's, worst over the two
segments; aggregates only, no response is named:

| Variant | Ringing excess, p90 / worst | Spikiness excess, p90 / worst | Periodicity excess, p90 / worst | Pairs within all four limits |
|---|---|---|---|---|
| Eight lines at the fitted Size (the engine before) | 15.7 / 21.9 | 3.5 / 6.8 | 0.15 / 0.22 | 4 of 20 |
| Eight lines, Size raised to a total delay of 0.15 × the longest band decay | 0.35 / 2.82 | 38.7 / 63.5 | 0.35 / 0.55 | 9 |
| Sixteen lines, no loop allpasses, total delay 0.5 × that decay | 0.07 / 0.67 | 27.0 / 67.6 | 0.35 / 0.55 | 11 |
| **Sixteen lines, two loop allpasses of 1.3–4.9 ms at 0.6, total delay max(0.15 × that decay, 1 s)** | **0.12 / 0.27** | **0.64 / 1.19** | **0.06 / 0.28** | **17** |
| The same, loop allpasses four times longer | 0.18 / 1.76 | 1.43 / 2.46 | 0.01 / 0.11 | 17 |
| The same, loop allpasses eight times longer | 0.32 / 3.54 | 1.46 / 23.3 | 0.04 / 0.07 | 17 |
| The same at the chosen lengths, total delay at least 0.5 s | 0.20 / 1.02 | 0.44 / 1.19 | 0.02 / 0.08 | 18 |

The limits are ringing and periodicity excess 0.5 and 0.1, spikiness excess 2, echo density deficit
0.1. Coefficients 0.5 and 0.7 and a single allpass per loop read no better. **The 1 s minimum is
kept over 0.5 s** because ringing is what was heard; 0.5 s let one short room ring (1.02).

- **The pairs still outside the limits** are responses whose own tails are sparser than any setting
  renders — distinct echoes the sixteen-line network smooths.
- **Not settled by measurement:** character. The owner's next listening round is on this network.

### Output taps inside the lines

**Output taps inside the lines**, added after the owner's second listening (2026-09-15: "it sounds
very good now", but more pre-delay on the fits, and a first sound, a gap, then the rest). The late field
left the network only at each line's end, a whole trip after the pre-delay — at least 37 ms once the fit
put Size at its density floor — so the fitted early reflections sounded alone across the gap. Each line
is now read at `OUTPUT_TAPS` (three) points inside it besides its end (`TAP_FRACTION`), outside the loop,
as Dattorro's tank reads its output (research page §7, recipe §13 step 4); `late_onset_s` is the earliest
a sound reaches the output, for the fit's pre-delay. Measured on the same twenty pairs, fit against
response, mean energy per window:

| | 20–30 ms, median | 30–40 ms | 40–50 ms | Windows more than 20 dB short | Pairs within all tail limits |
|---|---|---|---|---|---|
| Line ends only | −15.4 dB | −9.9 dB | −6.4 dB | 37 of 160 | 17 |
| **With the taps** | **−1.3 dB** | **+1.4 dB** | **+1.3 dB** | **13 of 160, none after 30 ms** | **19** |

With the taps, spikiness excess falls (p90 0.59 → 0.16) and ringing excess rises (p90 0.10 → 0.35, worst
0.45), still under its limit. **Not fixed:** three responses dense within 10 ms of the direct sound stay
28–48 dB short in their first window, where the input diffusers are still building.

### Cost

**Cost**, release, Windows, 48 kHz stereo noise at Size 60 ms, best of three: 2.08 % of a core, and
2.71 % with 1 ms of modulation, with four decay bands — 1.90 % and 2.43 % with three, 1.27 % and
1.85 % before the output taps, 0.77 % and 1.05 % for the eight-line network.

## Four decay bands

Plan D5, taken by measurement at P3.5.

**Why.** Refitted on three bands, the owner's pack missed its top octaves: the render's T30 error p90
was 13.4 % at 4 kHz and 17.7 % at 8 kHz. One high-band ratio shaped both octaves, so the least squares
split the difference — of the 44 fits whose 8 kHz residual passed 10 %, 41 were too long at 8 kHz and
37 of those too short at 4 kHz. At the density floor a trip is about 59 ms, so a fast top octave asks
for tens of dB of loss a trip, which a first-order shelf at 4 kHz cannot take from 8 kHz without taking
much of it from 2 and 4 kHz.

**What decided it**, measured outside the repository on the pool's 345 fits: each candidate filter
solved per fit through closed forms, and the realised T30 modelled as the analyser reads a band —
per-frequency exponential decays summed through its octave filter under the fitted tone. On the
three-band filter that model missed the pool's renders at 4 and 8 kHz by p90 2.4 % and 4.7 %, where
the closed form at the band's centre missed by 9.4 % and 7.5 %. Realised T30 against the response:

| Filter; solved | 4 kHz p90 | 8 kHz p90 | Worst band per fit, median / p90 | Fits with a band over 10 % |
|---|---|---|---|---|
| Three bands, one-pole shelf at 4 kHz; at band centres (before) | 11.5 % | 15.5 % | 7.3 / 21.0 % | 128 |
| The same; across each band | 11.8 % | 9.4 % | 6.2 / 18.7 % | 93 |
| Three bands, a second-order shelf at 4, 5.6 or 8 kHz; across each band | 13.7–21.0 % | 5.5–13.3 % | 6.1–8.0 / 18.9–28.1 % | 99–132 |
| Four bands, first- or second-order upper shelves; at band centres | 5.4–8.4 % | 12.9–18.0 % | 5.4–6.8 / 20.5–25.6 % | 86–111 |
| **Four bands, bilinear first-order shelves at 2 and 5.66 kHz; across each band** | **4.6 %** | **2.6 %** | **3.3 / 12.2 %** | **41** |
| Four bands, second-order shelves at 2.8 and 8 kHz; across each band | 4.8 % | 2.6 % | 3.8 / 12.8 % | 50 |

- **Neither half is enough alone.** A fourth band solved at its centres still misses the top octave,
  and solving across each band cannot shape two octaves with one ratio. The fit's half is
  `crates/mxm-classic-verb-fit/AGENTS.md`'s *Decay is solved across each band*.
- **The corners sit on a plateau.** Bilinear pairs from 1.7–2.4 kHz with 4.8–5.6 kHz all read a worst
  band p90 of 11.8–12.3 %. Each corner is the geometric mean of the calibration octaves either side of
  it: 2 kHz between 1 and 4 kHz, 5.66 kHz between 4 and 8 kHz.
- **Bilinear, not one-pole.** The one-pole's impulse-invariant form still passes `(1 − p)/(1 + p)` at
  Nyquist; at the same corners it read 15.7 %, with the top ratio at its bound in 31 fits against 13.
  Second-order shelves read no better and cost a biquad a line.

**What the engine reaches**, in closed form at Size 56.3 ms and a 1.5 s mid band: both upper ratios at
their 0.1 bound give 4 kHz 0.39 s and 8 kHz 0.24 s, where the three-band filter could not bring 8 kHz
under 0.41 s; the top ratio alone at 0.1 gives 8 kHz 0.49 s and leaves 4 kHz at 0.88 s. At a 4 s mid
band, both at 0.1: 0.62 s and 0.46 s. Rendered (`the_top_band_decays_apart_from_the_high_band`), the top
ratio at 0.3 takes 8 kHz from 1.51 to 0.71 s and 4 kHz from 1.51 to 1.05 s; at 0.1, to 0.55 and 0.94 s.

**On the pool, refitted** (the fit crate's *Size is the density floor*): the T30 error p90 at 4 kHz
13.4 → 6.9 %, at 8 kHz 17.7 → 9.3 %, and lower in every band but 1 kHz (6.0 → 6.7 %); the worst band
per fit a median of 5.4 % against 8.1 %, and 71 fits of 342 with a band over 10 % against 143. **Not
fixed:** 29 fits ask for a top ratio under the space's 0.1 bound — a median of 0.069 — and are clamped
there.

## The network, in full

Sixteen lines at `RATIOS` × Size (incommensurate, mean exactly one), each followed inside its loop by
two short Schroeder allpasses (`LOOP_ALLPASS_S`, coefficient `LOOP_ALLPASS_GAIN`), mixed by an
order-16 Walsh–Hadamard transform scaled to be orthogonal and followed by a rotation by one line, so
the matrix is not its own inverse. A product of orthogonal matrices is orthogonal, so the network is
lossless for any delays (Schlecht and Habets, 2017). An allpass is itself a delay network, so a loop
holding one is a larger network with the same matrix and stays lossless (research page §1). The
allpasses' coefficient is fixed: a time-varying allpass coefficient inside a lossless loop can
diverge (§6). The output reads every line at its end and at the output taps, all outside the loop.
`total_delay_s` and `size_for_total_delay_s` give the network's system order in seconds, its modal
density, for the fit's floor.

Each line's decay filter is `mid + (low − mid)·LP₂₅₀` into `B₂₀₀₀ + r_high·(1 − B₂₀₀₀)` into
`B₅₆₅₇ + r_top·(1 − B₅₆₅₇)`: a one-pole lowpass, then two first-order lowpasses by the prewarped
bilinear transform (*Four decay bands*, above). **Its bound is proved, not measured:** a one-pole
lowpass's response lies in the disc on the real segment from its Nyquist gain to one, so the first
stage is at most `max(low, mid)`; a bilinear first-order lowpass's lies on the circle on the segment
from zero to one, so each shelf is at most `max(1, r)`. `BandGains::limited` holds the product under
`LOOP_GAIN_CEILING` (0.9999), which covers a scooped decay where mid dies fastest, taking a lift off
the top shelf first; a quotient that rounds the product past the ceiling is stepped down an ulp at a
time (seen at 8 kHz). Rounding in f32 coefficients leaves a bilinear lowpass's DC gain 6e-6 short of one
at a 100 Hz corner and 192 kHz, far inside the ceiling's headroom
(`the_bilinear_lowpass_is_unity_at_dc_zero_at_nyquist_and_on_its_circle`). The loop is linear and
carries no saturator.

`every_corner_of_the_domain_stays_finite_and_under_the_loop_ceiling` sweeps the corners at 8, 22.05,
44.1, 96 and 192 kHz with full-scale noise. The claim is finiteness and the gain bound: a
near-lossless loop resonates above full scale on sustained input — measured worst peak 10.6, at
8 kHz with a 60 s decay, bass ×10, treble ×0.1 and both shelves +24 dB — and the test's ceiling of 1,000 catches a
bound that has broken, which grows without limit.

## The decay calibration, measured

Per trip, each band's gain is `10^(−3·(m + τ̄)/(f_s·T₆₀))`: `m` the line, τ̄ its loop allpasses'
mean group delay over that band's calibration octave (`CALIBRATION_HZ`: 125 Hz, 1, 4 and 8 kHz).
Measured at 1 kHz on a flat space by T₃₀ from Schroeder integration
(`the_realised_decay_follows_the_composed_decay_across_size_decay_and_rate`):

- **Where a decay spans at least 25 trips round the mean line, the formula is the calibration:**
  worst 3.3 % across Size 20–150 ms, Decay 0.8–5 s, at 48 and 96 kHz, read through the output taps.
- **Below that the decay is a staircase of returns**, which a T₃₀ fit reads with a wander of its own:
  +2.0 % at Size 60 ms and Decay 0.8 s. A property of short decays in long networks, not an error.

**Why the group delay, and why T₃₀.** A Schroeder allpass's group delay swings between a quarter and
four times its delay with a period of 1/d in frequency, and a mode decays at its loss per trip over
its trip's group delay. Over a band many periods wide the mean is the delay; over the 125 Hz octave
it is not, and calibrating on the plain delay left 125 Hz 13–16 % short at Size 20–50 ms. A band
past 0.45 of the rate keeps an octave's width below it, so it still reads a mean. The same dispersion
ripples a band's early energy decay: on the same renders T₂₀ scattered ±4.4 % at 1 kHz where T₃₀
scattered ±2.6 %, so the tests read T₃₀.

**The composed band targets are not the realised octave decays**, because first-order shelves only
approach their band gains. Measured with Decay 1.5 s on a flat space
(`bass_and_treble_multipliers_tilt_the_decay_the_way_they_say`): 125 Hz reads 1.41 s flat and 2.52 s
at bass ×2 (target 3.0 s); 8 kHz reads 1.50 s flat and 0.78 s at treble ×0.5 (target 0.75 s; 0.95 s
with three bands). **The
fit (P3) accounts for this mapping; it does not assume the targets are realised.**

**`Engine::predicted_decay_s` is that mapping in closed form**, and `predicted_decays_s` the same at
many frequencies with the lines' gains computed once: each line's attenuation per sample at a
frequency — the decay filter's exact response over a trip of the line plus its loop allpasses' mean
group delay in that octave — averaged across the lines.
`the_closed_form_predicts_the_realised_octave_decays` measured it within 5.2 % (125 Hz, treble ×0.5)
of the realised octave decay at 125 Hz, 1, 4 and 8 kHz, across flat, bass ×2, treble ×0.5, bass ×0.5
with treble ×3, and the top ratio at 0.4; `the_closed_form_over_many_frequencies_is_the_closed_form_at_each`
holds the two forms to the bit.

**Measure a strongly tilted decay with a steep band filter.** With one second-order section per
octave, the slow 8 kHz tail of that last setting leaked into the 125 Hz band and read as a 36 %
prediction error that belonged to the ruler; the tests' band filter is three cascaded passes.

## Modulation and shaped decays

### Modulation reads through a Thiran allpass — plan §3.1's decision, measured

A modulated or Size-gliding line is read at a fractional delay through a first-order Thiran allpass,
with the integer part chosen so the allpass supplies 0.5–1.5 samples and its coefficient stays inside
(−0.2, 0.34). Measured at 4 kHz, Decay 2.0 s, Size 50 ms, 2 ms depth at 1 Hz
(`modulation_costs_less_high_band_decay_through_the_allpass_than_through_linear_reads`): **allpass
1.996 s, linear 1.796 s.** Linear interpolation is kept behind `Engine::set_interpolation` as the
measurement seam; it does not ship. Line rates are spread ±50 % so the lines do not beat. Depth is
clamped to a quarter of the shortest line.

### Shaped decays live outside the loop

Gated and reverse are 96 feed-forward taps per channel across the shape's length, their gains the envelope (flat; rising as u²), into two short diffusers.
Measured on an impulse over 0.5 s (`the_shaped_decays_have_their_envelopes`): gated halves 3.41 and
3.41; reverse halves 0.18 and 5.92; both below 1e-3 of their energy afterwards. The level is
`SHAPED_LEVEL`, chosen.

## Transitions and parking, measured

- **A space change leaves the paths running.** Line lengths belong to Size, not to the space, so
  nothing in the network was computed for what the swap replaces; emptying it would cut off the
  signal feeding it and start its returns with a step. The wet fades out over `SPACE_FADE_S`, the
  space swaps, and it fades back in, hiding the early pattern, tone and width jumping. Measured on a
  sustained 220 Hz tone: worst step 0.026 across a change from Hall to Room, against 0.013 in the
  Hall alone and 0.036 in the Room alone, which plays the tone louder.
- **Quiet parks only when** the input and wet levels are below `QUIET_LEVEL`, the input history no
  longer holds anything being read (pre-delay, early span, shape length), and quiet has outlasted
  `QUIET_HOLD_S` plus a trip round the longest line — or a sparse network parks between its own
  returns. Followers are times, so the decision is rate-independent: parking after the same
  hertz-defined burst measured 1.431 and 1.421 s at 48 and 192 kHz. At 8 kHz it parked at 1.471 s,
  2.8 % later: there the loop allpasses round to 10–39 samples, and with their coefficient at zero the
  three rates agreed within 1.8 %. Later is the safe direction, so the test allows 8 kHz up to 8 % later.

## Memory

Sample rates are clamped to `MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE` (8–384 kHz); the validated set is
8–192 kHz. Memory scales with the rate: the input history holds 2.5 s per channel (the longest
pre-delay plus the longer of the early span and the longest shaped decay), the network 0.43 s per
line over sixteen lines, and 32 loop allpasses under 5 ms each.

## Chosen, not measured: the full list

`RATIOS`, `LOOP_ALLPASS_S`, `MOD_SPREAD` and `MOD_PHASE`, the injection and output patterns and their
scales, `OUTPUT_TAPS`, `TAP_FRACTION` and the tap patterns, `CALIBRATION_HZ`,
`LOW_CROSSOVER_HZ`, `HIGH_CROSSOVER_HZ`, `TONE_REFERENCE_HZ`, `HIGH_CUT_OPEN_HZ`, the diffuser times and
`DIFFUSION_MAX_GAIN`, every range bound in `lib.rs`, `LOOP_GAIN_CEILING`, `QUIET_LEVEL`,
`QUIET_HOLD_S`, `SPACE_FADE_S`, `SIZE_GLIDE_S`, `MIX_RAMP_S`, `PATH_ONSET_S`, `SHAPED_TAIL_S`,
`TAIL_MARGIN`, the follower and ducking times, `DUCK_REFERENCE`, `SHAPE_TAPS` and the shaped path's
diffusers and level, and **every value in the four hand-authored spaces**. None is read from a
reference product. **Measured, not chosen:** the high cut's order and `MIN_HIGH_CUT_HZ`; the line count, the loop
allpasses against longer sets, and `LOOP_ALLPASS_GAIN`; the four decay bands, their bilinear upper
shelves, and `DECAY_HIGH_CROSSOVER_HZ` and `DECAY_TOP_CROSSOVER_HZ`, on a measured plateau — all above.
