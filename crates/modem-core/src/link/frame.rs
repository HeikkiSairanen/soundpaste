//! Link layer frames byte by byte (DESIGN.md section 5.1).
//!
//! Byte 0: type (upper nibble) and flags (lower). Bytes 1–2: message
//! identifier. The rest depends on the type. Multi-byte fields are big-endian.

/// Signal quality reported by the receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Quality {
    /// Signal-to-noise ratio in the modem's own band, dB.
    pub snr_db: i8,
    /// Signal RMS at the microphone, dBFS.
    pub level_db: i8,
    /// The microphone is clipping.
    pub clipped: bool,
}

impl Quality {
    pub fn from_stats(snr_db: f32, rms: f32, clipped: f32) -> Quality {
        Quality {
            snr_db: snr_db.clamp(-128.0, 127.0) as i8,
            level_db: (20.0 * rms.max(1e-7).log10()).clamp(-128.0, 127.0) as i8,
            clipped: clipped > 0.01,
        }
    }
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[self.snr_db as u8, self.level_db as u8, self.clipped as u8]);
    }
    fn decode(b: &[u8]) -> Quality {
        Quality { snr_db: b[0] as i8, level_db: b[1] as i8, clipped: b[2] & 1 != 0 }
    }
}

const FLAG_EOB: u8 = 0x01; // DATA: last frame of the burst
const FLAG_RESTART: u8 = 0x01; // ACK: the assembled message did not match, start over
const FLAG_INLINE: u8 = 0x02; // START: the whole message is at the end of the frame
#[allow(dead_code)]
const FLAG_ENCRYPTED: u8 = 0x08; // reserved

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Start: message length, number of frames, frame data size and CRC-32 of the whole message.
    /// `inline`: a short message travels with the start frame.
    Start { id: u16, len: u16, frames: u8, chunk: u8, crc: u32, inline: Option<Vec<u8>> },
    StartAck { id: u16, quality: Quality },
    Data { id: u16, seq: u8, eob: bool, data: Vec<u8> },
    /// Combined ack: bit i is 1 if frame i has been stored.
    Ack { id: u16, quality: Quality, restart: bool, bitmap: Vec<u8> },
    /// Final ack: the message is complete and its CRC-32 is this.
    Done { id: u16, crc: u32 },
    /// The sender asks for the ack again.
    Poll { id: u16 },
    Cancel { id: u16 },
}

impl Frame {
    pub fn id(&self) -> u16 {
        match self {
            Frame::Start { id, .. }
            | Frame::StartAck { id, .. }
            | Frame::Data { id, .. }
            | Frame::Ack { id, .. }
            | Frame::Done { id, .. }
            | Frame::Poll { id }
            | Frame::Cancel { id } => *id,
        }
    }

    /// Is this frame type sent by the sender of the message (or by the receiver)?
    pub fn from_sender(&self) -> bool {
        matches!(self, Frame::Start { .. } | Frame::Data { .. } | Frame::Poll { .. } | Frame::Cancel { .. })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Frame::Start { .. } => "START",
            Frame::StartAck { .. } => "START_ACK",
            Frame::Data { .. } => "DATA",
            Frame::Ack { .. } => "ACK",
            Frame::Done { .. } => "DONE",
            Frame::Poll { .. } => "POLL",
            Frame::Cancel { .. } => "CANCEL",
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let head = |ty: u8, flags: u8, id: u16| vec![(ty << 4) | flags, (id >> 8) as u8, id as u8];
        match self {
            Frame::Start { id, len, frames, chunk, crc, inline } => {
                let mut b = head(1, if inline.is_some() { FLAG_INLINE } else { 0 }, *id);
                b.extend_from_slice(&len.to_be_bytes());
                b.push(*frames);
                b.push(*chunk);
                b.extend_from_slice(&crc.to_be_bytes());
                if let Some(data) = inline {
                    b.extend_from_slice(data);
                }
                b
            }
            Frame::StartAck { id, quality } => {
                let mut b = head(2, 0, *id);
                quality.encode(&mut b);
                b
            }
            Frame::Data { id, seq, eob, data } => {
                let mut b = head(3, if *eob { FLAG_EOB } else { 0 }, *id);
                b.push(*seq);
                b.extend_from_slice(data);
                b
            }
            Frame::Ack { id, quality, restart, bitmap } => {
                let mut b = head(4, if *restart { FLAG_RESTART } else { 0 }, *id);
                quality.encode(&mut b);
                b.extend_from_slice(bitmap);
                b
            }
            Frame::Done { id, crc } => {
                let mut b = head(5, 0, *id);
                b.extend_from_slice(&crc.to_be_bytes());
                b
            }
            Frame::Poll { id } => head(6, 0, *id),
            Frame::Cancel { id } => head(7, 0, *id),
        }
    }

