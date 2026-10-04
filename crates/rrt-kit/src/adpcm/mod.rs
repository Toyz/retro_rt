//! The PlayStation's ADPCM sound formats, decoded as the hardware decodes
//! them, for a port's own sound engine to play at play time.
//!
//! - [`spu`]: PS-ADPCM, the SPU's sample format (VAG, VAB banks, PS2 SPU2
//!   data), and the SPU's 4-point Gaussian interpolation.
//! - [`xa`]: CD-XA ADPCM, the CD drive's streamed audio (movie sound,
//!   speech), resampled to 44100 Hz by the drive's zigzag filter.

pub mod spu;
pub mod xa;
