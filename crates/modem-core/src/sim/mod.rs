//! Simulated acoustic channel.
//!
//! Model of one direction (speaker -> air -> microphone):
//!
//! ```text
//! transmitter (fs_tx) -> sample rate conversion (fs_rx, clock error)
//!                 -> room impulse response (echo) -> delay -> gain
//! microphone = Σ paths + background noise + interference -> clipping
//! ```
//!
//! All parts are streaming so that two modems can talk through the
//! simulator block by block. Randomness comes from the given
//! seed, so runs are repeatable.

pub mod channel;
pub mod duplex;
pub mod ir;
pub mod noise;

pub use channel::{run_oneway, OneWay, Path, PathConfig};
pub use duplex::{Duplex, DuplexConfig, Side};
pub use ir::synth_ir;
pub use noise::{Interference, InterferenceKind, NoiseKind, NoiseSource};
