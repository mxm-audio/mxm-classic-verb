//! The feedback delay network: sixteen delay lines mixed through an orthogonal matrix, two short
//! allpasses inside every line's loop, and each line losing energy through its own decay filter
//! (Jot and Chaigne, 1991; Stautner and Puckette, 1982; the allpasses after Dattorro, 1997).

use crate::delay::{DelayLine, flush};
use crate::filter::{Allpass, BandGains, BilinearCoefficients, DecayFilter};
use core::f32::consts::TAU;

/// Delay lines in the network. Plan D6, taken by measurement at P3.5: eight rang or clicked on the
/// owner's responses, sixteen with allpasses in the loop did neither (`AGENTS.md`).
pub const LINES: usize = 16;
/// Allpasses inside each line's loop.
pub const LOOP_ALLPASSES: usize = 2;

/// Line lengths as multiples of Size, the network's mean delay. **Chosen**: log-spaced over about
/// 1:2.1 with a bounded irrational jitter, searched for the largest distance from any ratio p/q with
/// p, q ≤ 4, and normalised to a mean of exactly one so Size *is* the mean delay.
pub const RATIOS: [f32; LINES] = [
    0.66732, 0.70380, 0.73857, 0.77895, 0.81743, 0.85781, 0.90471, 0.94940, 0.99630, 1.05077,
    1.10268, 1.16297, 1.22042, 1.28070, 1.35072, 1.41745,
];
pub const RATIO_MIN: f32 = RATIOS[0];
pub const RATIO_MAX: f32 = RATIOS[LINES - 1];

/// The allpasses inside each line's loop, seconds. **Chosen**, then measured against longer sets:
/// short, so a trip smears an echo into a burst instead of repeating it (Dattorro's decay diffusion,
/// `research:effects/feedback-delay-network-reverb.md` §7), and incommensurate by golden-ratio steps
/// over 1.3–3.0 ms and 3.2–4.9 ms. An allpass inside an orthogonal network keeps it lossless: it is
/// itself a delay network, so the whole is a larger one with the same matrix (§1).
const LOOP_ALLPASS_S: [[f32; LOOP_ALLPASSES]; LINES] = [
    [0.001300, 0.004000],
    [0.002412, 0.004746],
    [0.001725, 0.003691],
    [0.002837, 0.004437],
    [0.002150, 0.003382],
    [0.001462, 0.004128],
    [0.002575, 0.004874],
    [0.001887, 0.003819],
    [0.003000, 0.004565],
    [0.002312, 0.003510],
    [0.001625, 0.004256],
    [0.002737, 0.003201],
    [0.002050, 0.003947],
    [0.001362, 0.004693],
    [0.002474, 0.003638],
    [0.001787, 0.004384],
];
/// The loop allpasses' coefficient. **Chosen inside Dattorro's decay diffusion of 0.5–0.7, then
/// measured**: 0.5 and 0.7 read no better. Fixed, never modulated: a time-varying allpass coefficient
/// inside a lossless loop can diverge (§6, Schlecht).
pub const LOOP_ALLPASS_GAIN: f32 = 0.6;

/// Each line's modulation rate as a multiple of the control's, spread by about ±50 % so the lines
/// do not beat together (`research:effects/feedback-delay-network-reverb.md` §6). **Chosen.**
const MOD_SPREAD: [f32; LINES] = [
    0.55, 1.37, 0.81, 1.12, 0.66, 1.48, 0.94, 1.23, 0.72, 1.05, 1.41, 0.60, 1.29, 0.87, 1.18, 0.98,
];
/// Starting phases, in turns. **Chosen.**
const MOD_PHASE: [f32; LINES] = [
    0.0, 0.41, 0.83, 0.17, 0.62, 0.29, 0.95, 0.54, 0.08, 0.36, 0.71, 0.23, 0.88, 0.47, 0.12, 0.66,
];

/// Row `k` of the order-16 Sylvester–Hadamard matrix.
const fn hadamard(k: usize) -> [f32; LINES] {
    let mut row = [0.0; LINES];
    let mut i = 0;
    while i < LINES {
        row[i] = if (k & i).count_ones().is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        i += 1;
    }
    row
}

