//! The tail's texture: how diffuse a reverb tail is, where the echo density profile cannot see (this
//! crate's `AGENTS.md`, *The tail texture*).
//!
//! The echo density profile counts the samples that stand beyond one standard deviation, and that count
//! is blind to two failures a listener hears at once. **A sparse comb of high-Q modes** — metallic
//! ringing — sums sinusoids into an amplitude distribution close to Gaussian, so it reads fully dense;
//! its spectrum does not. **Separate echoes and flutter** leave a heavy-tailed amplitude distribution
//! and a repeating energy envelope. So four readings are taken over up to two segments of the tail,
//! each against what exponentially decaying Gaussian noise gives through the same pipeline:
//!
//! - **Spectral peakiness**: the periodogram's normalised second moment, mean(P²) / mean(P)², per
//!   sixth octave, and the median over sixth octaves. A Gaussian signal's periodogram bins are
//!   exponentially distributed, whose second moment is twice the squared mean; over a band's ten or
//!   more bins the sample ratio reads about 1.9. Power gathered into a few modes reads far above.
//! - **Kurtosis**, the fourth standardised moment: 3 for a Gaussian field, above it for sparse echoes.
//! - **Echo density**: Abel and Huang's normalised count (`density`), averaged over the segment.
//! - **Periodicity**: the largest normalised autocorrelation of the log energy envelope, its slow trend
//!   removed, at lags past the autocorrelation's main lobe, from 3 to 250 ms. Repeating echoes and
//!   flutter read high at their period.
//!
//! Peakiness, kurtosis and periodicity read the **decay-compensated** segment: each sample over the
//! square root of its 30 ms centred mean square, so a decaying tail's statistics are read as if its
//! level held steady. Echo density reads the samples as they are, as the profile does.
//!
//! Every moving mean here is centred and counts what lies outside its signal as zero, as numpy's
//! `convolve(x, ones(n) / n, "same")` does, because the reference measurement this module restates is
//! written that way.

use core::f64::consts::PI;
use core::ops::Range;

use crate::analysis::{Band, Segment, TailTexture};
use crate::density;
use crate::math::{clamped_f32, finite_f32, samples};
use crate::solve;

/// Added to a mean square before its square root divides a sample. **Representational**: it keeps a
/// digitally silent stretch at zero rather than dividing by zero, and lies far below any mean square a
/// sounding sample gives.
const COMPENSATION_FLOOR: f64 = 1e-30;
/// Added to an envelope before its logarithm. **Representational**, as above: a silent millisecond
/// reads −120 dB rather than −∞.
const LOG_FLOOR: f64 = 1e-12;

/// A segment's frames, after the direct sound.
pub(crate) type Frames = Range<usize>;

/// The mean of the T30 (else T20) of the required bands from 500 Hz to 2 kHz: the bands the fit reads
/// Decay from (`solve::decay_targets`), unweighted. Absent where none has a decay time.
pub(crate) fn mid_band_t60(bands: &[Band]) -> Option<f64> {
    let times: Vec<f64> = solve::decay_targets(bands)
        .iter()
        .filter(|target| solve::is_mid(target.centre_hz))
        .map(|target| f64::from(target.seconds))
        .collect();
    (!times.is_empty()).then(|| times.iter().sum::<f64>() / times.len() as f64)
}

/// Where the tail last stands [`crate::NOISE_MARGIN_DB`] over the broadband noise floor, in frames after
/// `origin`: the last frame whose centred [`crate::TEXTURE_USABLE_WINDOW_S`] mean of `energy` (summed
/// over channels, per frame) exceeds the floor by the margin, the mean taken over the energy from
/// `origin` on. Zero where none does.
pub(crate) fn usable_end(energy: &[f64], origin: usize, rate: f64, noise_energy: f64) -> usize {
    let values = &energy[origin.min(energy.len())..];
    let floor = noise_energy * 10f64.powf(crate::NOISE_MARGIN_DB / 10.0);
    let width = samples(crate::TEXTURE_USABLE_WINDOW_S, rate);
    CentredMean::new(values, width, 0..values.len())
        .enumerate()
        .filter(|&(_, mean)| mean > floor)
        .map(|(i, _)| i)
        .last()
        .unwrap_or(0)
}

