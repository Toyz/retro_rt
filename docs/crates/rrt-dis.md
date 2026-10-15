---
title: rrt-dis, disassembly, symbol files and program analysis
status: solid
crates: rrt-dis
covers: rrt_dis::Arch, rrt_dis::Arch::decode, rrt_dis::Arch::render, rrt_dis::Arch::reg_name, rrt_dis::Arch::evaluate, rrt_dis::Arch::global_pointer, rrt_dis::Arch::stack_pointer, rrt_dis::Arch::switch_bound, rrt_dis::Insn, rrt_dis::Insn::is_valid, rrt_dis::Insn::imm, rrt_dis::Flow, rrt_dis::Flow::falls_through, rrt_dis::Flow::target, rrt_dis::Operand, rrt_dis::Reg, rrt_dis::RegFile, rrt_dis::Mem, rrt_dis::Endian, rrt_dis::mips::Mips, rrt_dis::mips::Mips::r3000, rrt_dis::mips::Mips::r5900, rrt_dis::mips::Mips::canonical, rrt_dis::mips::Mips::decode_word, rrt_dis::mips::Isa, rrt_dis::Symbols, rrt_dis::Symbols::parse, rrt_dis::Symbols::to_text, rrt_dis::Symbols::mark, rrt_dis::Symbols::insert, rrt_dis::Symbols::remove, rrt_dis::Symbols::get, rrt_dis::Symbols::at, rrt_dis::Symbols::locate, rrt_dis::Symbols::range, rrt_dis::Symbols::describe, rrt_dis::Symbol, rrt_dis::Symbol::to_line, rrt_dis::Kind, rrt_dis::Confidence, rrt_dis::symbols::ParseError, rrt_dis::exe::PsxExe, rrt_dis::exe::PsxExe::parse, rrt_dis::exe::PsxExe::as_image, rrt_dis::exe::Elf, rrt_dis::exe::Elf::parse, rrt_dis::exe::Elf::flatten, rrt_dis::exe::Segment, rrt_dis::analysis::Image, rrt_dis::analysis::Program, rrt_dis::analysis::Program::analyze, rrt_dis::analysis::Program::value_before, rrt_dis::analysis::Program::arg_at_call, rrt_dis::analysis::Program::func_of, rrt_dis::analysis::Program::in_code, rrt_dis::analysis::Program::unowned_words, rrt_dis::analysis::Program::strings, rrt_dis::analysis::Found, rrt_dis::analysis::Func, rrt_dis::analysis::JumpTable, rrt_dis::analysis::Ref, rrt_dis::analysis::find_gp, rrt_dis::analysis::REGION_GAP, rrt_dis::listing::Listing, rrt_dis::listing::Listing::label, rrt_dis::listing::Listing::name, rrt_dis::listing::Listing::function, rrt_dis::listing::Listing::range
worklog: 10, 11
---

# rrt-dis

Every port reads the original's machine code, names what it finds, and
keeps those names in a file it commits. hwtr wrote its own decoder,
analyser and symbol file for the PS1; piney_apples wrote another decoder
for the PS2. This crate is both, behind one trait, for any port's tools.
The library has no dependencies (`rrt` feature `dis`); with the `cli`
feature it is also the `rrt-dis` command.

```rust
use rrt_dis::analysis::{find_gp, Image, Program};
use rrt_dis::{exe::PsxExe, listing::Listing, mips::Mips, Symbols};

let exe = PsxExe::parse(&std::fs::read("work/fs/GAME.EXE")?)?;
let symbols = Symbols::parse(&std::fs::read_to_string("symbols/game.txt")?)?;
let arch = Mips::r3000();
let image = exe.as_image();
let gp = find_gp(&arch, image, exe.pc0);
let program = Program::analyze(&arch, image, exe.pc0, gp, &symbols);
print!("{}", Listing::new(&program, &symbols).function(exe.pc0).unwrap());
```

## Instructions: `Arch` and `Insn`

