//! Delay storage, level followers and smoothing.

use mxm_dsp::delay::AgeLine;

/// The per-sample coefficient of a follower with the given time constant. **The one place a
/// follower's speed becomes a number**, so every follower is a time and the decisions it drives do
/// not depend on the sample rate.
pub(crate) fn follower_coefficient(seconds: f32, sample_rate: f32) -> f32 {
    let samples = (seconds * sample_rate).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

/// A circular delay line whose history is invalidated in constant time: the collection's
/// [`AgeLine`] (`mxm_dsp::delay`), sized and clamped as this crate's lines always were.
///
/// Reads are by **age**: age 1 is the sample most recently pushed. A read older than what has been
/// written since the last invalidation returns exact zero, which is what lets emptying the engine
/// cost nothing proportional to its memory. A fractional age is split into an integer age and a
/// fraction **before** any wrapping, so a position near the buffer's end cannot round onto it.
#[derive(Debug, Clone)]
pub(crate) struct DelayLine(AgeLine);

impl DelayLine {
    /// A line that can be read at any age up to `max_age` samples, fractional reads included. Its
    /// length is the power of two above `max_age + 4`, as before the line was shared.
    pub(crate) fn new(max_age: usize) -> Self {
        Self(AgeLine::new((max_age + 4).next_power_of_two()))
    }

    #[inline]
    pub(crate) fn push(&mut self, x: f32) {
        self.0.push(x);
    }

    /// The sample pushed `age` pushes ago; zero outside what has been written.
    #[inline]
    pub(crate) fn read(&self, age: usize) -> f32 {
        self.0.read(age)
    }

    /// A fractional age, linearly interpolated. Ages below one read the newest sample.
    #[inline]
    pub(crate) fn read_linear(&self, age: f32) -> f32 {
        self.0.read_linear(age.max(1.0))
    }

    /// Forgets every sample, in constant time.
    pub(crate) fn invalidate(&mut self) {
        self.0.clear();
    }
}

/// A linear ramp to a target, which arrives in a bounded time: `seconds` for a full-scale move.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ramp {
    value: f32,
    target: f32,
    step: f32,
}

impl Ramp {
    pub(crate) fn new(value: f32, seconds: f32, sample_rate: f32) -> Self {
        Self {
            value,
            target: value,
            step: 1.0 / (seconds * sample_rate).max(1.0),
        }
    }

    pub(crate) fn set(&mut self, target: f32) {
        self.target = target;
    }

    pub(crate) fn snap(&mut self) {
        self.value = self.target;
    }

    pub(crate) fn target(&self) -> f32 {
        self.target
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        if self.value < self.target {
            self.value = (self.value + self.step).min(self.target);
        } else if self.value > self.target {
            self.value = (self.value - self.step).max(self.target);
        }
        self.value
    }
}

/// A one-pole approach to a target, which lands on the target exactly.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Smoother {
    value: f32,
    target: f32,
    coef: f32,
}

impl Smoother {
    pub(crate) fn new(value: f32, seconds: f32, sample_rate: f32) -> Self {
        Self {
            value,
            target: value,
            coef: follower_coefficient(seconds, sample_rate),
        }
    }

    pub(crate) fn set(&mut self, target: f32) {
        self.target = target;
    }

    pub(crate) fn snap(&mut self) {
        self.value = self.target;
    }

    pub(crate) fn target(&self) -> f32 {
        self.target
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        let d = self.target - self.value;
        if d.abs() <= 1.0e-6 * (1.0 + self.target.abs()) {
            self.value = self.target;
        } else {
            self.value += self.coef * d;
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_reads_back_by_age_and_forgets_in_constant_time() {
        let mut line = DelayLine::new(8);
        for x in 1..=5 {
            line.push(x as f32);
        }
        assert_eq!(line.read(1), 5.0);
        assert_eq!(line.read(5), 1.0);
        assert_eq!(line.read(6), 0.0, "older than anything written reads zero");
        assert_eq!(line.read_linear(1.5), 4.5);
        line.invalidate();
        assert_eq!(line.read(1), 0.0);
        line.push(9.0);
        assert_eq!(line.read(1), 9.0);
        assert_eq!(line.read(2), 0.0);
    }

    #[test]
    fn a_smoother_lands_on_its_target_exactly() {
        let mut s = Smoother::new(1.0, 0.01, 48_000.0);
        s.set(0.0);
        let mut last = 1.0;
        for _ in 0..48_000 {
            last = s.next();
        }
        assert_eq!(last, 0.0);
    }
}