/// Segments A and B, in frames after the direct sound, from the mid-band T60 and the usable end
/// (frames after the origin). Both absent without a T60.
pub(crate) fn choose(t60_s: Option<f64>, usable: usize, rate: f64) -> [Option<Frames>; 2] {
    let Some(t60) = t60_s.filter(|t| t.is_finite() && *t > 0.0) else {
        return [None, None];
    };
    let frames = |seconds: f64| (seconds * rate).round() as usize;
    let a_start = frames(
        (crate::TEXTURE_A_START_T60 * t60)
            .clamp(crate::TEXTURE_A_START_MIN_S, crate::TEXTURE_A_START_MAX_S),
    );
    let a_length = frames(
        (crate::TEXTURE_A_LENGTH_T60 * t60)
            .clamp(crate::TEXTURE_A_LENGTH_MIN_S, crate::TEXTURE_A_LENGTH_MAX_S),
    );
    let a_end = (a_start + a_length).min(usable);
    let a =
        (a_end >= a_start + samples(crate::TEXTURE_MIN_SEGMENT_S, rate)).then_some(a_start..a_end);
    // B runs from A's end as capped, before A is shortened to a power of two.
    let b_end = (a_end + frames(crate::TEXTURE_B_MAX_S)).min(usable);
    let b = (b_end >= a_end + frames(crate::TEXTURE_B_MIN_S)).then_some(a_end..b_end);
    let origin = samples(crate::TEXTURE_ORIGIN_S, rate);
    [a, b].map(|segment| segment.map(|r| shorten(r.start + origin..r.end + origin)))
}

/// A segment holding at least [`crate::TEXTURE_POWER_OF_TWO_FROM`] frames, cut to the largest power of
/// two it holds from its start; a shorter one as it is.
fn shorten(r: Frames) -> Frames {
    let n = r.len();
    if n < crate::TEXTURE_POWER_OF_TWO_FROM {
        return r;
    }
    let kept = if n.is_power_of_two() {
        n
    } else {
        n.next_power_of_two() / 2
    };
    r.start..r.start + kept
}

/// A stated segment's frames after the direct sound, rounded from its seconds and shortened by the
/// same rule as a placed one, which leaves every segment an analysis records as it is. Absent where it
/// holds none.
pub(crate) fn frames_of(segment: &Segment, rate: f64) -> Option<Frames> {
    // A float-to-integer cast saturates, so a negative time reads as frame zero.
    let frame = |seconds: f32| (f64::from(seconds) * rate).round() as usize;
    let (start, end) = (frame(segment.start_s), frame(segment.end_s));
    (end > start).then(|| shorten(start..end))
}

/// The texture of `channels` over `after_direct` (frames after the `direct` frame), cut at the end of
/// the samples. Absent where nothing of the segment lies inside them.
pub(crate) fn measure(
    channels: &[&[f32]],
    rate: f64,
    direct: usize,
    after_direct: Frames,
) -> Option<TailTexture> {
    let len = channels[0].len();
    let start = direct.saturating_add(after_direct.start);
    let end = direct.saturating_add(after_direct.end).min(len);
    if end <= start {
        return None;
    }

    let (mut peakiness, mut peakiness_count) = (0.0f64, 0usize);
    let (mut kurtosis, mut kurtosis_count) = (0.0f64, 0usize);
    let (mut density_sum, mut density_count) = (0.0f64, 0usize);
    let mut periodic: Option<(f64, Option<usize>)> = None;
    for channel in channels {
        let segment = compensated(channel, start..end, rate);
        if let Some(value) = spectral_peakiness(&segment, rate) {
            peakiness += value;
            peakiness_count += 1;
        }
        if let Some(value) = kurtosis_of(&segment) {
            kurtosis += value;
            kurtosis_count += 1;
        }
        if let Some((value, lag)) = periodicity(&segment, rate) {
            // The channel with the larger value; the first on a tie.
            if periodic.is_none_or(|(best, _)| value > best) {
                periodic = Some((value, lag));
            }
        }
        let (sum, count) = echo_density(channel, start..end, rate);
        density_sum += sum;
        density_count += count;
    }
    let mean = |sum: f64, count: usize| (count > 0).then(|| sum / count as f64);
    Some(TailTexture {
        segment: Segment {
            start_s: clamped_f32((start - direct) as f64 / rate),
            end_s: clamped_f32((end - direct) as f64 / rate),
        },
        peakiness: mean(peakiness, peakiness_count).and_then(finite_f32),
        kurtosis: mean(kurtosis, kurtosis_count).and_then(finite_f32),
        echo_density: mean(density_sum, density_count).and_then(finite_f32),
        periodicity: periodic.and_then(|(value, _)| finite_f32(value)),
        periodicity_lag_s: periodic
            .and_then(|(_, lag)| lag)
            .and_then(|lag| finite_f32(lag as f64 / rate)),
    })
}

