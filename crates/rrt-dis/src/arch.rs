//! The backends this crate has, by name: what the `rrt-dis` command's
//! `--arch` and an executable's [`Loaded::arch`](crate::exe::Loaded::arch)
//! name.
//!
//! A program that knows its CPU constructs the backend directly
//! ([`Mips::r3000()`]); this registry is for tools that pick one at run
//! time. A new backend is an [`Arch`] implementation (see
//! `docs/guides/new-backend.md`) plus one [`Backend`] entry here.

use crate::insn::Arch;
use crate::mips::Mips;

/// A backend a tool can pick by name.
#[derive(Clone, Copy, Debug)]
pub struct Backend {
    /// Its name: `r3000`.
    pub name: &'static str,
    /// Other names it answers to: `ps1`, `psx`.
    pub aliases: &'static [&'static str],
    /// What it is, for a listing.
    pub about: &'static str,
    /// A new instance.
    pub make: fn() -> Box<dyn Arch>,
}

/// Every backend, in the order a listing shows them.
pub const BACKENDS: &[Backend] = &[
    Backend {
        name: "r3000",
        aliases: &["ps1", "psx", "r3000a"],
        about: "PlayStation: MIPS R3000A with the GTE",
        make: || Box::new(Mips::r3000()),
    },
    Backend {
        name: "r5900",
        aliases: &["ps2", "ee"],
        about: "PlayStation 2 Emotion Engine: MIPS R5900 with MMI, COP1 and VU0 macro mode",
        make: || Box::new(Mips::r5900()),
    },
];

/// The backend called `name` (or one of its aliases), ignoring case.
pub fn by_name(name: &str) -> Option<&'static Backend> {
    let name = name.to_ascii_lowercase();
    BACKENDS.iter().find(|b| b.name == name || b.aliases.contains(&name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backends_answer_to_their_names_and_aliases() {
        assert_eq!((by_name("r3000").unwrap().make)().name(), "R3000A");
        assert_eq!((by_name("PS2").unwrap().make)().name(), "R5900");
        assert!(by_name("z80").is_none());
        for b in BACKENDS {
            assert_eq!(by_name(b.name).map(|x| x.name), Some(b.name));
        }
    }
}