/// Injection and output patterns: rows of the Sylvester–Hadamard matrix, mutually orthogonal, so the
/// two channels enter and leave the network decorrelated. **Chosen** rows.
const IN_L: [f32; LINES] = hadamard(10);
const IN_R: [f32; LINES] = hadamard(5);
const OUT_L: [f32; LINES] = hadamard(9);
const OUT_R: [f32; LINES] = hadamard(14);

/// Injection scale, 1/√N, so an impulse puts unit energy into the network.
const IN_SCALE: f32 = 0.25;
/// Output taps read inside every line besides its end. Without them the late field reached the output
/// a whole trip after the pre-delay — a shortest line, about 38 ms at the fit's density floor — and the
/// owner heard the gap as pre-delay (P3.5). Read inside the lines, a sound arrives a small fraction of
/// a trip after the pre-delay, as Dattorro's tank reads its output from inside its delay lines
/// (`research:effects/feedback-delay-network-reverb.md` §7, recipe §13 step 4). They sit outside the
/// loop, so no decay, bound or loss changes.
pub const OUTPUT_TAPS: usize = 3;

/// Where each tap reads, as a fraction of its line's length. **Chosen**: one in each third of the line,
/// jittered by golden-ratio steps inside it and kept off both ends.
const TAP_FRACTION: [[f32; OUTPUT_TAPS]; LINES] = [
    [0.0333, 0.4495, 0.8657],
    [0.1569, 0.3731, 0.7893],
    [0.0805, 0.4967, 0.7129],
    [0.2042, 0.4203, 0.8365],
    [0.1278, 0.5439, 0.7601],
    [0.0514, 0.4675, 0.8837],
    [0.1750, 0.3912, 0.8073],
    [0.0986, 0.5148, 0.7309],
    [0.2222, 0.4384, 0.8545],
    [0.1458, 0.5620, 0.7781],
    [0.0694, 0.4856, 0.7018],
    [0.1930, 0.4092, 0.8254],
    [0.1166, 0.5328, 0.7490],
    [0.0402, 0.4564, 0.8726],
    [0.1638, 0.3800, 0.7962],
    [0.0874, 0.5036, 0.7198],
];

/// The taps' sign patterns per channel: rows of the Sylvester–Hadamard matrix, each tap's left and right
/// rows distinct and so orthogonal. **Chosen** rows.
const TAP_L: [[f32; LINES]; OUTPUT_TAPS] = [hadamard(3), hadamard(6), hadamard(10)];
const TAP_R: [[f32; LINES]; OUTPUT_TAPS] = [hadamard(12), hadamard(5), hadamard(15)];

/// Output scale, per read. **Chosen**; the late field's level against the dry is Mix's job and the
/// early/late balance is the space's. It is the eight-line network's 1/√8 over the reads per line: with
/// the lines' energy shared evenly and the reads of a diffuse field uncorrelated, sixteen lines read
/// at the end and at 3 taps give the output energy eight line ends did (*derived*).
const OUT_SCALE: f32 = 0.353_553_4 / 2.0;

/// The earliest a sound injected into the network reaches its output, in seconds at `size_s`: the
/// shortest tap into any line.
pub fn late_onset_s(size_s: f32) -> f32 {
    let mut earliest = f32::INFINITY;
    for (i, taps) in TAP_FRACTION.iter().enumerate() {
        for fraction in taps {
            earliest = earliest.min(fraction * RATIOS[i]);
        }
    }
    earliest * size_s
}

/// Samples of allpass delay inside line `i`'s loop, rounded as the allpasses round them.
pub(crate) fn loop_delay_samples(i: usize, sample_rate: f32) -> f32 {
    LOOP_ALLPASS_S[i]
        .iter()
        .map(|s| ((s * sample_rate).round() as usize).max(1) as f32)
        .sum()
}