/// `channel[range]`, each sample over the square root of its centred
/// [`crate::TEXTURE_COMPENSATION_WINDOW_S`] mean square, that mean read over the whole channel.
fn compensated(channel: &[f32], range: Range<usize>, rate: f64) -> Vec<f64> {
    let width = samples(crate::TEXTURE_COMPENSATION_WINDOW_S, rate);
    // Only the frames some window reaches are squared; what lies past the channel counts as zero, which
    // the moving mean supplies at the ends of this excerpt only where they are the channel's own ends.
    let first = range.start.saturating_sub(width / 2);
    let last = (range.end + (width - 1) / 2).min(channel.len());
    let squares: Vec<f64> = channel[first..last]
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .collect();
    // A running sum of squares can round a hair below zero where the signal falls into digital silence,
    // so the mean is held at zero there rather than taking the square root of a negative.
    CentredMean::new(&squares, width, range.start - first..range.end - first)
        .zip(&channel[range])
        .map(|(mean, &s)| f64::from(s) / (mean.max(0.0) + COMPENSATION_FLOOR).sqrt())
        .collect()
}

/// The median over sixth octaves of mean(P²) / mean(P)², P the Hann-windowed periodogram of `segment`:
/// the octaves from [`crate::PEAKINESS_LOW_HZ`], each 2^(1/6) above the last, while an octave's upper
/// edge is at most [`crate::PEAKINESS_HIGH_HZ`] or [`crate::PEAKINESS_HIGH_RATE_FRACTION`] of the rate,
/// and only those holding at least [`crate::PEAKINESS_MIN_BINS`] bins with any power. Absent where none
/// does.
fn spectral_peakiness(segment: &[f64], rate: f64) -> Option<f64> {
    let n = segment.len();
    if n < 2 {
        return None;
    }
    let bin_hz = rate / n as f64;
    let top_hz = crate::PEAKINESS_HIGH_HZ.min(crate::PEAKINESS_HIGH_RATE_FRACTION * rate);
    let bins = ((top_hz / bin_hz).floor() as usize + 1).min(n / 2 + 1);
    let windowed: Vec<f64> = segment
        .iter()
        .enumerate()
        .map(|(k, v)| v * (0.5 - 0.5 * (2.0 * PI * k as f64 / (n - 1) as f64).cos()))
        .collect();
    let power = periodogram(
        &windowed,
        bins,
        (crate::PEAKINESS_LOW_HZ / bin_hz).floor() as usize,
    );

    let ratio = 2f64.powf(1.0 / crate::PEAKINESS_BANDS_PER_OCTAVE);
    let mut moments = Vec::new();
    let mut low = crate::PEAKINESS_LOW_HZ;
    while low * ratio <= top_hz {
        let high = low * ratio;
        let (mut sum, mut sum_squares, mut count) = (0.0f64, 0.0f64, 0usize);
        let from = (low / bin_hz).floor() as usize;
        for (k, &p) in power.iter().enumerate().skip(from) {
            let hz = k as f64 * bin_hz;
            if hz >= high {
                break;
            }
            if hz >= low {
                sum += p;
                sum_squares += p * p;
                count += 1;
            }
        }
        if count >= crate::PEAKINESS_MIN_BINS && sum > 0.0 {
            let mean = sum / count as f64;
            moments.push(sum_squares / count as f64 / (mean * mean));
        }
        low = high;
    }
    median(&mut moments)
}

/// |X[k]|² for k below `bins`, zero below `from`. A power-of-two length goes through [`fft`]; a shorter
/// segment of any other length is transformed bin by bin, which is exact and costs no more than a few
/// million products under [`crate::TEXTURE_POWER_OF_TWO_FROM`] frames.
fn periodogram(x: &[f64], bins: usize, from: usize) -> Vec<f64> {
    let n = x.len();
    if !n.is_power_of_two() {
        return periodogram_by_bin(x, bins, from);
    }
    let mut re = x.to_vec();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..bins).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
}

