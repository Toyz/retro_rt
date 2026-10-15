---
title: rrt-dis, disassembly, symbol files, analysis and pseudo-code
status: solid
crates: rrt-dis
covers: rrt_dis::Arch, rrt_dis::Arch::decode, rrt_dis::Arch::render, rrt_dis::Arch::reg_name, rrt_dis::Arch::var_name, rrt_dis::Arch::lift, rrt_dis::Arch::call_conv, rrt_dis::Arch::fixed_size, rrt_dis::Arch::pointer_size, rrt_dis::Arch::evaluate, rrt_dis::Arch::global_pointer, rrt_dis::Arch::stack_pointer, rrt_dis::Arch::switch_bound, rrt_dis::CallConv, rrt_dis::Insn, rrt_dis::Insn::is_valid, rrt_dis::Insn::imm, rrt_dis::Flow, rrt_dis::Flow::falls_through, rrt_dis::Flow::target, rrt_dis::Operand, rrt_dis::Reg, rrt_dis::RegFile, rrt_dis::Mem, rrt_dis::Endian, rrt_dis::ir::Expr, rrt_dis::ir::Expr::eval, rrt_dis::ir::Expr::subst, rrt_dis::ir::Expr::simplify, rrt_dis::ir::Expr::simplify_all, rrt_dis::ir::BinOp, rrt_dis::ir::UnOp, rrt_dis::ir::Effect, rrt_dis::ir::Lifted, rrt_dis::ir::Lifted::reads, rrt_dis::ir::Lifted::writes, rrt_dis::arch::Backend, rrt_dis::arch::BACKENDS, rrt_dis::arch::by_name, rrt_dis::conformance::check, rrt_dis::conformance::Failure, rrt_dis::mips::Mips, rrt_dis::mips::Mips::r3000, rrt_dis::mips::Mips::r5900, rrt_dis::mips::Mips::canonical, rrt_dis::mips::Mips::decode_word, rrt_dis::mips::Isa, rrt_dis::Symbols, rrt_dis::Symbols::parse, rrt_dis::Symbols::to_text, rrt_dis::Symbols::mark, rrt_dis::Symbols::insert, rrt_dis::Symbols::remove, rrt_dis::Symbols::get, rrt_dis::Symbols::at, rrt_dis::Symbols::locate, rrt_dis::Symbols::range, rrt_dis::Symbols::describe, rrt_dis::Symbols::data_ranges, rrt_dis::Symbol, rrt_dis::Symbol::to_line, rrt_dis::Kind, rrt_dis::Confidence, rrt_dis::symbols::ParseError, rrt_dis::exe::load, rrt_dis::exe::format_of, rrt_dis::exe::Format, rrt_dis::exe::FORMATS, rrt_dis::exe::Loaded, rrt_dis::exe::flatten, rrt_dis::exe::PsxExe, rrt_dis::exe::PsxExe::parse, rrt_dis::exe::PsxExe::as_image, rrt_dis::exe::Elf, rrt_dis::exe::Elf::parse, rrt_dis::exe::Elf::arch, rrt_dis::exe::Segment, rrt_dis::analysis::Image, rrt_dis::analysis::Image::read, rrt_dis::analysis::Program, rrt_dis::analysis::Program::analyze, rrt_dis::analysis::Program::insn, rrt_dis::analysis::Program::insns_of, rrt_dis::analysis::Program::value_before, rrt_dis::analysis::Program::arg_at_call, rrt_dis::analysis::Program::func_of, rrt_dis::analysis::Program::in_code, rrt_dis::analysis::Program::unowned_words, rrt_dis::analysis::Program::strings, rrt_dis::analysis::Found, rrt_dis::analysis::Func, rrt_dis::analysis::JumpTable, rrt_dis::analysis::Ref, rrt_dis::analysis::find_gp, rrt_dis::analysis::REGION_GAP, rrt_dis::listing::Listing, rrt_dis::listing::Listing::program, rrt_dis::listing::Listing::label, rrt_dis::listing::Listing::name, rrt_dis::listing::Listing::function, rrt_dis::listing::Listing::range, rrt_dis::pseudo::Pseudo, rrt_dis::pseudo::Pseudo::new, rrt_dis::pseudo::Pseudo::with_names, rrt_dis::pseudo::Pseudo::function, rrt_dis::pseudo::Pseudo::arity
worklog: 10, 11, 12, 14
---

