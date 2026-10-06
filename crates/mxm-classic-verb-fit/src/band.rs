//! Octave-band filtering, applied time-reversed and then forward.
//!
//! **The filter.** A Butterworth band-pass from a [`crate::BAND_PROTOTYPE_ORDER`] low-pass prototype
//! (four poles), edges at the centre ÷ √2 and × √2, by the low-pass-to-band-pass transform and the
//! bilinear transform with both edges prewarped. `research:effects/feedback-delay-network-reverb.md`
//! §9.4 names second-order Butterworth per IEC 61260.
//!
//! **Why time-reversed.** A band filter run forward adds its own ring-down to the response's decay
//! and puts its start-up transient at the direct sound, which matters most in the low bands. Run
//! over the reversed response, the transient lands in the tail, which the noise-floor truncation
//! cuts, and the ringing lands *before* the direct sound. The second, forward pass cancels the
//! first pass's phase distortion (the same section, §9.4). Filtered in `f64`: the recursion runs for
//! the whole response and the root contract reserves `f64` for exactly that.

use std::f64::consts::{FRAC_1_SQRT_2, PI, SQRT_2};

use crate::buffer::{self, OutOfMemory};

/// Whether a band is analysed at this rate: its upper edge must lie at or below
/// [`crate::BAND_UPPER_EDGE_MAX_NYQUIST_FRACTION`] of Nyquist.
pub(crate) fn band_is_present(centre_hz: f64, rate: f64) -> bool {
    centre_hz * SQRT_2 <= crate::BAND_UPPER_EDGE_MAX_NYQUIST_FRACTION * 0.5 * rate
}

/// One band-pass biquad with its zeros at DC and Nyquist: `b0 · (1 − z⁻²) / (1 + a1 z⁻¹ + a2 z⁻²)`.
#[derive(Clone, Copy, Debug)]
struct Section {
    b0: f64,
    a1: f64,
    a2: f64,
}

/// An octave band-pass filter: two biquads in cascade, unity gain at the centre.
pub(crate) struct OctaveFilter {
    sections: [Section; crate::BAND_PROTOTYPE_ORDER],
}

impl OctaveFilter {
    pub fn new(centre_hz: f64, rate: f64) -> Self {
        let c = 2.0 * rate;
        let prewarp = |hz: f64| c * (PI * hz / rate).tan();
        let low = prewarp(centre_hz * FRAC_1_SQRT_2);
        let high = prewarp(centre_hz * SQRT_2);
        let centre_squared = low * high;
        let width = high - low;

        // The second-order Butterworth prototype's pole in the upper half-plane, e^(j3π/4). Under
        // s → (s² + ω₀²)/(B·s) it becomes the two roots of s² − p·B·s + ω₀² = 0; its conjugate pole
        // gives their conjugates, so each root and its conjugate make one real section.
        let pole = (-FRAC_1_SQRT_2, FRAC_1_SQRT_2);
        let pb = (pole.0 * width, pole.1 * width);
        let discriminant = (
            pb.0 * pb.0 - pb.1 * pb.1 - 4.0 * centre_squared,
            2.0 * pb.0 * pb.1,
        );
        let root = complex_sqrt(discriminant);
        let roots = [
            ((pb.0 + root.0) * 0.5, (pb.1 + root.1) * 0.5),
            ((pb.0 - root.0) * 0.5, (pb.1 - root.1) * 0.5),
        ];

        // Each analog section s / (s² + a1·s + a0), through s = c·(1 − z⁻¹)/(1 + z⁻¹).
        let mut sections = roots.map(|(re, im)| {
            let a1 = -2.0 * re;
            let a0 = re * re + im * im;
            let d0 = c * c + a1 * c + a0;
            Section {
                b0: c / d0,
                a1: (2.0 * a0 - 2.0 * c * c) / d0,
                a2: (c * c - a1 * c + a0) / d0,
            }
        });

        // The analog centre √(ω_l·ω_h) maps back to this digital frequency, where a Butterworth
        // band-pass peaks; the cascade is normalised to unity there.
        let digital_centre_hz = rate / PI * (centre_squared.sqrt() / c).atan();
        let gain = magnitude(&sections, digital_centre_hz, rate);
        sections[0].b0 /= gain;
        Self { sections }
    }

    /// The response filtered time-reversed, then forward. Reserved fallibly: the response's length
    /// sizes it.
    pub fn filter(&self, samples: &[f32]) -> Result<Vec<f64>, OutOfMemory> {
        let mut buffer =
            buffer::collected(samples.len(), samples.iter().rev().map(|&s| f64::from(s)))?;
        self.run(&mut buffer);
        buffer.reverse();
        self.run(&mut buffer);
        Ok(buffer)
    }