`Arch` is an instruction set. `decode(bytes, addr)` gives an `Insn` with
structure, not just text: the mnemonic, `Operand`s (`Reg`, `Imm`, `Target`,
`Mem`, `Code`), the control `Flow`, and the registers it `reads` and
`writes`, both named and implied (a `mult` writes hi and lo, a GTE command
the GTE registers it uses). Also the memory it touches (`Mem`: base,
offset, size, load or store) and whether it has a delay slot. An encoding
the CPU does not have decodes as `Flow::Invalid`, never as a guess.

`Flow` is what analysis needs: `Next`, `Branch { target, likely }`,
`Jump { target }` and `Call { target }` (`target` is `None` through a
register), `Return`, `Trap`, `Invalid`. A branch-likely skips its delay slot when not
taken.

`render(insn, label)` prints an instruction, asking `label` for a name for
every address operand. `reg_name` names a register. The analysis asks the
architecture four more things, so it never names an opcode itself:

| method | answers | MIPS |
| --- | --- | --- |
| `evaluate(insn, known)` | the value written, given known inputs | `lui`, `addiu`, `ori`, `addu`, `or`/`move`, `subu`, `sll` and 64-bit forms, on the low 32 bits; `zero` is 0 |
| `global_pointer` | the register small data is addressed from | `gp` |
| `stack_pointer` | the register frames are opened on | `sp` |
| `switch_bound(insn)` | the case count an instruction bounds | `sltiu rt, rs, N` |

Registers are a file and an index (`RegFile`: `Gpr`, `Special`, `Cop0`,
`Fpr`, `FpCtrl`, `Cop2Data`, `Cop2Ctrl`, `VecFloat`, `VecInt`).

## MIPS: `mips::Mips`

`Mips::r3000()` is the PS1's R3000A: MIPS I, COP0 as the PS1 has it, and
COP2 as the GTE, with each command's fields and register effects.
`Mips::r5900()` is the PS2's EE: MIPS III integer, `movz`/`movn`/`pref`,
`lq`/`sq`, the pipeline-1 multiplies, all of MMI, the EE's COP0, its
single-precision COP1 with the accumulator, and VU0 in macro mode. Both print
the way hwtr and piney did: o32 names without `$`, pseudo-ops (`nop`,
`move`, `li`, `b`, `beqz`, ...). `.canonical()` turns pseudo-ops off. The
module doc has the register numbering and the full pseudo-op table.

Tested three ways:

- hwtr's decoder tests and every one of piney's vectors, copied, in
  `tests/mips.rs`.
- Unit tests for reads, writes, memory and flow for each instruction class.
- `tests/llvm.rs`: 61,696 generated words per CPU are disassembled by
  `llvm-objdump` (`mips1` against the R3000A, `mips3`/`mips4` against the
  R5900) and must match this decoder's canonical text word for word. Words
  LLVM does not know (GTE, MMI, VU0) or the console does not have (COP1 on
  the PS1, doubles on the EE) are excluded by stated reason, counted and
  printed. `RRT_DIS_LLVM_VERBOSE=1` shows examples. The test skips with a
  note when LLVM is not installed; CI installs it.

## Symbol files: `Symbols`

A project's names for addresses, as a text file it commits, compatible with
hwtr's `symbols/*.txt`:

```text
# comments and blank lines are kept
0x80012345 DrawCar func size=0x1a0 # (sure) traced from the render loop
0x800c9090 g_cars data size=0x400 # (likely)
0x80012400 retry label
0x80013000 fn_80013000
```

Each entry is an address and a name. Optionally it has a `Kind` (`func`,
`data`, `label`), a size, a `Confidence` (`guess`, `likely`, `sure`) and a
note. `parse` errors on a repeated name or address, with the line number.

`to_text` writes back every untouched line byte for byte. `mark` replaces
the one line it changes in place, or appends a new entry. Renaming a symbol
is therefore a one-line diff
(`hwtrs_file_reads_and_writes_back_byte_for_byte`,
`marking_rewrites_one_line_in_place_and_adds_new_ones_at_the_end`).

For lookups: `get` takes a name, `at` an exact address; `locate` finds the
symbol at or before an address; `range(..=addr).rev()` walks back from an
address; `describe(addr, slack)` gives `name+0x10`.
rrt-emu uses this same type for `hook_symbol`, so the file a project's
disassembly uses is the one its oracle hooks by.

## Executables: `exe`

