//! The loop's decay filter, the output tilt and high cut, and the diffusion allpass.

use crate::delay::{DelayLine, flush};
use crate::space::HIGH_CUT_OPEN_HZ;
use core::f32::consts::TAU;

/// The pole of a one-pole lowpass at `hz`, kept below 0.45 of the sample rate.
pub(crate) fn pole(hz: f32, sample_rate: f32) -> f32 {
    let hz = hz.clamp(1.0, 0.45 * sample_rate);
    (-TAU * hz / sample_rate).exp()
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OnePole {
    state: f32,
}

impl OnePole {
    #[inline]
    pub(crate) fn lowpass(&mut self, x: f32, pole: f32) -> f32 {
        self.state = flush(x + pole * (self.state - x));
        self.state
    }

    pub(crate) fn reset(&mut self) {
        self.state = 0.0;
    }
}

/// One delay line's attenuation per trip in four bands, as linear gains.
///
/// Each shelf's ratio is its band's gain **relative to the band below it** — the high band's to the
/// mid band's, the top band's to the high band's — which is how the cascade realises them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BandGains {
    pub low: f32,
    pub mid: f32,
    pub high_ratio: f32,
    pub top_ratio: f32,
}

impl BandGains {
    pub(crate) const SILENT: BandGains = BandGains {
        low: 0.0,
        mid: 0.0,
        high_ratio: 1.0,
        top_ratio: 1.0,
    };

    /// The most the decay filter can pass at any frequency, **proved rather than measured**:
    ///
    /// - the first stage is `mid + (low − mid)·LP`, and a one-pole lowpass's response lies in the
    ///   disc whose diameter is the real segment from its Nyquist gain to 1, so the stage's magnitude
    ///   is at most `max(low, mid)`;
    /// - each shelf is `LP + r·(1 − LP)` on a bilinear lowpass, whose response lies on the circle
    ///   whose diameter is the segment from 0 to 1; the shelf maps that disc onto the disc on the
    ///   segment from `r` to 1, so it is at most `max(1, r)`.
    pub(crate) fn bound(&self) -> f32 {
        self.low.max(self.mid) * self.high_ratio.max(1.0) * self.top_ratio.max(1.0)
    }

    /// The gains limited so [`bound`](Self::bound) cannot exceed `ceiling`: the low and mid gains,
    /// then the top shelf's lift, then the high shelf's.
    pub(crate) fn limited(mut self, ceiling: f32) -> Self {
        self.low = self.low.clamp(0.0, ceiling);
        self.mid = self.mid.clamp(0.0, ceiling);
        self.high_ratio = self.high_ratio.max(0.0);
        self.top_ratio = self.top_ratio.max(0.0);
        let lm = self.low.max(self.mid);
        if lm > 0.0 && self.bound() > ceiling {
            let room = ceiling / (lm * self.high_ratio.max(1.0));
            if room >= 1.0 {
                self.top_ratio = room;
            } else {
                self.top_ratio = self.top_ratio.min(1.0);
                self.high_ratio = ceiling / lm;
            }
            // A quotient can round up by an ulp, and the product past the ceiling.
            while self.bound() > ceiling {
                if self.top_ratio > 1.0 {
                    self.top_ratio = self.top_ratio.next_down();
                } else {
                    self.high_ratio = self.high_ratio.next_down();
                }
            }
        }
        self
    }
}

/// A first-order lowpass by the bilinear transform with its corner prewarped:
/// `k·(1 + z⁻¹)/(1 + a·z⁻¹)`, with `t = tan(π·f_c/f_s)`, `k = t/(1 + t)` and `a = (t − 1)/(t + 1)`.
///
/// Unity at DC and zero at Nyquist at every rate, where the one-pole's impulse-invariant form still
/// passes `(1 − p)/(1 + p)` there, so a shelf on it reaches its band gain at the top of the spectrum.
/// The transform maps the unit circle onto the analog one-pole's frequency axis, so the response lies
/// exactly on the circle whose diameter is the segment from 0 to 1, which [`BandGains::bound`] uses.
/// Coefficients in f64, as the root contract asks of a recursive filter's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BilinearCoefficients {
    k: f32,
    a: f32,
}

