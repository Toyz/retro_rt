---
title: rrt-emu, the originals run for comparison
status: partial
crates: rrt-emu
covers: rrt_emu::Cpu, rrt_emu::Cpu::step, rrt_emu::Cpu::abi, rrt_emu::Cpu::in_delay_slot, rrt_emu::Cpu::settle, rrt_emu::Abi, rrt_emu::ReturnAddress, rrt_emu::Endian, rrt_emu::Stop, rrt_emu::Bus, rrt_emu::BusExt, rrt_emu::Bus::canonical, rrt_emu::Memory, rrt_emu::Memory::psx, rrt_emu::Memory::ps2, rrt_emu::Region, rrt_emu::Device, rrt_emu::IoLog, rrt_emu::SharedBus, rrt_emu::Machine, rrt_emu::Machine::call, rrt_emu::Machine::call_recorded, rrt_emu::Machine::hook, rrt_emu::Machine::hook_symbol, rrt_emu::Machine::stub, rrt_emu::Machine::on_syscall, rrt_emu::Machine::check, rrt_emu::Machine::canonical, rrt_emu::machine::Extensions, rrt_emu::Call, rrt_emu::HookResult, rrt_emu::Outcome, rrt_emu::Change, rrt_emu::CheckStats, rrt_emu::Heap, rrt_emu::Symbols, rrt_emu::PsxExe, rrt_emu::Elf, rrt_emu::hle::libc, rrt_emu::hle::install, rrt_emu::psx::R3000, rrt_emu::psx::Gte, rrt_emu::psx::Bios, rrt_emu::psx::machine, rrt_emu::psx::machine_with_exe, rrt_emu::psx::bios::critical_sections, rrt_emu::ee::Ee, rrt_emu::ee::Features, rrt_emu::ee::machine, rrt_emu::ee::machine_with_elf, rrt_emu::ee::kernel::install, rrt_emu::ee::float::add, rrt_emu::ee::float::mul, rrt_emu::ee::float::div, rrt_emu::ee::float::sqrt
---

# rrt-emu

A port is checked by running the original's code on the same inputs and
comparing. This crate is what that takes, for any console: CPUs behind one
trait, memory maps, HLE for the library and BIOS calls the code under test
reaches, and an oracle that reports what a call did. Nothing in it runs at
play time; a port's tests and tools use it (`rrt` feature `emu`).

## The CPU trait, for any architecture

`Cpu` assumes nothing about MIPS. A CPU declares its byte order (`Endian`),
its calling convention (`Abi`: argument registers, return register, stack
pointer, where stack arguments start and how wide they are, the stack
alignment) and where a call leaves its return address
(`ReturnAddress::Register(r)` for MIPS, ARM, SH, PowerPC;
`ReturnAddress::Stack { bytes }` for the 68000, x86, 6502, Z80).
Registers are numbered by the CPU and read as `u128`, the widest any has
(the EE's); `reg_name`/`reg_index` name them. `step(&mut dyn Bus)` returns
cycles or a `Stop`, so the trait is object safe (`Box<dyn Cpu>` for a
system of several CPUs). `in_delay_slot` tells the harness not to fire a
hook at an address reached as a branch's delay slot; `settle` lands a
pending load before registers are read.

`Stop` is shared by every CPU: `Syscall` (the instruction is complete and
the pc past it), `Break`, `Overflow`, `Address`, `Bus`, `Reserved` (an
instruction not interpreted, with its bits), `StepLimit`, `Halt`, `Hle`.

The harness is tested on a made-up CPU in both return conventions
(`machine::tests`), so nothing in it depends on MIPS.

## Memory

`Bus` is byte slices in and out, so one interface carries a byte or an EE
quadword; `BusExt` adds every width little-endian and `read_uint`/
`write_uint` in either byte order. `Memory` is a list of `Region`s matched
after an address mask (0x1fff_ffff folds the MIPS segments): RAM that
mirrors past its size, a `Device` (`IoLog` records accesses and answers
reads from a table), or ignored addresses.

| preset | regions |
| --- | --- |
| `Memory::psx()` | 2 MB RAM mirrored through 8 MB; 1 KB scratchpad 0x1f80_0000; I/O 0x1f80_1000-0x1f80_2fff (`IoLog`); BIOS area 0x1fc0_0000 (512 KB RAM to load into); cache control ignored |
| `Memory::ps2()` | 32 MB RAM; 16 KB scratchpad 0x7000_0000 (not folded); EE I/O 0x1000_0000-0x1000_ffff (`IoLog`) |

`Bus::canonical` folds an address to the one every alias shares (PS1 RAM at
0x8001_0000, 0xa001_0000 and 0x0021_0000 is 0x0001_0000). The write journal
behind the oracle records canonical addresses. `SharedBus` lets several
machines share one memory.

## The machine and HLE

`Machine<C, B = Memory>` holds a CPU, its bus, an HLE `Heap`, a `Symbols`
table and `ext` (a typed map for whatever the code around it keeps).

- `call(func, args)` places arguments by the CPU's `Abi` (registers, then
  the stack), plants `return_to` as the return address, runs to it and
  returns the return register. Pass a signed argument sign-extended to 64
  bits.
