---
title: Adding a backend to rrt-dis
status: solid
crates: rrt-dis
covers: rrt_dis::Arch, rrt_dis::Arch::lift, rrt_dis::Arch::call_conv, rrt_dis::Arch::fixed_size, rrt_dis::Arch::pointer_size, rrt_dis::CallConv, rrt_dis::ir::Lifted, rrt_dis::conformance::check, rrt_dis::arch::BACKENDS, rrt_dis::exe::FORMATS
---

# Adding a backend to rrt-dis

A backend is one instruction set: the 68000 for a Mega Drive port, SH-2
for a Saturn one, x86 for a 16- or 32-bit Windows game. Once it implements
`Arch`, the analysis, listings, pseudo-code and the `rrt-dis` command work
for it unchanged. Nothing outside a backend's module names an opcode. This
guide is the order to build one in; `crates/rrt-dis/src/mips` is the worked
example.

## 1. Decode and print

Make a module under `crates/rrt-dis/src/<isa>/` and implement the required
methods:

| method | gives |
| --- | --- |
| `name`, `endian`, `align` | "68000", `Endian::Big`, 2 |
| `decode(bytes, addr)` | an `Insn` |
| `reg_name(reg)` | "d0", "a7" |
| `render(insn, label)` | `move.l  d0, (a0)`, asking `label` for every address operand |

What `decode` must fill in:
- `size`: the bytes taken (variable for x86 and the 68000)
- `operands`
- `flow`:
  - `Call`, `Jump` and `Branch` with their targets
  - `Return`
  - `Trap` for a system call that comes back
  - `Halt` for one that does not
- `delay_slot`: set only if the architecture has them
- `reads` and `writes`: every register, implied ones included. A `call`
  writes the stack pointer; a compare writes flags (`RegFile::Flags`, one
  register per flag).
- `mem`: base register, offset, size, load or store

A word that is no instruction is `Flow::Invalid`, never a guess.

Test the decoder against a reference disassembler word by word, as
`tests/llvm.rs` does with `llvm-objdump`. Exclude by stated reason, and
count, what the reference does not know.

## 2. Check it conforms

```rust
#[test]
fn conforms() {
    let failures = rrt_dis::conformance::check(&M68k::new(), 200_000, 1);
    assert!(failures.is_empty(), "{}", failures.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n"));
}
```

This checks what the analysis and the decompiler assume: determinism,
sizes, delay slots, aligned targets, rendering, register names. Once
`lift` exists, it also checks that the lift agrees with `reads`, `writes`
and `mem`.

## 3. Register it

Add a `Backend` to `arch::BACKENDS` (a name, aliases, a description, a
constructor), and teach the executable formats that name the CPU to return
it (`Loaded::arch`). A new format, such as NE or PE, is a parser in
`exe.rs` and a `Format` in `exe::FORMATS`: a name, a cheap `sniff` on the
magic number, and a `load` giving segments, entry and symbols.
`rrt-dis --list` shows both.

At this point the command lists code and `--stats` finds functions by
calls, prologues and pointers.

## 4. Lift

`lift(insn)` says what an instruction computes, as `ir::Effect`s on
registers and memory:
- `Set(reg, expr)`
- `Store { size, addr, value }`
- `Opaque(text)`, for what the IR cannot say
- `Abort`

It also gives the branch's `cond` and an indirect jump's `target`. Every
expression reads registers as they were before the instruction.

Return None for anything not worth lifting yet: the decompiler shows it as
a comment, and the analysis just knows less. Lifting the integer ALU,
loads and stores, and branch conditions first gives most of the value.

From `lift`, defaults derive what the analysis asks:
- `evaluate`: a single `Set`, evaluated
- `switch_bound`: a `Set` to an unsigned less-than against a constant

Override either only if the idiom differs, as x86's `cmp eax, N; ja
default` bounds through the flags.

For a flags architecture, lift a compare into `Set`s of the flag
registers, and a conditional branch's `cond` into an expression over them.
Pseudo-code folds `Flags` registers, so `cmp eax, 3; jae L` reads as
`if (eax u< 3) ...` once the condition is written in terms of the compare
the flags came from.

## 5. Describe the calling convention

`call_conv` returns a `CallConv`:
- the argument registers in order, the result registers, callee-saved and
  clobbered registers
- the stack pointer, global pointer and return address

The defaults for `stack_pointer` and `global_pointer` come from it, and the
decompiler uses it to count a callee's arguments, to know what a call
changes, and to drop a prologue's saves.

A stack-passing convention (16-bit Windows `pascal`, 32-bit `stdcall`) has
no argument registers. Its calls then show no arguments until the
decompiler learns stack arguments, which is the next thing it needs for
x86.

## 6. Say what is different about the encoding

- `fixed_size`: return `Some(n)` when every instruction is n bytes.
  Variable-length sets return None, and the analysis then steps back only
  through instructions it has decoded, one ending exactly where the next
  begins (`a_variable_length_backend_gets_the_same_answers`).
- `pointer_size`: 2 for 16-bit code, so jump tables and function-pointer
  slots read 2-byte entries. Segmented far pointers (16-bit x86) are not
  modelled yet: a backend linearises `segment:offset` into the 32-bit
  addresses everything here uses.
- `var_name`: how pseudo-code names a register, if not as `reg_name` (MIPS
  writes the GTE's as `gte.sz1`).

## 7. Prove it on a real program

Run `rrt-dis GAME --stats` and `--pseudo-all` on an executable whose
functions you know: from its own symbols, or from an earlier tool. Compare
function starts and jump tables, and read the pseudo-code of a few
functions against the listing. The MIPS backends were held to hwtr's and
rugrats' own analysers this way (the rrt-dis page has the numbers).