impl BilinearCoefficients {
    /// The lowpass at `hz`, held below 0.45 of the sample rate as every corner in this crate is.
    pub(crate) fn new(hz: f32, sample_rate: f32) -> Self {
        let hz = f64::from(hz.clamp(1.0, 0.45 * sample_rate));
        let t = (core::f64::consts::PI * hz / f64::from(sample_rate)).tan();
        Self {
            k: (t / (1.0 + t)) as f32,
            a: ((t - 1.0) / (t + 1.0)) as f32,
        }
    }

    /// The complex response at `w` radians per sample, as (re, im).
    pub(crate) fn response(&self, w: f32) -> (f32, f32) {
        let (c, s) = (w.cos(), w.sin());
        let (nr, ni) = (self.k * (1.0 + c), -self.k * s);
        let (dr, di) = (1.0 + self.a * c, -self.a * s);
        let den = dr * dr + di * di;
        ((nr * dr + ni * di) / den, (ni * dr - nr * di) / den)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BilinearLowpass {
    state: f32,
}

impl BilinearLowpass {
    /// One sample, in transposed direct form II.
    #[inline]
    pub(crate) fn process(&mut self, x: f32, c: &BilinearCoefficients) -> f32 {
        let kx = c.k * x;
        let y = kx + self.state;
        self.state = flush(kx - c.a * y);
        y
    }

    pub(crate) fn reset(&mut self) {
        self.state = 0.0;
    }
}

/// The per-line decay filter, in four bands (plan D5): a low shelf on a one-pole lowpass at
/// `LOW_CROSSOVER_HZ`, then two shelves on bilinear lowpasses at `DECAY_HIGH_CROSSOVER_HZ` and
/// `DECAY_TOP_CROSSOVER_HZ`, every stage first order.
///
/// First-order shelves only approach their band gains, so the realised decay per octave differs from
/// the composed band targets near and between the crossovers, which the crate's `AGENTS.md` records
/// as measured and which the fit accounts for.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DecayFilter {
    lo: OnePole,
    high: BilinearLowpass,
    top: BilinearLowpass,
}

impl DecayFilter {
    #[inline]
    pub(crate) fn process(
        &mut self,
        x: f32,
        g: BandGains,
        pole_lo: f32,
        high: &BilinearCoefficients,
        top: &BilinearCoefficients,
    ) -> f32 {
        let y = g.mid * x + (g.low - g.mid) * self.lo.lowpass(x, pole_lo);
        let lp = self.high.process(y, high);
        let y = lp + g.high_ratio * (y - lp);
        let lp = self.top.process(y, top);
        lp + g.top_ratio * (y - lp)
    }

    pub(crate) fn reset(&mut self) {
        self.lo.reset();
        self.high.reset();
        self.top.reset();
    }
}

/// A one-pole lowpass's complex response at `w` radians per sample, as (re, im):
/// `(1 − p)/(1 − p·e^{−jw})`.
pub(crate) fn lowpass_response(pole: f32, w: f32) -> (f32, f32) {
    let (re, im) = (1.0 - pole * w.cos(), pole * w.sin());
    let den = re * re + im * im;
    ((1.0 - pole) * re / den, -(1.0 - pole) * im / den)
}

/// The tilt's magnitude at a frequency whose lowpass responses are `lo` and `hi`.
#[inline]
pub(crate) fn tilt_magnitude(low_gain: f32, high_gain: f32, lo: (f32, f32), hi: (f32, f32)) -> f32 {
    let (a, b) = (1.0 + (low_gain - 1.0) * lo.0, (low_gain - 1.0) * lo.1);
    let (c, d) = (
        high_gain + (1.0 - high_gain) * hi.0,
        (1.0 - high_gain) * hi.1,
    );
    ((a * c - b * d).powi(2) + (a * d + b * c).powi(2)).sqrt()
}

/// The decay filter's magnitude at a frequency whose lowpass responses are `lo` (the one-pole) and
/// `high` and `top` (the bilinear lowpasses): the product of its three stages' magnitudes.
pub(crate) fn decay_magnitude(
    g: BandGains,
    lo: (f32, f32),
    high: (f32, f32),
    top: (f32, f32),
) -> f32 {
    let low = g.low - g.mid;
    let first = ((g.mid + low * lo.0).powi(2) + (low * lo.1).powi(2)).sqrt();
    let shelf =
        |r: f32, h: (f32, f32)| ((r + (1.0 - r) * h.0).powi(2) + ((1.0 - r) * h.1).powi(2)).sqrt();
    first * shelf(g.high_ratio, high) * shelf(g.top_ratio, top)
}

/// The output tone: the same two shelves outside the loop. First-order shelves leak into the mid
/// band, so the engine divides by [`tilt_magnitude`] at its reference frequency, which is what keeps
/// tone from moving the wet level at mid (plan §2).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Tilt {
    lo: OnePole,
    hi: OnePole,
}

