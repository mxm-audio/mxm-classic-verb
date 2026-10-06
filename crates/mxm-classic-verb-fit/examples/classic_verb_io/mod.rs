//! Shared by the offline examples — `classic_verb_fit`, `classic_verb_audition`,
//! `classic_verb_generate` and `classic_verb_synthetic_responses` — and by `tests/generate.rs`:
//! reading WAV and AIFF files, logical names, a small JSON writer, the fit report (`report`), the
//! factory-space manifest (`manifest`) and the generator's pipeline (`generate`).
//!
//! **Not part of the library**: the fit does no file I/O (plan §4.1). Reading follows plan §4.5's two
//! stages: a file's header is read and passed to `preflight` before anything proportional to the
//! file is allocated, and float samples are checked as they are decoded, stopping at the first
//! non-finite one. WAV goes through `hound`; AIFF and AIFF-C are parsed here, from Apple's published
//! AIFF 1.3 and AIFF-C (1991) chunk layouts.
#![allow(dead_code)]

pub mod generate;
pub mod manifest;
pub mod report;

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use mxm_classic_verb_fit::{Header, Refusal, preflight};

pub struct Decoded {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
}

pub enum LoadError {
    Io(String),
    Format(String),
    Refused(Refusal),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "could not be read: {e}"),
            LoadError::Format(e) => write!(f, "is not a supported file: {e}"),
            LoadError::Refused(r) => write!(f, "refused: {r}"),
        }
    }
}

fn io(e: impl fmt::Display) -> LoadError {
    LoadError::Io(e.to_string())
}

pub fn is_supported(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "wav" | "wave" | "aif" | "aiff" | "aifc"
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Reads a WAV or AIFF file, refusing it from its header before its samples are read.
pub fn load(path: &Path) -> Result<Decoded, LoadError> {
    match extension(path).as_str() {
        "wav" | "wave" => load_wav(path),
        "aif" | "aiff" | "aifc" => load_aiff(path),
        other => Err(LoadError::Format(format!("extension `{other}`"))),
    }
}

fn load_wav(path: &Path) -> Result<Decoded, LoadError> {
    let mut reader = hound::WavReader::open(path).map_err(io)?;
    let spec = reader.spec();
    preflight(&Header {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        frames: u64::from(reader.duration()),
    })
    .map_err(LoadError::Refused)?;
    let count = usize::from(spec.channels);
    let frames = reader.duration() as usize;
    let mut channels = vec![Vec::with_capacity(frames); count];
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (i, sample) in reader.samples::<f32>().enumerate() {
                let sample = sample.map_err(io)?;
                if !sample.is_finite() {
                    return Err(LoadError::Refused(Refusal::NonFiniteSample {
                        channel: i % count,
                        frame: i / count,
                    }));
                }
                channels[i % count].push(sample);
            }
        }
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            for (i, sample) in reader.samples::<i32>().enumerate() {
                channels[i % count].push(sample.map_err(io)? as f32 * scale);
            }
        }
    }
    Ok(Decoded {
        channels,
        sample_rate: spec.sample_rate,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    BigEndianPcm,
    LittleEndianPcm,
    Float32,
    Float64,
}

struct Common {
    channels: u16,
    frames: u32,
    bits: u16,
    rate: f64,
    encoding: Encoding,
}

fn read_bytes<R: Read>(r: &mut R, n: usize) -> Result<Vec<u8>, LoadError> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).map_err(io)?;
    Ok(buf)
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

/// An IEEE 754 80-bit extended value, as AIFF stores its sample rate: a sign and 15-bit exponent,
/// then a 64-bit mantissa with an explicit integer bit.
fn extended(b: &[u8]) -> f64 {
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from(u16::from_be_bytes([b[0] & 0x7f, b[1]]));
    let mantissa = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exponent == 0 && mantissa == 0 {
        return 0.0;
    }
    sign * mantissa as f64 * 2f64.powi(exponent - 16_383 - 63)
}

