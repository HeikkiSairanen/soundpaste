//! WebAssembly-kääre.
//!
//! Selain näkee muutaman C-funktion (`#[no_mangle] extern "C"`) ja wasm-muistin.
//! Ei wasm-bindgeniä: JavaScript-liima (web/worklet.js) kopioi näytteet
//! kiinteisiin puskureihin ja lukee tapahtumat tavuina.
//!
//! Moduulissa on yksi asema kerrallaan. Kaikki kutsut tulevat AudioWorkletin
//! säikeestä, joten samanaikaisuutta ei ole.
//!
//! Tapahtumien tavumuoto (`sp_poll_event`), monitavuiset kentät little-endian:
//!
//! | tyyppi | sisältö |
//! |---|---|
//! | 1 lähetyksen tila | alatyyppi u8, a u32, b u32, c u32 |
//! | 2 perillä | tunniste u16, kesto ms u32 |
//! | 3 epäonnistui | syy u8, neuvo u8, onko laatu u8, SNR i8, taso i8, leikkautuu u8 |
//! | 4 peruutettu | |
//! | 5 vastaanotto alkoi | tunniste u16, pituus u32 |
//! | 6 vastaanoton edistyminen | saatu u32, yhteensä u32 |
//! | 7 vastaanotettu | tunniste u16, pituus u32, tavut |
//! | 8 vastaanotto keskeytyi | syy u8 |
//! | 9 loki | UTF-8-teksti |

use modem_core::link::{AbortReason, Event, FailReason, Hint, LinkConfig, SendError, TxStatus};
use modem_core::phy::PhyKind;
use modem_core::station::Station;

/// Suurin lohko, jonka `sp_process` käsittelee kerralla (AudioWorklet käyttää 128:aa).
const MAX_BLOCK: usize = 4096;

struct State {
    station: Station,
    input: Vec<f32>,
    output: Vec<f32>,
}

static mut STATE: Option<State> = None;

#[allow(static_mut_refs)]
fn state() -> Option<&'static mut State> {
    // Turvallista, koska wasm-moduulia kutsutaan vain yhdestä säikeestä.
    unsafe { STATE.as_mut() }
}

/// Varaa `len` tavua wasm-muistista JavaScriptin käyttöön.
#[no_mangle]
pub extern "C" fn sp_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Vapauta `sp_alloc`:lla varattu muisti.
///
/// # Safety
/// `ptr` ja `len` ovat samat kuin varattaessa.
#[no_mangle]
pub unsafe extern "C" fn sp_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// Luo asema. Näytteenottotaajuus tulee AudioContextista, siemen selaimen
/// satunnaislähteestä. Liput: bitti 0 = aina erillinen kättely (ei pikalähetystä).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sp_new(sample_rate: f32, seed_lo: u32, seed_hi: u32, flags: u32) {
    let cfg = LinkConfig { inline_start: flags & 1 == 0, ..LinkConfig::default() };
    let seed = ((seed_hi as u64) << 32) | seed_lo as u64;
    let station = Station::new(PhyKind::DEFAULT, sample_rate as f64, cfg, seed);
    unsafe {
        STATE = Some(State { station, input: vec![0.0; MAX_BLOCK], output: vec![0.0; MAX_BLOCK] });
    }
}

#[no_mangle]
pub extern "C" fn sp_max_block() -> usize {
    MAX_BLOCK
}

/// Mikrofonipuskurin osoite: JavaScript kirjoittaa tänne ennen `sp_process`-kutsua.
#[no_mangle]
pub extern "C" fn sp_input_ptr() -> *mut f32 {
    state().map_or(std::ptr::null_mut(), |s| s.input.as_mut_ptr())
}

/// Kaiutinpuskurin osoite: JavaScript lukee täältä `sp_process`-kutsun jälkeen.
#[no_mangle]
pub extern "C" fn sp_output_ptr() -> *const f32 {
    state().map_or(std::ptr::null(), |s| s.output.as_ptr())
}

/// Käsittele `n` näytettä puskureista.
#[no_mangle]
pub extern "C" fn sp_process(n: usize) {
    if let Some(s) = state() {
        let n = n.min(MAX_BLOCK);
        let (input, output) = (&s.input[..n], &mut s.output[..n]);
        s.station.process(input, output);
    }
}

/// Aloita lähetys. Palauttaa 0 = ok, 1 = edellinen kesken, 2 = liian pitkä, 3 = asemaa ei ole.
///
/// # Safety
/// `ptr` osoittaa `len` tavuun wasm-muistissa.
#[no_mangle]
pub unsafe extern "C" fn sp_send(ptr: *const u8, len: usize) -> i32 {
    let Some(s) = state() else { return 3 };
    let data = if len == 0 { &[][..] } else { std::slice::from_raw_parts(ptr, len) };
    match s.station.send(data) {
        Ok(_) => 0,
        Err(SendError::Busy) => 1,
        Err(SendError::TooLong { .. }) => 2,
    }
}