impl Tilt {
    #[inline]
    pub(crate) fn process(
        &mut self,
        x: f32,
        low_gain: f32,
        high_gain: f32,
        pole_lo: f32,
        pole_hi: f32,
    ) -> f32 {
        let y = x + (low_gain - 1.0) * self.lo.lowpass(x, pole_lo);
        let lp = self.hi.lowpass(y, pole_hi);
        lp + high_gain * (y - lp)
    }

    pub(crate) fn reset(&mut self) {
        self.lo.reset();
        self.hi.reset();
    }
}

/// The space's high cut: a second-order Butterworth lowpass (Butterworth, 1930), by the bilinear
/// transform with its corner prewarped — the form of Bristow-Johnson's *Audio EQ Cookbook* low-pass
/// at Q = 1/√2. Its magnitude is therefore exactly `1/√(1 + (tan(ω/2)/tan(ω_c/2))⁴)`: 3 dB down at
/// the corner at every rate, maximally flat below it, never above unity, and zero at Nyquist.
///
/// **Why second order**, measured through the fit's band model on tone curves built from the owner's
/// pack's aggregate figures (this crate's `AGENTS.md`): a first-order cut cannot fall 9 dB an octave
/// faster than a white field between 4 and 8 kHz without denting 2 kHz by 3–4 dB; third and fourth
/// order fall too abruptly for a tone that darkens gradually. The corner is in hertz, held below 0.45
/// of the sample rate as every corner in this crate is. `None` is an open cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HighCutCoefficients {
    b0: f32,
    a1: f32,
    a2: f32,
    /// `tan(ω_c/2)`, for the closed form.
    warped: f32,
}

impl HighCutCoefficients {
    /// The cut at `corner_hz`, or `None` where the corner is open (at or above
    /// [`HIGH_CUT_OPEN_HZ`], or not a number).
    pub(crate) fn new(corner_hz: f32, sample_rate: f32) -> Option<Self> {
        if corner_hz.is_nan() || corner_hz >= HIGH_CUT_OPEN_HZ {
            return None;
        }
        let corner = f64::from(corner_hz.clamp(1.0, 0.45 * sample_rate));
        // Coefficients in f64, as the root contract asks of a recursive filter's.
        let k = (core::f64::consts::PI * corner / f64::from(sample_rate)).tan();
        let norm = 1.0 / (1.0 + core::f64::consts::SQRT_2 * k + k * k);
        Some(Self {
            b0: (k * k * norm) as f32,
            a1: (2.0 * (k * k - 1.0) * norm) as f32,
            a2: ((1.0 - core::f64::consts::SQRT_2 * k + k * k) * norm) as f32,
            warped: k as f32,
        })
    }

    /// The magnitude at `w` radians per sample, in closed form.
    pub(crate) fn magnitude(&self, w: f32) -> f32 {
        let r = (0.5 * w).tan() / self.warped;
        let r2 = r * r;
        1.0 / (1.0 + r2 * r2).sqrt()
    }
}

/// The high cut's state, in transposed direct form II. Its zeros sit exactly at Nyquist, because
/// `b1 = 2·b0` and `b2 = b0` survive rounding.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HighCut {
    s1: f32,
    s2: f32,
}

impl HighCut {
    #[inline]
    pub(crate) fn process(&mut self, x: f32, c: &HighCutCoefficients) -> f32 {
        let bx = c.b0 * x;
        let y = bx + self.s1;
        self.s1 = flush(2.0 * bx - c.a1 * y + self.s2);
        self.s2 = flush(bx - c.a2 * y);
        y
    }

    pub(crate) fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// A Schroeder allpass (Schroeder and Logan, 1961): `(z⁻ᵈ − g) / (1 − g·z⁻ᵈ)`, unit magnitude.
#[derive(Debug, Clone)]
pub(crate) struct Allpass {
    line: DelayLine,
    delay: usize,
}

impl Allpass {
    pub(crate) fn new(delay_s: f32, sample_rate: f32) -> Self {
        let delay = ((delay_s * sample_rate).round() as usize).max(1);
        Self {
            line: DelayLine::new(delay),
            delay,
        }
    }

