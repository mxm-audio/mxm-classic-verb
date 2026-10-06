//! Reading an impulse response: WAV through `hound`, AIFF and AIFF-C parsed here.
//!
//! **Ported from the logic of `crates/mxm-classic-verb-fit/examples/classic_verb_io`**, which the
//! offline fitter reads the owner's responses with; the plugin does not depend on example code. The
//! AIFF chunk layouts are Apple's published AIFF 1.3 and AIFF-C (1991) ones.
//!
//! **Plan §4.5's two stages, in order.** The header is read and handed to `preflight` before
//! anything proportional to the file is allocated; samples are then decoded and checked as they are
//! read, stopping at the first non-finite one. Every failure is a named [`Refusal`], never a panic:
//! a dropped file is arbitrary bytes.
//!
//! **Every buffer a file sizes is reserved fallibly** (`reserve`). This runs on the host's background
//! thread, and an allocation that fails the ordinary way aborts the host: the header preflight still
//! admits thirty seconds of two channels at 192 kHz, 46 MB. Memory that cannot be had is
//! [`Refusal::OutOfMemory`], and the space that was sounding keeps sounding.
//!
//! Accepted: WAV 8–32-bit integer PCM and 32-bit float (`hound`, including `WAVE_FORMAT_EXTENSIBLE`);
//! AIFF 8–32-bit PCM; AIFF-C `NONE`, `twos` and `sowt` PCM, and `fl32` and `fl64` float.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::loading::Refusal;

/// What a header says: all the fit crate's preflight needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Wav,
    Aiff,
}

/// Stage 1, decided by the caller from the header.
pub type Preflight<'a> = &'a mut dyn FnMut(&Header) -> Result<(), Refusal>;

/// Frames decoded between two looks at whether the job is still wanted, and read at a time.
const FRAMES_PER_READ: usize = 4_096;
/// The most frames reserved up front on a header's word. Preflight has already bounded the header;
/// this keeps a permissive caller from reserving whatever a hostile one claims.
const RESERVE_CAP: usize = 1 << 24;

/// **The seam every buffer a file sizes is reserved through**: `samples` more in `channel`, or the
/// refusal naming how much could not be had. Never an abort.
fn reserve(channel: &mut Vec<f32>, samples: usize) -> Result<(), Refusal> {
    let bytes = samples.saturating_mul(core::mem::size_of::<f32>());
    #[cfg(test)]
    if seam::refuses() {
        return Err(Refusal::OutOfMemory { bytes });
    }
    channel
        .try_reserve(samples)
        .map_err(|_| Refusal::OutOfMemory { bytes })
}

/// `count` channels, each reserved for `samples` up front (at most [`RESERVE_CAP`]).
fn channels_for(count: usize, samples: usize) -> Result<Vec<Vec<f32>>, Refusal> {
    // At most `u16::MAX` empty vectors, and two once preflight has passed: nothing a file sizes.
    let mut channels = Vec::with_capacity(count);
    for _ in 0..count {
        let mut channel = Vec::new();
        reserve(&mut channel, samples.min(RESERVE_CAP))?;
        channels.push(channel);
    }
    Ok(channels)
}

/// Room for `run` more samples in every channel before they are decoded, so no `push` grows a buffer.
/// Nothing is reserved while the header's reservation still holds them, which is always, below
/// [`RESERVE_CAP`].
fn make_room(channels: &mut [Vec<f32>], run: usize) -> Result<(), Refusal> {
    for channel in channels {
        if channel.capacity() - channel.len() < run {
            reserve(channel, run)?;
        }
    }
    Ok(())
}

/// **Test builds only**: refuse every reservation made inside [`seam::refusing`]. Thread-local, because
/// the harness runs tests in parallel and a job runs on the thread that called it.
#[cfg(test)]
pub(crate) mod seam {
    use std::cell::Cell;

    thread_local! {
        static REFUSING: Cell<bool> = const { Cell::new(false) };
    }

    pub(crate) fn refusing<R>(body: impl FnOnce() -> R) -> R {
        REFUSING.with(|refusing| refusing.set(true));
        let result = body();
        REFUSING.with(|refusing| refusing.set(false));
        result
    }