#[no_mangle]
pub extern "C" fn sp_cancel() {
    if let Some(s) = state() {
        s.station.cancel();
    }
}

#[no_mangle]
pub extern "C" fn sp_max_message() -> usize {
    state().map_or(0, |s| s.station.max_message())
}

/// Arvio siirron kestosta sekunteina `len` tavun viestille.
#[no_mangle]
pub extern "C" fn sp_estimate(len: usize) -> f32 {
    state().map_or(0.0, |s| s.station.estimate_seconds(len) as f32)
}

/// Mikrofonin tehollisarvo (noin 100 ms:n liukuva keskiarvo).
#[no_mangle]
pub extern "C" fn sp_level() -> f32 {
    state().map_or(0.0, |s| s.station.input_level().0)
}

/// Mikrofonin huippuarvo viimeisimmässä lohkossa.
#[no_mangle]
pub extern "C" fn sp_peak() -> f32 {
    state().map_or(0.0, |s| s.station.input_level().1)
}

/// Aseman kello millisekunteina (käsitellyistä näytteistä).
#[no_mangle]
pub extern "C" fn sp_now_ms() -> f64 {
    state().map_or(0.0, |s| s.station.now_ms() as f64)
}

/// Fyysisen kerroksen laskurit: 5 × u32 (alkumerkit, ehjät, hylätyt, lähetetyt,
/// korjatut tavut) ja 2 × f32 (viimeisin SNR dB, viimeisin taso).
///
/// # Safety
/// `out` osoittaa vähintään 28 tavuun.
#[no_mangle]
pub unsafe extern "C" fn sp_stats(out: *mut u8) {
    let Some(s) = state() else { return };
    let st = s.station.stats();
    let mut bytes = Vec::with_capacity(28);
    for v in [st.preambles, st.frames_ok, st.frames_failed, st.frames_sent, st.corrected] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(&st.last_snr_db.to_le_bytes());
    bytes.extend_from_slice(&st.last_rms.to_le_bytes());
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
}

pub fn encode_event(e: &Event) -> Vec<u8> {
    let mut b = Vec::new();
    let u32le = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
    match e {
        Event::Tx(status) => {
            let (sub, a, x, c) = match *status {
                TxStatus::Listening => (0, 0, 0, 0),
                TxStatus::Starting { attempt, of } => (1, attempt, of, 0),
                TxStatus::Sending { frame, of, round } => (2, frame, of, round),
                TxStatus::WaitingAck => (3, 0, 0, 0),
                TxStatus::Polling { attempt, of } => (4, attempt, of, 0),
                TxStatus::Retrying { missing } => (5, missing, 0, 0),
                TxStatus::Deferred => (6, 0, 0, 0),
            };
            b.extend_from_slice(&[1, sub]);
            for v in [a, x, c] {
                u32le(&mut b, v);
            }
        }
        Event::Delivered { id, elapsed } => {
            b.push(2);
            b.extend_from_slice(&id.to_le_bytes());
            u32le(&mut b, *elapsed as u32);
        }
        Event::Failed { reason, hint, quality } => {
            let reason = match reason {
                FailReason::NoReceiver => 0,
                FailReason::NoResponse => 1,
                FailReason::TooManyRetries => 2,
                FailReason::Mismatch => 3,
            };
            let hint = match hint {
                Hint::None => 0,
                Hint::TooQuiet => 1,
                Hint::Clipping => 2,
                Hint::Noisy => 3,
            };
            let q = quality.unwrap_or_default();
            b.extend_from_slice(&[3, reason, hint, quality.is_some() as u8, q.snr_db as u8, q.level_db as u8, q.clipped as u8]);
        }
        Event::Cancelled => b.push(4),
        Event::RxStarted { id, len } => {
            b.push(5);
            b.extend_from_slice(&id.to_le_bytes());
            u32le(&mut b, *len as u32);
        }
        Event::RxProgress { have, of } => {
            b.push(6);
            u32le(&mut b, *have);
            u32le(&mut b, *of);
        }
        Event::Received { id, data } => {
            b.push(7);
            b.extend_from_slice(&id.to_le_bytes());
            u32le(&mut b, data.len() as u32);
            b.extend_from_slice(data);
        }
        Event::RxAborted { reason } => b.extend_from_slice(&[8, matches!(reason, AbortReason::Cancelled) as u8]),
        Event::Log(s) => {
            b.push(9);
            b.extend_from_slice(s.as_bytes());
        }
    }
    b
}