    #[inline]
    pub(crate) fn process(&mut self, x: f32, g: f32) -> f32 {
        let delayed = self.line.read(self.delay);
        let v = flush(x + g * delayed);
        self.line.push(v);
        delayed - g * v
    }

    pub(crate) fn invalidate(&mut self) {
        self.line.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady-state magnitude at `hz`: the RMS of the last second of a two-second sine, ×√2. The
    /// sine's phase is computed in f64: in f32 its rounding at tens of thousands of radians is a
    /// broadband noise about 52 dB under the sine, which a deep stopband would read instead.
    fn magnitude(mut f: impl FnMut(f32) -> f32, hz: f32, fs: f32) -> f32 {
        let n = (fs * 2.0) as usize;
        let mut sum = 0.0f64;
        for i in 0..n {
            let phase = core::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(fs);
            let y = f64::from(f(phase.sin() as f32));
            if i >= n / 2 {
                sum += y * y;
            }
        }
        (2.0 * sum / (n / 2) as f64).sqrt() as f32
    }

    /// The same, in closed form and in f64, from the corners rather than the crate's coefficients: the
    /// one-pole `(1 − p)/(1 − p·e^{−jω})` and each bilinear lowpass
    /// `k·(1 + e^{−jω})/(1 + a·e^{−jω})`, composed as the filter composes them.
    fn analytic(g: BandGains, pole_lo: f32, high_hz: f32, top_hz: f32, hz: f32, fs: f32) -> f32 {
        let (w, fs) = (TAU as f64 * hz as f64 / fs as f64, fs as f64);
        let e = (w.cos(), -w.sin());
        let div = |(nr, ni): (f64, f64), (dr, di): (f64, f64)| {
            let den = dr * dr + di * di;
            ((nr * dr + ni * di) / den, (ni * dr - nr * di) / den)
        };
        let one_pole = |p: f64| div((1.0 - p, 0.0), (1.0 - p * e.0, -p * e.1));
        let bilinear = |corner: f64| {
            let t = (std::f64::consts::PI * corner / fs).tan();
            let (k, a) = (t / (1.0 + t), (t - 1.0) / (t + 1.0));
            div((k * (1.0 + e.0), k * e.1), (1.0 + a * e.0, a * e.1))
        };
        let shelf = |r: f64, (re, im): (f64, f64)| (r + (1.0 - r) * re).hypot((1.0 - r) * im);
        let (low, mid) = (g.low as f64, g.mid as f64);
        let lo = one_pole(pole_lo as f64);
        ((mid + (low - mid) * lo.0).hypot((low - mid) * lo.1)
            * shelf(g.high_ratio as f64, bilinear(high_hz as f64))
            * shelf(g.top_ratio as f64, bilinear(top_hz as f64))) as f32
    }

    #[test]
    fn the_decay_filter_matches_its_closed_form_and_never_exceeds_its_bound() {
        let fs = 48_000.0;
        let (high_hz, top_hz) = (
            crate::DECAY_HIGH_CROSSOVER_HZ,
            crate::DECAY_TOP_CROSSOVER_HZ,
        );
        let pl = pole(250.0, fs);
        let (ch, ct) = (
            BilinearCoefficients::new(high_hz, fs),
            BilinearCoefficients::new(top_hz, fs),
        );
        let natural = BandGains {
            low: 0.95,
            mid: 0.8,
            high_ratio: 0.7,
            top_ratio: 0.5,
        };
        // Scooped settings — mid decaying fastest, both shelves lifting — are the case the bound
        // exists for: one inside the ceiling, and one the limit has to pull back.
        let lifted = BandGains {
            low: 0.9,
            mid: 0.5,
            high_ratio: 1.05,
            top_ratio: 1.04,
        };
        let limited = BandGains {
            low: 0.99,
            mid: 0.5,
            high_ratio: 1.4,
            top_ratio: 1.6,
        }
        .limited(0.999);
        assert!(lifted.limited(0.999) == lifted);
        assert!(limited.bound() <= 0.999, "{limited:?}");
        for g in [natural, lifted, limited] {
            for hz in [
                5.0, 60.0, 250.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 12_000.0, 20_000.0,
            ] {
                let mut f = DecayFilter::default();
                let m = magnitude(|x| f.process(x, g, pl, &ch, &ct), hz, fs);
                let exact = analytic(g, pl, high_hz, top_hz, hz, fs);
                let w = TAU * hz / fs;
                let closed =
                    decay_magnitude(g, lowpass_response(pl, w), ch.response(w), ct.response(w));
                assert!(
                    (m - exact).abs() < 0.01 * exact.max(0.05),
                    "{g:?} at {hz} Hz: rendered {m} against {exact}"
                );
                assert!(
                    (closed - exact).abs() < 1e-4 * exact.max(0.05),
                    "{g:?} at {hz} Hz: closed form {closed} against {exact}"
                );
                assert!(
                    m <= g.bound() + 1e-3,
                    "{hz} Hz passed {m} against bound {}",
                    g.bound()
                );
            }
            // Between the rendered frequencies too, in closed form, up to Nyquist.
            for k in 0..=2_000 {
                let hz = 0.5 * fs * k as f32 / 2_000.0;
                let exact = analytic(g, pl, high_hz, top_hz, hz, fs);
                assert!(
                    exact <= g.bound() * (1.0 + 1e-5),
                    "{g:?} at {hz} Hz: {exact} against {}",
                    g.bound()
                );
            }
        }
        // The shelves approach the band gains well inside each band; at Nyquist the bilinear shelves
        // reach theirs exactly, and only the one-pole's low shelf still leaks.
        assert!((analytic(natural, pl, high_hz, top_hz, 5.0, fs) - 0.95).abs() < 0.01);
        assert!(
            (analytic(natural, pl, high_hz, top_hz, 24_000.0, fs) - 0.8 * 0.7 * 0.5).abs() < 0.01
        );
    }

    #[test]
    fn the_bilinear_lowpass_is_unity_at_dc_zero_at_nyquist_and_on_its_circle() {
        for fs in [8_000.0, 44_100.0, 192_000.0] {
            for corner in [100.0, 2_000.0, 5_656.854, 30_000.0] {
                let c = BilinearCoefficients::new(corner, fs);
                // f32 coefficients: at 100 Hz and 192 kHz, 1 + a is 0.0033 and rounding leaves the DC
                // gain 6e-6 short (measured), far inside the loop ceiling's 1e-4 of headroom.
                let dc = c.response(0.0);
                assert!(
                    (dc.0 - 1.0).abs() < 1e-5 && dc.1.abs() < 1e-6,
                    "{corner} at {fs}: DC {dc:?}"
                );
                let nyquist = c.response(core::f32::consts::PI);
                assert!(
                    nyquist.0.hypot(nyquist.1) < 1e-6,
                    "{corner} at {fs}: Nyquist {nyquist:?}"
                );
                for k in 0..=400 {
                    let (re, im) = c.response(core::f32::consts::PI * k as f32 / 400.0);
                    assert!(
                        re.hypot(im) <= 1.0 + 1e-6,
                        "{corner} at {fs}, step {k}: ({re}, {im})"
                    );
                    // |H − ½| = ½: the circle the bound's argument needs, to rounding.
                    assert!(
                        ((re - 0.5).hypot(im) - 0.5).abs() < 1e-4,
                        "{corner} at {fs}, step {k}"
                    );
                }
                // Prewarped: 3 dB down at its corner, held below Nyquist, at every rate.
                let held = corner.min(0.45 * fs);
                let mut lp = BilinearLowpass::default();
                let rendered = magnitude(|x| lp.process(x, &c), held, fs);
                assert!(
                    (rendered - core::f32::consts::FRAC_1_SQRT_2).abs() < 0.01,
                    "{corner} at {fs}: {rendered} at the corner"
                );
            }
        }
    }

    #[test]
    fn the_tilt_normalised_at_its_reference_leaves_the_mid_band_at_unity() {
        let fs = 48_000.0;
        let (pl, ph) = (pole(250.0, fs), pole(4_000.0, fs));
        let w = TAU * 1_000.0 / fs;
        let raw = tilt_magnitude(4.0, 0.25, lowpass_response(pl, w), lowpass_response(ph, w));
        let mut t = Tilt::default();
        let unnormalised = magnitude(|x| t.process(x, 4.0, 0.25, pl, ph), 1_000.0, fs);
        assert!(
            (unnormalised - raw).abs() < 0.01 * raw,
            "the closed form matches the filter: {unnormalised} against {raw}"
        );
        assert!(
            (raw - 1.0).abs() > 0.05,
            "without the normalisation the tilt does move the mid band: {raw}"
        );
        let mut t = Tilt::default();
        let at_mid = magnitude(|x| t.process(x, 4.0, 0.25, pl, ph) / raw, 1_000.0, fs);
        assert!((at_mid - 1.0).abs() < 0.01, "{at_mid}");
    }

    #[test]
    fn the_public_tone_curve_is_the_engines_normalised_tilt() {
        // The whole tone path — both shelves, then the high cut — rendered, divided by its own
        // magnitude at the reference, against the public closed form. The cut runs from open through
        // its floor, where the stopband at 12 kHz lies about 60 dB under the passband.
        for fs in [44_100.0, 48_000.0, 96_000.0] {
            let (pl, ph) = (
                pole(crate::LOW_CROSSOVER_HZ, fs),
                pole(crate::HIGH_CROSSOVER_HZ, fs),
            );
            let w_ref = TAU * crate::TONE_REFERENCE_HZ / fs;
            for (low_db, high_db) in [(12.0_f32, -12.0_f32), (-18.0, 6.0), (24.0, 24.0)] {
                let (gl, gh) = (10f32.powf(low_db / 20.0), 10f32.powf(high_db / 20.0));
                for cut_hz in [
                    HIGH_CUT_OPEN_HZ,
                    crate::space::MIN_HIGH_CUT_HZ,
                    3_000.0,
                    9_000.0,
                ] {
                    let cut = HighCutCoefficients::new(cut_hz, fs);
                    let norm = tilt_magnitude(
                        gl,
                        gh,
                        lowpass_response(pl, w_ref),
                        lowpass_response(ph, w_ref),
                    ) * cut.map_or(1.0, |c| c.magnitude(w_ref));
                    for hz in [80.0, 250.0, 1_000.0, 4_000.0, 8_000.0, 12_000.0] {
                        let (mut t, mut h) = (Tilt::default(), HighCut::default());
                        let rendered = magnitude(
                            |x| {
                                let y = t.process(x, gl, gh, pl, ph);
                                cut.map_or(y, |c| h.process(y, &c)) / norm
                            },
                            hz,
                            fs,
                        );
                        let closed = crate::tone_magnitude(low_db, high_db, cut_hz, fs, hz);
                        assert!(
                            (rendered - closed).abs() < 0.01 * closed,
                            "{fs} Hz, {low_db}/{high_db} dB, cut {cut_hz} Hz, at {hz} Hz: {rendered} against {closed}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_high_cut_is_open_at_its_top_three_decibels_down_at_its_corner_and_never_above_unity() {
        for fs in [8_000.0, 44_100.0, 192_000.0] {
            for open in [HIGH_CUT_OPEN_HZ, 1.0e6, f32::INFINITY, f32::NAN] {
                assert!(
                    HighCutCoefficients::new(open, fs).is_none(),
                    "{open} at {fs}"
                );
            }
            for corner in [crate::space::MIN_HIGH_CUT_HZ, 3_000.0, 19_999.0] {
                let c = HighCutCoefficients::new(corner, fs).expect("below open is a filter");
                // Held below Nyquist as every corner is; 3 dB down there, prewarped, at every rate.
                let held = corner.min(0.45 * fs);
                let closed = c.magnitude(TAU * held / fs);
                assert!(
                    (closed - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4,
                    "{corner} at {fs}: {closed}"
                );
                let mut h = HighCut::default();
                let rendered = magnitude(|x| h.process(x, &c), held, fs);
                assert!(
                    (rendered - closed).abs() < 0.01 * closed,
                    "{corner} at {fs}: rendered {rendered} against {closed}"
                );
                for k in 0..=400 {
                    let w = core::f32::consts::PI * k as f32 / 400.0;
                    assert!(c.magnitude(w) <= 1.0 + 1e-6, "{corner} at {fs}, w {w}");
                }
            }
        }
    }

    #[test]
    fn the_allpass_has_unit_magnitude() {
        let fs = 48_000.0;
        for hz in [100.0, 1_000.0, 7_000.0] {
            let mut ap = Allpass::new(0.004, fs);
            let m = magnitude(|x| ap.process(x, 0.7), hz, fs);
            assert!((m - 1.0).abs() < 0.01, "{hz} Hz: {m}");
        }
    }
}
