---
number: 10
title: rrt-dis: one disassembler, symbol file and analyser for every port
date: 2026-10-14
area: dis, emu, design, test, docs
files: crates/rrt-dis/src/insn.rs, crates/rrt-dis/src/mips/mod.rs, crates/rrt-dis/src/symbols.rs, crates/rrt-dis/src/exe.rs, crates/rrt-dis/src/analysis.rs, crates/rrt-dis/src/listing.rs, crates/rrt-dis/src/main.rs, crates/rrt-dis/tests/llvm.rs, crates/rrt-emu/src/load.rs, docs/crates/rrt-dis.md
---

# 10. rrt-dis: one disassembler, symbol file and analyser for every port

hwtr has its own MIPS decoder, recursive-descent analyser and symbol file for
the PS1, piney_apples another decoder for the PS2, and the next port would
have written a third. `rrt-dis` is one of each, behind an `Arch` trait, with
no dependencies.

Decoding is structural: an `Insn` carries its operands, its `Flow` (next,
branch and whether likely, jump, call, return, trap, invalid), the registers
it reads and writes, named and implied, and the memory it touches. The
analyser therefore never names an opcode; it asks the architecture what an
instruction writes given known inputs (`evaluate`), which registers are gp
and sp, and what bounds a switch.

`Mips::r3000()` (MIPS I, COP0, GTE) and `Mips::r5900()` (MIPS III, MMI,
COP1, VU0 macro mode) carry over hwtr's and piney's test vectors. They are
also compared word by word with `llvm-objdump` over 61,696 generated words
per CPU. Words LLVM cannot judge (GTE, MMI, VU0, PS1 COP1, EE doubles) are
excluded, each with a stated reason and a count. LLVM 23 locally and
Ubuntu's LLVM 18 (run in a container, as CI will have it) give the same
counts: 5848 and 10163 words equal, no mismatches. CI now installs `llvm`.

The symbol file is hwtr's format made richer and kept editable:
`0xADDR name [func|data|label] [size=N] # (sure|likely|guess) note`.
Untouched lines write back byte for byte, and `mark` rewrites one line or
appends one; hwtr's `cccpsx.txt` round-trips unchanged. rrt-emu's `Symbols`
and `Endian` are now this crate's. The PS-X EXE and ELF parsers moved here,
so the disassembler reads executables without depending on the emulator.
rrt-emu writes them into memory through a new `Load` trait. ELF symbols now
keep their kind and size.

The analyser is hwtr's `program.rs`, ported, and was checked against hwtr's
own output for Hot Wheels Turbo Racing's `CCCPSX.EXE`: the same 1838
function starts, the same 39 switch sites over 26 tables, the same gp, code
end and 29 unowned words, and every slotted call site hwtr found. Getting
there turned up three things:

- **Stale switch bases.** The backward constant scan runs in a straight
  line, as hwtr's does, so an index register set on another path read as a
  constant. Two switches got bogus table addresses. The finder now tries
  each input of the add that builds the base, then the base itself, and
  keeps the first that reads as a table.
- **Stopping at jumps went too far.** Stopping the scan at any jump or
  return fixed the switches too, but it lost slot bases held in
  s-registers across loops, so that change was removed.
- **Junk slots, and linker-filled tables.** Misaligned or below-image slot
  addresses (0x121, 0xfffffce1) are stale values and are dropped. A slot in
  the image that no code stores to now resolves through its initial word,
  so 659 call sites resolve against hwtr's 642.

.hack//Infection's ELF needed more. Metrowerks puts code, a 1.5 MB blob of
unnamed data and named data in one section, with nine stray functions at the
end. The first run found 6314 "functions" and 4365 unresolved jumps,
decoding bitmaps as code. The fixes:

- **Code regions instead of one code end.** Function extents are merged
  when under 256 KiB apart. Heuristic seeds and their walks stay inside the
  regions and out of sized `data` symbols.
- **Dead code stays with its function.** A gap seed strictly inside a sized
  `func` symbol is that function's dead code, not a new function.
- **`find_gp` follows crt0 further.** It takes the start-up code's last
  write of gp, which catches the EE's `move gp, a0`.
- **`Elf::flatten` keeps file bytes only.** It leaves out bss and the empty
  overlay sections. Zero-filled, they had the overlay functions' symbols
  walking megabytes of `nop`s.
- **A hidden loop bug.** When the linear scan found no seeds, the loop never
  ran, so gap seeds were never tried. The PS1 numbers had hidden this.

The result is 2244 functions covering every ELF function symbol, 97 jump
tables, 53 unowned words and one unresolved jump (C++ exception unwinding),
in 0.7 s. The full 672,000-line listing takes 1.2 s.

The `rrt-dis` command (feature `cli`, on `rrt_kit::cli`) prints stats,
function and range listings, strings, and the unnamed functions as
symbol-file lines to review. `--mark NAME=ADDR:kind:confidence` edits the
project's file in place.

Not done, on purpose: Ghidra or IDA sync, VU microcode, overlays as one
image, real data flow.

**Still unknown:** nothing