# rrt-dis

Every port reads the original's machine code, names what it finds, and
keeps those names in a file it commits. hwtr wrote its own decoder,
analyser and symbol file for the PS1. piney_apples wrote another decoder
for the PS2, and rugrats a decompiler. This crate is all of them, behind
one trait, for any port's tools.

The library has no dependencies (`rrt` feature `dis`). With the `cli`
feature it is also the `rrt-dis` command.

```rust
use rrt_dis::analysis::{find_gp, Program};
use rrt_dis::{exe, listing::Listing, pseudo::Pseudo, Symbols};

let file = exe::load(&std::fs::read("work/fs/GAME.EXE")?)?;
let arch = (rrt_dis::arch::by_name(file.arch.unwrap()).unwrap().make)();
let (base, bytes) = file.flatten()?;
let image = rrt_dis::analysis::Image { base, bytes: &bytes };
let symbols = Symbols::parse(&std::fs::read_to_string("symbols/game.txt")?)?;
let gp = file.gp.or_else(|| find_gp(arch.as_ref(), image, file.entry));
let program = Program::analyze(arch.as_ref(), image, file.entry, gp, &symbols);
let listing = Listing::new(&program, &symbols);
print!("{}", listing.function(file.entry).unwrap());
print!("{}", Pseudo::new(&listing).function(file.entry).unwrap());
```

A program that knows its CPU uses `Mips::r3000()` directly instead of the
registry.

## Backends

A backend is an `Arch`. Everything else here (the analysis, listings, the
decompiler, the command) is written against the trait and names no opcode,
so a new instruction set is one implementation. The guide is
[Adding a backend](../guides/new-backend.md).

| method | what | required |
| --- | --- | --- |
| `name`, `endian`, `align` | identity, byte order, instruction alignment | yes |
| `decode(bytes, addr)` | an `Insn` (below) | yes |
| `reg_name`, `render(insn, label)` | text, with `label` naming every address operand | yes |
| `lift(insn)` | what it computes, as IR (below) | for constants and pseudo-code |
| `call_conv` | `CallConv`: argument, result, saved and clobbered registers; sp, gp, ra | for pseudo-code |
| `fixed_size` | `Some(n)` when every instruction is n bytes | no: None steps back through decoded instructions |
| `pointer_size` | bytes in a jump table entry or function pointer | no: 4 |
| `var_name` | a register as pseudo-code writes it (`gte.sz1`) | no: `reg_name` |
| `evaluate`, `switch_bound`, `stack_pointer`, `global_pointer` | what the analysis asks | no: derived from `lift` and `call_conv` |

`arch::BACKENDS` lists them by name with aliases (`r3000`/`ps1`/`psx`,
`r5900`/`ps2`/`ee`), and `arch::by_name` picks one at run time.

`conformance::check(arch, rounds, seed)` decodes random bytes, including at
address 0 and near the top of the address space, and reports every
assumption a backend breaks:
- decoding is deterministic and never panics
- sizes are within what was given
- delay slots follow only transfers, and targets are aligned
- text is one line, and every register has a name
- **the lift agrees with the decoder**: it reads and sets only registers in
  `reads`/`writes`, its loads and stores match `mem`, and conditions and
  targets come only with branches and jumps

Both MIPS backends pass it on 200,000 inputs each, in pseudo-op and
canonical form (`tests/conformance.rs`). It caught a lifter bug while this
was written: `sqc2 vf0` read the hard-wired `vf0` as a register.

## Instructions: `Insn` and `Flow`