/// The mean group delay, in samples, of line `i`'s loop allpasses over `lo_hz`–`hi_hz`: their phase
/// lag across the band over the band's width, in closed form.
///
/// A Schroeder allpass's group delay swings between `d(1−g)/(1+g)` and `d(1+g)/(1−g)` with period
/// `1/d` in frequency and averages `d` over each period, so a band many periods wide reads `d` and a
/// low octave does not. A mode decays at its loop's loss per trip over the trip's group delay, which is
/// why a band's decay is calibrated and predicted on this (measured: without it, the 125 Hz octave
/// read 13–16 % short at Size 20–50 ms).
pub(crate) fn loop_group_delay_samples(i: usize, sample_rate: f32, lo_hz: f32, hi_hz: f32) -> f32 {
    let fs = f64::from(sample_rate);
    let g = f64::from(LOOP_ALLPASS_GAIN);
    // A band past the top keeps an octave's width below it: collapsed onto one frequency, it would
    // read a single point of the ripple rather than its mean.
    let top = 0.45 * fs;
    let hi = f64::from(hi_hz).clamp(2.0, top);
    let lo = f64::from(lo_hz).clamp(1.0, 0.5 * hi);
    let (w1, w2) = (
        core::f64::consts::TAU * lo / fs,
        core::f64::consts::TAU * hi / fs,
    );
    LOOP_ALLPASS_S[i]
        .iter()
        .map(|s| {
            let d = ((s * sample_rate).round() as usize).max(1) as f64;
            if w2 - w1 < 1.0e-9 {
                return d * (1.0 - g * g) / (1.0 - 2.0 * g * (w1 * d).cos() + g * g);
            }
            // The allpass's phase is −ωd − 2·atan2(g·sin ωd, 1 − g·cos ωd); the atan2 stays inside
            // ±π/2 because g < 1, so the difference needs no unwrapping.
            let arg = |w: f64| (g * (w * d).sin()).atan2(1.0 - g * (w * d).cos());
            d + 2.0 * (arg(w2) - arg(w1)) / (w2 - w1)
        })
        .sum::<f64>() as f32
}

/// The network's total delay at `size_s` in seconds, loop allpasses included: its system order over
/// the sample rate, which is its modal density in modes per hertz (Rocchesso and Smith, §3.4).
pub fn total_delay_s(size_s: f32) -> f32 {
    let allpasses: f32 = LOOP_ALLPASS_S.iter().flatten().sum();
    RATIOS.iter().sum::<f32>() * size_s + allpasses
}

/// The Size whose total delay is `total_s`, or zero where the loop allpasses alone reach it.
pub fn size_for_total_delay_s(total_s: f32) -> f32 {
    let allpasses: f32 = LOOP_ALLPASS_S.iter().flatten().sum();
    ((total_s - allpasses) / RATIOS.iter().sum::<f32>()).max(0.0)
}

/// How a line is read at a fractional delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    /// A first-order Thiran allpass: unit magnitude at every frequency, so it costs no decay.
    Allpass,
    /// Linear interpolation: a lowpass whose zero moves with the fraction, so it costs decay in the
    /// high band (Dattorro, *Effect Design* Part 2).
    Linear,
}

#[derive(Debug, Clone)]
pub(crate) struct Network {
    lines: [DelayLine; LINES],
    readers: [f32; LINES],
    filters: [DecayFilter; LINES],
    gains: [BandGains; LINES],
    loop_allpasses: [[Allpass; LOOP_ALLPASSES]; LINES],
    phases: [f32; LINES],
    pole_lo: f32,
    high: BilinearCoefficients,
    top: BilinearCoefficients,
    sample_rate: f32,
    interpolation: Interpolation,
}

impl Network {
    pub(crate) fn new(
        sample_rate: f32,
        max_delay_samples: usize,
        pole_lo: f32,
        high: BilinearCoefficients,
        top: BilinearCoefficients,
    ) -> Self {
        Self {
            lines: core::array::from_fn(|_| DelayLine::new(max_delay_samples + 2)),
            readers: [0.0; LINES],
            filters: [DecayFilter::default(); LINES],
            gains: [BandGains::SILENT; LINES],
            loop_allpasses: core::array::from_fn(|i| {
                core::array::from_fn(|k| Allpass::new(LOOP_ALLPASS_S[i][k], sample_rate))
            }),
            phases: MOD_PHASE,
            pole_lo,
            high,
            top,
            sample_rate,
            interpolation: Interpolation::Allpass,
        }
    }