    pub(super) fn refuses() -> bool {
        REFUSING.with(Cell::get)
    }
}

/// The format a file's extension names, or the refusal that it names none this plugin reads.
pub fn format_of(path: &Path) -> Result<Format, Refusal> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "wav" | "wave" => Ok(Format::Wav),
        "aif" | "aiff" | "aifc" => Ok(Format::Aiff),
        _ => Err(Refusal::Unsupported { extension }),
    }
}

pub fn read_file(
    path: &Path,
    preflight: Preflight<'_>,
    keep_going: &dyn Fn() -> bool,
) -> Result<Decoded, Refusal> {
    let format = format_of(path)?;
    let file = File::open(path).map_err(|error| Refusal::Unreadable(error.to_string()))?;
    read(BufReader::new(file), format, preflight, keep_going)
}

pub fn read<R: Read + Seek>(
    reader: R,
    format: Format,
    preflight: Preflight<'_>,
    keep_going: &dyn Fn() -> bool,
) -> Result<Decoded, Refusal> {
    match format {
        Format::Wav => read_wav(reader, preflight, keep_going),
        Format::Aiff => read_aiff(reader, preflight, keep_going),
    }
}

// ---------------------------------------------------------------------------------------------------
// WAV
// ---------------------------------------------------------------------------------------------------

fn wav_refusal(error: hound::Error) -> Refusal {
    match error {
        // `hound` reports a short read as `ErrorKind::Other` ("Failed to read enough bytes."), and
        // the file itself opened, so a failed read here is a file that ends inside its header.
        hound::Error::IoError(_) => Refusal::Malformed(
            "this is not a readable WAV file: it ends inside its header".to_owned(),
        ),
        hound::Error::FormatError(why) => {
            Refusal::Malformed(format!("this is not a readable WAV file ({why})"))
        }
        hound::Error::TooWide => Refusal::Encoding("WAV samples wider than 32 bits".to_owned()),
        hound::Error::UnfinishedSample => {
            Refusal::Malformed("the WAV file ends part way through a sample".to_owned())
        }
        hound::Error::Unsupported => Refusal::Encoding("this kind of WAV file".to_owned()),
        hound::Error::InvalidSampleFormat => Refusal::Encoding("this WAV sample format".to_owned()),
    }
}

/// A sample that cannot be read once the header has been: the data chunk promised more than the file
/// holds. `hound` reports a short read with whatever error kind its reader gives, so every I/O
/// failure here is named as the file ending early; a format error keeps its own name.
fn truncated(error: hound::Error) -> Refusal {
    match error {
        hound::Error::IoError(_) => {
            Refusal::Malformed("the WAV file ends before its audio does".to_owned())
        }
        other => wav_refusal(other),
    }
}

fn read_wav<R: Read>(
    reader: R,
    preflight: Preflight<'_>,
    keep_going: &dyn Fn() -> bool,
) -> Result<Decoded, Refusal> {
    let mut wav = hound::WavReader::new(reader).map_err(wav_refusal)?;
    let spec = wav.spec();
    let frames = u64::from(wav.duration());
    preflight(&Header {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        frames,
    })?;
    let count = usize::from(spec.channels);
    if count == 0 {
        return Err(Refusal::Malformed(
            "the WAV file has no channels".to_owned(),
        ));
    }
    // Every sample the data chunk holds, per channel, so no channel grows.
    let total = usize::try_from(wav.len()).unwrap_or(usize::MAX);
    let mut channels = channels_for(count, total.div_ceil(count))?;
    let check = FRAMES_PER_READ * count;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            if spec.bits_per_sample != 32 {
                return Err(Refusal::Encoding(format!(
                    "{}-bit float WAV",
                    spec.bits_per_sample
                )));
            }
            for (index, sample) in wav.samples::<f32>().enumerate() {
                if index % check == 0 {
                    if !keep_going() {
                        return Err(Refusal::Stopped);
                    }
                    make_room(
                        &mut channels,
                        total
                            .saturating_sub(index)
                            .div_ceil(count)
                            .min(FRAMES_PER_READ),
                    )?;
                }
                let sample = sample.map_err(truncated)?;
                if !sample.is_finite() {
                    return Err(Refusal::NonFinite {
                        channel: index % count,
                        frame: index / count,
                    });
                }
                channels[index % count].push(sample);
            }
        }
        hound::SampleFormat::Int => {
            if !(1..=32).contains(&spec.bits_per_sample) {
                return Err(Refusal::Encoding(format!(
                    "{}-bit integer WAV",
                    spec.bits_per_sample
                )));
            }
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f64;
            for (index, sample) in wav.samples::<i32>().enumerate() {
                if index % check == 0 {
                    if !keep_going() {
                        return Err(Refusal::Stopped);
                    }
                    make_room(
                        &mut channels,
                        total
                            .saturating_sub(index)
                            .div_ceil(count)
                            .min(FRAMES_PER_READ),
                    )?;
                }
                let sample = sample.map_err(truncated)?;
                channels[index % count].push((f64::from(sample) * scale) as f32);
            }
        }
    }
    even_out(&mut channels);
    Ok(Decoded {
        channels,
        sample_rate: spec.sample_rate,
    })
}