    /// Parse a frame. Unknown type or too short a frame -> None.
    pub fn decode(b: &[u8]) -> Option<Frame> {
        if b.len() < 3 {
            return None;
        }
        let (ty, flags) = (b[0] >> 4, b[0] & 0x0F);
        let id = u16::from_be_bytes([b[1], b[2]]);
        let body = &b[3..];
        Some(match ty {
            1 if body.len() >= 8 => Frame::Start {
                id,
                len: u16::from_be_bytes([body[0], body[1]]),
                frames: body[2],
                chunk: body[3],
                crc: u32::from_be_bytes([body[4], body[5], body[6], body[7]]),
                inline: (flags & FLAG_INLINE != 0).then(|| body[8..].to_vec()),
            },
            2 if body.len() >= 3 => Frame::StartAck { id, quality: Quality::decode(body) },
            3 if !body.is_empty() => Frame::Data { id, seq: body[0], eob: flags & FLAG_EOB != 0, data: body[1..].to_vec() },
            4 if body.len() >= 3 => Frame::Ack { id, quality: Quality::decode(body), restart: flags & FLAG_RESTART != 0, bitmap: body[3..].to_vec() },
            5 if body.len() >= 4 => Frame::Done { id, crc: u32::from_be_bytes([body[0], body[1], body[2], body[3]]) },
            6 => Frame::Poll { id },
            7 => Frame::Cancel { id },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_types_roundtrip() {
        let q = Quality { snr_db: -3, level_db: -42, clipped: true };
        let frames = vec![
            Frame::Start { id: 0xBEEF, len: 300, frames: 5, chunk: 64, crc: 0xDEADBEEF, inline: None },
            Frame::Start { id: 1, len: 3, frames: 1, chunk: 64, crc: 7, inline: Some(b"abc".to_vec()) },
            Frame::Start { id: 2, len: 0, frames: 0, chunk: 64, crc: 0, inline: Some(vec![]) },
            Frame::StartAck { id: 9, quality: q },
            Frame::Data { id: 9, seq: 4, eob: true, data: vec![1, 2, 3] },
            Frame::Data { id: 9, seq: 0, eob: false, data: vec![] },
            Frame::Ack { id: 9, quality: q, restart: false, bitmap: vec![0b1011_0000] },
            Frame::Ack { id: 9, quality: Quality::default(), restart: true, bitmap: vec![0] },
            Frame::Done { id: 9, crc: 0x01020304 },
            Frame::Poll { id: 0xFFFF },
            Frame::Cancel { id: 0 },
        ];
        for f in frames {
            assert_eq!(Frame::decode(&f.encode()), Some(f.clone()), "{f:?}");
        }
    }

    #[test]
    fn sizes_match_design() {
        assert_eq!(Frame::Start { id: 1, len: 60, frames: 1, chunk: 64, crc: 0, inline: None }.encode().len(), 11);
        assert_eq!(Frame::Poll { id: 1 }.encode().len(), 3);
        assert_eq!(Frame::Done { id: 1, crc: 0 }.encode().len(), 7);
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(Frame::decode(&[]), None);
        assert_eq!(Frame::decode(&[0x10, 0, 0, 1]), None); // START too short
        assert_eq!(Frame::decode(&[0xF0, 0, 0]), None); // unknown type
    }
}
