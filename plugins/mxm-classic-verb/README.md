# mxm-classic-verb

An everyday algorithmic reverb: rooms, chambers, halls and plates, with controls that reach past
what any real space does. A space is the shape — its early reflections, how its bands decay against
each other, its tone and width. The controls are the scale — how long, how big, how far away — and
the relative ones show the space as fitted at their neutral mark.

A CLAP audio effect with mono-to-stereo and stereo-to-stereo layouts. Mix at zero is Off. Four
cards — Space, Decay, Texture and Output — and a display of the decay across the bands.

**Status: in development.** The DSP, the plugin and its editor exist, and a space can be fitted from
your own impulse response: drop a WAV or AIFF file on the Space card, and the fitted space becomes
the `Loaded` position and travels with your presets and projects. The factory spaces are generated
by the same fitter, and are provisional: today they are fitted from synthetic renders of four
hand-authored stand-ins, until spaces chosen from real impulse responses replace them. See
`plans/plan-mxm-classic-verb.md`.

MIT licensed; see `LICENSE`.
