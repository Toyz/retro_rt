//! Sony's libsnd as PS1 games use it: VAB sound banks, SEP/SEQ songs, and
//! a software SPU that plays them the way libsnd drives the hardware.
//!
//! - [`vab`]: the `.VH` bank header (programs, tones) and where each
//!   sample sits in the `.VB` body.
//! - [`sep`]: SEP song packs (several SEQ songs back to back).
//! - [`Bank`]: a VH and its VB, samples decoded ([`crate::adpcm::spu`]).
//! - [`Spu`]: 24 voices with the SPU's ADSR envelope and Gaussian
//!   interpolation, libsnd's SEQ sequencer in song slots (tempo, running
//!   status, the NRPN loop marks, volume, pan, expression, pitch bend),
//!   slot volumes and fades (`SsSeqSetVol`, `SsSeqSetCrescendo`,
//!   `SsSeqSetDecrescendo`), and `SsUtKeyOn`-style effects.
//!
//! What a game adds stays in the game: which files form its song sets, its
//! effect table, the voices its effects take, when songs switch. Moved
//! from the Rugrats port (rugrats' `audio::sound` and `rugrats_data::vab`
//! / `sep`).

pub mod sep;
mod synth;
pub mod vab;

pub use synth::{Bank, MAIN_VOLUME, RATE, Spu, VOICES};
