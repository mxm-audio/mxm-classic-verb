//! The space: the shape a fit produces, which the controls act on (plan §2).

/// Early reflection taps in every space. Fixed, so any two spaces can be interpolated.
pub const EARLY_TAPS: usize = 12;

/// The latest an early tap may sit, in units of the network's mean delay (Size). Chosen: Moorer's
/// taps span 40–80 ms and a mid-sized network's mean delay is tens of milliseconds.
pub const EARLY_MAX_UNITS: f32 = 3.0;

/// The version of the space's form. From release on it moves when a field is added, removed or
/// reinterpreted. **Before release nothing is stored**, so the form may change under version 1: the
/// high cut and the top decay band were added that way, and there is no stored space a new version
/// would have to tell apart.
pub const SPACE_VERSION: u32 = 1;

/// Band decay ratios a space may carry, relative to the mid band.
pub const MIN_DECAY_RATIO: f32 = 0.1;
pub const MAX_DECAY_RATIO: f32 = 10.0;

/// The largest tone correction a space may carry, either way.
pub const MAX_SPACE_TONE_DB: f32 = 24.0;

/// The lowest corner a space's high cut may carry. **Chosen from measurement**: on a tone curve with
/// every band at the tenth percentile of the owner's pack's clamped fits, the fit's band model came
/// within 0.77 dB worst with its floor here, 1.29 dB at 1.5 kHz and 2.96 dB at 2 kHz (this crate's
/// `AGENTS.md`). *Derived*: the cut is 3 dB down at its corner and
/// `TONE_REFERENCE_HZ` is 1 kHz, so the tone's normalisation never lifts the rest of the curve by
/// more than 3 dB.
pub const MIN_HIGH_CUT_HZ: f32 = 1_000.0;

/// A high-cut corner at or above this is **open**: the cut is bypassed exactly, and the space renders
/// sample for sample as it would with no cut at all. **Chosen**: the top of the audible band.
pub const HIGH_CUT_OPEN_HZ: f32 = 20_000.0;

/// The widest a space's measured width may be recorded as.
pub const MAX_SPACE_WIDTH: f32 = 2.0;

