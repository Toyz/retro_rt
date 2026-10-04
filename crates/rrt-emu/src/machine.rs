//! A CPU, its memory and the harness around them: calls, hooks, system
//! calls, shadow checks and the oracle's record of what a call changed.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};

use crate::bus::{Bus, BusExt, Memory};
use crate::cpu::{Cpu, ReturnAddress, Stop};
use crate::heap::Heap;
use crate::symbols::Symbols;

/// What a hook or a system call handler sees: the CPU, the bus and the HLE
/// heap, with the arguments read through the CPU's calling convention.
pub struct Call<'a, C: ?Sized, B: ?Sized> {
    /// The CPU, settled (no load in flight).
    pub cpu: &'a mut C,
    /// Its memory.
    pub bus: &'a mut B,
    /// The heap HLE `malloc` and `free` use.
    pub heap: &'a mut Heap,
}

impl<C: Cpu + ?Sized, B: Bus + ?Sized> Call<'_, C, B> {
    /// Argument `i` (from 0) as the convention passes it: in a register, or
    /// on the stack past the registers. A stack argument that cannot be read
    /// is 0.
    pub fn arg(&mut self, i: usize) -> u64 {
        let abi = self.cpu.abi();
        if let Some(&r) = abi.args.get(i) {
            return self.cpu.reg(r) as u64;
        }
        let k = (i - abi.args.len()) as u32;
        let sp = self.cpu.reg(abi.sp) as u32;
        let at = sp.wrapping_add(abi.stack_args_offset).wrapping_add(k * abi.stack_slot);
        let endian = self.cpu.endian();
        self.bus.read_uint(at, abi.stack_slot.min(8) as usize, endian).unwrap_or(0)
    }

    /// Argument `i`'s low 32 bits: a pointer, an `int`.
    pub fn arg32(&mut self, i: usize) -> u32 {
        self.arg(i) as u32
    }

    /// The NUL-terminated string argument `i` points at ("" when unreadable).
    pub fn cstr(&mut self, i: usize) -> String {
        let at = self.arg32(i);
        self.bus.cstr(at).unwrap_or_default()
    }
}

/// Values of any type a [`Machine`] carries for the code built around it:
/// a console preset keeps its HLE tables here (`psx::machine` keeps its
/// BIOS table), a test its fixtures. One value per type.
#[derive(Default)]
pub struct Extensions(HashMap<TypeId, Box<dyn Any>>);

impl Extensions {
    /// Stores `value`, replacing one of the same type.
    pub fn insert<T: 'static>(&mut self, value: T) {
        self.0.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// The stored value of type `T`.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.0.get(&TypeId::of::<T>()).and_then(|b| b.downcast_ref())
    }

    /// The stored value of type `T`, mutably.
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.0.get_mut(&TypeId::of::<T>()).and_then(|b| b.downcast_mut())
    }

    /// Takes out the stored value of type `T`.
    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.0.remove(&TypeId::of::<T>()).and_then(|b| b.downcast().ok()).map(|b| *b)
    }
}

/// What a hook decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookResult {
    /// The function is done: this is its return value, and the call returns
    /// to its caller without running the original code.
    Return(u64),
    /// From a hook: run the original code after all (a hook that only
    /// watches or logs). From a system call handler: not mine - the next
    /// handler is asked, and with none left the run stops with
    /// [`Stop::Syscall`].
    Continue,
    /// Stop the run with this.
    Stop(Stop),
}

impl HookResult {
    /// A 32-bit `int` return, sign-extended as a 64-bit register holds one
    /// (the EE sign-extends 32-bit results).
    pub fn ret32(v: u32) -> HookResult {
        HookResult::Return(v as i32 as i64 as u64)
    }
}