`decode` gives structure, not just text: the mnemonic and `Operand`s
(`Reg`, `Imm`, `Target`, `Mem`, `Code`); the control `Flow`; the registers
it `reads` and `writes`, both named and implied (`mult` writes hi and lo, a
GTE command the GTE registers it uses); the memory it touches (`Mem`: base,
offset, size, load or store); and whether it has a delay slot. An encoding
the CPU does not have decodes as `Flow::Invalid`, never as a guess.

`Flow` is one of:
- `Next`
- `Branch { target, likely }`: a branch-likely skips its delay slot when
  not taken
- `Jump { target }` and `Call { target }`: `target` is None through a
  register
- `Return`
- `Trap`: comes back (`syscall`)
- `Halt`: does not come back (MIPS `break`, which compilers place after a
  call that never returns); the walk ends there
- `Invalid`

Registers are a file and an index (`RegFile`: `Gpr`, `Special`, `Cop0`,
`Fpr`, `FpCtrl`, `Cop2Data`, `Cop2Ctrl`, `VecFloat`, `VecInt`, and `Flags`
and `Segment` for future backends). `RegFile` and `Flow` are
`#[non_exhaustive]`: a backend may need more.

## What instructions compute: `ir`

`lift` gives a `Lifted`:
- `effects`, which all read the registers as they were before the
  instruction:
  - `Set(reg, expr)`
  - `Store { size, addr, value }`
  - `Opaque(text)`, for what the IR does not model (a GTE command, a system
    call)
  - `Abort`
- `cond`, a branch's condition
- `target`, the address of a jump or call through a register

An `Expr` is a register, a constant, an operation (`BinOp`: integer
arithmetic and compares, the 64-bit product's halves, divide and
remainder, and single-precision float operations; `UnOp`: negate, not,
float negate/abs/sqrt, int and float conversions), a load, the fixed-point
multiply, or a conditional `c ? a : b`.