fn periodogram_by_bin(x: &[f64], bins: usize, from: usize) -> Vec<f64> {
    let n = x.len();
    (0..bins)
        .map(|k| {
            if k < from {
                return 0.0;
            }
            let step = -2.0 * PI * k as f64 / n as f64;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (m, &v) in x.iter().enumerate() {
                let angle = step * m as f64;
                re += v * angle.cos();
                im += v * angle.sin();
            }
            re * re + im * im
        })
        .collect()
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[middle]
    } else {
        0.5 * (values[middle - 1] + values[middle])
    })
}

/// The fourth standardised moment, the mean removed. Absent for a segment without variance.
fn kurtosis_of(x: &[f64]) -> Option<f64> {
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n;
    let (mut second, mut fourth) = (0.0f64, 0.0f64);
    for &v in x {
        let d = v - mean;
        second += d * d;
        fourth += d * d * d * d;
    }
    let variance = second / n;
    (variance > 0.0).then(|| fourth / n / (variance * variance))
}

/// The largest normalised autocorrelation of the segment's log energy envelope past its main lobe, and
/// its lag in frames: the envelope a centred [`crate::PERIODICITY_ENVELOPE_S`] mean square, in log10,
/// less its centred [`crate::PERIODICITY_TREND_S`] mean, less its own mean.
///
/// **Searched past the main lobe**, from the later of [`crate::PERIODICITY_MIN_LAG_S`] and the first
/// local minimum (walking from lag one while the autocorrelation keeps falling) to the earlier of
/// [`crate::PERIODICITY_MAX_LAG_S`] and half the segment. An envelope's own smoothness, which follows the
/// signal's bandwidth, keeps the autocorrelation high for the first few milliseconds whether anything
/// repeats or not: searched from 3 ms regardless, a dark tail read 0.78 at 3.0 ms (measured by the
/// reference, 2026-09-15). **An empty range reads zero with no lag**: the main lobe outlasting the
/// search is a smooth envelope, not a repetition. Absent only where the envelope does not vary.
fn periodicity(segment: &[f64], rate: f64) -> Option<(f64, Option<usize>)> {
    let n = segment.len();
    let squares: Vec<f64> = segment.iter().map(|v| v * v).collect();
    let mut envelope: Vec<f64> =
        CentredMean::new(&squares, samples(crate::PERIODICITY_ENVELOPE_S, rate), 0..n)
            // Held at zero for the same reason as the compensation's.
            .map(|e| (e.max(0.0) + LOG_FLOOR).log10())
            .collect();
    let trend: Vec<f64> =
        CentredMean::new(&envelope, samples(crate::PERIODICITY_TREND_S, rate), 0..n).collect();
    for (e, t) in envelope.iter_mut().zip(&trend) {
        *e -= t;
    }
    let mean = envelope.iter().sum::<f64>() / n as f64;

    // Wiener–Khinchin: the autocorrelation is the transform of the power spectrum. Zero-padded to at
    // least twice the length, no lag under the length wraps.
    let size = 2 * n.next_power_of_two();
    let mut re = vec![0.0f64; size];
    let mut im = vec![0.0f64; size];
    for (r, e) in re.iter_mut().zip(&envelope) {
        *r = e - mean;
    }
    fft(&mut re, &mut im);
    for (r, i) in re.iter_mut().zip(im.iter_mut()) {
        *r = *r * *r + *i * *i;
        *i = 0.0;
    }
    // The power spectrum is real and even, so its forward transform is its inverse times the length,
    // which normalising by lag zero removes.
    fft(&mut re, &mut im);
    let zero = re[0];
    if !(zero > 0.0 && zero.is_finite()) {
        return None;
    }
    // The main lobe ends at the first local minimum, looked for no further than the search reaches.
    let hi = samples(crate::PERIODICITY_MAX_LAG_S, rate).min(n / 2);
    let mut lobe = 1;
    while lobe + 1 < hi && re[lobe + 1] < re[lobe] {
        lobe += 1;
    }
    let lo = samples(crate::PERIODICITY_MIN_LAG_S, rate).max(lobe);
    let mut best: (f64, Option<usize>) = (0.0, None);
    for (lag, &value) in re.iter().enumerate().take(hi).skip(lo) {
        let normalised = value / zero;
        if best.1.is_none() || normalised > best.0 {
            best = (normalised, Some(lag));
        }
    }
    Some(best)
}