fn load_aiff(path: &Path) -> Result<Decoded, LoadError> {
    let mut file = BufReader::new(File::open(path).map_err(io)?);
    let form = read_bytes(&mut file, 12)?;
    if &form[0..4] != b"FORM" {
        return Err(LoadError::Format("no FORM chunk".into()));
    }
    let aifc = match &form[8..12] {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => return Err(LoadError::Format("not an AIFF or AIFF-C form".into())),
    };
    let mut common: Option<Common> = None;
    let mut sound: Option<(u64, u64)> = None;
    let mut position = 12u64;
    while common.is_none() || sound.is_none() {
        let mut head = [0u8; 8];
        if file.read_exact(&mut head).is_err() {
            break;
        }
        let size = u64::from(be_u32(&head[4..8]));
        let body = position + 8;
        match &head[0..4] {
            b"COMM" => {
                let b = read_bytes(&mut file, size.min(64) as usize)?;
                if b.len() < 18 {
                    return Err(LoadError::Format("short COMM chunk".into()));
                }
                let bits = be_u16(&b[6..8]);
                let encoding = if aifc && b.len() >= 22 {
                    match &b[18..22] {
                        b"NONE" | b"twos" => Encoding::BigEndianPcm,
                        b"sowt" => Encoding::LittleEndianPcm,
                        b"fl32" | b"FL32" => Encoding::Float32,
                        b"fl64" | b"FL64" => Encoding::Float64,
                        other => {
                            return Err(LoadError::Format(format!(
                                "AIFF-C compression `{}`",
                                String::from_utf8_lossy(other)
                            )));
                        }
                    }
                } else {
                    Encoding::BigEndianPcm
                };
                common = Some(Common {
                    channels: be_u16(&b[0..2]),
                    frames: be_u32(&b[2..6]),
                    bits,
                    rate: extended(&b[8..18]),
                    encoding,
                });
            }
            b"SSND" => {
                let b = read_bytes(&mut file, 8)?;
                let offset = u64::from(be_u32(&b[0..4]));
                sound = Some((body + 8 + offset, size.saturating_sub(8 + offset)));
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        position = body + size + (size & 1);
        file.seek(SeekFrom::Start(position)).map_err(io)?;
    }
    let common = common.ok_or_else(|| LoadError::Format("no COMM chunk".into()))?;
    let (data_start, data_len) = sound.ok_or_else(|| LoadError::Format("no SSND chunk".into()))?;
    if !common.rate.is_finite() || common.rate < 1.0 || common.rate > f64::from(u32::MAX) {
        return Err(LoadError::Format(format!("sample rate {}", common.rate)));
    }
    let sample_rate = common.rate.round() as u32;
    preflight(&Header {
        channels: common.channels,
        sample_rate,
        frames: u64::from(common.frames),
    })
    .map_err(LoadError::Refused)?;

    let width = match common.encoding {
        Encoding::Float32 => 4,
        Encoding::Float64 => 8,
        _ => usize::from(common.bits).div_ceil(8),
    };
    if !(1..=8).contains(&width) {
        return Err(LoadError::Format(format!("{}-bit samples", common.bits)));
    }
    let count = usize::from(common.channels);
    let frames = (common.frames as usize).min((data_len / (width * count) as u64) as usize);
    file.seek(SeekFrom::Start(data_start)).map_err(io)?;
    let mut channels = vec![Vec::with_capacity(frames); count];
    let scale = 1.0 / (1u64 << (8 * width - 1)) as f64;
    let mut buf = [0u8; 8];
    for frame in 0..frames {
        for (channel, samples) in channels.iter_mut().enumerate() {
            file.read_exact(&mut buf[..width]).map_err(io)?;
            let b = &buf[..width];
            let value = match common.encoding {
                Encoding::Float32 => f64::from(f32::from_be_bytes([b[0], b[1], b[2], b[3]])),
                Encoding::Float64 => {
                    f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
                Encoding::BigEndianPcm | Encoding::LittleEndianPcm => {
                    let mut raw = 0i64;
                    for k in 0..width {
                        let byte = if common.encoding == Encoding::BigEndianPcm {
                            b[k]
                        } else {
                            b[width - 1 - k]
                        };
                        raw = (raw << 8) | i64::from(byte);
                    }
                    // Sign-extend from the sample's own width.
                    let shift = 64 - 8 * width;
                    ((raw << shift) >> shift) as f64 * scale
                }
            };
            if !value.is_finite() {
                return Err(LoadError::Refused(Refusal::NonFiniteSample {
                    channel,
                    frame,
                }));
            }
            samples.push(value as f32);
        }
    }
    Ok(Decoded {
        channels,
        sample_rate,
    })
}

/// A logical name: lower case letters and digits, anything else a single hyphen.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "response".to_string()
    } else {
        trimmed
    }
}

/// Every supported file named, or found under a named folder, in a stable order.
pub fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found = Vec::new();
            walk(path, &mut found);
            found.sort();
            out.extend(found);
        } else if is_supported(path) {
            out.push(path.clone());
        }
    }
    out
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
        } else if is_supported(&path) {
            found.push(path);
        }
    }
}

/// A JSON value, written by hand because the examples take no serialiser.
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn num(v: f32) -> Json {
        Json::Number(f64::from(v))
    }

    pub fn opt(v: Option<f32>) -> Json {
        v.map_or(Json::Null, Json::num)
    }

    pub fn text(s: impl Into<String>) -> Json {
        Json::Text(s.into())
    }

    pub fn object(fields: Vec<(&str, Json)>) -> Json {
        Json::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        let pad = |n: usize| "  ".repeat(n);
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Number(v) if v.is_finite() => out.push_str(&format!("{v}")),
            Json::Number(_) => out.push_str("null"),
            Json::Text(s) => {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    item.write(out, depth + 1);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(depth));
                out.push(']');
            }
            Json::Object(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (i, (key, value)) in fields.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    Json::Text(key.clone()).write(out, depth + 1);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                    out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(depth));
                out.push('}');
            }
        }
    }
}
