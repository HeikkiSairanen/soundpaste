//! Simuloitu akustinen kanava.
//!
//! Malli yhdestä suunnasta (kaiutin -> ilma -> mikrofoni):
//!
//! ```text
//! lähetin (fs_tx) -> näytteenottotaajuuden muunnos (fs_rx, kellovirhe)
//!                 -> huoneen impulssivaste (kaiku) -> viive -> vahvistus
//! mikrofoni = Σ polut + taustakohina + häiriöt -> leikkautuminen
//! ```
//!
//! Kaikki osat ovat virtaavia, jotta kaksi modeemia voi keskustella
//! simulaattorin läpi lohko kerrallaan. Satunnaisuus tulee annetusta
//! siemenestä, joten ajot ovat toistettavia.

pub mod channel;
pub mod duplex;
pub mod ir;
pub mod noise;

pub use channel::{run_oneway, OneWay, Path, PathConfig};
pub use duplex::{Duplex, DuplexConfig, Side};
pub use ir::synth_ir;
pub use noise::{Interference, InterferenceKind, NoiseKind, NoiseSource};
