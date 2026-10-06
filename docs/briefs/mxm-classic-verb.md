# mxm-classic-verb — UI design brief

Required by `MXM_DESIGN_SYSTEM.md` §14. **Status: approved by the owner, 2026-09-14.** Plan §9's P5
builds the editor from it.

**Plugin:** `mxm-classic-verb`; CLAP id `dk.mxm.mxm-classic-verb`. An original everyday
reverb — rooms, chambers, halls and plates — whose spaces can be fitted from impulse responses.
Stereo wet topology, with mono-to-stereo and stereo-to-stereo layouts. The plan is
`plans/plan-mxm-classic-verb.md`; the technique is `research:effects/feedback-delay-network-reverb.md`.

## 1. Primary sound-design task

**Put the source in a space, then decide how far past that space the sound goes.** A player picks
a space, which fixes its shape: early reflections, how the bands decay against each other, tone
and width. They then set its scale — how long, how big, how far away. Each control that is
relative to the space has a neutral mark that means *the space as it is*, and the travel beyond
that mark is the creative half of the product. That distinction must be visible, not inferred.

## 2. Controls reached for most

1. **Space** — which room, chamber, hall or plate; later also a space loaded from a file.
2. **Decay** — the mid-band decay time, in seconds.
3. **Size** — the network's mean delay, in milliseconds.
4. **Mix** — the dry/wet balance, and Off at zero.
5. **Pre-delay** — the gap before the space answers.

## 3. Signal flow that must be visible

```text
input -> pre-delay --+-> early reflections (the space's pattern x Size) ----------+
                     |                                                            |
                     +-> diffusion -> delay network (decay per band, modulation) -+-> tone -> width -> ducking -> Mix
                         or, for a gated or reverse shape, a shaped tap line -----+
input ---------------------------------------------------------------------------------------------------------> Mix
```

Three facts the interface must carry:

- **The space is shape; the controls are scale.** Changing the space never moves a knob.
- **Relative controls read against the space.** Bass and treble decay are multipliers with ×1 as
  neutral; tone is a decibel offset with 0 dB as neutral; width is a share with 100 % as neutral;
  early/late is an offset with 0 dB as neutral. Each shows its neutral detent, and a double-click
  returns it there.
- **A shaped decay replaces the network.** Gated and reverse are envelopes, not settings of Decay
  alone; Decay becomes the shape's length.

## 4. Play view

**None, and no view bar.** Like the collection's other effects, this is one set of Effects cards,
paged by the shared renderer when they no longer fit.

## 5. Advanced controls and disclosure

No control is hidden. The cards are the disclosure, in signal-flow order:

| Card | Controls | Job |
|---|---|---|
| **Space** | Space, Pre-delay, Size, Early and late | Choose the space and place the listener in it |
| **Decay** | Decay, Bass decay, Treble decay, Decay shape | Set how long, how it tilts across the bands, and its envelope |
| **Texture** | Diffusion, Mod depth, Mod rate | Density and motion |
| **Output** | Mix, Width, Low tone, High tone, Ducking | How the reverb sits in the mix |

**Loading a space from a file (plan P6)** belongs on the Space card. It appears as a drop target on
the card and a `Loaded` position on the selector. It shows the fit report's summary in one line —
confidence, and the largest descriptor error — with the full report on hover. A refusal replaces
that line with its reason and emits no parameter change.

**Recorded at P6, where this section was silent** (`plugins/mxm-classic-verb/NOTES.md`, *Loading a
space from an impulse response*, holds the detail and the tests):

- **The target is the whole Space card**, and its visible face is the one line, placed under the
  selector at a fixed `MIN_TARGET` height. There is no Browse button; this section asks for none.
- **Hover feedback** (design system §5.4, §11): while a file is held over the card the line reads
  *Release to fit a space to this file* on the selection fill with a 2-point accent border, so the
  state is words and weight as well as hue.
