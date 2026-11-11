//! Physical layer: bytes -> f32 samples and f32 samples -> bytes.
//!
//! Every waveform implements the [`Modulator`] and [`Demodulator`] traits.
//! Old waveforms are kept ([`PhyKind`]) so that a new version can be
//! compared with the old one using the same bench.
//!
//! The receiver is streaming: it is fed sample blocks as they
//! arrive, and it returns events ([`PhyEvent`]).

pub mod bfsk;
pub mod mfsk;

/// Reception quality data for one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RxStats {
    /// Start and end of the frame in the receiver's sample counter.
    pub start_sample: u64,
    pub end_sample: u64,
    /// Estimate of the signal-to-noise ratio in the modem's own band (dB).
    pub snr_db: f32,
    /// RMS of the input signal during the frame (full scale = 1.0).
    pub rms: f32,
    /// Share of samples that are clipped (|x| >= 0.99).
    pub clipped: f32,
    /// Number of symbols corrected by error correction.
    pub corrected: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// The header could not be decoded.
    Header,
    /// The checksum does not match (error correction was not enough).
    Checksum,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhyEvent {
    /// Preamble detected; frame reception started at sample `at`.
    Detected { at: u64 },
    /// Intact frame. `raw` is the encoded byte string before error correction
    /// (for measuring the bit error rate).
    Frame { payload: Vec<u8>, raw: Vec<u8>, stats: RxStats },
    /// A frame was detected but could not be decoded.
    Failed { reason: FailReason, raw: Vec<u8>, stats: RxStats },
}

pub trait Modulator {
    /// Largest payload in bytes in one frame.
    fn max_payload(&self) -> usize;
    /// Waveform of a frame. The sample rate was given at creation.
    fn modulate(&self, payload: &[u8]) -> Vec<f32>;
    /// The encoded byte string going to the channel (reference for `PhyEvent::raw`).
    fn coded(&self, payload: &[u8]) -> Vec<u8>;
    /// Frame duration in seconds.
    fn duration(&self, payload_len: usize) -> f64;
}

pub trait Demodulator {
    /// Feed the next sample block. Finished events are appended to `events`.
    fn push(&mut self, samples: &[f32], events: &mut Vec<PhyEvent>);
    /// Is a frame reception in progress?
    fn receiving(&self) -> bool;
    /// Forget the unfinished frame (e.g. after our own transmission).
    fn reset(&mut self);
    /// Number of samples fed so far.
    fn position(&self) -> u64;
}

/// Known waveforms. Every MFSK parameter set is its own waveform
/// with a name: experimental versions stay comparable and corpus recordings
/// know what they were made with.
#[derive(Debug, Clone, Copy)]
pub enum PhyKind {
    /// Crude first version: 2-FSK, 100 baud, no error correction.
    V0Bfsk,
    /// Multitone MFSK with tone hopping (see [`mfsk`]).
    Mfsk(&'static mfsk::MfskParams),
}

impl PartialEq for PhyKind {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

/// MFSK parameter sets. The first is the default.
pub static MFSK_PRESETS: &[mfsk::MfskParams] = &[
    mfsk::BASE,
    // Experimental versions (EXPERIMENTS.md entry 2). The name tells what differs from the base version.
    mfsk::MfskParams { name: "v1-8x20", k: 8, t_win: 0.020, t_guard: 0.005, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-noera", erasure_ratio: 0.0, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-time", layout: mfsk::ByteLayout::Time, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-p3", parity_num: 1, parity_den: 3, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-g2", g: 2, ..mfsk::BASE },
    // Thresholds of the preamble shape check (EXPERIMENTS.md entry 5).
    mfsk::MfskParams { name: "v1-cf0", chirp_fraction: 0.0, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-cf03", chirp_fraction: 0.03, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-cf06", chirp_fraction: 0.06, ..mfsk::BASE },
    mfsk::MfskParams { name: "v1-cf12", chirp_fraction: 0.12, ..mfsk::BASE },
];

impl PhyKind {
    /// Default waveform: the MFSK base version.
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

/// Number of bit errors between two byte strings. Bits of the length difference count
/// as errors by half (a guess is right on every other bit).
pub fn bit_errors(sent: &[u8], received: &[u8]) -> (u64, u64) {
    let common = sent.len().min(received.len());
    let mut errors: u64 = sent[..common].iter().zip(&received[..common]).map(|(a, b)| (a ^ b).count_ones() as u64).sum();
    let extra = (sent.len() - common) as u64 * 8;
    errors += extra / 2;
    (errors, sent.len() as u64 * 8)
}