/// A last frame cut part way through leaves the channels uneven; the analyser refuses uneven
/// channels, and a partial frame is not part of the response.
fn even_out(channels: &mut [Vec<f32>]) {
    let shortest = channels.iter().map(Vec::len).min().unwrap_or(0);
    for channel in channels {
        channel.truncate(shortest);
    }
}

// ---------------------------------------------------------------------------------------------------
// AIFF and AIFF-C
// ---------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

fn be_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// An IEEE 754 80-bit extended value, as AIFF stores its sample rate: a sign and a 15-bit exponent,
/// then a 64-bit mantissa with an explicit integer bit.
fn extended(bytes: &[u8]) -> f64 {
    let sign = if bytes[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from(u16::from_be_bytes([bytes[0] & 0x7f, bytes[1]]));
    let mut mantissa = [0u8; 8];
    mantissa.copy_from_slice(&bytes[2..10]);
    let mantissa = u64::from_be_bytes(mantissa);
    if exponent == 0 && mantissa == 0 {
        return 0.0;
    }
    sign * mantissa as f64 * 2f64.powi(exponent - 16_383 - 63)
}

fn cut_off(what: &str) -> impl FnOnce(std::io::Error) -> Refusal + '_ {
    move |_| Refusal::Malformed(format!("the AIFF file ends inside its {what}"))
}

