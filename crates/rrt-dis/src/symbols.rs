//! Names for addresses, as a reverse engineering project keeps them: a text
//! file committed with the project, edited by hand and by tools.
//!
//! One symbol a line:
//!
//! ```text
//! # Names for CCCPSX.EXE.                       comment lines are kept
//! 0x8009fc10 _start          # (sure) PsyQ crt0: clears bss, sets gp and sp
//! 0x800a2d98 strcmp func size=0x48 # (likely) compares world names
//! 0x800c5bf4 track_names data      # (sure) char*[4][3]
//! ```
//!
//! An address (hex, `0x` optional) and a name; then, optionally, the kind
//! (`func`, `data`, `label`) and `size=N` (hex with `0x`, else decimal); then
//! `#` and a note, which may begin with how sure the name is - `(sure)`,
//! `(likely)` or `(guess)`. This is hwtr's `symbols/cccpsx.txt` format, with
//! the kind and size added.
//!
//! [`Symbols::to_text`] writes the file back with every line nobody changed
//! exactly as it was read - comments, blank lines, alignment - so a tool can
//! mark symbols in a file people also edit. A changed symbol is rewritten in
//! place; a new one is added at the end.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// What a symbol names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Not said.
    #[default]
    Unknown,
    /// A function's entry.
    Func,
    /// Data: a variable, a table, a string.
    Data,
    /// A place inside a function: a loop, a case, a jump target.
    Label,
}

impl Kind {
    /// The word the symbol file writes, None for [`Kind::Unknown`].
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Kind::Unknown => None,
            Kind::Func => Some("func"),
            Kind::Data => Some("data"),
            Kind::Label => Some("label"),
        }
    }
}

impl std::str::FromStr for Kind {
    type Err = String;

    /// `func`, `data`, `label`, or `unknown`.
    fn from_str(s: &str) -> Result<Kind, String> {
        match s {
            "func" => Ok(Kind::Func),
            "data" => Ok(Kind::Data),
            "label" => Ok(Kind::Label),
            "unknown" => Ok(Kind::Unknown),
            _ => Err(format!("{s:?} is not a kind: want func, data or label")),
        }
    }
}

/// How sure a name is, as the project's convention grades it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Confidence {
    /// A guess to be attacked.
    Guess,
    /// From strings, shape or context, not traced.
    Likely,
    /// Traced: the code proves it.
    Sure,
}

impl Confidence {
    /// The word the symbol file writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Guess => "guess",
            Confidence::Likely => "likely",
            Confidence::Sure => "sure",
        }
    }
}

impl std::str::FromStr for Confidence {
    type Err = String;

    /// `guess`, `likely` or `sure`.
    fn from_str(s: &str) -> Result<Confidence, String> {
        match s {
            "guess" => Ok(Confidence::Guess),
            "likely" => Ok(Confidence::Likely),
            "sure" => Ok(Confidence::Sure),
            _ => Err(format!("{s:?} is not a confidence: want guess, likely or sure")),
        }
    }
}

/// One named address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// The name.
    pub name: String,
    /// The address.
    pub addr: u32,
    /// What it names.
    pub kind: Kind,
    /// Its size in bytes, when known.
    pub size: Option<u32>,
    /// How sure the name is, when said.
    pub confidence: Option<Confidence>,
    /// The note after the confidence, empty when none.
    pub note: String,
}

impl Symbol {
    /// A symbol of unknown kind, size and confidence, with no note.
    pub fn new(name: impl Into<String>, addr: u32) -> Symbol {
        Symbol { name: name.into(), addr, kind: Kind::Unknown, size: None, confidence: None, note: String::new() }
    }

    /// The symbol as a line of the file.
    pub fn to_line(&self) -> String {
        let mut s = format!("0x{:08x} {}", self.addr, self.name);
        if let Some(k) = self.kind.as_str() {
            s.push(' ');
            s.push_str(k);
        }
        if let Some(n) = self.size {
            s.push_str(&format!(" size=0x{n:x}"));
        }
        let mut comment = String::new();
        if let Some(c) = self.confidence {
            comment.push_str(&format!("({})", c.as_str()));
        }
        if !self.note.is_empty() {
            if !comment.is_empty() {
                comment.push(' ');
            }
            comment.push_str(&self.note);
        }
        if !comment.is_empty() {
            s.push_str(" # ");
            s.push_str(&comment);
        }
        s
    }

    /// Whether `addr` falls inside the symbol (its size known).
    pub fn contains(&self, addr: u32) -> bool {
        self.size.is_some_and(|n| addr >= self.addr && addr - self.addr < n)
    }
}

/// A line of the file as read: kept verbatim until its symbol changes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// A comment or blank line.
    Text(String),
    /// A symbol's line: its address, and the text as read (None once the
    /// symbol changed, so it is rewritten).
    Entry(u32, Option<String>),
}