Values are 32 bits, which is what the consoles' address arithmetic is. A
64-bit instruction is lifted on its low 32 bits where that is exact
(`daddu` as the EE's `move`), and not lifted where it is not (below).

`Expr::eval` computes a value from known registers. `simplify` folds
constants, moves constants to the right of commutative operations so
offsets meet, and recognises GCC's fixed-point multiply (`(lo u>> n) |
(hi << 32-n)` of one signed product) as `fx(a * b >> n)`.

The MIPS lifter covers:
- the integer ALU, hi/lo and the multiplies and divides
- loads and stores of every width
- moves to and from COP0, COP1 and the GTE, and `lwc1`/`swc1`/`lwc2`/`swc2`,
  `lqc2`/`sqc2`
- the EE's single-precision COP1, including the accumulator forms and
  compares into the condition bit `bc1t`/`bc1f` test
- `movz`/`movn`
- branch conditions and register targets

GTE commands and `syscall` are opaque, `break` aborts, and the EE's
conditional traps (GCC's divide checks) and `sync`/`cache`/`pref` do
nothing.

## MIPS: `mips::Mips`

`Mips::r3000()` is the PS1's R3000A: MIPS I, COP0 as the PS1 has it, and
COP2 as the GTE, with each command's fields and register effects.

`Mips::r5900()` is the PS2's EE: MIPS III integer, `movz`/`movn`/`pref`,
`lq`/`sq`, the pipeline-1 multiplies, all of MMI, the EE's COP0, its
single-precision COP1 with the accumulator, and VU0 in macro mode.

Both print the way hwtr and piney did: o32 names without `$`, and pseudo-ops
(`nop`, `move`, `li`, `b`, `beqz`, ...). `.canonical()` turns pseudo-ops off.
The calling conventions are o32 on the PS1 (a0-a3) and the EE's EABI on the
PS2 (a0-a3 and t0-t3). The module doc has the register numbering and the
full pseudo-op table.

The decoder is tested four ways:
- **hwtr's and piney's vectors.** hwtr's decoder tests and every one of
  piney's vectors are copied into `tests/mips.rs`.
- **Unit tests** for reads, writes, memory and flow, per instruction class.
- **`tests/llvm.rs`.** 61,696 generated words per CPU are disassembled by
  `llvm-objdump` (`mips1` against the R3000A, `mips3`/`mips4` against the
  R5900) and must match this decoder's canonical text word for word.
  - Words LLVM does not know (GTE, MMI, VU0) or the console does not have
    (COP1 on the PS1, doubles on the EE) are excluded, each with a stated
    reason, counted and printed. `RRT_DIS_LLVM_VERBOSE=1` shows examples.
  - LLVM 23 and Ubuntu's LLVM 18 give the same counts.
  - The test skips with a note when LLVM is not installed; CI installs it.
- **Conformance**, above.

## Symbol files: `Symbols`

A project's names for addresses, as a text file it commits, compatible with
hwtr's and rugrats' `symbols/*.txt`:

```text
# comments and blank lines are kept
@data 0x80012760 0x80017c0c   # rodata between the asm and the C
0x80012345 DrawCar func size=0x1a0 # (sure) traced from the render loop
0x800c9090 g_cars data size=0x400 # (likely)
0x80012400 retry label
0x80013000 fn_80013000
```

Each entry is an address and a name. Optionally it has a `Kind` (`func`,
`data`, `label`), a size, a `Confidence` (`guess`, `likely`, `sure`) and a
note. `parse` errors on a repeated name or address, or an unknown `@`
directive, with the line number.

`@data START END` declares data inside the code, which the analysis keeps
out of its functions (`data_ranges`). A sized `data` symbol does the same
for one object and also names it, so `g_cars+0x10` reads as a field. A
region of unrelated data (all of rodata) is an `@data` range, not a symbol.

`to_text` writes back every untouched line byte for byte. `mark` replaces
the one line it changes in place, or appends a new entry, so renaming a
symbol is a one-line diff:
- `hwtrs_file_reads_and_writes_back_byte_for_byte`
- `marking_rewrites_one_line_in_place_and_adds_new_ones_at_the_end`

For lookups:
- `get` takes a name, `at` an exact address
- `locate` finds the symbol at or before an address
- `range(..=addr).rev()` walks back from an address
- `describe(addr, slack)` gives `name+0x10`

rrt-emu uses this same type for `hook_symbol`.

## Executables: `exe`

`exe::load(bytes)` recognises a file by `exe::FORMATS` (each a name, a
`sniff` and a `load`) and gives a `Loaded`: the format, the backend it
names, the entry, gp when the file gives one, segments and the file's own
symbols. `flatten` lays segments' file bytes out as one image, leaving out
bss and segments with no bytes in the file (overlays loaded later). A new
format (an NE or PE, a cartridge header) is a parser and a `Format` entry.

| format | gives |
| --- | --- |
| `psx-exe` (`PsxExe`) | pc0, gp0, the text, data, bss and stack addresses and sizes, the region marker, the image; backend `r3000` |
| `elf` (`Elf`) | 32-bit, either byte order: `PT_LOAD` segments, `e_flags`, `.symtab` into `Symbols` (functions as `func`, objects as `data`, with sizes); `Elf::arch` is `r5900` for the R5900 or MIPS III and up, `r3000` for other MIPS |

rrt-emu's `Load` writes either into an emulated machine.

## Analysis: `Program`

`Program::analyze(arch, image, entry, gp, symbols)` is hwtr's recursive
descent, proven on Hot Wheels Turbo Racing, made architecture-neutral.

1. **Certain starts first.** The entry, every `func` symbol, and everything
   reachable from them by direct calls are walked first. That fixes the
   code regions: the functions' extents merged wherever they are less than
   `REGION_GAP` (256 KiB) apart.
2. **Heuristic seeds, inside the code regions only.** Seeds are never taken
   from sized `data` symbols or `@data` ranges. The seeds are a call found
   by a linear scan, a frame opened right after a return, a data word
   pointing at a frame opening, and the start of an unowned stretch of
   code. A seed strictly inside a sized `func` symbol is that function's
   dead code.
3. **Each function is walked**, along both arms of every branch, until a
   return, a halt, a tail jump (to a known start, or backwards), or a jump
   through a register. For a jump through a register:
   - a register holding a constant is a direct tail call or call (the
     PS1's BIOS stubs: `li t2, 0xa0; jr t2`)
   - otherwise it looks for a switch: a pointer-sized load from a table
     the code builds, bounded by `switch_bound`. Each candidate base is
     tried until one reads as a table, whose entries must be code, not
     data, and not inside the table. A table may sit before its code
     (rugrats' rodata) or after it (hwtr's). The bounding compare's
     register is the switch's `index`.
4. **Overlaps are pruned.** Heuristic starts another function's walk
   reaches are dropped.
5. **Indirect calls go through interface slots.** A call through a
   register loaded from a fixed word is resolved when that word only ever
   holds one function: the one a store puts there, or the word's initial
   value in the image.

| field | holds |
| --- | --- |
| `funcs` | `Func` by start: extent, `Found` (how), calls, tail calls, indirect and unresolved jumps |
| `owner` | instruction address to its function (`insns_of` lists one function's) |
| `callers` | callee to call sites, those resolved through slots included |
| `jump_tables` | `JumpTable` by the dispatching jump: table, targets, index register |
| `slots`, `indirect` | interface slots and the calls through them |
| `refs` | every address the code refers to, with each site and the `Ref` (call, jump, read, write, address taken) |
| `code`, `code_end` | the code regions |

`value_before(site, reg)` is the constant a register holds before an
instruction: it walks back through `evaluate`, and knows `gp`. It does not
walk past a call for a register the calling convention says calls clobber:
past the call's delay slot, that register is the call's. A saved register
survives (`a_call_clobbers_what_was_built_before_it`). `arg_at_call(site,
reg)` is the value at the callee's entry, its delay slot included.
`strings(min)` returns the strings the code takes the address of.
`find_gp` evaluates the start-up code's last write to gp before its first
call, which covers both `lui`/`addiu` and the EE's `move gp, a0`.

For a variable-length instruction set (`fixed_size` None) the analysis
steps back through instructions it has decoded, and only to one ending
exactly where the next begins. `a_variable_length_backend_gets_the_same_answers`
runs MIPS with its size hidden this way. On all of Hot Wheels it found the
same 1838 functions, 39 jump tables and 186 slots in the same time, and
1.4% fewer data references, the cost of never guessing across a gap.

Small hand-assembled programs test the analysis:
- `walks_calls_and_switches` (hwtr's)
- `data_refs_strings_tail_calls_and_slots`
- `symbols_marked_func_are_certain_starts`
- `heuristics_stay_in_code_regions_and_out_of_sized_functions`
- `a_variable_length_backend_gets_the_same_answers`

On real executables, compared with each game's own tools (worklog 10 and
12):

| executable | result |
| --- | --- |
| Hot Wheels Turbo Racing, `CCCPSX.EXE` | hwtr's analyser exactly: the same 1838 function starts, 39 switch sites over 26 tables, and 29 unowned words. Every slotted call site it found, plus more. 0 unresolved jumps (36 before the BIOS stubs were resolved). 0.7 s |
| Rugrats: Search for Reptar, `SLUS_006.50` | every one of rugrats' 2161 functions, plus 76 it missed, and all 81 jump tables. 191 unowned words against its 2145. 2 unresolved jumps |
| .hack//Infection, `SLUS_202.67` (Metrowerks, code and data in one section) | 2244 functions covering every ELF function symbol, 97 jump tables, 53 unowned words, 0 unresolved jumps. 0.7 s |

## Listings: `Listing`

`Listing::new(&program, &symbols)` renders a function (`function(start)`)
or a range (`range(from, to)`). Owned instructions are decoded. Everything
else is data: `.asciz` for a string the code refers to, otherwise `.word`,
named when the word points at a label.

Names come from the symbols first. Otherwise they are generated in the
style splat users know: `func_80010020`, `.L80010040` for a target inside a
function, `str_80010070`, `D_80018010`. Inside a sized symbol an address is
`name+0x10`.

Comments show:
- a function's callers (the first four and a count)
- the symbol's note
- what a load or store reaches
- the string an argument points at
- a switch's case count and table
- the callee of a slotted `jalr`

```text
puts:                              # 1 caller: 80010004; prints a line
  80010018  03e00008  jr      ra
```

## Pseudo-code: `Pseudo`

`Pseudo::new(&listing).function(start)` decompiles one function to C-like
pseudo-code. It is rugrats' decompiler, ported onto the IR, so it works for
any backend that lifts and has a calling convention.

Each block is executed symbolically: registers hold expression trees, so a
run of instructions folds into what it computes. Stores, calls and branches
become statements. A liveness pass decides what is written: a value shows
only where it is used, and is named when it is used more than once.
Control flow stays as labels and gotos, and a jump table becomes a
`switch` on its index.

```c
// func_800193a8() 800193a8..800193fc, 21 instructions, frame 32
L_800193a8:
    a0 = 0xf2000000;
    s1 = 0x1f8003f4;
L_800193c4:
    s0 = s1->0x0;
    r_func_8009abc4 = func_8009abc4((a0 | 1));
    ...
    return (s0 + v0);
```

Folded idioms:
- Addresses become names, and strings become literals.
- The stack frame: slots read as `local_NN`, and the prologue's saves and
  the epilogue's restores disappear.
- gp-relative slots become globals.
- `table + index` loads read as `*(s32*)(table + i)`.
- The fixed-point multiply becomes `fx(a * b >> n)`.
- GCC's divide-by-zero guards around an aborting trap are dropped.
- A call or jump through a register holding a constant is direct.
- A call shows as many arguments as the callee reads, from the same
  liveness pass run on it (`arity`). Functions that call each other are
  solved together: each strongly connected component of the call graph,
  callees first, as the least fixpoint from no arguments, so the answer
  does not depend on which is decompiled first
  (`arity_in_a_call_cycle_does_not_depend_on_order`).
- A data symbol's memory is its name and its address is `&name`; a
  function is its name and a string its literal
  (`a_data_symbols_address_and_its_memory_read_differently`).

General-purpose, special (hi/lo) and flag registers fold. A coprocessor's
are written as statements where they change (`gte.sz1 = t9;`). An opaque
statement writes out first anything that reads a register it overwrites.
An instruction the backend does not lift prints as a comment, and its
results read as themselves.

Tests:
- one per idiom in `pseudo::tests`
- `a_value_read_from_the_gte_is_kept_across_the_next_command`, which
  holds a real ordering bug found against rugrats
- the hostile-input fuzz

Against rugrats' own output for all 2161 functions of its game, 1794 are
identical after normalising names. The rest are deliberate differences:
- switches on the index rather than the table load
- BIOS stubs as tail calls rather than `goto *0xa0`
- folded `~(0 | x)` and constant compares
- conditions that reuse the register just written
- names where rugrats printed addresses

In checking those, two cases were on rugrats' side. One was our bug, and
the other was rugrats': it re-read memory that a store had just changed
where the instruction tests the register.

## The command: `rrt-dis`

```text
cargo run -p rrt-dis --features cli -- FILE [options]
```

| option | does |
| --- | --- |
| `--list` | the backends and executable formats |
| `-s FILE` | the project's symbol file; an ELF's own symbols are used under it |
| `--arch NAME` | a backend from `--list`; default the one the file names, r3000 for raw bytes |
| `--base`, `--entry`, `--gp ADDR` | for raw bytes, or to override |
| `--stats` | functions by how found, code regions, jump tables, slot resolution, where unresolved jumps are |
| `-f NAME\|ADDR` | list a function (repeatable); an address inside one lists the whole function |
| `-p NAME\|ADDR` | decompile a function (repeatable) |
| `--pseudo-all` | decompile every function |
| `--from A --to B` | list a range |
| `--functions` | the functions found that the file does not name, as symbol-file lines to review |
| `--strings` | the strings the code refers to |
| `--mark NAME=ADDR[:KIND[:CONFIDENCE]]` | name an address in the symbol file, in place (repeatable) |
| `--force` | let `--mark` rename a `sure` name, or make a name less sure |
| `--no-addresses` | listings without addresses and encodings, for diffing |

`--mark` is careful with the project's file:
- Renaming keeps everything the mark does not say: kind, size, confidence
  and note.
- Replacing a `sure` name, or lowering a confidence, needs `--force`.
- Every mark is checked before anything is written.
- The new text must parse back to the same symbols.
- The file is replaced through a temporary and a rename, and keeps its
  permissions.
- Nothing is written if the file changed after it was read
  (`save_refuses_a_file_changed_since_it_was_read_and_keeps_permissions`).

With no option it lists the whole image. Addresses are hex, with or without
`0x`. A closed pipe (`| head`) ends it quietly. It warns when the entry is
outside the image or the image reaches the top of the address space.

Tested end to end in `tests/cli.rs`:
- `stats_listing_and_marking`
- `raw_bytes_need_a_base`
- `marking_keeps_what_it_does_not_say_and_guards_sure_names`

## Hostile input

The crate forbids `unsafe` code. Executables come from discs and dumps, and
nothing here panics on them:

- **Parsers.** A malformed PS-X EXE or ELF is an `Err` naming the problem.
  An ELF symbol whose name is unreadable, empty or contains whitespace is
  skipped.
- **Analysis.** `analyze` cuts the image short 16 bytes below 4 GiB,
  `Image::end` saturates, and every backward step is checked. Recursion is
  bounded (six levels for `value_before`, eight for `find_gp`), as are the
  seeding rounds (32) and the liveness passes (64).
- **`tests/robust.rs`.** It feeds mutated ELFs and PS-X EXEs, random bytes
  and random symbol tables through parsing, analysis, listing and
  pseudo-code. Images sit at 0, at the top of the address space, and
  anywhere between, and it runs in the test profile, where integer
  overflow panics. It runs 400 rounds by default and was run for 20,000;
  `RRT_DIS_ROBUST_ROUNDS` sets the count.

## Not here

- **64-bit values.** The IR's values are 32 bits, so the EE's 64-bit
  shifts (`dsll32`, `dsra32`, `dsrl32`, `dsrl`), MMI and VU0 macro
  operations show as comments in pseudo-code. Its pipeline-1 multiplies do
  too. On .hack those are 4,323 comment lines across 2244 functions,
  mostly `dsll32`/`dsra32` sign extensions.
- **Which BIOS function a stub calls.** A PS1 stub reads as a tail call to
  0xa0/0xb0/0xc0; the function number in t1 is the console's, not the
  CPU's. Name the stubs in the symbol file.
- **Ghidra or IDA sync.** The symbol file is the source of truth, and a
  script on the tool's side can read it.
- **VU microcode** (VU0/VU1 micro mode). It is a different instruction
  stream, uploaded at run time.
- **Overlays.** A PS2 game's overlay modules load over one another at the
  same address. Analyse each as its own image.
- **Data flow, types and structured control flow.**
  - `value_before` walks back in a straight line, as hwtr's did. The switch
    finder tries every candidate base, and a slot needs an aligned
    in-image address.
  - Pseudo-code keeps gotos and knows types only from load widths.
- **x86, NE and PE.** The trait, the IR (`Flags` and `Segment` register
  files) and the variable-length analysis are ready for them. The guide
  says what such a backend needs.

## Gaps

Nothing known.