/// The sum of Abel and Huang's normalised echo density over windows of [`crate::ECHO_DENSITY_WINDOW_S`]
/// centred every [`crate::TEXTURE_DENSITY_HOP_S`] within `range`, and how many windows it holds. A
/// window may reach outside the segment; one without variance is not counted.
fn echo_density(channel: &[f32], range: Range<usize>, rate: f64) -> (f64, usize) {
    let half = samples(crate::ECHO_DENSITY_WINDOW_S, rate) / 2;
    let hop = samples(crate::TEXTURE_DENSITY_HOP_S, rate);
    let (mut sum, mut count) = (0.0f64, 0usize);
    let mut centre = range.start;
    while centre < range.end {
        let first = centre.saturating_sub(half);
        let last = (centre + half + 1).min(channel.len());
        let window = density::window(&channel[first..last]);
        if window.sigma > 0.0 {
            sum += window.density;
            count += 1;
        }
        centre += hop;
    }
    (sum, count)
}

/// Centred moving means of `values` over `width` samples at each index of a range: the mean at `i` is
/// over `i − width/2 ..= i + (width − 1)/2`, a sample outside `values` counting as zero. The running
/// sum is summed afresh every `width` steps, so rounding cannot carry from a loud start into a quiet
/// end.
struct CentredMean<'a> {
    values: &'a [f64],
    width: usize,
    next: usize,
    end: usize,
    steps: usize,
    sum: f64,
}

impl<'a> CentredMean<'a> {
    fn new(values: &'a [f64], width: usize, range: Range<usize>) -> Self {
        Self {
            values,
            width: width.max(1),
            next: range.start,
            end: range.end,
            steps: 0,
            sum: 0.0,
        }
    }

    fn window(&self, i: usize) -> (usize, usize) {
        let len = self.values.len();
        let hi = (i + (self.width - 1) / 2 + 1).min(len);
        (i.saturating_sub(self.width / 2).min(hi), hi)
    }
}

impl Iterator for CentredMean<'_> {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        if self.next >= self.end {
            return None;
        }
        let i = self.next;
        let (lo, hi) = self.window(i);
        if self.steps.is_multiple_of(self.width) {
            self.sum = self.values[lo..hi].iter().sum();
        } else {
            let (previous_lo, previous_hi) = self.window(i - 1);
            for v in &self.values[previous_hi.max(lo)..hi] {
                self.sum += v;
            }
            for v in &self.values[previous_lo..lo.min(previous_hi)] {
                self.sum -= v;
            }
        }
        self.steps += 1;
        self.next += 1;
        Some(self.sum / self.width as f64)
    }
}

