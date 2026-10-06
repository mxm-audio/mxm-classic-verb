//! Test support: planted impulse responses, the bytes of WAV and AIFF files written from them, and a
//! host that applies what it is sent.
//!
//! **No test reads a real impulse response.** Every response here is synthesised from a seeded
//! generator — a direct impulse, three reflections, and white noise decaying exponentially over a
//! noise floor — and every file is written by these functions, into memory or a temporary file that
//! is removed afterwards.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use mxm_classic_verb_dsp::Space;
use nice_plug::context::gui::GuiContextInner;
use nice_plug::params::internals::ParamPtr;
use nice_plug::params::{InternalParamMut, Param};
use nice_plug::prelude::{PluginApi, PluginState};

use crate::loading::{Fitted, FittedControls};
use crate::payload::{Descriptor, DescriptorError, Report};

/// Marsaglia's xorshift64, seeded, so a planted response is bit-repeatable.
pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> Self {
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

/// A planted response: 50 ms of noise floor 90 dB under the direct sound, a unit direct impulse,
/// reflections 7, 13 and 19 ms after it, and a white-noise tail from 4 ms that falls 60 dB in
/// `t60_s`.
pub fn response(rate: u32, seconds: f32, t60_s: f32, channels: usize, seed: u64) -> Vec<Vec<f32>> {
    let fs = f64::from(rate);
    let frames = (f64::from(seconds) * fs).round() as usize;
    let direct = ((0.05 * fs) as usize).min(frames.saturating_sub(1));
    let tail = direct + (0.004 * fs) as usize;
    let mut rng = XorShift::new(seed);
    (0..channels)
        .map(|channel| {
            let mut out: Vec<f32> = (0..frames)
                .map(|n| {
                    let floor = 3.0e-5 * rng.bipolar();
                    let late = if n >= tail {
                        let t = (n - tail) as f64 / fs;
                        0.1 * 10f64.powf(-3.0 * t / f64::from(t60_s)) * rng.bipolar()
                    } else {
                        0.0
                    };
                    (floor + late) as f32
                })
                .collect();
            if frames > 0 {
                out[direct] += 1.0;
            }
            for (delay_s, gain) in [(0.007, 0.5), (0.013, 0.35), (0.019, 0.25)] {
                let at = direct + (delay_s * fs) as usize;
                if at < frames {
                    out[at] += gain * if channel == 1 { 0.9 } else { 1.0 };
                }
            }
            out
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub enum WavFormat {
    Int16,
    Int24,
    Float32,
}

/// A WAV file's bytes, written by `hound`.
pub fn wav_bytes(channels: &[Vec<f32>], rate: u32, format: WavFormat) -> Vec<u8> {
    let (bits_per_sample, sample_format) = match format {
        WavFormat::Int16 => (16, hound::SampleFormat::Int),
        WavFormat::Int24 => (24, hound::SampleFormat::Int),
        WavFormat::Float32 => (32, hound::SampleFormat::Float),
    };
    let spec = hound::WavSpec {
        channels: channels.len() as u16,
        sample_rate: rate,
        bits_per_sample,
        sample_format,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("a WAV writer");
        for frame in 0..channels.first().map_or(0, Vec::len) {
            for channel in channels {
                let value = channel[frame];
                match format {
                    WavFormat::Int16 => writer
                        .write_sample((value * 32_767.0).round() as i16)
                        .expect("a sample"),
                    WavFormat::Int24 => writer
                        .write_sample((value * 8_388_607.0).round() as i32)
                        .expect("a sample"),
                    WavFormat::Float32 => writer.write_sample(value).expect("a sample"),
                }
            }
        }
        writer.finalize().expect("a finished WAV");
    }
    cursor.into_inner()
}

#[derive(Clone, Copy, Debug)]
pub enum AiffEncoding {
    Pcm16,
    Pcm24,
    /// AIFF-C `fl32`.
    Float32,
    /// AIFF-C `fl64`.
    Float64,
    /// AIFF-C `sowt`, little-endian 16-bit.
    Sowt16,
    /// AIFF-C `ulaw`, which the plugin refuses.
    Ulaw,
}

fn extended(rate: u32) -> [u8; 10] {
    let mut bytes = [0u8; 10];
    if rate == 0 {
        return bytes;
    }
    let shift = 31 - rate.leading_zeros();
    bytes[0..2].copy_from_slice(&(16_383 + shift as u16).to_be_bytes());
    bytes[2..10].copy_from_slice(&(u64::from(rate) << (63 - shift)).to_be_bytes());
    bytes
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

/// An AIFF or AIFF-C file's bytes, written to Apple's chunk layouts.
pub fn aiff_bytes(channels: &[Vec<f32>], rate: u32, encoding: AiffEncoding) -> Vec<u8> {
    let frames = channels.first().map_or(0, Vec::len);
    let (bits, compression): (u16, Option<&[u8; 4]>) = match encoding {
        AiffEncoding::Pcm16 => (16, None),
        AiffEncoding::Pcm24 => (24, None),
        AiffEncoding::Float32 => (32, Some(b"fl32")),
        AiffEncoding::Float64 => (64, Some(b"fl64")),
        AiffEncoding::Sowt16 => (16, Some(b"sowt")),
        AiffEncoding::Ulaw => (8, Some(b"ulaw")),
    };
    let mut data = Vec::new();
    for frame in 0..frames {
        for channel in channels {
            let value = channel[frame];
            match encoding {
                AiffEncoding::Pcm16 => {
                    data.extend_from_slice(&((value * 32_767.0).round() as i16).to_be_bytes())
                }
                AiffEncoding::Pcm24 => {
                    let sample = (value * 8_388_607.0).round() as i32;
                    data.extend_from_slice(&sample.to_be_bytes()[1..]);
                }
                AiffEncoding::Float32 => data.extend_from_slice(&value.to_be_bytes()),
                AiffEncoding::Float64 => data.extend_from_slice(&f64::from(value).to_be_bytes()),
                AiffEncoding::Sowt16 => {
                    data.extend_from_slice(&((value * 32_767.0).round() as i16).to_le_bytes())
                }
                AiffEncoding::Ulaw => data.push(0x80),
            }
        }
    }
    let mut comm = Vec::new();
    comm.extend_from_slice(&(channels.len() as u16).to_be_bytes());
    comm.extend_from_slice(&(frames as u32).to_be_bytes());
    comm.extend_from_slice(&bits.to_be_bytes());
    comm.extend_from_slice(&extended(rate));
    let mut body = Vec::new();
    match compression {
        Some(kind) => {
            body.extend_from_slice(b"AIFC");
            chunk(&mut body, b"FVER", &0xA280_5140u32.to_be_bytes());
            comm.extend_from_slice(kind);
            // An empty Pascal-string compression name, padded to an even length.
            comm.extend_from_slice(&[0, 0]);
        }
        None => body.extend_from_slice(b"AIFF"),
    }
    chunk(&mut body, b"COMM", &comm);
    let mut sound = vec![0u8; 8];
    sound.extend_from_slice(&data);
    chunk(&mut body, b"SSND", &sound);
    let mut file = b"FORM".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_be_bytes());
    file.extend_from_slice(&body);
    file
}

/// A file in the temporary directory, removed when dropped.
pub struct TempFile(PathBuf);

impl TempFile {
    pub fn new(name: &str, bytes: &[u8]) -> Self {
        let path =
            std::env::temp_dir().join(format!("mxm-classic-verb-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).expect("the temporary file is written");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A fitted space as a fitter would return it, with plausible controls and one measured error.
pub fn fitted(space: Space, confidence: f32) -> Fitted {
    Fitted {
        space,
        controls: FittedControls {
            decay_s: 1.5,
            size_s: 0.03,
            diffusion: 0.6,
            pre_delay_s: 0.012,
        },
        report: Report {
            confidence,
            errors: vec![DescriptorError {
                what: Descriptor::Decay,
                hz: Some(1_000.0),
                value: 3.0,
            }],
            clamps: Vec::new(),
        },
    }
}

/// Sets a parameter directly, the way a host's state or automation would.
pub fn set<P: Param + InternalParamMut>(param: &P, value: P::Plain) {
    unsafe {
        let _ = param._internal_set_normalized_value(param.preview_normalized(value));
    }
}

type Hook = Box<dyn Fn() + Send + Sync>;

/// A host that applies each value at once, counts gestures, and can run a hook at each gesture's
/// start — which is how a test looks at the barrier from inside the gestures, or supersedes a fit
/// at that exact boundary.
#[derive(Default)]
pub struct ApplyingHost {
    begins: AtomicUsize,
    ends: AtomicUsize,
    hook: Mutex<Option<Hook>>,
}

impl ApplyingHost {
    pub fn begins(&self) -> usize {
        self.begins.load(Ordering::Relaxed)
    }

    pub fn ends(&self) -> usize {
        self.ends.load(Ordering::Relaxed)
    }

    pub fn on_gesture(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.hook.lock().unwrap() = Some(Box::new(hook));
    }
}

impl GuiContextInner for ApplyingHost {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {
        if let Some(hook) = self.hook.lock().unwrap().as_ref() {
            hook();
        }
        self.begins.fetch_add(1, Ordering::Relaxed);
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        unsafe {
            param._internal_set_normalized_value(normalized);
        }
    }

    unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {
        self.ends.fetch_add(1, Ordering::Relaxed);
    }

    fn get_state(&self) -> PluginState {
        PluginState {
            version: String::new(),
            params: Default::default(),
            fields: Default::default(),
        }
    }

    fn set_state(&self, _state: PluginState) {}
}
