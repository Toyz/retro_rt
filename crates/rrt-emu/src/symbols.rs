//! Names for addresses: what HLE hooks by, what traces print.

use std::collections::{BTreeMap, HashMap};

/// A symbol table: names to addresses and back. Filled from an ELF's symbol
/// table ([`crate::load::Elf::symbols`]) or a text map ([`Symbols::parse`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbols {
    by_name: HashMap<String, u32>,
    by_addr: BTreeMap<u32, String>,
}

impl Symbols {
    /// An empty table.
    pub fn new() -> Symbols {
        Symbols::default()
    }

    /// Names `addr`. A name given twice keeps the later address.
    pub fn insert(&mut self, name: impl Into<String>, addr: u32) {
        let name = name.into();
        self.by_addr.insert(addr, name.clone());
        self.by_name.insert(name, addr);
    }

    /// The address named `name`.
    pub fn addr(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    /// The name at exactly `addr`.
    pub fn name(&self, addr: u32) -> Option<&str> {
        self.by_addr.get(&addr).map(String::as_str)
    }

    /// The nearest name at or below `addr`, and how far past it `addr` is:
    /// `func+0x1c` in a trace.
    pub fn locate(&self, addr: u32) -> Option<(&str, u32)> {
        self.by_addr.range(..=addr).next_back().map(|(a, n)| (n.as_str(), addr - a))
    }

    /// Symbols held.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// No symbols.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// A text map: one symbol a line, `ADDRESS NAME` (hex, with or without
    /// `0x`), whitespace separated; anything after the name, blank lines and
    /// lines starting with `#` are ignored. Errors name the line.
    pub fn parse(text: &str) -> Result<Symbols, String> {
        let mut s = Symbols::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let (Some(addr), Some(name)) = (words.next(), words.next()) else {
                return Err(format!("line {}: want ADDRESS NAME", n + 1));
            };
            let hex = addr.trim_start_matches("0x").trim_start_matches("0X");
            let addr = u32::from_str_radix(hex, 16).map_err(|_| format!("line {}: {addr:?} is not hex", n + 1))?;
            s.insert(name, addr);
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_map_parses_and_locates() {
        let s = Symbols::parse("# names\n0x80010000 main\n80010a5c  game_init  extra\n\n").unwrap();
        assert_eq!((s.addr("main"), s.addr("game_init"), s.len()), (Some(0x8001_0000), Some(0x8001_0a5c), 2));
        assert_eq!(s.name(0x8001_0000), Some("main"));
        assert_eq!(s.locate(0x8001_0a60), Some(("game_init", 4)));
        assert_eq!(s.locate(0x10), None);
        assert!(Symbols::parse("zz main").unwrap_err().contains("line 1"));
        assert!(Symbols::parse("1234").is_err());
    }
}