    pub(crate) fn set_gains(&mut self, gains: [BandGains; LINES]) {
        self.gains = gains;
    }

    pub(crate) fn gains(&self) -> &[BandGains; LINES] {
        &self.gains
    }

    pub(crate) fn set_interpolation(&mut self, interpolation: Interpolation) {
        self.interpolation = interpolation;
    }

    /// The longest trip round any line at `size_samples`, loop allpasses included.
    pub(crate) fn longest_trip_samples(&self, size_samples: f32) -> f32 {
        (0..LINES)
            .map(|i| RATIOS[i] * size_samples + loop_delay_samples(i, self.sample_rate))
            .fold(0.0, f32::max)
    }

    /// Empties every line, allpass and filter, in time independent of their length.
    pub(crate) fn invalidate(&mut self) {
        for line in &mut self.lines {
            line.invalidate();
        }
        for ap in self.loop_allpasses.iter_mut().flatten() {
            ap.invalidate();
        }
        self.readers = [0.0; LINES];
        for f in &mut self.filters {
            f.reset();
        }
        self.phases = MOD_PHASE;
    }

    /// One sample. `size_samples` is the mean delay; `depth_samples` the modulation's peak excursion.
    #[inline]
    pub(crate) fn process(
        &mut self,
        in_l: f32,
        in_r: f32,
        size_samples: f32,
        depth_samples: f32,
        rate_hz: f32,
    ) -> (f32, f32) {
        let mut outs = [0.0f32; LINES];
        let modulating = depth_samples > 0.0;
        for (i, out) in outs.iter_mut().enumerate() {
            let mut delay = RATIOS[i] * size_samples;
            if modulating {
                let phase = self.phases[i];
                delay += depth_samples * (TAU * phase).sin();
                let mut next = phase + rate_hz * MOD_SPREAD[i] / self.sample_rate;
                if next >= 1.0 {
                    next -= 1.0;
                }
                self.phases[i] = next;
            }
            let mut raw = match self.interpolation {
                Interpolation::Allpass => thiran(&self.lines[i], &mut self.readers[i], delay),
                Interpolation::Linear => self.lines[i].read_linear(delay),
            };
            for ap in &mut self.loop_allpasses[i] {
                raw = ap.process(raw, LOOP_ALLPASS_GAIN);
            }
            *out = self.filters[i].process(raw, self.gains[i], self.pole_lo, &self.high, &self.top);
        }

        let mut late_l = 0.0;
        let mut late_r = 0.0;
        for i in 0..LINES {
            late_l += OUT_L[i] * outs[i];
            late_r += OUT_R[i] * outs[i];
            // Inside the line, before this sample is pushed: outside the loop, so a linear read's small
            // loss is the output's, never the decay's.
            let length = RATIOS[i] * size_samples;
            for k in 0..OUTPUT_TAPS {
                let tap = self.lines[i].read_linear((TAP_FRACTION[i][k] * length).max(1.0));
                late_l += TAP_L[k][i] * tap;
                late_r += TAP_R[k][i] * tap;
            }
        }

        mix(&mut outs);
        let (in_l, in_r) = (in_l * IN_SCALE, in_r * IN_SCALE);
        for i in 0..LINES {
            self.lines[i].push(flush(outs[i] + IN_L[i] * in_l + IN_R[i] * in_r));
        }
        (late_l * OUT_SCALE, late_r * OUT_SCALE)
    }
}

/// Reads `line` at a fractional `delay` through a first-order Thiran allpass.
///
/// The integer part is chosen so the allpass supplies between 0.5 and 1.5 samples, where its
/// coefficient `(1 − d)/(1 + d)` stays inside (−0.2, 0.34) — well away from the Nyquist pole that a
/// fraction near zero would otherwise bring.
#[inline]
fn thiran(line: &DelayLine, y1: &mut f32, delay: f32) -> f32 {
    let delay = delay.max(1.5);
    let whole = (delay - 0.5).floor();
    let d = delay - whole;
    let a = (1.0 - d) / (1.0 + d);
    let n = whole as usize;
    let y = a * (line.read(n) - *y1) + line.read(n + 1);
    *y1 = flush(y);
    y
}