- `hook(addr, f)` runs host code instead of the function at `addr`:
  `HookResult::Return(v)` returns to the caller, `Continue` runs the
  original after all, `Stop` stops. `hook_symbol(name, f)` hooks by name;
  `stub(addr, v)`. A hook receives a `Call`: `cpu`, `bus`, `heap`, and
  `arg(i)`/`arg32(i)`/`cstr(i)` read through the convention.
- `on_syscall(f)` adds a system call handler. Handlers are asked newest
  first; `Continue` passes a call on; a call none takes stops with
  `Stop::Syscall`.
- `step_limit`, `steps`, `cycles`, `trace`.

`hle::libc(machine)` hooks, by symbol, `memcpy`, `memmove`, `memset`,
`memcmp`, `strlen`, `strcpy`, `strcat`, `strcmp`, `strncmp` (comparisons
return newlib's first-byte difference), `malloc`/`free` on the machine's
heap and `printf` (0), on any CPU; `hle::install(machine, addr, name)` for a
symbol under another name.

## The oracle

`call_recorded(func, args)` returns an `Outcome`: the return value, every
run of bytes whose value the call changed (`Change { addr, before, after }`,
at canonical addresses; a byte rewritten with its old value is not a
change), the steps taken and every register after. `Outcome::wrote` and
`report` help compare it with what the port computed.

`check(addr, entry)` watches every call of a function while the original
runs whole: `entry` sees the machine as the function is entered and returns
the check to run at its return; results collect in `CheckStats` (passed,
by function, failed with what differed). This is hwtr's lockstep pattern.

## Loading

`PsxExe::parse`/`load` (the PS-X EXE header and image) and `Elf::parse`/
`load` (32-bit ELF of either byte order: `PT_LOAD` segments, bss zeroed,
`.symtab` into `Symbols`). `Symbols::parse` reads a text map (`ADDRESS NAME`
a line).

## PlayStation: `psx`

`R3000`: the R3000A with its load and branch delay slots, the unaligned
`lwl`/`lwr`/`swl`/`swr` merges, the divide-by-zero results, overflow traps,
COP0 as registers and the GTE as COP2 - every instruction hwtr's interpreter
runs, from which it was ported. A fault other than a system call leaves the
instruction unexecuted. Registers: 0-31 the GPRs (`zero`..`ra`), 32 `hi`, 33
`lo`, 34-65 GTE data, 66-97 GTE control, 98-129 COP0. `Gte` (public, for
ports doing the GTE's arithmetic bit-exactly) is hwtr's, line for line.

`bios`: the BIOS function tables (code jumps to 0xa0, 0xb0 or 0xc0 with the
function in t1). `Bios` holds hwtr's set - `memcpy`, `memset`, `strcmp`,
`strcpy`, `strlen`, `printf`, `FlushCache`, the event and interrupt
plumbing, the memory card stubs, `GetC0Table`/`GetB0Table`, the GPU port
calls, `malloc`/`free`/`InitHeap` - and a game adds or replaces entries
with `insert(table, function, f)`; an unknown entry stops with `Stop::Hle`
naming it. `critical_sections` answers the Enter/ExitCriticalSection system
calls.

`psx::machine()` is an `R3000` on `Memory::psx()`, stack at 0x801f_ff00,
heap 0x8018_0000 (512 KB), BIOS tables hooked (the `Bios` is in `ext`),
critical sections answered; `machine_with_exe` loads an executable and sets
gp from its header.

## PlayStation 2: `ee`

`Ee`: the R5900 ported from piney_apples' interpreter - 128-bit GPRs, hi/lo
and hi1/lo1, branch-likely, 64-bit operations, `lq`/`sq`, COP1 with the
EE's float rules, and behind `Features`: `vu` (COP2 macro mode, `lqc2`/
`sqc2`, `ldl`/`ldr`, twelve MMI instructions), `vi` (VU0's integer
registers and data memory at 0x1100_4000 on the bus), `ee_div` (the EE's
divide-by-zero results). Registers: 0-31 GPRs, 32-36 `hi lo hi1 lo1 sa`,
37-68 `f0`-`f31`, 69 `fcr31`, 70 `acc`, and with `vu` 71-102 `vf0`-`vf31`,
103 `vacc`, 104 `q`, 105-120 `vi0`-`vi15`. EABI: arguments in a0-a7.

`ee::float` gives ports the EE's single-precision arithmetic bit for bit on
`u32` patterns: `add`, `sub`, `mul`, `div`, `madd`, `msub`, `sqrt`, `rsqrt`,
`max`, `min`, conversions and comparisons, and newlib's `sqrtf`. The EE
truncates toward zero, has no infinity (overflow clamps to the largest
float) and flushes denormals to zero; each function's doc says which rule
it follows.

`ee::kernel::install` stubs, by symbol name, the kernel and library
functions piney_apples' harness stubs: `CreateSema`, `DeleteSema`,
`SignalSema`, `WaitSema`, `iSignalSema`, `DIntr`, `EIntr` return 1;
`FlushCache` and the `sceGs*`/`sceDma*` set return 0. No syscall numbers are
assumed. `ee::machine(features)` is an `Ee` on `Memory::ps2()` plus VU0 data
memory, stack at 0x01ff_f000, heap 0x0100_0000-0x01f0_0000 (16-byte
blocks); `machine_with_elf` loads an ELF, copies its symbols, sets gp from
`_gp`, and installs the kernel stubs and `hle::libc`.

## SPU: `spu`

`rrt_kit::adpcm::spu` re-exported: PS-ADPCM and the SPU's Gaussian
interpolation, for the reference's own sound model. A port reaches them
through `rrt::kit::adpcm` at play time; see the rrt-kit page.

## Tests

77 in the crate. The harness on a made-up CPU in both conventions:
`calls_pass_register_and_stack_arguments_in_either_convention`,
`hooks_and_syscalls_stand_in_for_code`, `syscall_handlers_chain_newest_first`,
`the_oracle_records_what_a_call_changed`,
`a_shadow_check_compares_each_call_at_entry_and_return`,
`a_runaway_call_hits_the_step_limit`, `extensions_hold_one_value_per_type`.
Memory: mirrors and folding, every width, either byte order, the journal's
canonical addresses, a shared bus. Loaders: a PS-X EXE and an ELF of each
byte order with symbols. PS1 (27): hwtr's CPU and GTE tests ported, calls
with five arguments, overflow, system calls, BIOS calls through the PsyQ
stub, an unknown BIOS function, a game-supplied one, the BIOS heap, a hook
not firing in a delay slot, `call_recorded`. PS2 (28): the float rules,
delay slots and branch-likely, 128-bit loads and stores, MMI lanes, VU0
macro instructions through the bus, the kernel stubs by name, an ELF
machine, a hook not firing in a delay slot. The generic C library HLE
(3). The SPU format's tests moved with it to rrt-kit.

## Not here

A whole-console emulator: no GPU, SPU voices, DMA or interrupts are run -
the oracle calls functions, it does not boot games (hwtr-hle's vblank and
device scheduling stay in hwtr). Exceptions do not vector; they stop the
run. No savestates of the machine.

## Gaps

- The EE interprets exactly the instruction set piney_apples' harness
  needed. `lwl`, `lwr`, `swl`, `swr`, `sdl`, `sdr`, `mfsa`, `mtsa`, `qfsrv`
  and the MMI instructions beyond the twelve above stop with
  `Stop::Reserved`. They were not added without a reference to test them
  against.
- piney's harness zeroed the GPRs and set gp before every call;
  `Machine::call` sets neither, so `machine_with_elf` sets gp once.
