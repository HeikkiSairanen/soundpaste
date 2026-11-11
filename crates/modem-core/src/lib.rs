//! Core of the soundpaste modem.
//!
//! Layers from the bottom up:
//! - [`dsp`]: FFT, frequency detectors, convolution, sample rate conversion
//! - [`phy`]: physical layer, bytes <-> f32 samples
//! - [`sim`]: simulated acoustic channel for tests and measurements
//!
//! The core has no external dependencies and knows nothing about the browser.

pub mod crc;
pub mod dsp;
pub mod rng;
pub mod wav;
pub mod phy;
pub mod sim;
pub mod measure;
pub mod rs;
pub mod link;
pub mod station;
