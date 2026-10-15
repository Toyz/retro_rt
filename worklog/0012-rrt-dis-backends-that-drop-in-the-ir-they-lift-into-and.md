---
number: 12
title: rrt-dis: backends that drop in, the IR they lift into, and rugrats' decompiler
date: 2026-10-14
area: dis, design, test, docs
files: crates/rrt-dis/src/ir.rs, crates/rrt-dis/src/insn.rs, crates/rrt-dis/src/mips/lift.rs, crates/rrt-dis/src/arch.rs, crates/rrt-dis/src/conformance.rs, crates/rrt-dis/src/exe.rs, crates/rrt-dis/src/analysis.rs, crates/rrt-dis/src/pseudo.rs, crates/rrt-dis/src/symbols.rs, docs/crates/rrt-dis.md, docs/guides/new-backend.md
---

# 12. rrt-dis: backends that drop in, the IR they lift into, and rugrats' decompiler

Two asks: rugrats' pseudo-decompiler in rrt-dis, and backends that are easy
to add, with 16- and 32-bit Windows (x86, NE/PE) as the case to design
for, though not to build yet.

**Backends.** The decompiler needed to know what instructions compute, and
rugrats' version matched MIPS opcodes throughout. So the trait gained a
lift into a small IR, `ir`:

- Expressions over registers.
- Effects: set, store, opaque, abort.
- A branch's condition, and an indirect target.

Alongside it:

- `call_conv`: arguments, results, saved and clobbered registers, sp, gp
  and ra.
- `fixed_size` and `pointer_size`.
- `var_name`, so the GTE's registers read `gte.sz1`.

`evaluate`, `switch_bound`, `stack_pointer` and `global_pointer` now
derive from `lift` and `call_conv`, so a backend writes decode, render and
lift and everything works. MIPS's hand-written `evaluate` was deleted:
derived from the lifter, hwtr's numbers came out identical (1838
functions, 39 tables, 659 slot calls), which validates the lifter
against the old code.

`arch::BACKENDS` and `exe::FORMATS` (`load` sniffs and gives one `Loaded`
shape) make a backend or a format one entry. `conformance::check` is what
any backend runs. It asserts, among the rest, that a lift reads and writes
only what the decoder declares. It caught `sqc2 vf0` reading the
hard-wired `vf0`. The guide is `docs/guides/new-backend.md`.

**Variable-length code.** The analysis walked backwards by `address - 4`.
It now steps through decoded instruction starts when `fixed_size` is None.
MIPS with its size hidden got different answers on all of Hot Wheels at
first: stepping back across gaps into unrelated code picked up stale
constants. Stepping only to an instruction that ends exactly where the
next begins fixed that. The result is the same 1838 functions, 39 tables
and 186 slots in the same time, with 1.4% fewer data references.

**The decompiler** is rugrats' `decomp.rs` on the IR. Against rugrats' own
output for all 2161 functions of its game, 1794 match after normalising
names. Reading the rest:

- **An ordering bug of ours.** An opaque GTE command was printed before
  the pending values that read the registers it overwrites. Rugrats'
  `v1 = -gte.mac0` belongs before the second `nclip`. Now a test.
- **A bug of rugrats'.** It re-read memory a store had just changed,
  where the instruction tests the register.
- **Presentation.** Switches show their index (the analysis now records
  the bound compare's register); table loads read `*(s32*)(table + i)`;
  constant comparisons fold; coprocessor writes print where they happen,
  as rugrats did.
- **Missed folds.** Addresses inside loads did not fold until
  simplification went through the whole tree.

On the PS2, single-precision COP1 (with the EE's accumulator forms),
`movz`/`movn` (a new conditional expression) and `lqc2`/`sqc2` were
lifted. That took .hack's comment lines from 9,098 to 4,323. What remains
is the EE's 64-bit shifts and MMI, which a 32-bit value model cannot say
(kept as comments, documented).

**Rugrats' own analysis**, through the agent porting its tools, found
these:

- **Jump tables before their code.** Its jump tables sit in rodata
  before the C code, and the table reader, inherited from hwtr, stopped
  at a target above the table: 0 of 80 found. Entries now only have to
  be code, not data, and not inside the table. All 80 are found, plus one.
- **`break` let walks continue.** `_start` swallowed the function after
  it. `Flow::Halt` now ends the walk, as hwtr's analyser did.
- **BIOS stubs counted as unresolved.** A jump or call through a
  register holding a constant is now direct, so the PS1's BIOS stubs
  stop counting as unresolved: hwtr 36 to 0.
- **A loop bug.** A call target inside already-owned code was re-queued
  every round, spending all 32 rounds before gap seeds ran again. Rugrats'
  unowned code went from 2129 words to 191 when it stopped.
- **`@data START END`.** Rugrats' symbol directive for data inside code is
  now part of the format.
- **The PS-X EXE header.** Its data, bss and stack fields and region
  marker are parsed.

The rugrats agent suggested a region-sized data symbol should not name
addresses inside it. That is what `@data` is for; a sized data symbol
stays an object whose fields read `name+0x10`.

**Still unknown:** how the decompiler should take stack arguments, which an x86 backend will need
