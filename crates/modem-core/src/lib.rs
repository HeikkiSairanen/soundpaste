//! soundpaste-modeemin ydin.
//!
//! Kerrokset alhaalta ylös:
//! - [`dsp`]: FFT, taajuusilmaisimet, konvoluutio, näytteenottotaajuuden muunnos
//! - [`phy`]: fyysinen kerros, tavut <-> f32-näytteet
//! - [`sim`]: simuloitu akustinen kanava testejä ja mittauksia varten
//!
//! Ytimellä ei ole ulkoisia riippuvuuksia eikä se tiedä selaimesta mitään.

pub mod crc;
pub mod dsp;
pub mod rng;
pub mod wav;
pub mod phy;
pub mod sim;
pub mod measure;