/// Host code run instead of the function at an address.
pub type Hook<C, B> = Box<dyn FnMut(&mut Call<'_, C, B>) -> HookResult>;

/// Host code answering a system call: its code as the instruction carries
/// it; the number a console's kernel switches on is usually in a register
/// (the EE's `v1`), read through `Call::cpu`.
pub type SyscallHook<C, B> = Box<dyn FnMut(&mut Call<'_, C, B>, u32) -> HookResult>;

/// What a shadow check does when the function it watches returns: given the
/// machine then, `Err` with what differs.
pub type CheckExit<C, B> = Box<dyn FnOnce(&C, &mut B) -> Result<(), String>>;

/// A shadow check, run as a function is entered (the original still runs):
/// sees the machine at entry and returns what to check at the return.
pub type Check<C, B> = Box<dyn FnMut(&C, &mut B) -> CheckExit<C, B>>;

/// Shadow checks so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckStats {
    /// Calls whose check passed.
    pub passed: u64,
    /// Calls passed, by function address.
    pub by_function: BTreeMap<u32, u64>,
    /// (function, what differed), for each that failed.
    pub failed: Vec<(u32, String)>,
}

struct OpenCheck<C, B> {
    func: u32,
    ret: u32,
    sp: u32,
    exit: CheckExit<C, B>,
}

/// Bytes a call changed: where, what they were, what they became.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The first byte's address.
    pub addr: u32,
    /// The bytes before the call.
    pub before: Vec<u8>,
    /// The bytes after.
    pub after: Vec<u8>,
}

/// What a call of an original function did: the oracle's answer, for a port
/// to be compared with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The return value register.
    pub ret: u64,
    /// Every run of bytes whose value the call changed, in canonical address
    /// order ([`Machine::canonical`]: PS1 RAM is reported from 0, whichever
    /// segment wrote it). Bytes written with the value they already held are
    /// not changes.
    pub changes: Vec<Change>,
    /// Instructions executed.
    pub steps: u64,
    /// Every register after the call.
    pub regs: Vec<(&'static str, u128)>,
}

impl Outcome {
    /// Whether any byte of `addr..addr + len` changed. Addresses are
    /// canonical ([`Machine::canonical`]): fold an alias first.
    pub fn wrote(&self, addr: u32, len: u32) -> bool {
        self.changes.iter().any(|c| c.addr < addr.wrapping_add(len) && addr < c.addr + c.before.len() as u32)
    }

    /// The changes as text, one run a line: address, before, after.
    pub fn report(&self) -> String {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let mut s = format!("returned {:#x} after {} steps\n", self.ret, self.steps);
        for c in &self.changes {
            s.push_str(&format!("{:#010x}: {} -> {}\n", c.addr, hex(&c.before), hex(&c.after)));
        }
        s
    }
}

/// A CPU and its bus with the harness: HLE hooks and system calls, calls
/// into the original's functions, shadow checks, and the oracle.
pub struct Machine<C, B = Memory> {
    /// The CPU.
    pub cpu: C,
    /// Its memory.
    pub bus: B,
    /// The heap HLE `malloc` uses.
    pub heap: Heap,
    /// Names for addresses, for hooking by name.
    pub symbols: Symbols,
    /// Instructions [`Machine::run`] may execute before stopping with
    /// [`Stop::StepLimit`]. 50 million by default.
    pub step_limit: u64,
    /// Instructions executed by the last call or run.
    pub steps: u64,
    /// Cycles the CPU reported for them.
    pub cycles: u64,
    /// Where [`Machine::call`] puts the stack.
    pub stack_top: u32,
    /// The return address [`Machine::call`] plants; reaching it ends the
    /// call. Never executed, so it need not be mapped; pick one the code
    /// cannot reach (the default, 0xffff_fff0, suits 32-bit CPUs).
    pub return_to: u32,
    /// Addresses executed, in order, when set.
    pub trace: Option<Vec<u32>>,
    /// Shadow check results.
    pub check_stats: CheckStats,
    /// Values the code around the machine keeps with it.
    pub ext: Extensions,
    hooks: HashMap<u32, Hook<C, B>>,
    syscalls: Vec<SyscallHook<C, B>>,
    checks: HashMap<u32, Check<C, B>>,
    open_checks: Vec<OpenCheck<C, B>>,
}

impl<C: Cpu, B: Bus> Machine<C, B> {
    /// `cpu` over `bus`, with an empty heap and symbol table, the stack at
    /// 0x0010_0000 (set [`Machine::stack_top`] for the console).
    pub fn new(cpu: C, bus: B) -> Machine<C, B> {
        Machine {
            cpu,
            bus,
            heap: Heap::new(0, 0, 8),
            symbols: Symbols::default(),
            step_limit: 50_000_000,
            steps: 0,
            cycles: 0,
            stack_top: 0x0010_0000,
            return_to: 0xffff_fff0,
            trace: None,
            check_stats: CheckStats::default(),
            ext: Extensions::default(),
            hooks: HashMap::new(),
            syscalls: Vec::new(),
            checks: HashMap::new(),
            open_checks: Vec::new(),
        }
    }