/// A line that did not parse: its number (from 1) and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The line, from 1.
    pub line: usize,
    /// What was wrong.
    pub what: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.what)
    }
}

impl std::error::Error for ParseError {}

/// A symbol table: names to addresses and back, in a file's layout. Two
/// tables are equal when they hold the same symbols, however their files
/// are laid out.
#[derive(Clone, Debug, Default)]
pub struct Symbols {
    by_addr: BTreeMap<u32, Symbol>,
    by_name: HashMap<String, u32>,
    lines: Vec<Line>,
    /// `@data START END` ranges: data inside the code.
    data: Vec<(u32, u32)>,
}

/// One line's symbol, or None for a comment or blank line.
fn parse_line(text: &str) -> Result<Option<Symbol>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }
    let (fields, comment) = match trimmed.split_once('#') {
        Some((f, c)) => (f, c.trim()),
        None => (trimmed, ""),
    };
    let mut words = fields.split_whitespace();
    let (Some(addr), Some(name)) = (words.next(), words.next()) else {
        return Err("want ADDRESS NAME".into());
    };
    let addr = parse_addr(addr).ok_or_else(|| format!("{addr:?} is not a hex address"))?;
    let mut sym = Symbol::new(name, addr);
    for w in words {
        match w {
            "func" => sym.kind = Kind::Func,
            "data" => sym.kind = Kind::Data,
            "label" => sym.kind = Kind::Label,
            _ => {
                let Some(n) = w.strip_prefix("size=") else {
                    return Err(format!("{w:?} is not a kind (func, data, label) or size=N"));
                };
                let size = match n.strip_prefix("0x") {
                    Some(h) => u32::from_str_radix(h, 16),
                    None => n.parse(),
                };
                sym.size = Some(size.map_err(|_| format!("{w:?}: the size is not a number"))?);
            }
        }
    }
    let mut note = comment;
    for c in [Confidence::Sure, Confidence::Likely, Confidence::Guess] {
        if let Some(rest) = note.strip_prefix(&format!("({})", c.as_str())) {
            sym.confidence = Some(c);
            note = rest.trim_start();
            break;
        }
    }
    sym.note = note.to_string();
    Ok(Some(sym))
}