/// The feedback matrix: an order-16 fast Walsh–Hadamard transform scaled to be orthogonal, then a
/// rotation by one line so the matrix is not its own inverse. A product of orthogonal matrices is
/// orthogonal, so the network is lossless for any delays (Schlecht and Habets, 2017, Lemma 1).
#[inline]
fn mix(v: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        let mut i = 0;
        while i < LINES {
            for j in i..i + h {
                let (a, b) = (v[j], v[j + h]);
                v[j] = a + b;
                v[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    for x in v.iter_mut() {
        *x *= IN_SCALE;
    }
    v.rotate_right(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ratios_average_one_and_are_ordered() {
        let mean = RATIOS.iter().sum::<f32>() / LINES as f32;
        assert!((mean - 1.0).abs() < 1e-4, "mean {mean}");
        assert!(RATIOS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_total_delay_and_the_size_for_it_invert_each_other() {
        for size in [0.002f32, 0.054, 0.3] {
            let back = size_for_total_delay_s(total_delay_s(size));
            assert!((back - size).abs() < 1e-6, "{size} → {back}");
        }
        assert_eq!(size_for_total_delay_s(0.0), 0.0);
    }

    #[test]
    fn the_matrix_preserves_energy() {
        let mut v: [f32; LINES] = core::array::from_fn(|i| ((i * 7 % 5) as f32 - 2.0) * 0.37);
        let before: f32 = v.iter().map(|x| x * x).sum();
        mix(&mut v);
        let after: f32 = v.iter().map(|x| x * x).sum();
        assert!((before - after).abs() < 1e-4, "{before} → {after}");
    }

    #[test]
    fn the_patterns_are_orthogonal() {
        let dot =
            |a: &[f32; LINES], b: &[f32; LINES]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert_eq!(dot(&IN_L, &IN_R), 0.0);
        assert_eq!(dot(&OUT_L, &OUT_R), 0.0);
        for k in 0..OUTPUT_TAPS {
            assert_eq!(dot(&TAP_L[k], &TAP_R[k]), 0.0, "tap {k}");
        }
    }

    #[test]
    fn the_taps_sit_inside_their_lines_and_the_onset_is_the_shortest() {
        for (i, taps) in TAP_FRACTION.iter().enumerate() {
            assert!(taps.windows(2).all(|w| w[0] < w[1]), "line {i}: {taps:?}");
            assert!(
                taps.iter().all(|f| (0.03..0.91).contains(f)),
                "line {i}: {taps:?}"
            );
        }
        let onset = late_onset_s(1.0);
        assert!(onset > 0.0 && onset < 0.1 * RATIO_MIN, "{onset}");
    }

    #[test]
    fn a_lossless_network_keeps_its_energy_and_a_lossy_one_loses_it() {
        let fs = 48_000.0;
        let run = |gain: f32| {
            let any = BilinearCoefficients::new(1_000.0, fs);
            let mut n = Network::new(fs, 4096, 0.0, any, any);
            n.set_gains(
                [BandGains {
                    low: gain,
                    mid: gain,
                    high_ratio: 1.0,
                    top_ratio: 1.0,
                }; LINES],
            );
            let mut energy_late = 0.0f64;
            n.process(1.0, 0.0, 480.0, 0.0, 0.0);
            for i in 0..96_000 {
                let (l, r) = n.process(0.0, 0.0, 480.0, 0.0, 0.0);
                if i > 48_000 {
                    energy_late += (l as f64).powi(2) + (r as f64).powi(2);
                }
            }
            energy_late
        };
        let (lossless, lossy) = (run(1.0), run(0.9));
        assert!(
            lossless > 0.1,
            "a lossless network, loop allpasses included, still rings a second later: {lossless}"
        );
        // Derived: a trip is about 480 samples plus about 150 of loop allpass, so the second second
        // holds some 76 trips at 0.9 each, leaving ~3e-4 in amplitude and ~1e-7 in energy.
        assert!(lossy < 1e-3 * lossless, "{lossy} against {lossless}");
    }
}
