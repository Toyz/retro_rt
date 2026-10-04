//! The era's compression: LZSS and run-length encoding, configurable to the
//! variant a game used, with encoders for repacking and tests.
//!
//! - [`Lzss`]: a ring-buffer LZ77 with flag bytes. [`Lzss::OKUMURA`] is
//!   Haruhiko Okumura's 1989 `LZSS.C`, which a great many games copied
//!   (often with their own fill byte or start position - change the fields);
//!   [`Lzss::LZ10`] is Nintendo's LZ10 (GBA and DS BIOS, type 0x10), whose
//!   header [`lz10_decode`] and [`lz10_encode`] handle.
//! - [`packbits_decode`] / [`packbits_encode`]: Apple's PackBits, the RLE of
//!   TIFF, ILBM and many game image formats.
//! - [`rl_decode`] / [`rl_encode`]: Nintendo's RL (BIOS type 0x30).
//!
//! Every decoder checks its input and returns [`Error`] rather than
//! panicking on corrupt data.

mod lzss;
mod rle;

pub use lzss::{FlagOrder, Lzss, Token, lz10_decode, lz10_encode};
pub use rle::{packbits_decode, packbits_encode, rl_decode, rl_encode};

use std::fmt;

/// Corrupt or truncated compressed data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// The offset into the compressed data where decoding failed.
    pub at: usize,
    /// What was wrong.
    pub what: &'static str,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {:#x}", self.what, self.at)
    }
}

impl std::error::Error for Error {}