- **Busy** is the line reading *Fitting a space to ‹file›…*, with no progress: the fit crate measured
  0.73–0.83 s for 2–6 s stereo responses at 44.1 kHz and 1.89 s at 96 kHz, in release.
- **The largest descriptor error** is the one largest against a per-descriptor scale — 5 % of a
  decay time, 1 dB of a level, 10 ms of a time — so errors in different units can be compared.
- **Low confidence** above the refusal floor reads *Low confidence 0.31 · …* in the warning ink.
- **The summary survives a reload**: the fit report's numbers travel with the loaded space in presets
  and host state, never the response or its path.

## 6. Categories, cards and grouping

All four cards are **Effects**, ordered Space, Decay, Texture, Output. Decay stays whole because
separating the multipliers from Decay would hide what they multiply. Mix sits with the output
shaping rather than beside Space: it is the control a player reaches for last, once the space is
right.

## 7. Identity accent

The collection accent, unchanged in both themes, for the reason the other effects' briefs give:
an effect is told apart in a chain by its name and its role, not by a new hue.

## 8. Live visualisation

**One reserved display on the Decay card: the decay across the bands.**

- **Geometry comes from the DSP:** three decay lines — low, mid and high — drawn to the times the
  engine predicts (`Engine::predicted_decay_s`), on a time axis that does not rescale itself with
  every knob move. A bass or treble multiplier visibly tilts the lines, and a shaped decay is drawn as
  its envelope instead.
- **Brightness is the wet the engine actually added**, read from lock-free telemetry, so a parked
  effect goes dark.
- **Text states carry what hue cannot:** **Off** when Mix is zero, and **Nothing loaded** when the
  `Loaded` position has no space.

The app bar keeps the shared peak and clip meter.

## 9. What is removed from source layouts

There is no hardware panel to preserve. The makers' manuals read for the research — Lexicon's
224X, PCM70, 480L and PCM96, and AMS's RMX16 — contributed vocabulary: decay as a multiplier,
diffusion as density, a shaped envelope with a cutoff. They contributed no layout, mode list,
program name or preset. Programs become **spaces**, and a space's name is descriptive, never a
venue, maker or product.

## 10. Fit, reflow and zoom

Four cards, each capped. The opening size and the minimum are decided at build time against
measured card floors, and pinned by tests:

- the minimum holds the widest indivisible card;
- the opening size fits the 1920 × 1080 quarter-4K dimensions at 1×;
- at smaller sizes the shared renderer pages the cards and scrolls an indivisible overflow rather
  than clipping a control.

Native 100 %, 150 % and 200 % zoom inspection, in both themes, remains §15 visual QA.

## The controller page

Not a §14 question, but it is decided with the panel. **Mix fills `fx.reverb`**, as
`mxm-folded-spring` and `mxm-shimmer` do. The performed controls take a new page 13 with seven
roles: Space (stepped), Decay (log), Size (log), Pre-delay (log), Bass decay, Treble decay and
Decay shape (stepped). Its eighth slot is left free. Diffusion, the modulation pair, Width, the tone
pair, Early and late, and Ducking are shape controls: they stay unmapped, automatable and on the
panel. Role ids are written into `docs/MXM_CONTROL_MAP.md` in the same pass.

## Deliberate deviations from the design system

| § | Rule | Deviation | Why |
|---|---|---|---|
| §5.3 | Each instrument has an identity accent | The collection accent is kept | §7 |
| §6 | Views bar | No view bar | §4 |

## Sign-off

- [x] §14's ten questions are answered.
- [x] **The owner approved this brief** (2026-09-14) — the P5 gate.
- [x] Every parameter appears exactly once, with a one-sentence tooltip.
- [x] Off and Nothing loaded have text channels, not hue alone.
- [x] The opening size and minimum are pinned by tests.
- [ ] Native-window inspection in both themes, and fixed-window zoom inspection.
- [ ] Owner listening and visual sign-off.
