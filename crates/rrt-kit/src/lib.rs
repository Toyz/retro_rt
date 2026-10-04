//! The small things every port rewrites.
//!
//! - Recycling, so a frame allocates nothing once warm: [`Staging`] packs
//!   values into a byte buffer kept from frame to frame (vertices for the
//!   GPU), [`Pool`] lends out `Vec`s and takes them back, and [`SyncPool`]
//!   does the same across threads.
//! - Containers: [`Slab`], a generational arena whose [`Handle`]s go stale
//!   when their slot is reused (objects, entities, sounds); [`Ring`], a
//!   fixed-capacity history that overwrites its oldest (frame times, input
//!   logs, rewind).
//! - Randomness a port can reproduce: [`rng`]'s [`Generator`] trait, the
//!   era's generators (C and MSVC `rand`, newlib, xorshift, LFSRs, tables),
//!   the reductions games applied ([`RngExt`]), and wrappers that count,
//!   record or force values.
//! - Data as the consoles hold it: [`Fixed`] fixed-point numbers ([`Q12`],
//!   the PS1 GTE's 4.12 and 20.12), [`Reader`] for little- and big-endian file
//!   formats, [`bcd`], and the era's colour formats behind one [`Pixel`]
//!   trait ([`color`]: RGB555 to RGB5A3, Mega Drive CRAM to PSMCT32, YCbCr,
//!   palettes).
//!
//! No dependencies, no I/O, nothing console-specific beyond these shared
//! encodings. See `docs/crates/rrt-kit.md`.

pub mod bcd;
pub mod bytes;
pub mod color;
pub mod fixed;
pub mod pool;
pub mod ring;
pub mod rng;
pub mod slab;
pub mod staging;

pub use bytes::{Eof, Reader};
pub use color::{
    Abgr4444, Argb1555, Argb4444, Bgr565, Ia8, Ia16, Md333, Nibbles, Pixel, PsmCt32, Range, Rgb5a3, Rgb332, Rgb555,
    Rgb565, Rgba5551, Sms222,
};
pub use fixed::{Fixed, Q12};
pub use pool::{Pool, Pooled, SyncPool};
pub use ring::Ring;
pub use rng::{Generator, RngExt};
pub use slab::{Handle, Slab};
pub use staging::{Pack, Staging};