fn read_aiff<R: Read + Seek>(
    mut reader: R,
    preflight: Preflight<'_>,
    keep_going: &dyn Fn() -> bool,
) -> Result<Decoded, Refusal> {
    let mut form = [0u8; 12];
    reader
        .read_exact(&mut form)
        .map_err(|_| Refusal::Malformed("this is too short to be an AIFF file".to_owned()))?;
    if &form[0..4] != b"FORM" {
        return Err(Refusal::Malformed(
            "this is not an AIFF file: it has no FORM chunk".to_owned(),
        ));
    }
    let aifc = match &form[8..12] {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => {
            return Err(Refusal::Malformed(
                "this is not an AIFF or AIFF-C file".to_owned(),
            ));
        }
    };

    let mut common: Option<Common> = None;
    let mut sound: Option<(u64, u64)> = None;
    let mut position = 12u64;
    while common.is_none() || sound.is_none() {
        let mut head = [0u8; 8];
        if reader.read_exact(&mut head).is_err() {
            break;
        }
        let size = u64::from(be_u32(&head[4..8]));
        let body = position + 8;
        match &head[0..4] {
            b"COMM" => {
                if size < 18 {
                    return Err(Refusal::Malformed(
                        "the AIFF COMM chunk is too short".to_owned(),
                    ));
                }
                let mut bytes = vec![0u8; size.min(64) as usize];
                reader
                    .read_exact(&mut bytes)
                    .map_err(cut_off("COMM chunk"))?;
                let encoding = if aifc && bytes.len() >= 22 {
                    match &bytes[18..22] {
                        b"NONE" | b"twos" => Encoding::BigEndianPcm,
                        b"sowt" => Encoding::LittleEndianPcm,
                        b"fl32" | b"FL32" => Encoding::Float32,
                        b"fl64" | b"FL64" => Encoding::Float64,
                        other => {
                            return Err(Refusal::Encoding(format!(
                                "compressed AIFF-C (`{}`)",
                                String::from_utf8_lossy(other).trim()
                            )));
                        }
                    }
                } else {
                    Encoding::BigEndianPcm
                };
                common = Some(Common {
                    channels: be_u16(&bytes[0..2]),
                    frames: be_u32(&bytes[2..6]),
                    bits: be_u16(&bytes[6..8]),
                    rate: extended(&bytes[8..18]),
                    encoding,
                });
            }
            b"SSND" => {
                if size < 8 {
                    return Err(Refusal::Malformed(
                        "the AIFF SSND chunk is too short".to_owned(),
                    ));
                }
                let mut bytes = [0u8; 8];
                reader
                    .read_exact(&mut bytes)
                    .map_err(cut_off("SSND chunk"))?;
                let offset = u64::from(be_u32(&bytes[0..4]));
                sound = Some((body + 8 + offset, size.saturating_sub(8 + offset)));
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        position = body + size + (size & 1);
        reader
            .seek(SeekFrom::Start(position))
            .map_err(|error| Refusal::Unreadable(error.to_string()))?;
    }
    let common =
        common.ok_or_else(|| Refusal::Malformed("the AIFF file has no COMM chunk".to_owned()))?;
    let (data_start, data_len) =
        sound.ok_or_else(|| Refusal::Malformed("the AIFF file has no SSND chunk".to_owned()))?;
    if !common.rate.is_finite() || common.rate < 1.0 || common.rate > f64::from(u32::MAX) {
        return Err(Refusal::Malformed(format!(
            "the AIFF file's sample rate ({}) is not a rate",
            common.rate
        )));
    }
    let sample_rate = common.rate.round() as u32;
    preflight(&Header {
        channels: common.channels,
        sample_rate,
        frames: u64::from(common.frames),
    })?;

    let width = match common.encoding {
        Encoding::Float32 => 4,
        Encoding::Float64 => 8,
        Encoding::BigEndianPcm | Encoding::LittleEndianPcm => {
            let width = usize::from(common.bits).div_ceil(8);
            if !(1..=4).contains(&width) {
                return Err(Refusal::Encoding(format!(
                    "{}-bit AIFF samples",
                    common.bits
                )));
            }
            width
        }
    };
    let count = usize::from(common.channels);
    if count == 0 {
        return Err(Refusal::Malformed(
            "the AIFF file has no channels".to_owned(),
        ));
    }
    let frame_bytes = width * count;
    let frames = (u64::from(common.frames).min(data_len / frame_bytes as u64)) as usize;
    reader
        .seek(SeekFrom::Start(data_start))
        .map_err(|error| Refusal::Unreadable(error.to_string()))?;

    let mut channels = channels_for(count, frames)?;
    let scale = 1.0 / (1u64 << (8 * width - 1)) as f64;
    let mut buffer = vec![0u8; FRAMES_PER_READ * frame_bytes];
    let mut done = 0usize;
    while done < frames {
        if !keep_going() {
            return Err(Refusal::Stopped);
        }
        let run = (frames - done).min(FRAMES_PER_READ);
        make_room(&mut channels, run)?;
        let bytes = &mut buffer[..run * frame_bytes];
        reader.read_exact(bytes).map_err(|_| {
            Refusal::Malformed("the AIFF sound data ends before its chunk says it does".to_owned())
        })?;
        for (index, sample) in bytes.chunks_exact(width).enumerate() {
            let value = match common.encoding {
                Encoding::Float32 => f64::from(f32::from_be_bytes([
                    sample[0], sample[1], sample[2], sample[3],
                ])),
                Encoding::Float64 => {
                    let mut raw = [0u8; 8];
                    raw.copy_from_slice(sample);
                    f64::from_be_bytes(raw)
                }
                Encoding::BigEndianPcm | Encoding::LittleEndianPcm => {
                    let mut raw = 0i64;
                    for k in 0..width {
                        let byte = if common.encoding == Encoding::BigEndianPcm {
                            sample[k]
                        } else {
                            sample[width - 1 - k]
                        };
                        raw = (raw << 8) | i64::from(byte);
                    }
                    // Sign-extend from the sample's own width; AIFF PCM is left-justified in it.
                    let shift = 64 - 8 * width;
                    ((raw << shift) >> shift) as f64 * scale
                }
            };
            let value = value as f32;
            if !value.is_finite() {
                return Err(Refusal::NonFinite {
                    channel: index % count,
                    frame: done + index / count,
                });
            }
            channels[index % count].push(value);
        }
        done += run;
    }
    Ok(Decoded {
        channels,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, AiffEncoding, WavFormat};
    use std::io::Cursor;

    fn accept(_: &Header) -> Result<(), Refusal> {
        Ok(())
    }

    fn decode(bytes: Vec<u8>, format: Format) -> Result<Decoded, Refusal> {
        read(Cursor::new(bytes), format, &mut accept, &|| true)
    }

    /// A planted response inside what every integer width can write.
    fn clamped(planted: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        planted
            .into_iter()
            .map(|c| c.into_iter().map(|v| v.clamp(-0.99, 0.99)).collect())
            .collect()
    }

    fn assert_close(decoded: &Decoded, planted: &[Vec<f32>], tolerance: f32, what: &str) {
        assert_eq!(decoded.channels.len(), planted.len(), "{what}");
        for (got, want) in decoded.channels.iter().zip(planted) {
            assert_eq!(got.len(), want.len(), "{what}");
            for (frame, (a, b)) in got.iter().zip(want).enumerate() {
                assert!(
                    (a - b).abs() <= tolerance,
                    "{what}: frame {frame}: {a} against {b}"
                );
            }
        }
    }

    #[test]
    fn every_accepted_encoding_decodes_to_the_planted_samples() {
        for channel_count in [1, 2] {
            let planted = testing::response(24_000, 0.05, 0.02, channel_count, 3);
            let planted: Vec<Vec<f32>> = planted
                .into_iter()
                .map(|c| c.into_iter().map(|v| v.clamp(-0.99, 0.99)).collect())
                .collect();
            for (format, tolerance) in [
                (WavFormat::Int16, 2.0 / 32_000.0),
                (WavFormat::Int24, 2.0 / 8_000_000.0),
                (WavFormat::Float32, 0.0),
            ] {
                let decoded = decode(testing::wav_bytes(&planted, 24_000, format), Format::Wav)
                    .unwrap_or_else(|refusal| panic!("{format:?}: {refusal}"));
                assert_eq!(decoded.sample_rate, 24_000);
                assert_close(&decoded, &planted, tolerance, &format!("{format:?}"));
            }
            for (encoding, tolerance) in [
                (AiffEncoding::Pcm16, 2.0 / 32_000.0),
                (AiffEncoding::Pcm24, 2.0 / 8_000_000.0),
                (AiffEncoding::Float32, 0.0),
                (AiffEncoding::Float64, 0.0),
                (AiffEncoding::Sowt16, 2.0 / 32_000.0),
            ] {
                let decoded = decode(
                    testing::aiff_bytes(&planted, 44_100, encoding),
                    Format::Aiff,
                )
                .unwrap_or_else(|refusal| panic!("{encoding:?}: {refusal}"));
                assert_eq!(decoded.sample_rate, 44_100, "{encoding:?}");
                assert_close(&decoded, &planted, tolerance, &format!("{encoding:?}"));
            }
        }
    }

    /// **Stage 1 comes first.** A header whose sound data is missing entirely is refused by the
    /// preflight, not by the decoder tripping over the absent samples, which is what proves the
    /// header was judged before a sample was read.
    #[test]
    fn the_header_is_judged_before_any_sample_is_read() {
        let mut bytes =
            testing::aiff_bytes(&vec![vec![0.5; 48_000]; 3], 48_000, AiffEncoding::Pcm16);
        // Keep FORM, COMM and the SSND header; drop every sample.
        let ssnd = bytes.windows(4).position(|w| w == b"SSND").unwrap();
        bytes.truncate(ssnd + 16);
        let mut seen = None;
        let refused = read(
            Cursor::new(bytes),
            Format::Aiff,
            &mut |header: &Header| {
                seen = Some(*header);
                crate::fitting::preflight(header)
            },
            &|| true,
        )
        .unwrap_err();
        assert_eq!(
            seen,
            Some(Header {
                channels: 3,
                sample_rate: 48_000,
                frames: 48_000
            })
        );
        assert!(
            matches!(&refused, Refusal::Input(why) if why.contains("3 channels")),
            "{refused:?}"
        );

        let wav = testing::wav_bytes(&vec![vec![0.1; 4_800]; 3], 48_000, WavFormat::Int16);
        let refused = read(
            Cursor::new(wav),
            Format::Wav,
            &mut crate::fitting::preflight,
            &|| panic!("a refused header went on to decode"),
        )
        .unwrap_err();
        assert!(matches!(refused, Refusal::Input(_)), "{refused:?}");
    }

    #[test]
    fn decoding_stops_at_the_first_non_finite_sample() {
        let mut planted = vec![vec![0.25f32; 64]; 2];
        planted[1][5] = f32::NAN;
        planted[0][9] = f32::INFINITY;
        assert_eq!(
            decode(
                testing::wav_bytes(&planted, 48_000, WavFormat::Float32),
                Format::Wav
            ),
            Err(Refusal::NonFinite {
                channel: 1,
                frame: 5
            })
        );
        assert_eq!(
            decode(
                testing::aiff_bytes(&planted, 48_000, AiffEncoding::Float32),
                Format::Aiff
            ),
            Err(Refusal::NonFinite {
                channel: 1,
                frame: 5
            })
        );
    }

    #[test]
    fn every_decoder_failure_is_a_named_refusal() {
        assert_eq!(
            format_of(Path::new("song.mp3")),
            Err(Refusal::Unsupported {
                extension: "mp3".into()
            })
        );
        assert_eq!(format_of(Path::new("ROOM.WAV")), Ok(Format::Wav));
        assert_eq!(format_of(Path::new("hall.AIFC")), Ok(Format::Aiff));
        assert!(matches!(
            format_of(Path::new("no-extension")),
            Err(Refusal::Unsupported { .. })
        ));

        let garbage = b"this is not audio at all, just a text file with a wav name".to_vec();
        assert!(matches!(
            decode(garbage.clone(), Format::Wav),
            Err(Refusal::Malformed(_))
        ));
        assert!(matches!(
            decode(garbage, Format::Aiff),
            Err(Refusal::Malformed(_))
        ));

        let ulaw = testing::aiff_bytes(&[vec![0.0; 32]], 48_000, AiffEncoding::Ulaw);
        assert!(
            matches!(decode(ulaw, Format::Aiff), Err(Refusal::Encoding(ref what)) if what.contains("ulaw"))
        );

        // A response cut off part way through its sound data.
        let mut cut = testing::aiff_bytes(&[vec![0.1; 4_000]], 48_000, AiffEncoding::Pcm24);
        cut.truncate(cut.len() - 1_000);
        assert!(matches!(
            decode(cut, Format::Aiff),
            Err(Refusal::Malformed(_))
        ));
        let mut cut = testing::wav_bytes(&[vec![0.1; 4_000]], 48_000, WavFormat::Int24);
        cut.truncate(cut.len() - 1_001);
        assert!(matches!(
            decode(cut, Format::Wav),
            Err(Refusal::Malformed(_))
        ));
    }

    /// Arbitrary bytes never panic the decoder: every prefix of a valid file, of each kind.
    #[test]
    fn no_prefix_of_a_valid_file_panics_the_decoder() {
        let planted = testing::response(22_050, 0.01, 0.005, 2, 9);
        let files = [
            (
                testing::wav_bytes(&planted, 22_050, WavFormat::Int16),
                Format::Wav,
            ),
            (
                testing::wav_bytes(&planted, 22_050, WavFormat::Float32),
                Format::Wav,
            ),
            (
                testing::aiff_bytes(&planted, 22_050, AiffEncoding::Pcm24),
                Format::Aiff,
            ),
            (
                testing::aiff_bytes(&planted, 22_050, AiffEncoding::Float32),
                Format::Aiff,
            ),
        ];
        for (bytes, format) in files {
            for end in 0..bytes.len() {
                let _ = decode(bytes[..end].to_vec(), format);
            }
            assert!(decode(bytes, format).is_ok());
        }
    }

    /// **Every channel is reserved once, for what the file holds, and never grows.** A buffer that
    /// grows while decoding reallocates the ordinary way, and an allocation that fails that way aborts
    /// the host. Stereo, because `vec![Vec::with_capacity(n); 2]` clones its first element, and a clone
    /// keeps the contents and not the capacity.
    #[test]
    fn every_channel_is_reserved_once_for_what_the_file_holds() {
        let planted = clamped(testing::response(24_000, 0.5, 0.2, 2, 4));
        let frames = planted[0].len();
        for (bytes, format) in [
            (
                testing::wav_bytes(&planted, 24_000, WavFormat::Float32),
                Format::Wav,
            ),
            (
                testing::wav_bytes(&planted, 24_000, WavFormat::Int24),
                Format::Wav,
            ),
            (
                testing::aiff_bytes(&planted, 24_000, AiffEncoding::Pcm16),
                Format::Aiff,
            ),
        ] {
            let decoded = decode(bytes, format).unwrap_or_else(|refusal| panic!("{refusal}"));
            let capacities: Vec<usize> = decoded.channels.iter().map(Vec::capacity).collect();
            assert_eq!(
                capacities,
                vec![frames; 2],
                "{format:?}: a channel was not reserved once for the {frames} frames the file holds"
            );
        }
    }

    /// **A reservation that cannot be made is a named refusal**, at the header's reservation and at
    /// any growth past it, and a reservation already made is not asked for again.
    #[test]
    fn a_reservation_that_cannot_be_made_is_refused_by_name() {
        let planted = clamped(testing::response(24_000, 0.5, 0.2, 2, 6));
        // The first channel's reservation, which is the first asked for.
        let bytes = planted[0].len() * core::mem::size_of::<f32>();
        for (file, format) in [
            (
                testing::wav_bytes(&planted, 24_000, WavFormat::Int24),
                Format::Wav,
            ),
            (
                testing::aiff_bytes(&planted, 24_000, AiffEncoding::Float32),
                Format::Aiff,
            ),
        ] {
            assert_eq!(
                seam::refusing(|| decode(file.clone(), format)),
                Err(Refusal::OutOfMemory { bytes }),
                "{format:?}"
            );
            assert!(decode(file, format).is_ok(), "{format:?} after the refusal");
        }

        let mut empty = vec![Vec::new()];
        assert_eq!(
            seam::refusing(|| make_room(&mut empty, 100)),
            Err(Refusal::OutOfMemory { bytes: 400 })
        );
        let mut held = vec![Vec::with_capacity(100)];
        assert_eq!(seam::refusing(|| make_room(&mut held, 100)), Ok(()));
    }

    #[test]
    fn a_superseded_decode_stops() {
        let planted = testing::response(24_000, 0.5, 0.2, 1, 1);
        for (bytes, format) in [
            (
                testing::wav_bytes(&planted, 24_000, WavFormat::Int16),
                Format::Wav,
            ),
            (
                testing::aiff_bytes(&planted, 24_000, AiffEncoding::Pcm16),
                Format::Aiff,
            ),
        ] {
            assert_eq!(
                read(Cursor::new(bytes), format, &mut accept, &|| false),
                Err(Refusal::Stopped)
            );
        }
    }

    #[test]
    fn a_file_on_disk_is_read_by_its_extension() {
        let planted = testing::response(24_000, 0.02, 0.01, 1, 5);
        let aiff = testing::TempFile::new(
            "disk.AIF",
            &testing::aiff_bytes(&planted, 24_000, AiffEncoding::Float32),
        );
        let decoded = read_file(aiff.path(), &mut accept, &|| true).expect("an AIFF on disk");
        assert_close(&decoded, &planted, 0.0, "AIFF on disk");
        assert!(matches!(
            read_file(Path::new("missing-response.wav"), &mut accept, &|| true),
            Err(Refusal::Unreadable(_))
        ));
    }
}
