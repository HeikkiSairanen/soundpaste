//! Reading and writing WAV files (RIFF/WAVE) without libraries.
//!
//! Writing: mono, 16-bit PCM or 32-bit float.
//! Reading: PCM 8/16/24/32 bit and IEEE float 32/64, any number of channels
//! (channels are averaged to mono). Unknown chunks are skipped.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

#[derive(Debug, PartialEq)]
pub enum WavError {
    NotRiff,
    Truncated,
    MissingFmt,
    MissingData,
    Unsupported(String),
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WavError::NotRiff => write!(f, "not a RIFF/WAVE file"),
            WavError::Truncated => write!(f, "file is truncated"),
            WavError::MissingFmt => write!(f, "fmt chunk missing"),
            WavError::MissingData => write!(f, "data chunk missing"),
            WavError::Unsupported(s) => write!(f, "unsupported format: {s}"),
        }
    }
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn decode(bytes: &[u8]) -> Result<Wav, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotRiff);
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // (format, channels, rate, bits)
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32le(bytes, pos + 4) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            if body + 16 > bytes.len() {
                return Err(WavError::Truncated);
            }
            let mut format = u16le(bytes, body);
            let channels = u16le(bytes, body + 2);
            let rate = u32le(bytes, body + 4);
            let bits = u16le(bytes, body + 14);
            // WAVE_FORMAT_EXTENSIBLE: the actual format is at the start of the subformat GUID.
            if format == 0xFFFE && size >= 40 && body + 26 <= bytes.len() {
                format = u16le(bytes, body + 24);
            }
            fmt = Some((format, channels, rate, bits));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or(WavError::MissingFmt)?;
            // In files written as a stream (e.g. sox into a pipe) the size can be
            // 0 or 0xFFFFFFFF; in that case read to the end of the file.
            let end = if size == 0 || body + size > bytes.len() { bytes.len() } else { body + size };
            let data = &bytes[body..end];
            if channels == 0 {
                return Err(WavError::Unsupported("0 channels".into()));
            }
            let bps = (bits / 8) as usize;
            let frame = bps * channels as usize;
            if frame == 0 {
                return Err(WavError::Unsupported("0-bit sample".into()));
            }
            let conv: fn(&[u8]) -> f32 = match (format, bits) {
                (1, 8) => |b| (b[0] as f32 - 128.0) / 128.0,
                (1, 16) => |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                (1, 24) => |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
                (1, 32) => |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
                (3, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (3, 64) => |b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32,
                _ => return Err(WavError::Unsupported(format!("format={format} bits={bits}"))),
            };
            let n = data.len() / frame;
            let mut samples = Vec::with_capacity(n);
            let scale = 1.0 / channels as f32;
            for i in 0..n {
                let mut acc = 0.0;
                for c in 0..channels as usize {
                    let o = i * frame + c * bps;
                    acc += conv(&data[o..o + bps]);
                }
                samples.push(acc * scale);
            }
            return Ok(Wav { sample_rate: rate, samples });
        }
        pos = body + size + (size & 1);
    }
    if fmt.is_none() {
        Err(WavError::MissingFmt)
    } else {
        Err(WavError::MissingData)
    }
}

fn header(sample_rate: u32, format: u16, bits: u16, data_len: u32) -> Vec<u8> {
    let block = bits / 8;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * block as u32).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out
}

/// Mono, 16-bit PCM. Values are clipped to [-1, 1].
pub fn encode_pcm16(sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let mut out = header(sample_rate, 1, 16, (samples.len() * 2) as u32);
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Mono, 32-bit float. Lossless, suitable for measurements.
pub fn encode_f32(sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let mut out = header(sample_rate, 3, 32, (samples.len() * 4) as u32);
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_pcm16() {
        let x: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.05).sin() * 0.8).collect();
        let w = decode(&encode_pcm16(44100, &x)).unwrap();
        assert_eq!(w.sample_rate, 44100);
        assert_eq!(w.samples.len(), x.len());
        for (a, b) in w.samples.iter().zip(&x) {
            assert!((a - b).abs() < 1.0 / 16000.0);
        }
    }

    #[test]
    fn roundtrip_f32_exact() {
        let x: Vec<f32> = (0..257).map(|i| (i as f32 * 0.3).cos()).collect();
        let w = decode(&encode_f32(48000, &x)).unwrap();
        assert_eq!(w.samples, x);
    }

    #[test]
    fn stereo_is_downmixed_and_unknown_chunks_skipped() {
        // Hand-built stereo PCM16 with a LIST chunk before the data.
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&48000u32.to_le_bytes());
        b.extend_from_slice(&(48000u32 * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"LIST");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&[1, 2, 3, 0]); // odd size + pad byte
        b.extend_from_slice(b"data");
        b.extend_from_slice(&8u32.to_le_bytes());
        for v in [16384i16, 0, -16384, -16384] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let w = decode(&b).unwrap();
        assert_eq!(w.samples, vec![0.25, -0.5]);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(decode(b"hello"), Err(WavError::NotRiff));
    }
}