    /// One forward pass, in place, transposed direct form II.
    fn run(&self, buffer: &mut [f64]) {
        for section in &self.sections {
            let (mut z1, mut z2) = (0.0f64, 0.0f64);
            for value in buffer.iter_mut() {
                let input = *value;
                let output = section.b0 * input + z1;
                z1 = z2 - section.a1 * output;
                z2 = -section.b0 * input - section.a2 * output;
                *value = output;
            }
        }
    }

    /// The single-pass magnitude response at a frequency. The fit weights its tone model by it, so
    /// the tone it predicts is read through the same band the analyser reads.
    pub fn gain_at(&self, hz: f64, rate: f64) -> f64 {
        magnitude(&self.sections, hz, rate)
    }
}

fn magnitude(sections: &[Section], hz: f64, rate: f64) -> f64 {
    let w = 2.0 * PI * hz / rate;
    let (cos1, sin1) = (w.cos(), w.sin());
    let (cos2, sin2) = ((2.0 * w).cos(), (2.0 * w).sin());
    sections
        .iter()
        .map(|s| {
            let numerator = (s.b0 * (1.0 - cos2)).hypot(s.b0 * sin2);
            let denominator = (1.0 + s.a1 * cos1 + s.a2 * cos2).hypot(s.a1 * sin1 + s.a2 * sin2);
            numerator / denominator
        })
        .product()
}

/// The principal square root of a complex number held as `(re, im)`.
fn complex_sqrt((re, im): (f64, f64)) -> (f64, f64) {
    let modulus = re.hypot(im);
    let real = ((modulus + re) * 0.5).max(0.0).sqrt();
    let imaginary = ((modulus - re) * 0.5).max(0.0).sqrt();
    (real, if im < 0.0 { -imaginary } else { imaginary })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OCTAVE_BANDS_HZ;

    #[test]
    fn each_band_is_unity_at_its_centre_and_half_power_at_its_edges() {
        // Closed form: a Butterworth band-pass is 0 dB at its centre and −3.01 dB at both prewarped
        // edges, whatever its order. Measured: the centre within 4.5e-16 of unity and the edges
        // within 1.3e-11 of 1/√2 at every band and rate below. The tolerances sit three orders above
        // that and still catch a mis-paired pole or an unwarped edge, which move an edge by whole
        // decibels.
        for rate in [22_050.0, 48_000.0, 192_000.0] {
            for centre in OCTAVE_BANDS_HZ.map(f64::from) {
                if !band_is_present(centre, rate) {
                    continue;
                }
                let filter = OctaveFilter::new(centre, rate);
                let digital_centre = {
                    let c = 2.0 * rate;
                    let lo = c * (PI * centre * FRAC_1_SQRT_2 / rate).tan();
                    let hi = c * (PI * centre * SQRT_2 / rate).tan();
                    rate / PI * ((lo * hi).sqrt() / c).atan()
                };
                let centre_gain = filter.gain_at(digital_centre, rate);
                assert!(
                    (centre_gain - 1.0).abs() < 1e-12,
                    "{centre} Hz at {rate}: {centre_gain}"
                );
                for edge in [centre * FRAC_1_SQRT_2, centre * SQRT_2] {
                    let gain = filter.gain_at(edge, rate);
                    assert!(
                        (gain - FRAC_1_SQRT_2).abs() < 1e-8,
                        "{centre} Hz edge {edge} at {rate}: {gain}"
                    );
                }
                assert!(filter.gain_at(0.0, rate) < 1e-12);
            }
        }
    }

    #[test]
    fn the_eight_kilohertz_band_is_absent_only_at_the_lowest_rate() {
        assert!(!band_is_present(8000.0, 22_050.0));
        assert!(band_is_present(4000.0, 22_050.0));
        for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            assert!(band_is_present(8000.0, rate));
        }
    }

    #[test]
    fn reversed_then_forward_filtering_is_zero_phase() {
        // The second pass cancels the first's phase, so an impulse comes out symmetric about itself:
        // as much energy before it as after. Measured: the two halves agree to 3.8e-14 of the total
        // at 125 Hz, where the ringing is longest; a forward-only filter puts all of it after, and
        // one reversed pass alone puts all of it before.
        let rate = 48_000.0;
        let mut impulse = vec![0.0f32; 48_000];
        impulse[24_000] = 1.0;
        let filtered = OctaveFilter::new(125.0, rate)
            .filter(&impulse)
            .expect("a second of samples");
        let before: f64 = filtered[..24_000].iter().map(|v| v * v).sum();
        let after: f64 = filtered[24_001..].iter().map(|v| v * v).sum();
        let total = before + after + filtered[24_000] * filtered[24_000];
        assert!(
            (before - after).abs() < 1e-10 * total,
            "before {before}, after {after}"
        );
    }
}
