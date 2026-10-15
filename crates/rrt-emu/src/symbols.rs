//! Names for addresses: the disassembler's symbol table ([`rrt_dis::Symbols`]),
//! so hooks, traces and listings share one file. See `rrt_dis::symbols` for
//! the format.

pub use rrt_dis::symbols::{Confidence, Kind, ParseError, Symbol, Symbols};
