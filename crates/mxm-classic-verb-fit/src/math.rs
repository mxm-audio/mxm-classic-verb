//! Small numeric helpers shared by the analysis modules: levels in decibels and a least-squares line.

/// The smallest energy a level in decibels is taken from: −300 dB re a mean square of one.
///
/// **Representational, not a measurement.** Exact digital silence has an energy of zero and a level
/// of −∞; the crate's contract is *finite or absent*, so a zero energy reads at this floor. It is far
/// below anything a 32-bit float sample can carry above zero once squared and averaged over a block
/// (a full-scale sample is 0 dB; 24-bit dither is about −150 dB), so it never decides a reading on a
/// real response.
pub(crate) const ENERGY_FLOOR: f64 = 1e-30;

/// An energy (a mean square, or a sum of squares) in decibels, floored at [`ENERGY_FLOOR`].
pub(crate) fn energy_db(energy: f64) -> f64 {
    // `f64::max` returns the other operand for a NaN, so a NaN energy reads at the floor rather than
    // propagating; no path in this crate produces one from finite samples.
    10.0 * energy.max(ENERGY_FLOOR).log10()
}

/// A duration in whole samples, at least one. Rounded, so a window means the same time at every rate
/// to within half a sample.
pub(crate) fn samples(seconds: f64, rate: f64) -> usize {
    // A float-to-integer cast saturates, and a NaN casts to zero, which `max` lifts to one.
    ((seconds * rate).round() as usize).max(1)
}

/// A value converted to `f32` only if it stays finite there.
pub(crate) fn finite_f32(value: f64) -> Option<f32> {
    let narrowed = value as f32;
    narrowed.is_finite().then_some(narrowed)
}

/// A value converted to `f32`, clamped into the finite `f32` range. For fields that are finite by
/// construction and must not be absent; a NaN reads as zero.
pub(crate) fn clamped_f32(value: f64) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
}

/// A straight line `y = intercept + slope · x`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    pub slope: f64,
    pub intercept: f64,
}

impl Line {
    pub fn at(&self, x: f64) -> f64 {
        self.intercept + self.slope * x
    }
}

/// Ordinary least squares, accumulated one point at a time.
///
/// `x` is accumulated relative to the first point so a short range far from zero does not lose its
/// spread to cancellation.
#[derive(Default)]
pub(crate) struct LineFit {
    origin: Option<f64>,
    n: f64,
    sx: f64,
    sy: f64,
    sxx: f64,
    sxy: f64,
}

impl LineFit {
    pub fn add(&mut self, x: f64, y: f64) {
        let origin = *self.origin.get_or_insert(x);
        let dx = x - origin;
        self.n += 1.0;
        self.sx += dx;
        self.sy += y;
        self.sxx += dx * dx;
        self.sxy += dx * y;
    }

    /// The fitted line, or absence for fewer than two distinct abscissae.
    pub fn line(&self) -> Option<Line> {
        let origin = self.origin?;
        if self.n < 2.0 {
            return None;
        }
        let denominator = self.n * self.sxx - self.sx * self.sx;
        if denominator <= 0.0 || !denominator.is_finite() {
            return None;
        }
        let slope = (self.n * self.sxy - self.sx * self.sy) / denominator;
        let intercept_at_origin = (self.sy - slope * self.sx) / self.n;
        let line = Line {
            slope,
            intercept: intercept_at_origin - slope * origin,
        };
        (line.slope.is_finite() && line.intercept.is_finite()).then_some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_through_exact_points_is_recovered() {
        let mut fit = LineFit::default();
        for i in 0..10 {
            let x = 5.0 + f64::from(i) * 0.001;
            fit.add(x, 3.0 - 60.0 * x);
        }
        let line = fit.line().expect("ten distinct points fit a line");
        assert!((line.slope + 60.0).abs() < 1e-9, "slope {}", line.slope);
        assert!(
            (line.intercept - 3.0).abs() < 1e-7,
            "intercept {}",
            line.intercept
        );
    }

    #[test]
    fn one_point_has_no_line_and_silence_reads_at_the_floor() {
        let mut fit = LineFit::default();
        fit.add(1.0, 1.0);
        assert!(fit.line().is_none());
        assert_eq!(energy_db(0.0), -300.0);
        assert_eq!(energy_db(f64::NAN), -300.0);
        assert!(finite_f32(1e300).is_none());
        assert_eq!(clamped_f32(f64::NAN), 0.0);
    }
}