`PsxExe::parse` reads the PS-X EXE header (entry, initial gp, load address)
and image. `as_image` gives the `Image` to analyse.

`Elf::parse` reads a 32-bit ELF of either byte order: `PT_LOAD` segments,
and `.symtab` into `Symbols`, with functions as `func` and objects as
`data`, both with sizes. `flatten` lays the segments' file bytes out as one
image. Bss and segments with no bytes in the file (overlays loaded later)
are left out (`an_elf_of_either_byte_order_parses_with_its_symbols`).
rrt-emu's `Load` writes either into an emulated machine.

## Analysis: `Program`

`Program::analyze(arch, image, entry, gp, symbols)` is hwtr's recursive
descent, which was proven on Hot Wheels Turbo Racing, made
architecture-neutral.

1. **Certain starts are walked first.** These are the entry, every `func`
   symbol, and everything reachable from them by direct calls. That fixes
   the code regions: the functions' extents merged wherever they are less
   than `REGION_GAP` (256 KiB) apart.
2. **Heuristic seeds are tried inside the code regions only, and never in a
   sized `data` symbol.** The seeds are:
   - a call found by a linear scan
   - a frame opened right after a return
   - a data word pointing at a frame opening
   - the start of an unowned stretch of code

   A heuristic seed strictly inside a sized `func` symbol is that
   function's dead code, and is rejected.
3. **Each function is walked.** The walk follows both arms of every branch
   until a return, a tail jump (to a known start, or backwards), or a jump
   through a register. For a jump through a register it looks for a switch:
   a 4-byte load from a table the code builds, bounded by `switch_bound`.
   Each candidate base is tried until one reads as a table.
4. **Overlaps are pruned.** Heuristic starts that another function's walk
   reaches are dropped.
5. **Indirect calls are resolved through interface slots.** A call through
   a register loaded from a fixed word is resolved when that word only ever
   holds one function. That function comes from a store of a function's
   address, or is the word's initial value in the image.

Results:

| field | holds |
| --- | --- |
| `funcs` | `Func` by start: extent, `Found` (how), calls, tail calls, indirect and unresolved jumps |
| `owner` | instruction address to its function |
| `callers` | callee to call sites, indirect ones resolved through slots included |
| `jump_tables` | `JumpTable` by the dispatching jump: table address and targets |
| `slots`, `indirect` | interface slots and the calls through them |
| `refs` | every address the code refers to, with each site and the `Ref` (call, jump, read, write, address taken) |
| `code`, `code_end` | the code regions |

`value_before(site, reg)` is the constant a register holds before an
instruction. It walks back through `evaluate` and knows `gp`.
`arg_at_call(site, reg)` is the same, taken after the call's delay slot.
`strings(min)` returns the strings the code takes the address of.
`find_gp` evaluates the start-up code's last write to gp before its first
call. That covers both `lui`/`addiu` and the EE's `move gp, a0`.

Tested on small hand-assembled programs:

