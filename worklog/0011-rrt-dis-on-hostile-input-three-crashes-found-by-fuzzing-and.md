---
number: 11
title: rrt-dis on hostile input: three crashes found by fuzzing, and a careful --mark
date: 2026-10-14
area: dis, bug, test
files: crates/rrt-dis/src/exe.rs, crates/rrt-dis/src/analysis.rs, crates/rrt-dis/src/main.rs, crates/rrt-dis/tests/robust.rs, crates/rrt-dis/tests/cli.rs, docs/crates/rrt-dis.md
---

# 11. rrt-dis on hostile input: three crashes found by fuzzing, and a careful --mark

Executables come from discs and dumps, and a symbol file holds a project's
work. Hardening rrt-dis started with a mutation fuzz in the test profile,
where integer overflow panics. It found three real crashes:

- **The ELF symbol table.** A symbol's name offset was used unchecked, and
  so was its info byte, so a corrupt `.symtab` panicked the parser. Both are
  now bounds-checked. A name that is empty or has whitespace is skipped,
  because the symbol file could not hold it.
- **An image at address 0.** The gap seeder looked at `pc - 4`, and at the
  image's first word that underflowed.
- **An image at the top of the address space.** `Image::end` wrapped, so an
  image loaded near 0xffffffff analysed nothing without a word, and loops
  of the form `pc + 4 <= end` overflowed. Now `end` saturates, `analyze`
  cuts the image short 16 bytes below 4 GiB, and backward steps are
  checked. The CLI warns when this happens, or when the entry is outside
  the image.

`tests/robust.rs` keeps it that way. It mutates a small ELF and a PS-X EXE,
generates random bytes and random symbol tables, and runs them through
parsing, analysis and listing, with bases at 0, at the top, and anywhere.
It runs 400 rounds in CI and was run for 30,000 by hand.

The other risk was to data. `--mark foo=ADDR` on an address that already
had a name replaced the whole entry, so a traced `sure` name lost its kind,
size and note. Now:

- A mark keeps what it does not say.
- Renaming a `sure` name, or lowering a confidence, needs `--force`.
- All marks are checked before the file is touched.
- The new text must parse back to the same symbols.
- The file is replaced through a temporary and a rename.

`--mark` also refuses to write if the file changed after it was read, so an
editor or another tool that saved it meanwhile is not overwritten, and the
file keeps its permissions. The crate now forbids `unsafe` code. Naming an
address inside a symbol used to scan every symbol; it now walks back from
the address through the new `Symbols::range`. The full .hack listing went
from 1.2 s to 1.0 s.

hwtr parity is unchanged: 1838 functions, 39 switches.

**Still unknown:** nothing