/// A hex address, with or without `0x`.
fn parse_addr(t: &str) -> Option<u32> {
    u32::from_str_radix(t.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

impl PartialEq for Symbols {
    fn eq(&self, other: &Symbols) -> bool {
        self.by_addr == other.by_addr && self.data == other.data
    }
}

impl Eq for Symbols {}

impl Symbols {
    /// An empty table.
    pub fn new() -> Symbols {
        Symbols::default()
    }

    /// Reads a symbol file. A name or an address given twice is an error
    /// naming both lines' first.
    pub fn parse(text: &str) -> Result<Symbols, ParseError> {
        let mut s = Symbols::new();
        for (i, line) in text.lines().enumerate() {
            let bad = |what: String| ParseError { line: i + 1, what };
            if let Some(directive) = line.trim_start().strip_prefix('@') {
                let fields = directive.split('#').next().unwrap_or("");
                let mut w = fields.split_whitespace();
                match w.next() {
                    Some("data") => {
                        let mut addr = |what: &str| -> Result<u32, ParseError> {
                            let t = w.next().ok_or_else(|| bad(format!("@data needs {what}")))?;
                            parse_addr(t).ok_or_else(|| bad(format!("{t:?} is not a hex address")))
                        };
                        let (start, end) = (addr("a start")?, addr("an end")?);
                        if end <= start {
                            return Err(bad("@data's end is not after its start".into()));
                        }
                        s.data.push((start, end));
                    }
                    other => return Err(bad(format!("unknown directive @{}", other.unwrap_or("")))),
                }
                s.lines.push(Line::Text(line.to_string()));
                continue;
            }
            match parse_line(line).map_err(bad)? {
                None => s.lines.push(Line::Text(line.to_string())),
                Some(sym) => {
                    if let Some(at) = s.by_name.get(&sym.name) {
                        return Err(bad(format!("{} is already 0x{at:08x}", sym.name)));
                    }
                    if let Some(other) = s.by_addr.get(&sym.addr) {
                        return Err(bad(format!("0x{:08x} is already {}", sym.addr, other.name)));
                    }
                    s.lines.push(Line::Entry(sym.addr, Some(line.to_string())));
                    s.by_name.insert(sym.name.clone(), sym.addr);
                    s.by_addr.insert(sym.addr, sym);
                }
            }
        }
        Ok(s)
    }

    /// The `@data START END` ranges: data inside the code (rodata between
    /// hand-written assembly and the compiler's output, a table in a text
    /// section), which the analysis keeps out of its functions.
    pub fn data_ranges(&self) -> &[(u32, u32)] {
        &self.data
    }

    /// The file: untouched lines as read, changed symbols rewritten in place,
    /// new ones at the end. Ends with a newline when not empty.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Text(t) => out.push_str(t),
                Line::Entry(addr, Some(t)) if self.by_addr.contains_key(addr) => out.push_str(t),
                Line::Entry(addr, _) => match self.by_addr.get(addr) {
                    Some(sym) => out.push_str(&sym.to_line()),
                    None => continue,
                },
            }
            out.push('\n');
        }
        out
    }

    /// Adds `sym`, or replaces the symbol at its address (and drops any other
    /// address its name had): what a tool marking a name does.
    pub fn mark(&mut self, sym: Symbol) {
        if let Some(old) = self.by_name.get(&sym.name).copied().filter(|a| *a != sym.addr) {
            self.remove_addr(old);
        }
        if let Some(prev) = self.by_addr.get(&sym.addr) {
            self.by_name.remove(&prev.name);
            for line in &mut self.lines {
                if *line == Line::Entry(sym.addr, None) || matches!(line, Line::Entry(a, Some(_)) if *a == sym.addr) {
                    *line = Line::Entry(sym.addr, None);
                }
            }
        } else {
            self.lines.push(Line::Entry(sym.addr, None));
        }
        self.by_name.insert(sym.name.clone(), sym.addr);
        self.by_addr.insert(sym.addr, sym);
    }

    /// Names `addr` (kind, size and confidence unknown): [`Symbols::mark`]
    /// with a plain [`Symbol`].
    pub fn insert(&mut self, name: impl Into<String>, addr: u32) {
        self.mark(Symbol::new(name, addr));
    }

    fn remove_addr(&mut self, addr: u32) -> Option<Symbol> {
        let sym = self.by_addr.remove(&addr)?;
        self.by_name.remove(&sym.name);
        self.lines.retain(|l| !matches!(l, Line::Entry(a, _) if *a == addr));
        Some(sym)
    }

    /// Removes the symbol named `name`.
    pub fn remove(&mut self, name: &str) -> Option<Symbol> {
        let addr = *self.by_name.get(name)?;
        self.remove_addr(addr)
    }

    /// The symbol named `name`.
    pub fn get(&self, name: &str) -> Option<&Symbol> {
        self.by_name.get(name).and_then(|a| self.by_addr.get(a))
    }

    /// The symbol at exactly `addr`.
    pub fn at(&self, addr: u32) -> Option<&Symbol> {
        self.by_addr.get(&addr)
    }

    /// The address named `name`.
    pub fn addr(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    /// The name at exactly `addr`.
    pub fn name(&self, addr: u32) -> Option<&str> {
        self.by_addr.get(&addr).map(|s| s.name.as_str())
    }

    /// The symbols with addresses in `range`, in address order (reversible:
    /// `range(..=addr).rev()` walks back from `addr`).
    pub fn range(&self, range: impl std::ops::RangeBounds<u32>) -> impl DoubleEndedIterator<Item = &Symbol> {
        self.by_addr.range(range).map(|(_, s)| s)
    }

    /// The nearest symbol at or below `addr`, and how far past it `addr` is:
    /// `func+0x1c` in a trace.
    pub fn locate(&self, addr: u32) -> Option<(&str, u32)> {
        self.by_addr.range(..=addr).next_back().map(|(a, s)| (s.name.as_str(), addr - a))
    }

    /// `addr` as a name: `name` at a symbol, `name+0x1c` inside one whose
    /// size covers it (or within `slack` bytes past one of unknown size),
    /// else None.
    pub fn describe(&self, addr: u32, slack: u32) -> Option<String> {
        let (_, sym) = self.by_addr.range(..=addr).next_back()?;
        let off = addr - sym.addr;
        let inside = match sym.size {
            Some(n) => off < n,
            None => off <= slack,
        };
        match (inside, off) {
            (_, 0) => Some(sym.name.clone()),
            (true, off) => Some(format!("{}+0x{off:x}", sym.name)),
            (false, _) => None,
        }
    }

    /// Every symbol, in address order.
    pub fn iter(&self) -> impl Iterator<Item = &Symbol> {
        self.by_addr.values()
    }

    /// Symbols held.
    pub fn len(&self) -> usize {
        self.by_addr.len()
    }

    /// No symbols.
    pub fn is_empty(&self) -> bool {
        self.by_addr.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines from hwtr's `symbols/cccpsx.txt`, as that project writes them.
    const HWTR: &str = "# Names for CCCPSX.EXE (image 0x80010000, $gp 0x800d0b48).
# One `0xADDRESS name` per line. Read by hwtr-re disasm/funcs/strings.

0x8009fc10 _start                  # (sure) PsyQ crt0: clears bss, sets gp and sp, InitHeap, calls main
0x80010a5c main                    # (sure) called by _start
0x800a2d98 strcmp                  # (likely) compares world names in world_colour

# Data
0x800c5bf4 track_archive_names     # (sure) char*[4][3]; Haunted 1 slot is NULL
";

    #[test]
    fn range_walks_back_from_an_address() {
        let s = Symbols::parse("0x10 a\n0x20 b\n0x30 c\n").unwrap();
        let back: Vec<&str> = s.range(..=0x2c).rev().map(|s| s.name.as_str()).collect();
        assert_eq!(back, ["b", "a"]);
        assert_eq!(s.range(0x20..).count(), 2);
    }

    #[test]
    fn hwtrs_file_reads_and_writes_back_byte_for_byte() {
        let s = Symbols::parse(HWTR).unwrap();
        assert_eq!(s.len(), 4);
        let start = s.get("_start").unwrap();
        assert_eq!((start.addr, start.confidence), (0x8009_fc10, Some(Confidence::Sure)));
        assert_eq!(start.note, "PsyQ crt0: clears bss, sets gp and sp, InitHeap, calls main");
        assert_eq!(s.get("strcmp").unwrap().confidence, Some(Confidence::Likely));
        assert_eq!(s.to_text(), HWTR);
    }

    #[test]
    fn marking_rewrites_one_line_in_place_and_adds_new_ones_at_the_end() {
        let mut s = Symbols::parse(HWTR).unwrap();
        let mut strcmp = s.get("strcmp").unwrap().clone();
        strcmp.kind = Kind::Func;
        strcmp.size = Some(0x48);
        strcmp.confidence = Some(Confidence::Sure);
        s.mark(strcmp);
        s.mark(Symbol { kind: Kind::Data, ..Symbol::new("game_state", 0x800d_0000) });
        let text = s.to_text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[5], "0x800a2d98 strcmp func size=0x48 # (sure) compares world names in world_colour");
        assert_eq!(lines[4], "0x80010a5c main                    # (sure) called by _start", "neighbours untouched");
        assert_eq!(*lines.last().unwrap(), "0x800d0000 game_state data");
        let again = Symbols::parse(&text).unwrap();
        assert_eq!(again, s, "what was written reads back the same");
        assert_eq!(again.to_text(), text, "and writes out the same");
    }

    #[test]
    fn renaming_and_removing_keep_both_lookups_straight() {
        let mut s = Symbols::new();
        s.insert("a", 0x100);
        s.insert("b", 0x100);
        assert_eq!((s.name(0x100), s.addr("a"), s.len()), (Some("b"), None, 1), "a new name for an address");
        s.insert("b", 0x200);
        assert_eq!((s.name(0x100), s.addr("b")), (None, Some(0x200)), "a name moved");
        assert_eq!(s.remove("b").map(|x| x.addr), Some(0x200));
        assert!(s.is_empty() && s.to_text().is_empty());
    }

    #[test]
    fn locating_and_describing_addresses() {
        let s = Symbols::parse("0x1000 f func size=0x20\n0x2000 g\n").unwrap();
        assert_eq!(s.locate(0x1004), Some(("f", 4)));
        assert_eq!(s.describe(0x1000, 0).as_deref(), Some("f"));
        assert_eq!(s.describe(0x101c, 0).as_deref(), Some("f+0x1c"));
        assert_eq!(s.describe(0x1020, 0x100), None, "past f's size");
        assert_eq!(s.describe(0x2010, 0x100).as_deref(), Some("g+0x10"), "unknown size: within the slack");
        assert_eq!(s.describe(0x2200, 0x100), None);
        assert!(s.at(0x1000).unwrap().contains(0x101f) && !s.at(0x1000).unwrap().contains(0x1020));
    }

    #[test]
    fn mistakes_name_their_line() {
        for (text, line, why) in [
            ("0x10 a\nzz b", 2, "not a hex address"),
            ("0x10", 1, "ADDRESS NAME"),
            ("0x10 a fnuc", 1, "not a kind"),
            ("0x10 a size=lots", 1, "not a number"),
            ("0x10 a\n0x20 a", 2, "already"),
            ("0x10 a\n0x10 b", 2, "already a"),
        ] {
            let e = Symbols::parse(text).unwrap_err();
            assert_eq!(e.line, line, "{text}");
            assert!(e.what.contains(why), "{text}: {e}");
        }
        let s = Symbols::parse("0x10 a size=16 # (guess)\n").unwrap();
        let a = s.get("a").unwrap();
        assert_eq!((a.size, a.confidence, a.note.as_str()), (Some(16), Some(Confidence::Guess), ""));
    }
}