/// The loudest a space's early reflections may sit against its late field, and the loudest one tap.
pub const MAX_EARLY_LEVEL: f32 = 4.0;
pub const MAX_TAP_GAIN: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EarlyTap {
    /// When the reflection arrives after the pre-delay, in units of Size.
    pub time: f32,
    pub gain_l: f32,
    pub gain_r: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Space {
    /// The early reflection pattern, normalised to Size.
    pub early: [EarlyTap; EARLY_TAPS],
    /// The fitted balance: the early pattern's level against the late field, linear.
    pub early_level: f32,
    /// Decay time in the low, high and top bands — the octaves at 125 Hz, 4 kHz and 8 kHz — as ratios
    /// of the mid band's at 1 kHz. Exactly one at mid by definition.
    pub decay_ratio_low: f32,
    pub decay_ratio_high: f32,
    pub decay_ratio_top: f32,
    /// The initial-spectrum correction in the low and high bands, 0 dB at mid.
    pub tone_low_db: f32,
    pub tone_high_db: f32,
    /// The initial spectrum's high cut, hertz: a second-order Butterworth lowpass on the output tone,
    /// for air absorption and a darkened source, whose fall a first-order shelf cannot follow. Part
    /// of the tone, so normalised with it at `TONE_REFERENCE_HZ`. Open at [`HIGH_CUT_OPEN_HZ`].
    pub high_cut_hz: f32,
    /// The measured width: the late field's side against its mid component.
    pub width: f32,
}

const fn tap(time: f32, gain_l: f32, gain_r: f32) -> EarlyTap {
    EarlyTap {
        time,
        gain_l,
        gain_r,
    }
}

const SILENT_TAP: EarlyTap = tap(0.0, 0.0, 0.0);

fn finite_or(x: f32, fallback: f32) -> f32 {
    if x.is_finite() { x } else { fallback }
}

impl Space {
    /// The space with every value bounded — a space fitted from somebody's own file is
    /// user-generated input, and the DSP bounds it rather than trusting the editor.
    pub fn sanitised(mut self) -> Self {
        for t in &mut self.early {
            t.time = finite_or(t.time, 0.0).clamp(0.0, EARLY_MAX_UNITS);
            t.gain_l = finite_or(t.gain_l, 0.0).clamp(-MAX_TAP_GAIN, MAX_TAP_GAIN);
            t.gain_r = finite_or(t.gain_r, 0.0).clamp(-MAX_TAP_GAIN, MAX_TAP_GAIN);
        }
        self.early_level = finite_or(self.early_level, 0.0).clamp(0.0, MAX_EARLY_LEVEL);
        self.decay_ratio_low =
            finite_or(self.decay_ratio_low, 1.0).clamp(MIN_DECAY_RATIO, MAX_DECAY_RATIO);
        self.decay_ratio_high =
            finite_or(self.decay_ratio_high, 1.0).clamp(MIN_DECAY_RATIO, MAX_DECAY_RATIO);
        self.decay_ratio_top =
            finite_or(self.decay_ratio_top, 1.0).clamp(MIN_DECAY_RATIO, MAX_DECAY_RATIO);
        self.tone_low_db =
            finite_or(self.tone_low_db, 0.0).clamp(-MAX_SPACE_TONE_DB, MAX_SPACE_TONE_DB);
        self.tone_high_db =
            finite_or(self.tone_high_db, 0.0).clamp(-MAX_SPACE_TONE_DB, MAX_SPACE_TONE_DB);
        self.high_cut_hz =
            finite_or(self.high_cut_hz, HIGH_CUT_OPEN_HZ).clamp(MIN_HIGH_CUT_HZ, HIGH_CUT_OPEN_HZ);
        self.width = finite_or(self.width, 1.0).clamp(0.0, MAX_SPACE_WIDTH);
        self
    }

    /// A space between two, field by field. Every space has the same form, so this always exists.
    pub fn lerp(&self, other: &Space, t: f32) -> Space {
        let t = finite_or(t, 0.0).clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let mut early = [SILENT_TAP; EARLY_TAPS];
        for (i, e) in early.iter_mut().enumerate() {
            let (a, b) = (self.early[i], other.early[i]);
            *e = tap(
                mix(a.time, b.time),
                mix(a.gain_l, b.gain_l),
                mix(a.gain_r, b.gain_r),
            );
        }
        Space {
            early,
            early_level: mix(self.early_level, other.early_level),
            decay_ratio_low: mix(self.decay_ratio_low, other.decay_ratio_low),
            decay_ratio_high: mix(self.decay_ratio_high, other.decay_ratio_high),
            decay_ratio_top: mix(self.decay_ratio_top, other.decay_ratio_top),
            tone_low_db: mix(self.tone_low_db, other.tone_low_db),
            tone_high_db: mix(self.tone_high_db, other.tone_high_db),
            high_cut_hz: mix(self.high_cut_hz, other.high_cut_hz),
            width: mix(self.width, other.width),
        }
        .sanitised()
    }

    // ── Hand-authored spaces for P1 ────────────────────────────────────────────────────────────
    //
    // Every value below is **chosen**, by hand, to exercise the engine before any fit exists. None
    // is measured from a response; P3 replaces them with fitted spaces. Their high cut is open, so
    // every figure measured on them before the cut existed still stands. Each top ratio is the one
    // high ratio the space carried before the fourth decay band, when that ratio was calibrated at
    // 8 kHz; each high ratio is the geometric mean of that and one, rounded.

    /// A small, fairly bright room with a dense early cluster.
    pub const ROOM: Space = Space {
        early: [
            tap(0.18, 0.82, 0.55),
            tap(0.29, 0.48, 0.77),
            tap(0.41, 0.63, 0.40),
            tap(0.52, 0.35, 0.58),
            tap(0.63, 0.47, 0.30),
            tap(0.77, 0.26, 0.42),
            tap(0.88, 0.33, 0.24),
            tap(1.02, 0.19, 0.29),
            tap(1.14, 0.22, 0.15),
            tap(1.27, 0.12, 0.18),
            tap(1.39, 0.14, 0.10),
            tap(1.49, 0.08, 0.11),
        ],
        early_level: 0.7,
        decay_ratio_low: 1.1,
        decay_ratio_high: 0.77,
        decay_ratio_top: 0.6,
        tone_low_db: 0.0,
        tone_high_db: -3.0,
        high_cut_hz: HIGH_CUT_OPEN_HZ,
        width: 0.8,
    };

    /// A large hall: sparse, later early reflections and a long, darkening tail.
    pub const HALL: Space = Space {
        early: [
            tap(0.21, 0.52, 0.30),
            tap(0.37, 0.28, 0.49),
            tap(0.58, 0.41, 0.25),
            tap(0.76, 0.22, 0.38),
            tap(0.97, 0.33, 0.21),
            tap(1.18, 0.18, 0.30),
            tap(1.41, 0.25, 0.16),
            tap(1.63, 0.14, 0.22),
            tap(1.86, 0.17, 0.11),
            tap(2.07, 0.09, 0.14),
            tap(2.29, 0.10, 0.07),
            tap(2.48, 0.05, 0.08),
        ],
        early_level: 0.5,
        decay_ratio_low: 1.3,
        decay_ratio_high: 0.74,
        decay_ratio_top: 0.55,
        tone_low_db: 1.0,
        tone_high_db: -4.0,
        high_cut_hz: HIGH_CUT_OPEN_HZ,
        width: 1.0,
    };

    /// A hard chamber: dense, even reflections and a flatter decay.
    pub const CHAMBER: Space = Space {
        early: [
            tap(0.11, 0.70, 0.62),
            tap(0.19, 0.58, 0.66),
            tap(0.27, 0.61, 0.52),
            tap(0.34, 0.49, 0.57),
            tap(0.43, 0.52, 0.44),
            tap(0.51, 0.41, 0.47),
            tap(0.62, 0.43, 0.36),
            tap(0.71, 0.33, 0.38),
            tap(0.83, 0.34, 0.28),
            tap(0.94, 0.25, 0.29),
            tap(1.06, 0.26, 0.21),
            tap(1.18, 0.18, 0.21),
        ],
        early_level: 0.6,
        decay_ratio_low: 1.0,
        decay_ratio_high: 0.84,
        decay_ratio_top: 0.7,
        tone_low_db: 0.0,
        tone_high_db: -2.0,
        high_cut_hz: HIGH_CUT_OPEN_HZ,
        width: 0.7,
    };

    /// A plate: no early reflections, a bright and nearly even decay.
    pub const PLATE: Space = Space {
        early: [SILENT_TAP; EARLY_TAPS],
        early_level: 0.0,
        decay_ratio_low: 0.9,
        decay_ratio_high: 0.92,
        decay_ratio_top: 0.85,
        tone_low_db: -3.0,
        tone_high_db: 1.0,
        high_cut_hz: HIGH_CUT_OPEN_HZ,
        width: 1.0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hand_authored_spaces_are_already_inside_their_bounds() {
        for s in [Space::ROOM, Space::HALL, Space::CHAMBER, Space::PLATE] {
            assert_eq!(s.sanitised(), s);
            assert_eq!(
                s.high_cut_hz, HIGH_CUT_OPEN_HZ,
                "the hand-authored cuts are open"
            );
        }
    }

    #[test]
    fn a_hostile_space_is_bounded_and_interpolation_lands_on_its_ends() {
        let mut bad = Space::HALL;
        bad.decay_ratio_low = f32::NAN;
        bad.tone_high_db = 1.0e9;
        bad.early[3].time = -4.0;
        bad.width = f32::INFINITY;
        bad.high_cut_hz = f32::NAN;
        let s = bad.sanitised();
        assert_eq!(s.decay_ratio_low, 1.0);
        assert_eq!(s.tone_high_db, MAX_SPACE_TONE_DB);
        assert_eq!(s.early[3].time, 0.0);
        assert_eq!(s.width, 1.0);
        assert_eq!(s.high_cut_hz, HIGH_CUT_OPEN_HZ, "a non-finite cut is open");
        for (asked, bounded) in [
            (-5.0, MIN_HIGH_CUT_HZ),
            (0.0, MIN_HIGH_CUT_HZ),
            (3_000.0, 3_000.0),
            (1.0e9, HIGH_CUT_OPEN_HZ),
        ] {
            bad.high_cut_hz = asked;
            assert_eq!(bad.sanitised().high_cut_hz, bounded, "{asked}");
        }
        assert_eq!(Space::ROOM.lerp(&Space::HALL, 0.0), Space::ROOM);
        assert_eq!(Space::ROOM.lerp(&Space::HALL, 1.0), Space::HALL);
    }
}
