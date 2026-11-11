//! Tarkistussummat.
//!
//! - CRC-16/CCITT-FALSE (polynomi 0x1021, alkuarvo 0xFFFF): kehyksen eheys.
//!   Havaitsee kaikki enintään 16 bitin ryöppyvirheet ja kaikki 1–3 bitin virheet
//!   meidän kehyspituuksillamme.
//! - CRC-32 (IEEE 802.3, heijastettu 0xEDB88320): koko viestin eheys.
//! - CRC-8 (polynomi 0x07): lyhyet otsakkeet.
//!
//! Toteutus on bitti kerrallaan: kehykset ovat kymmeniä tavuja, joten
//! taulukko ei toisi mitattavaa hyötyä ja veisi wasm-binääristä tilaa.

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

    // Tunnetut tarkistusarvot merkkijonolle "123456789".
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