/// An in-place radix-2 fast Fourier transform, X[k] = Σₙ x[n]·e^(−2πikn/N), unnormalised, for N a
/// power of two: the iterative form in Cormen, Leiserson, Rivest and Stein's *Introduction to
/// Algorithms*, ch. 30 — the input permuted by bit-reversed index, then log₂N stages of butterflies,
/// every stage reading its roots from one table of e^(−2πij/N).
///
/// **Why in this crate**: its one runtime dependency is the DSP crate, and `mxm-measure`'s transform is
/// a dev-dependency by contract (this crate's `AGENTS.md`); the transform is a few dozen lines.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    if n < 2 {
        return;
    }
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let roots: Vec<(f64, f64)> = (0..n / 2)
        .map(|j| {
            let angle = -2.0 * PI * j as f64 / n as f64;
            (angle.cos(), angle.sin())
        })
        .collect();
    let mut length = 2;
    while length <= n {
        let half = length / 2;
        let stride = n / length;
        for start in (0..n).step_by(length) {
            for j in 0..half {
                let (c, s) = roots[j * stride];
                let (a, b) = (start + j, start + j + half);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        length *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, mut state: u64) -> Vec<f64> {
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 11) as f64 / (1u64 << 52) as f64 - 1.0
            })
            .collect()
    }

    #[test]
    fn the_fft_is_the_discrete_fourier_transform() {
        for n in [1usize, 2, 8, 64, 256] {
            let x = noise(n, 0x1234_5678 + n as u64);
            let (mut re, mut im) = (x.clone(), vec![0.0; n]);
            fft(&mut re, &mut im);
            for k in 0..n {
                let (mut dr, mut di) = (0.0, 0.0);
                for (m, v) in x.iter().enumerate() {
                    let angle = -2.0 * PI * (k * m) as f64 / n as f64;
                    dr += v * angle.cos();
                    di += v * angle.sin();
                }
                assert!(
                    (re[k] - dr).abs() < 1e-9 && (im[k] - di).abs() < 1e-9,
                    "n {n} bin {k}: {} {} against {dr} {di}",
                    re[k],
                    im[k]
                );
            }
        }
    }

    #[test]
    fn the_two_periodograms_agree() {
        let x = noise(512, 99);
        let fast = periodogram(&x, 257, 0);
        let slow = periodogram_by_bin(&x, 257, 3);
        for (k, (f, s)) in fast.iter().zip(&slow).enumerate() {
            if k < 3 {
                assert_eq!(*s, 0.0, "bin {k} is below `from`");
            } else {
                assert!(
                    (f - s).abs() < 1e-9 * s.max(1.0),
                    "bin {k}: {f} against {s}"
                );
            }
        }
    }

    #[test]
    fn periodicity_is_searched_past_the_main_lobe() {
        // A smooth envelope with nothing repeating: noise through a long moving mean, so the log
        // envelope's autocorrelation falls slowly from lag zero. Searched from 3 ms regardless it would
        // read its main lobe; past the lobe it reads what little correlation lies beyond.
        let rate = 48_000.0;
        let raw = noise(16_384, 7);
        let smooth: Vec<f64> = CentredMean::new(&raw, 96, 0..raw.len()).collect();
        let (value, lag) = periodicity(&smooth, rate).expect("the envelope varies");
        let lag = lag.expect("the search holds lags");
        assert!(
            lag > samples(crate::PERIODICITY_MIN_LAG_S, rate),
            "lag {lag}"
        );
        assert!(value < 0.5, "{value} at {lag}");

        // A burst every 20 ms reads its period.
        let mut state = 3u64;
        let pulses: Vec<f64> = (0..16_384)
            .map(|n| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let white = (state >> 11) as f64 / (1u64 << 52) as f64 - 1.0;
                if n % 960 < 96 { white } else { 0.01 * white }
            })
            .collect();
        let (value, lag) = periodicity(&pulses, rate).expect("the envelope varies");
        assert_eq!(lag, Some(960), "{value}");
        assert!(value > 0.5, "{value}");
    }

    #[test]
    fn centred_means_are_numpys_same_mode_convolution() {
        // numpy.convolve(arange(10), ones(4) / 4, "same") and the same with ones(5) / 5.
        let values: Vec<f64> = (0..10).map(f64::from).collect();
        let even: Vec<f64> = CentredMean::new(&values, 4, 0..10).collect();
        assert_eq!(even, [0.25, 0.75, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 6.0]);
        let odd: Vec<f64> = CentredMean::new(&values, 5, 0..10).collect();
        let want = [0.6, 1.2, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 6.0, 4.8];
        for (got, want) in odd.iter().zip(want) {
            assert!((got - want).abs() < 1e-12, "{odd:?}");
        }
        // A range inside the values, and one reaching past them, give the same means as the whole run.
        let part: Vec<f64> = CentredMean::new(&values, 4, 3..12).collect();
        assert_eq!(&part[..7], &even[3..]);
        assert_eq!(part[7..], [4.25, 2.25]);
    }

    #[test]
    fn a_long_segment_is_cut_to_a_power_of_two_and_a_short_one_is_not() {
        assert_eq!(shorten(10..2057), 10..2057);
        assert_eq!(shorten(0..3000), 0..2048);
        assert_eq!(shorten(5..4101), 5..4101);
        assert_eq!(shorten(0..2047), 0..2047);
    }

    #[test]
    fn a_stated_segment_reads_back_to_its_frames() {
        let stated = |start: usize, end: usize, rate: f64| Segment {
            start_s: clamped_f32(start as f64 / rate),
            end_s: clamped_f32(end as f64 / rate),
        };
        for rate in [22_050.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            // Every length a placed segment can have: a power of two from 2048, or anything shorter.
            for (start, end) in [(4_411usize, 4_411 + 16_384), (1, 2), (100_000, 231_072)] {
                assert_eq!(
                    frames_of(&stated(start, end, rate), rate),
                    Some(start..end),
                    "{rate}"
                );
            }
            // A longer stated segment is shortened by the same rule.
            assert_eq!(
                frames_of(&stated(4_411, 36_411, rate), rate),
                Some(4_411..4_411 + 16_384),
                "{rate}"
            );
        }
    }
}