    /// Runs `hook` instead of the function at `addr` (or before it, when the
    /// hook returns [`HookResult::Continue`]).
    pub fn hook(&mut self, addr: u32, hook: impl FnMut(&mut Call<'_, C, B>) -> HookResult + 'static) {
        self.hooks.insert(addr, Box::new(hook));
    }

    /// Hooks the function the symbol table calls `name`.
    pub fn hook_symbol(
        &mut self,
        name: &str,
        hook: impl FnMut(&mut Call<'_, C, B>) -> HookResult + 'static,
    ) -> Result<u32, String> {
        let addr = self.symbols.addr(name).ok_or_else(|| format!("no symbol {name}"))?;
        self.hook(addr, hook);
        Ok(addr)
    }

    /// Makes the function at `addr` return `v` without running.
    pub fn stub(&mut self, addr: u32, v: u64) {
        self.hook(addr, move |_| HookResult::Return(v));
    }

    /// Removes a hook.
    pub fn unhook(&mut self, addr: u32) {
        self.hooks.remove(&addr);
    }

    /// Whether the function at `addr` is hooked.
    pub fn is_hooked(&self, addr: u32) -> bool {
        self.hooks.contains_key(&addr)
    }

    /// Adds a system call handler. Handlers are asked newest first; one that
    /// returns [`HookResult::Continue`] passes the call on, so a game can add
    /// its own on top of a console preset's. A call no handler takes stops
    /// the run with [`Stop::Syscall`].
    pub fn on_syscall(&mut self, handler: impl FnMut(&mut Call<'_, C, B>, u32) -> HookResult + 'static) {
        self.syscalls.push(Box::new(handler));
    }

    /// The canonical address of `addr` on this bus ([`Bus::canonical`]): the
    /// address an [`Outcome`] reports a change at.
    pub fn canonical(&self, addr: u32) -> u32 {
        self.bus.canonical(addr)
    }

    /// Checks every call of the function at `addr` while the original runs:
    /// `check` sees the machine at entry, and what it returns sees it at the
    /// return. Results collect in [`Machine::check_stats`].
    pub fn check(&mut self, addr: u32, check: impl FnMut(&C, &mut B) -> CheckExit<C, B> + 'static) {
        self.checks.insert(addr, Box::new(check));
    }

    /// Calls the function at `func` with `args` (registers first, then the
    /// stack, as the CPU's convention says) and runs until it returns.
    /// Returns the return value register. Arguments are written as given:
    /// pass a signed one sign-extended to 64 bits (`-1i64 as u64`), as a
    /// 64-bit register holds it; a 32-bit CPU keeps the low half.
    pub fn call(&mut self, func: u32, args: &[u64]) -> Result<u64, Stop> {
        let abi = self.cpu.abi();
        let endian = self.cpu.endian();
        let stacked = args.len().saturating_sub(abi.args.len()) as u32;
        let ra_bytes = match abi.return_address {
            ReturnAddress::Stack { bytes } => u32::from(bytes),
            ReturnAddress::Register(_) => 0,
        };
        let frame = abi.stack_args_offset.max(ra_bytes) + stacked * abi.stack_slot;
        let sp = self.stack_top.wrapping_sub(frame) & !(abi.stack_align.max(1) - 1);
        for (i, &a) in args.iter().enumerate() {
            match abi.args.get(i) {
                Some(&r) => self.cpu.set_reg(r, u128::from(a)),
                None => {
                    let k = (i - abi.args.len()) as u32;
                    let at = sp + abi.stack_args_offset + k * abi.stack_slot;
                    self.bus
                        .write_uint(at, abi.stack_slot.min(8) as usize, endian, a)
                        .map_err(|addr| Stop::Bus { pc: func, addr })?;
                }
            }
        }
        self.cpu.set_reg(abi.sp, u128::from(sp));
        match abi.return_address {
            ReturnAddress::Register(r) => self.cpu.set_reg(r, u128::from(self.return_to)),
            ReturnAddress::Stack { bytes } => self
                .bus
                .write_uint(sp, bytes as usize, endian, u64::from(self.return_to))
                .map_err(|addr| Stop::Bus { pc: func, addr })?,
        }
        self.cpu.set_pc(func);
        self.steps = 0;
        self.cycles = 0;
        // Checks a stopped call left open never close.
        self.open_checks.clear();
        self.run()?;
        self.cpu.settle();
        Ok(self.cpu.reg(abi.ret) as u64)
    }

    /// [`Machine::call`], recording every byte the call changes: the
    /// oracle's answer for the function on these inputs.
    pub fn call_recorded(&mut self, func: u32, args: &[u64]) -> Result<Outcome, Stop> {
        self.bus.start_journal();
        let result = self.call(func, args);
        let journal = self.bus.take_journal().unwrap_or_default();
        let ret = result?;
        // The earliest value each byte had, then its value now.
        let mut before: BTreeMap<u32, u8> = BTreeMap::new();
        for (addr, bytes) in journal {
            for (k, b) in bytes.into_iter().enumerate() {
                before.entry(addr.wrapping_add(k as u32)).or_insert(b);
            }
        }
        let mut changes: Vec<Change> = Vec::new();
        for (addr, old) in before {
            let new = self.bus.read_u8(addr).unwrap_or(old);
            if new == old {
                continue;
            }
            match changes.last_mut() {
                Some(c) if c.addr + c.before.len() as u32 == addr => {
                    c.before.push(old);
                    c.after.push(new);
                }
                _ => changes.push(Change { addr, before: vec![old], after: vec![new] }),
            }
        }
        Ok(Outcome { ret, changes, steps: self.steps, regs: self.cpu.regs() })
    }

    /// Returns from the current function as the convention does: to the
    /// return address register, or popping it from the stack.
    fn return_to_caller(&mut self) -> Result<(), Stop> {
        let abi = self.cpu.abi();
        match abi.return_address {
            ReturnAddress::Register(r) => {
                let ra = self.cpu.reg(r) as u32;
                self.cpu.set_pc(ra);
            }
            ReturnAddress::Stack { bytes } => {
                let sp = self.cpu.reg(abi.sp) as u32;
                let pc = self.cpu.pc();
                let ra =
                    self.bus.read_uint(sp, bytes as usize, self.cpu.endian()).map_err(|addr| Stop::Bus { pc, addr })?;
                self.cpu.set_reg(abi.sp, u128::from(sp.wrapping_add(u32::from(bytes))));
                self.cpu.set_pc(ra as u32);
            }
        }
        Ok(())
    }

    /// Where the function just entered returns to, and the stack pointer
    /// its caller will have then.
    fn return_point(&mut self) -> (u32, u32) {
        let abi = self.cpu.abi();
        let sp = self.cpu.reg(abi.sp) as u32;
        match abi.return_address {
            ReturnAddress::Register(r) => (self.cpu.reg(r) as u32, sp),
            ReturnAddress::Stack { bytes } => {
                let ra = self.bus.read_uint(sp, bytes as usize, self.cpu.endian()).unwrap_or(0) as u32;
                (ra, sp.wrapping_add(u32::from(bytes)))
            }
        }
    }

    /// Runs until the program counter reaches [`Machine::return_to`], a
    /// stop, or the step limit.
    pub fn run(&mut self) -> Result<(), Stop> {
        loop {
            let pc = self.cpu.pc();
            // Before the end test: the function `call` asked for may itself
            // be checked, and returning to the sentinel is its return.
            self.close_checks();
            if pc == self.return_to {
                return Ok(());
            }
            if self.steps >= self.step_limit {
                return Err(Stop::StepLimit { pc });
            }
            // An address reached as a branch's delay slot is not entered.
            let entered = !self.cpu.in_delay_slot();
            if let Some(check) = self.checks.get_mut(&pc).filter(|_| entered) {
                self.cpu.settle();
                let exit = check(&self.cpu, &mut self.bus);
                let (ret, sp) = self.return_point();
                self.open_checks.push(OpenCheck { func: pc, ret, sp, exit });
            }
            // Only taken out when it will run: removing it at a delay slot
            // would lose it for every later call.
            if let Some(mut hook) = entered.then(|| self.hooks.remove(&pc)).flatten() {
                self.cpu.settle();
                let result = hook(&mut Call { cpu: &mut self.cpu, bus: &mut self.bus, heap: &mut self.heap });
                self.hooks.insert(pc, hook);
                match result {
                    HookResult::Return(v) => {
                        let ret = self.cpu.abi().ret;
                        self.cpu.set_reg(ret, u128::from(v));
                        self.return_to_caller()?;
                        continue;
                    }
                    // The instruction at pc is stepped below, in this pass,
                    // so the hook does not run twice.
                    HookResult::Continue => {}
                    HookResult::Stop(s) => return Err(s),
                }
            }
            if let Some(t) = &mut self.trace {
                t.push(pc);
            }
            match self.cpu.step(&mut self.bus) {
                Ok(cycles) => {
                    self.steps += 1;
                    self.cycles += u64::from(cycles);
                }
                Err(Stop::Syscall { pc, code }) => {
                    self.steps += 1;
                    self.cpu.settle();
                    let mut handlers = std::mem::take(&mut self.syscalls);
                    let mut result = HookResult::Continue;
                    for h in handlers.iter_mut().rev() {
                        result = h(&mut Call { cpu: &mut self.cpu, bus: &mut self.bus, heap: &mut self.heap }, code);
                        if result != HookResult::Continue {
                            break;
                        }
                    }
                    // Handlers added while running stay, after the others.
                    handlers.append(&mut self.syscalls);
                    self.syscalls = handlers;
                    match result {
                        HookResult::Return(v) => {
                            let ret = self.cpu.abi().ret;
                            self.cpu.set_reg(ret, u128::from(v));
                        }
                        HookResult::Continue => return Err(Stop::Syscall { pc, code }),
                        HookResult::Stop(s) => return Err(s),
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Runs the exit checks of functions that have just returned.
    fn close_checks(&mut self) {
        while let Some(open) = self.open_checks.last() {
            let sp = self.cpu.reg(self.cpu.abi().sp) as u32;
            if self.cpu.pc() != open.ret || sp != open.sp {
                break;
            }
            let open = self.open_checks.pop().unwrap();
            self.cpu.settle();
            match (open.exit)(&self.cpu, &mut self.bus) {
                Ok(()) => {
                    self.check_stats.passed += 1;
                    *self.check_stats.by_function.entry(open.func).or_default() += 1;
                }
                Err(e) => self.check_stats.failed.push((open.func, e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{BusExt, Memory, Region};
    use crate::cpu::{Abi, Endian};

    /// A made-up CPU, to test the harness without any real architecture:
    /// r0-r5, r6 the stack pointer, r7 the link register. Each instruction
    /// is a little-endian word [op, a, b, c], some followed by a 32-bit
    /// operand.
    ///
    /// ```text
    /// 1 ADD d s t     r[d] = r[s] + r[t]
    /// 2 ARG d k       r[d] = stack argument k
    /// 3 STORE s; A    mem[A] = r[s]
    /// 4 CALL; A       call A (return address to r7, or pushed)
    /// 5 RET           return (from r7, or popped)
    /// 6 SYSCALL code
    /// 7 PUSHLR        push r7 (register mode only; a no-op on the stack)
    /// 8 POPLR         pop r7
    /// 9 JUMP; A       pc = A
    /// ```
    struct Toy {
        r: [u32; 8],
        pc: u32,
        stack_ra: bool,
    }

    const NAMES: [&str; 8] = ["r0", "r1", "r2", "r3", "r4", "r5", "sp", "lr"];

    impl Cpu for Toy {
        fn name(&self) -> &'static str {
            "toy"
        }
        fn endian(&self) -> Endian {
            Endian::Little
        }
        fn abi(&self) -> Abi {
            Abi {
                args: &[0, 1, 2],
                ret: 0,
                sp: 6,
                return_address: if self.stack_ra {
                    ReturnAddress::Stack { bytes: 4 }
                } else {
                    ReturnAddress::Register(7)
                },
                stack_args_offset: if self.stack_ra { 4 } else { 0 },
                stack_slot: 4,
                stack_align: 4,
            }
        }
        fn step(&mut self, bus: &mut dyn Bus) -> Result<u32, Stop> {
            let pc = self.pc;
            let w = bus.read_u32(pc).map_err(|addr| Stop::Bus { pc, addr })?.to_le_bytes();
            let operand = |bus: &mut dyn Bus| bus.read_u32(pc + 4).map_err(|addr| Stop::Bus { pc, addr });
            let (a, b, c) = (w[1] as usize, w[2] as usize, w[3] as usize);
            let sp = self.r[6];
            self.pc = pc + 4;
            match w[0] {
                1 => self.r[a] = self.r[b].wrapping_add(self.r[c]),
                2 => {
                    let off = self.abi().stack_args_offset;
                    self.r[a] = bus.read_u32(sp + off + 4 * b as u32).map_err(|addr| Stop::Bus { pc, addr })?;
                }
                3 => {
                    let at = operand(bus)?;
                    bus.write_u32(at, self.r[a]).map_err(|addr| Stop::Bus { pc, addr })?;
                    self.pc = pc + 8;
                }
                4 => {
                    let target = operand(bus)?;
                    if self.stack_ra {
                        self.r[6] = sp - 4;
                        bus.write_u32(sp - 4, pc + 8).map_err(|addr| Stop::Bus { pc, addr })?;
                    } else {
                        self.r[7] = pc + 8;
                    }
                    self.pc = target;
                }
                5 => {
                    if self.stack_ra {
                        self.pc = bus.read_u32(sp).map_err(|addr| Stop::Bus { pc, addr })?;
                        self.r[6] = sp + 4;
                    } else {
                        self.pc = self.r[7];
                    }
                }
                6 => return Err(Stop::Syscall { pc, code: a as u32 }),
                7 if !self.stack_ra => {
                    self.r[6] = sp - 4;
                    bus.write_u32(sp - 4, self.r[7]).map_err(|addr| Stop::Bus { pc, addr })?;
                }
                8 if !self.stack_ra => {
                    self.r[7] = bus.read_u32(sp).map_err(|addr| Stop::Bus { pc, addr })?;
                    self.r[6] = sp + 4;
                }
                7 | 8 => {}
                9 => self.pc = operand(bus)?,
                _ => return Err(Stop::Reserved { pc, word: u64::from(u32::from_le_bytes(w)) }),
            }
            Ok(1)
        }
        fn pc(&self) -> u32 {
            self.pc
        }
        fn set_pc(&mut self, pc: u32) {
            self.pc = pc;
        }
        fn reg_count(&self) -> usize {
            8
        }
        fn reg(&self, i: usize) -> u128 {
            u128::from(self.r[i])
        }
        fn set_reg(&mut self, i: usize, v: u128) {
            self.r[i] = v as u32;
        }
        fn reg_name(&self, i: usize) -> &'static str {
            NAMES[i]
        }
    }

    const F: u32 = 0x100;
    const G: u32 = 0x200;
    const H: u32 = 0x300;
    const OUT: u32 = 0x2000;

    /// f(a, b, c, d) = a + b + c + d, stored at OUT and returned (d on the
    /// stack); g() calls h (hooked), then syscall 7, and returns r0; k() calls
    /// f(1, 2, 3, 4) through the stack.
    fn machine(stack_ra: bool) -> Machine<Toy, Memory> {
        let mut mem = Memory::new();
        mem.regions.push(Region::ram("ram", 0, 0x1_0000, 0x1_0000, u32::MAX));
        let mut m = Machine::new(Toy { r: [0; 8], pc: 0, stack_ra }, mem);
        m.stack_top = 0x8000;
        let mut put = |at: u32, words: &[u32]| {
            for (i, w) in words.iter().enumerate() {
                m.bus.write_u32(at + 4 * i as u32, *w).unwrap();
            }
        };
        let op = |o: u8, a: u8, b: u8, c: u8| u32::from_le_bytes([o, a, b, c]);
        put(F, &[op(2, 3, 0, 0), op(1, 0, 0, 1), op(1, 0, 0, 2), op(1, 0, 0, 3), op(3, 0, 0, 0), OUT, op(5, 0, 0, 0)]);
        put(G, &[op(7, 0, 0, 0), op(4, 0, 0, 0), H, op(6, 7, 0, 0), op(8, 0, 0, 0), op(5, 0, 0, 0)]);
        put(H, &[op(9, 0, 0, 0), H]);
        m.symbols.insert("h", H);
        m
    }

    #[test]
    fn calls_pass_register_and_stack_arguments_in_either_convention() {
        for stack_ra in [false, true] {
            let mut m = machine(stack_ra);
            assert_eq!(m.call(F, &[1, 2, 3, 4]), Ok(10), "stack_ra {stack_ra}");
            assert_eq!(m.bus.read_u32(OUT), Ok(10));
        }
    }

    #[test]
    fn hooks_and_syscalls_stand_in_for_code() {
        for stack_ra in [false, true] {
            let mut m = machine(stack_ra);
            m.hook_symbol("h", |_| HookResult::Return(5)).unwrap();
            m.on_syscall(|c, code| {
                let r0 = c.cpu.reg(0) as u64;
                HookResult::Return(r0 + u64::from(code))
            });
            assert_eq!(m.call(G, &[]), Ok(12), "h gave 5, syscall 7 added 7; stack_ra {stack_ra}");
            assert!(m.hook_symbol("nope", |_| HookResult::Continue).is_err());
        }
    }

    /// Handlers are asked newest first; Continue passes the call on.
    #[test]
    fn syscall_handlers_chain_newest_first() {
        let mut m = machine(false);
        m.stub(H, 1);
        m.on_syscall(|_, code| HookResult::Return(u64::from(code) * 100));
        m.on_syscall(|_, code| if code == 7 { HookResult::Continue } else { HookResult::Return(0) });
        assert_eq!(m.call(G, &[]), Ok(700), "the newer passed 7 on to the older");
        m.on_syscall(|_, _| HookResult::Return(5));
        assert_eq!(m.call(G, &[]), Ok(5), "the newest answers first");
    }

    #[test]
    fn extensions_hold_one_value_per_type() {
        let mut m = machine(false);
        m.ext.insert(41u32);
        *m.ext.get_mut::<u32>().unwrap() += 1;
        m.ext.insert("table");
        assert_eq!((m.ext.get::<u32>(), m.ext.get::<&str>()), (Some(&42), Some(&"table")));
        assert_eq!(m.ext.remove::<u32>(), Some(42));
        assert_eq!(m.ext.get::<u32>(), None);
    }

    #[test]
    fn without_a_handler_a_syscall_stops_the_run() {
        let mut m = machine(false);
        m.stub(H, 0);
        assert_eq!(m.call(G, &[]), Err(Stop::Syscall { pc: G + 12, code: 7 }));
    }

    #[test]
    fn the_oracle_records_what_a_call_changed() {
        let mut m = machine(true);
        m.bus.write_u32(OUT, 10).unwrap();
        let o = m.call_recorded(F, &[1, 2, 3, 5]).unwrap();
        assert_eq!(o.ret, 11);
        let out = o.changes.iter().find(|c| c.addr == OUT).expect("OUT changed");
        assert_eq!((out.before.as_slice(), out.after.as_slice()), (&[10u8][..], &[11u8][..]));
        assert!(o.wrote(OUT, 4) && !o.wrote(OUT + 4, 4));
        assert!(o.report().contains("0a -> 0b"));
        let same = m.call_recorded(F, &[1, 2, 3, 5]).unwrap();
        assert!(!same.wrote(OUT, 4), "writing the value already there is no change");
    }

    #[test]
    fn a_shadow_check_compares_each_call_at_entry_and_return() {
        for stack_ra in [false, true] {
            let mut m = machine(stack_ra);
            m.check(F, |cpu, _| {
                let want = (0..3).map(|i| cpu.r[i]).sum::<u32>();
                Box::new(move |cpu, bus| {
                    let d = bus.read_u32(OUT).unwrap() - want;
                    if cpu.r[0] == want + d { Ok(()) } else { Err(format!("{} != {}", cpu.r[0], want + d)) }
                })
            });
            m.call(F, &[1, 2, 3, 4]).unwrap();
            m.call(F, &[5, 5, 5, 5]).unwrap();
            assert_eq!((m.check_stats.passed, m.check_stats.failed.len()), (2, 0), "stack_ra {stack_ra}");
            assert_eq!(m.check_stats.by_function[&F], 2);
        }
    }

    #[test]
    fn a_runaway_call_hits_the_step_limit() {
        let mut m = machine(false);
        m.step_limit = 100;
        assert_eq!(m.call(H, &[]), Err(Stop::StepLimit { pc: H }));
        assert_eq!(m.steps, 100);
    }
}
