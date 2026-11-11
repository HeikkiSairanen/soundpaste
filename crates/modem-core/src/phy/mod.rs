//! Fyysinen kerros: tavut -> f32-näytteet ja f32-näytteet -> tavut.
//!
//! Jokainen aaltomuoto toteuttaa [`Modulator`]- ja [`Demodulator`]-rajapinnat.
//! Vanhat aaltomuodot pidetään mukana ([`PhyKind`]), jotta uutta versiota voi
//! verrata vanhaan samalla mittaristolla.
//!
//! Vastaanotin on virtaava: sille syötetään näytelohkoja sitä mukaa kuin niitä
//! tulee, ja se palauttaa tapahtumia ([`PhyEvent`]).

pub mod bfsk;
pub mod mfsk;

/// Vastaanoton laatutiedot yhdestä kehyksestä.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RxStats {
    /// Kehyksen alku ja loppu vastaanottimen näytelaskurissa.
    pub start_sample: u64,
    pub end_sample: u64,
    /// Arvio signaali-kohinasuhteesta modeemin omalla kaistalla (dB).
    pub snr_db: f32,
    /// Tulosignaalin tehollisarvo kehyksen aikana (täysi asteikko = 1.0).
    pub rms: f32,
    /// Osuus näytteistä, jotka ovat leikkautuneet (|x| >= 0,99).
    pub clipped: f32,
    /// Virheenkorjauksen korjaamien symbolien määrä.
    pub corrected: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// Otsake ei ollut tulkittavissa.
    Header,
    /// Tarkistussumma ei täsmää (virheenkorjaus ei riittänyt).
    Checksum,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhyEvent {
    /// Alkumerkki havaittu; kehyksen vastaanotto alkoi näytteestä `at`.
    Detected { at: u64 },
    /// Ehjä kehys. `raw` on koodattu tavujono ennen virheenkorjausta
    /// (bittivirhesuhteen mittaamista varten).
    Frame { payload: Vec<u8>, raw: Vec<u8>, stats: RxStats },
    /// Kehys havaittiin, mutta sitä ei saatu purettua.
    Failed { reason: FailReason, raw: Vec<u8>, stats: RxStats },
}

pub trait Modulator {
    /// Suurin hyötykuorma tavuina yhdessä kehyksessä.
    fn max_payload(&self) -> usize;
    /// Kehyksen aaltomuoto. Näytteenottotaajuus annettiin luotaessa.
    fn modulate(&self, payload: &[u8]) -> Vec<f32>;
    /// Kanavalle lähtevä koodattu tavujono (vertailukohta `PhyEvent::raw`:lle).
    fn coded(&self, payload: &[u8]) -> Vec<u8>;
    /// Kehyksen kesto sekunteina.
    fn duration(&self, payload_len: usize) -> f64;
}

pub trait Demodulator {
    /// Syötä seuraava näytelohko. Valmistuneet tapahtumat lisätään `events`-vektoriin.
    fn push(&mut self, samples: &[f32], events: &mut Vec<PhyEvent>);
    /// Onko kehyksen vastaanotto kesken?
    fn receiving(&self) -> bool;
    /// Unohda keskeneräinen kehys (esim. oman lähetyksen jälkeen).
    fn reset(&mut self);
    /// Tähän mennessä syötettyjen näytteiden määrä.
    fn position(&self) -> u64;
}

/// Tunnetut aaltomuodot. Jokainen MFSK-parametrijoukko on oma aaltomuotonsa,
/// jolla on nimi: kokeiluversiot pysyvät vertailtavina ja korpuksen äänitteet
/// tietävät, millä ne on tehty.
#[derive(Debug, Clone, Copy)]
pub enum PhyKind {
    /// Karkea ensimmäinen versio: 2-FSK, 100 baudia, ei virheenkorjausta.
    V0Bfsk,
    /// Moniääni-MFSK sävelhypyllä (ks. [`mfsk`]).
    Mfsk(&'static mfsk::MfskParams),
}

impl PartialEq for PhyKind {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

/// MFSK-parametrijoukot. Ensimmäinen on oletus.
pub static MFSK_PRESETS: &[mfsk::MfskParams] = &[
    mfsk::BASE,
    // Kokeiluversiot (EXPERIMENTS.md kohta 2). Nimi kertoo, mikä poikkeaa perusversiosta.
    mfsk::MfskParams { name: "v1-8x20", k: 8, t_win: 0.020, t_guard: 0.005, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-noera", erasure_ratio: 0.0, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-time", layout: mfsk::ByteLayout::Time, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-p3", parity_num: 1, parity_den: 3, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-g2", g: 2, ..mfsk::BASE },
    // Alkumerkin muototarkistuksen kynnykset (EXPERIMENTS.md kohta 5).
    mfsk::MfskParams { name: "v1-cf0", chirp_fraction: 0.0, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-cf03", chirp_fraction: 0.03, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-cf06", chirp_fraction: 0.06, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-cf12", chirp_fraction: 0.12, ..mfsk::BASE },
];

impl PhyKind {
    /// Oletusaaltomuoto: MFSK-perusversio.
    pub const DEFAULT: PhyKind = PhyKind::Mfsk(&mfsk::BASE);

    pub fn all() -> Vec<PhyKind> {
        let mut v = vec![PhyKind::V0Bfsk];
        v.extend(MFSK_PRESETS.iter().map(PhyKind::Mfsk));
        v
    }

    pub fn name(self) -> &'static str {
        match self {
            PhyKind::V0Bfsk => "v0-bfsk",
            PhyKind::Mfsk(p) => p.name,
        }
    }

    pub fn from_name(s: &str) -> Option<PhyKind> {
        Self::all().into_iter().find(|k| k.name() == s)
    }

    pub fn modulator(self, fs: f64) -> Box<dyn Modulator> {
        match self {
            PhyKind::V0Bfsk => Box::new(bfsk::BfskModulator::new(fs)),
            PhyKind::Mfsk(p) => Box::new(mfsk::MfskModulator::new(p, fs)),
        }
    }

    pub fn demodulator(self, fs: f64) -> Box<dyn Demodulator> {
        match self {
            PhyKind::V0Bfsk => Box::new(bfsk::BfskDemodulator::new(fs)),
            PhyKind::Mfsk(p) => Box::new(mfsk::MfskDemodulator::new(p, fs)),
        }
    }
}

/// Bittivirheiden määrä kahden tavujonon välillä. Pituuseron bitit lasketaan
/// virheiksi puoliksi (arvaus osuu oikeaan joka toisella bitillä).
pub fn bit_errors(sent: &[u8], received: &[u8]) -> (u64, u64) {
    let common = sent.len().min(received.len());
    let mut errors: u64 = sent[..common].iter().zip(&received[..common]).map(|(a, b)| (a ^ b).count_ones() as u64).sum();
    let extra = (sent.len() - common) as u64 * 8;
    errors += extra / 2;
    (errors, sent.len() as u64 * 8)
}