- `walks_calls_and_switches` (hwtr's)
- `data_refs_strings_tail_calls_and_slots`
- `symbols_marked_func_are_certain_starts`
- `heuristics_stay_in_code_regions_and_out_of_sized_functions`

On real executables, compared by hand when this was written (worklog 10):

| executable | result |
| --- | --- |
| Hot Wheels Turbo Racing's `CCCPSX.EXE` | the same 1838 function starts as hwtr's analyser, the same 39 switch sites over 26 tables, the same 29 unowned words, every slotted call site hwtr found, in 0.6 s |
| .hack//Infection's `SLUS_202.67` (Metrowerks, code and data in one section) | 2244 functions covering every ELF function symbol, 97 jump tables, 53 unowned words, one unresolved jump (C++ exception unwinding), in 0.7 s |

## Listings: `Listing`

`Listing::new(&program, &symbols)` renders a function (`function(start)`)
or a range (`range(from, to)`). Owned instructions are decoded. Everything
else is data: `.asciz` for a string the code refers to, otherwise `.word`,
named when the word points at a label.

Names come from the symbols first. Otherwise they are generated in the
style splat users know: `func_80010020`, `.L80010040` for a branch or case
target inside a function, `str_80010070`, `D_80018010`. Inside a sized
symbol an address is `name+0x10`.

Comments show a function's callers (the first four and a count), the
symbol's note, what a load or store reaches, the string an argument points
at, a switch's case count and table, and the callee of a slotted `jalr`
(`names_calls_switches_and_strings`).

```text
puts:                              # 1 caller: 80010004; prints a line
  80010018  03e00008  jr      ra
```

## The command: `rrt-dis`

```text
cargo run -p rrt-dis --features cli -- FILE [options]
```

| option | does |
| --- | --- |
| `-s FILE` | the project's symbol file; an ELF's own symbols are used under it |
| `--arch r3000\|r5900` | default r3000 for a PS-X EXE or raw bytes, r5900 for an ELF |
| `--base`, `--entry`, `--gp ADDR` | for raw bytes, or to override |
| `--stats` | functions by how found, code regions, jump tables, slot resolution, where unresolved jumps are |
| `-f NAME\|ADDR` | list a function (repeatable); an address inside one lists the whole function |
| `--from A --to B` | list a range |
| `--functions` | the functions found that the file does not name, as symbol-file lines to review |
| `--strings` | the strings the code refers to |
| `--mark NAME=ADDR[:KIND[:CONFIDENCE]]` | name an address in the symbol file, in place (repeatable) |
| `--force` | let `--mark` rename a `sure` name, or make a name less sure |
| `--no-addresses` | listings without addresses and encodings, for diffing |

`--mark` is careful with the project's file:

- Renaming an address keeps everything the mark does not say: kind, size,
  confidence and note.
- Replacing a `sure` name, or lowering a name's confidence, needs
  `--force`.
- Every mark is checked before anything is written, so one bad mark leaves
  the file untouched.
- The new text must parse back to the same symbols, or nothing is written.
- The file is replaced through a temporary and a rename, so a crash cannot
  leave it half written. The file keeps its permissions.
- If the file changed after `rrt-dis` read it (an editor or another tool
  saved it), nothing is written; run the command again. Unit test:
  `save_refuses_a_file_changed_since_it_was_read_and_keeps_permissions`.

With no option it lists the whole image. Addresses are hex, with or without
`0x`. A closed pipe (`| head`) ends it quietly. It warns when the entry is outside the image or the image reaches the top
of the address space. Tested end to end in `tests/cli.rs`:

- `stats_listing_and_marking`
- `raw_bytes_need_a_base`
- `marking_keeps_what_it_does_not_say_and_guards_sure_names`

## Hostile input

The crate forbids `unsafe` code, in the library and the command.
Executables come from discs and dumps. They may be truncated, corrupted, or
not what their header says, so nothing here panics on them:

- **Parsers.** A malformed PS-X EXE or ELF is an `Err` naming the problem.
  An ELF symbol whose name is unreadable, empty or contains whitespace is
  skipped.
- **Analysis.** `analyze` cuts the image short 16 bytes below 4 GiB, so
  stepping through it never wraps. `Image::end` saturates instead of
  wrapping. Every backward step is checked. Recursion is bounded (six
  levels for `value_before`, eight for `find_gp`), and so are the seeding
  rounds (32).
- **The test.** `tests/robust.rs` (`hostile_input_never_panics`) feeds
  mutated ELFs and PS-X EXEs, random bytes and random symbol tables
  through parsing, analysis and listing. It uses images at address 0, at
  the top of the address space, and anywhere between, and runs in the test
  profile, where integer overflow panics. It runs 400 rounds by default; it
  was run for 30,000. `RRT_DIS_ROBUST_ROUNDS` sets the count.

## Not here

- **Ghidra or IDA sync.** The symbol file is the source of truth, and a
  script on the tool's side can read it. An exporter can come when a port
  needs one.
- **VU microcode** (VU0/VU1 micro mode). It is a different instruction
  stream, uploaded at run time.
- **Overlays.** A PS2 game's overlay modules load over one another at the
  same address. Analyse each as its own `Image` with its own entry and
  symbols.
- **Data-flow analysis.** `value_before` walks back in a straight line, as
  hwtr's did, so a register set on another path can be read as a constant.
  The switch finder guards against this by trying every candidate base. A
  wrong slot needs a misaligned or below-image address to be rejected.
- **Decompilation.**

## Gaps

Nothing known.
