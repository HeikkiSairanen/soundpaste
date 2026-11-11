//! Checksums.
//!
//! - CRC-16/CCITT-FALSE (polynomial 0x1021, initial value 0xFFFF): frame integrity.
//!   Detects all burst errors of at most 16 bits and all 1–3 bit errors
//!   at our frame lengths.
//! - CRC-32 (IEEE 802.3, reflected 0xEDB88320): integrity of the whole message.
//! - CRC-8 (polynomial 0x07): short headers.
//!
//! The implementation is bit by bit: frames are tens of bytes, so a
//! table would bring no measurable benefit and would take space in the wasm binary.

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known check values for the string "123456789".
    #[test]
    fn check_values() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc8(b"123456789"), 0xF4);
    }

    #[test]
    fn detects_single_bit_flip() {
        let mut d = *b"soundpaste";
        let c = crc16(&d);
        d[3] ^= 0x10;
        assert_ne!(crc16(&d), c);
    }
}