/// Hae seuraava tapahtuma puskuriin. Palauttaa tavumäärän, 0 jos tapahtumia ei
/// ole, tai negatiivisen tarvittavan koon, jos puskuri on liian pieni (tapahtuma
/// säilyy jonossa).
///
/// # Safety
/// `out` osoittaa `cap` tavuun wasm-muistissa.
#[no_mangle]
pub unsafe extern "C" fn sp_poll_event(out: *mut u8, cap: usize) -> i32 {
    static mut PENDING: Option<Vec<u8>> = None;
    #[allow(static_mut_refs)]
    let pending = &mut PENDING;
    if pending.is_none() {
        let Some(s) = state() else { return 0 };
        *pending = s.station.poll_event().map(|e| encode_event(&e));
    }
    match pending.take() {
        None => 0,
        Some(bytes) if bytes.len() > cap => {
            let need = bytes.len() as i32;
            *pending = Some(bytes);
            -need
        }
        Some(bytes) => {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
            bytes.len() as i32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modem_core::link::Quality;

    #[test]
    fn event_encoding_is_stable() {
        assert_eq!(encode_event(&Event::Tx(TxStatus::Sending { frame: 2, of: 5, round: 1 })), [1, 2, 2, 0, 0, 0, 5, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(encode_event(&Event::Delivered { id: 0x1234, elapsed: 2820 }), [2, 0x34, 0x12, 0x04, 0x0B, 0, 0]);
        assert_eq!(
            encode_event(&Event::Failed { reason: FailReason::TooManyRetries, hint: Hint::Noisy, quality: Some(Quality { snr_db: -2, level_db: -40, clipped: false }) }),
            [3, 2, 3, 1, 0xFE, 0xD8, 0]
        );
        assert_eq!(encode_event(&Event::Received { id: 1, data: "ä".as_bytes().to_vec() }), [7, 1, 0, 2, 0, 0, 0, 0xC3, 0xA4]);
        assert_eq!(encode_event(&Event::RxAborted { reason: AbortReason::Cancelled }), [8, 1]);
        assert_eq!(encode_event(&Event::Log("ok".into())), [9, b'o', b'k']);
    }

    #[test]
    fn two_wasm_style_stations_talk_through_the_c_api() {
        // Sama polku kuin selaimessa: puskurit, sp_process, tapahtumien tavut.
        sp_new(48000.0, 1, 2, 0);
        let msg = b"wasm-rajapinnan testi";
        let ptr = sp_alloc(msg.len());
        unsafe {
            std::ptr::copy_nonoverlapping(msg.as_ptr(), ptr, msg.len());
            assert_eq!(sp_send(ptr, msg.len()), 0);
            assert_eq!(sp_send(ptr, msg.len()), 1);
            sp_free(ptr, msg.len());
        }
        assert!(sp_estimate(msg.len()) > 1.0 && sp_estimate(msg.len()) < 4.0);
        assert_eq!(sp_max_message(), 512);
        // Vastapuoli on tavallinen asema; ääni kulkee suoraan puskurista toiseen.
        let mut peer = Station::new(PhyKind::DEFAULT, 48000.0, LinkConfig::default(), 9);
        let mut to_peer = vec![0.0f32; 128];
        let mut from_peer = vec![0.0f32; 128];
        let mut got = None;
        let mut delivered = false;
        let mut buf = vec![0u8; 1024];
        for _ in 0..(48000 * 8 / 128) {
            unsafe {
                // Kumpikin kuulee toisen ja pienen kohinan (tyhjä hiljaisuus ei ole realistinen).
                let input = std::slice::from_raw_parts_mut(sp_input_ptr(), 128);
                for (i, v) in input.iter_mut().enumerate() {
                    *v = from_peer[i] * 0.2 + ((i * 7919) % 13) as f32 * 1e-5;
                }
                sp_process(128);
                to_peer.copy_from_slice(std::slice::from_raw_parts(sp_output_ptr(), 128));
                loop {
                    let n = sp_poll_event(buf.as_mut_ptr(), buf.len());
                    if n <= 0 {
                        break;
                    }
                    if buf[0] == 2 {
                        delivered = true;
                    }
                }
            }
            let input: Vec<f32> = to_peer.iter().enumerate().map(|(i, v)| v * 0.2 + ((i * 104729) % 11) as f32 * 1e-5).collect();
            peer.process(&input, &mut from_peer);
            while let Some(e) = peer.poll_event() {
                if let Event::Received { data, .. } = e {
                    got = Some(data);
                }
            }
        }
        assert_eq!(got.as_deref(), Some(&msg[..]));
        assert!(delivered);
        assert!(sp_level() >= 0.0 && sp_now_ms() > 7000.0);
    }
}
