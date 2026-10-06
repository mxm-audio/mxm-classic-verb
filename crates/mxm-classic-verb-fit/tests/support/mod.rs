//! Planted synthetic impulse responses: every descriptor is known because it was put there.
//!
//! **Independent of the analyser under test.** The bands are brick-wall octaves made in the frequency
//! domain with `mxm-measure`'s transform, not the analyser's Butterworth filters, so a filter defect
//! cannot plant the answer it then reads. Nothing here reads or writes a file.
#![allow(dead_code)]

use mxm_measure::spectrum::ifft;

pub const BANDS_HZ: [f64; 7] = [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

/// Marsaglia's xorshift64 with the (13, 7, 17) triple. Seeded, so every response is bit-repeatable.
pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> Self {
        // Multiplying by an odd constant is a bijection, so distinct seeds give distinct states; the
        // one seed that maps to zero, which xorshift cannot leave, is moved off it.
        let state = seed.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        Self(if state == 0 { 1 } else { state })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in −1..1.
    pub fn bipolar(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// Noise with a spectrum confined to `low_hz..high_hz` and shaped by `weight(hz)` on amplitude,
/// normalised to a mean square of one over `frames`.
fn shaped_noise(
    frames: usize,
    rate: f64,
    low_hz: f64,
    high_hz: f64,
    weight: impl Fn(f64) -> f64,
    rng: &mut XorShift,
) -> Vec<f64> {
    let n = frames.next_power_of_two().max(4);
    let mut re = vec![0.0f64; n];
    let mut im = vec![0.0f64; n];
    let bin_hz = rate / n as f64;
    let first = ((low_hz / bin_hz).ceil() as usize).max(1);
    let last = ((high_hz / bin_hz).ceil() as usize).min(n / 2);
    for k in first..last {
        let w = weight(k as f64 * bin_hz);
        let (a, b) = (rng.bipolar() * w, rng.bipolar() * w);
        re[k] = a;
        im[k] = b;
        re[n - k] = a;
        im[n - k] = -b;
    }
    ifft(&mut re, &mut im).expect("a power-of-two length transforms");
    re.truncate(frames);
    let mean_square = re.iter().map(|v| v * v).sum::<f64>() / frames as f64;
    let scale = if mean_square > 0.0 {
        mean_square.sqrt().recip()
    } else {
        0.0
    };
    re.iter_mut().for_each(|v| *v *= scale);
    re
}

/// How the two channels of a planted response relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Mono,
    /// Independent tails and noise in each channel.
    Decorrelated,
    /// The same samples in both channels.
    Identical,
}

#[derive(Clone, Copy, Debug)]
pub struct PlantedReflection {
    /// After the direct sound's peak.
    pub delay_s: f64,
    /// Amplitude against the direct sound, per channel.
    pub gain: [f64; 2],
}

/// A second exponential added to every band's energy envelope.
#[derive(Clone, Copy, Debug)]
pub struct SecondSlope {
    pub t60_s: f64,
    /// Its starting energy below the first slope's, in dB.
    pub below_db: f64,
}

/// What is planted. Levels are mean squares per channel in dB re one.
#[derive(Clone, Debug)]
pub struct Plan {
    pub rate: f64,
    pub layout: Layout,
    pub duration_s: f64,
    /// Where the direct sound peaks.
    pub direct_s: f64,
    pub reflections: Vec<PlantedReflection>,
    /// After the direct sound: where the decaying tail begins, abruptly.
    pub tail_start_s: f64,
    /// The 1 kHz band's tail at its start.
    pub tail_db: f64,
    /// Each band's tail at its start, against the 1 kHz band's.
    pub band_db: [f64; 7],
    pub t60_s: [f64; 7],
    pub second_slope: Option<SecondSlope>,
    /// The noise floor in every octave band (pink, so equal per octave).
    pub noise_db: f64,
    pub seed: u64,
}

/// The directly planted reflections' delays, in seconds after the direct sound.
pub const ROOM_REFLECTIONS: [PlantedReflection; 5] = [
    PlantedReflection {
        delay_s: 0.0071,
        gain: [0.50, 0.45],
    },
    PlantedReflection {
        delay_s: 0.0123,
        gain: [0.35, 0.40],
    },
    PlantedReflection {
        delay_s: 0.0187,
        gain: [0.32, 0.25],
    },
    PlantedReflection {
        delay_s: 0.0262,
        gain: [0.22, 0.25],
    },
    PlantedReflection {
        delay_s: 0.0339,
        gain: [0.18, 0.15],
    },
];

impl Plan {
    /// An ordinary room: five reflections, a tail from 45 ms with T60 falling from 1.4 s at 125 Hz to
    /// 0.6 s at 8 kHz, and a noise floor 51–57 dB under the tail's start.
    pub fn room(rate: f64, layout: Layout) -> Self {
        Self {
            rate,
            layout,
            duration_s: 2.5,
            direct_s: 0.05,
            reflections: ROOM_REFLECTIONS.to_vec(),
            tail_start_s: 0.045,
            tail_db: -30.0,
            band_db: [2.0, 1.5, 1.0, 0.0, -1.0, -2.5, -4.0],
            t60_s: [1.4, 1.3, 1.2, 1.0, 0.9, 0.75, 0.6],
            second_slope: None,
            noise_db: -85.0,
            seed: 1,
        }
    }

    pub fn build(&self) -> Planted {
        let rate = self.rate;
        let frames = (self.duration_s * rate).round() as usize;
        let count = if self.layout == Layout::Mono { 1 } else { 2 };
        let direct = (self.direct_s * rate).round() as usize;
        let pulse = hann_pulse(rate);
        let half = pulse.len() / 2;
        let nyquist = rate * 0.5;

        let mut rest: Vec<Vec<f64>> = vec![vec![0.0; frames]; count];
        let mut rng = XorShift::new(self.seed);
        for (channel, signal) in rest.iter_mut().enumerate() {
            if channel == 1 && self.layout == Layout::Identical {
                break;
            }
            // Reflections: the direct sound's pulse, delayed and scaled.
            for reflection in &self.reflections {
                let centre = direct + (reflection.delay_s * rate).round() as usize;
                for (k, &p) in pulse.iter().enumerate() {
                    let i = centre + k - half;
                    if i < frames {
                        signal[i] += reflection.gain[channel] * p;
                    }
                }
            }
            // The tail, band by band.
            let tail_first = direct + (self.tail_start_s * rate).round() as usize;
            let tail_frames = frames.saturating_sub(tail_first);
            for (b, &centre_hz) in BANDS_HZ.iter().enumerate() {
                let high = centre_hz * std::f64::consts::SQRT_2;
                if high >= nyquist || tail_frames == 0 {
                    continue;
                }
                let noise = shaped_noise(
                    tail_frames,
                    rate,
                    centre_hz * std::f64::consts::FRAC_1_SQRT_2,
                    high,
                    |_| 1.0,
                    &mut rng,
                );
                let start = 10f64.powf((self.tail_db + self.band_db[b]) / 10.0);
                for (n, &v) in noise.iter().enumerate() {
                    let t = n as f64 / rate;
                    let mut envelope = start * 10f64.powf(-6.0 * t / self.t60_s[b]);
                    if let Some(second) = self.second_slope {
                        envelope += start
                            * 10f64.powf(-second.below_db / 10.0)
                            * 10f64.powf(-6.0 * t / second.t60_s);
                    }
                    signal[tail_first + n] += envelope.sqrt() * v;
                }
            }
            // The noise floor: pink across the planted octaves, so each octave holds `noise_db`.
            let octaves = BANDS_HZ
                .iter()
                .filter(|&&c| c * std::f64::consts::SQRT_2 < nyquist)
                .count();
            let top = BANDS_HZ[octaves - 1] * std::f64::consts::SQRT_2;
            let floor = shaped_noise(
                frames,
                rate,
                BANDS_HZ[0] * std::f64::consts::FRAC_1_SQRT_2,
                top,
                |hz| hz.sqrt().recip(),
                &mut rng,
            );
            let amplitude = (octaves as f64 * 10f64.powf(self.noise_db / 10.0)).sqrt();
            for (s, v) in signal.iter_mut().zip(floor) {
                *s += amplitude * v;
            }
        }
        if self.layout == Layout::Identical {
            rest[1] = rest[0].clone();
        }

        let mut channels: Vec<Vec<f32>> = Vec::with_capacity(count);
        let mut direct_energy = 0.0;
        let mut rest_energy = 0.0;
        for signal in &rest {
            let mut out: Vec<f64> = signal.clone();
            for (k, &p) in pulse.iter().enumerate() {
                out[direct + k - half] += p;
                direct_energy += p * p;
            }
            rest_energy += signal.iter().map(|v| v * v).sum::<f64>();
            channels.push(out.iter().map(|&v| v as f32).collect());
        }
        Planted {
            channels,
            direct_index: direct,
            drr_db: 10.0 * (direct_energy / rest_energy).log10(),
        }
    }
}

/// The direct sound and every reflection: a Hann pulse about 0.3 ms long, peak one.
pub fn hann_pulse(rate: f64) -> Vec<f64> {
    let len = 2 * (0.000_15 * rate).round() as usize + 1;
    (0..len)
        .map(|k| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * k as f64 / (len - 1) as f64).cos())
        .collect()
}

/// Every numeric field of an analysis is finite or absent.
///
/// `Debug` prints every field, nested ones included, and prints a NaN as `NaN` and an infinity as
/// `inf`; no field or variant name in the analysis contains either, so their absence from the text is
/// the absence from every field. A handful of fields are checked directly as well, so the scan is not
/// the only line of defence.
pub fn assert_finite(analysis: &mxm_classic_verb_fit::Analysis) {
    let text = format!("{analysis:?}");
    assert!(!text.contains("NaN"), "a NaN in {text}");
    assert!(!text.contains("inf"), "an infinity in {text}");
    assert!(analysis.validity.confidence.is_finite());
    assert!(analysis.onset.direct_s.is_finite());
    for band in &analysis.bands {
        if let Some(decay) = band.decay() {
            assert!(decay.noise_floor_db.is_finite() && decay.margin_db.is_finite());
            for t in [decay.t20_s, decay.t30_s, decay.edt_s]
                .into_iter()
                .flatten()
            {
                assert!(t.is_finite());
            }
        }
    }
}

/// A standard normal variate, by Box and Muller's transform of two uniform variates.
pub fn gaussian(rng: &mut XorShift) -> f64 {
    // (0, 1], so the logarithm is finite.
    let u1 = ((rng.next_u64() >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
    let u2 = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// What decays in a planted tail whose texture is known because it was put there.
#[derive(Clone, Copy, Debug)]
pub enum TextureTail {
    /// Gaussian noise, independent in each channel: the ideal diffuse field.
    Noise,
    /// Sinusoids from 100 Hz to 12 kHz (or 0.45 of the rate), each the last plus `spacing_hz` times a
    /// uniform variate from 0.5 to 1.5, at random phases in each channel: a sparse comb of high-Q modes.
    Modes { spacing_hz: f64 },
    /// Single-sample impulses of random sign, arriving as a Poisson process, independently in each
    /// channel: separate echoes.
    Clicks { per_second: f64 },
    /// One burst of Gaussian noise `burst_s` long, repeated every `period_s`, a burst of its own in each
    /// channel: flutter.
    Repeats { period_s: f64, burst_s: f64 },
}

/// A planted tail: leading silence under a noise floor, a unit direct impulse at `DIRECT_S`, and the
/// tail from the next frame, its mean square `tail_db` at its start and falling 60 dB in `t60_s`.
#[derive(Clone, Copy, Debug)]
pub struct TexturePlan {
    pub rate: f64,
    pub channels: usize,
    pub t60_s: f64,
    pub duration_s: f64,
    pub tail: TextureTail,
    pub tail_db: f64,
    /// White Gaussian noise over the whole response, mean square per channel.
    pub noise_db: f64,
    pub seed: u64,
}

impl TexturePlan {
    pub const DIRECT_S: f64 = 0.05;

    /// The tail 20 dB under the direct impulse and 70 dB over its noise floor, and long enough for the
    /// decay to reach the floor with a tenth of the response left.
    pub fn new(rate: f64, channels: usize, t60_s: f64, tail: TextureTail, seed: u64) -> Self {
        Self {
            rate,
            channels,
            t60_s,
            duration_s: 1.3 * t60_s + 0.6,
            tail,
            tail_db: -20.0,
            noise_db: -90.0,
            seed,
        }
    }

    pub fn build(&self) -> Vec<Vec<f32>> {
        let rate = self.rate;
        let frames = (self.duration_s * rate).round() as usize;
        let direct = (Self::DIRECT_S * rate).round() as usize;
        let tail_frames = frames - direct - 1;
        let level = 10f64.powf(self.tail_db / 20.0);
        let floor = 10f64.powf(self.noise_db / 20.0);
        let mut rng = XorShift::new(self.seed);
        (0..self.channels)
            .map(|_| {
                let mut tail = vec![0.0f64; tail_frames];
                match self.tail {
                    TextureTail::Noise => {
                        for v in &mut tail {
                            *v = level * gaussian(&mut rng);
                        }
                    }
                    TextureTail::Modes { spacing_hz } => {
                        let top = 12_000.0f64.min(0.45 * rate);
                        let mut modes = Vec::new();
                        let mut hz = 100.0;
                        while hz < top {
                            modes.push((
                                hz,
                                2.0 * std::f64::consts::PI * 0.5 * (rng.bipolar() + 1.0),
                            ));
                            hz += spacing_hz * (1.0 + 0.5 * rng.bipolar());
                        }
                        // Each mode's mean square is a²/2, so the sum's is the tail's level.
                        let a = level * (2.0 / modes.len() as f64).sqrt();
                        for (hz, phase) in modes {
                            // Two terms of a sinusoid, then its own recurrence.
                            let w = 2.0 * std::f64::consts::PI * hz / rate;
                            let two_cos = 2.0 * w.cos();
                            let (mut previous, mut current) = ((phase - w).sin(), phase.sin());
                            for v in &mut tail {
                                *v += a * current;
                                let next = two_cos * current - previous;
                                previous = current;
                                current = next;
                            }
                        }
                    }
                    TextureTail::Clicks { per_second } => {
                        let amplitude = level * (rate / per_second).sqrt();
                        let mut t = 0.0f64;
                        loop {
                            let u = ((rng.next_u64() >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
                            t += -u.ln() / per_second;
                            let n = (t * rate) as usize;
                            if n >= tail_frames {
                                break;
                            }
                            tail[n] += if rng.bipolar() < 0.0 {
                                -amplitude
                            } else {
                                amplitude
                            };
                        }
                    }
                    TextureTail::Repeats { period_s, burst_s } => {
                        let length = (burst_s * rate).round() as usize;
                        let period = (period_s * rate).round() as usize;
                        let amplitude = level * (period as f64 / length as f64).sqrt();
                        let burst: Vec<f64> = (0..length)
                            .map(|_| amplitude * gaussian(&mut rng))
                            .collect();
                        for (n, v) in tail.iter_mut().enumerate() {
                            if n % period < length {
                                *v = burst[n % period];
                            }
                        }
                    }
                }
                let mut out = vec![0.0f32; frames];
                for (n, v) in out.iter_mut().enumerate() {
                    let mut x = floor * gaussian(&mut rng);
                    if n == direct {
                        x += 1.0;
                    } else if n > direct {
                        let t = (n - direct - 1) as f64 / rate;
                        x += tail[n - direct - 1] * 10f64.powf(-3.0 * t / self.t60_s);
                    }
                    *v = x as f32;
                }
                out
            })
            .collect()
    }
}

pub struct Planted {
    pub channels: Vec<Vec<f32>>,
    pub direct_index: usize,
    /// The planted direct pulse's energy over every other planted component's.
    pub drr_db: f64,
}

impl Planted {
    pub fn slices(&self) -> Vec<&[f32]> {
        self.channels.iter().map(Vec::as_slice).collect()
    }
}
