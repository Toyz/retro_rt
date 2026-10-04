//! The PlayStation BIOS's function tables, high-level: the calls through
//! 0xa0, 0xb0 and 0xc0 answered on the host.
//!
//! On the PS1, code calls the BIOS by jumping to 0xa0, 0xb0 or 0xc0 with the
//! function number in t1 (r9), usually through a three-instruction stub
//! (`li t2, 0xa0; jr t2; li t1, n`). [`install`] hooks the three addresses
//! with a dispatcher on (table, t1 & 0xff) over a [`Bios`]: the functions
//! the PsyQ libraries reach that are pure enough to provide on the host, as
//! hwtr's harness provides them. A function the table lacks stops the run
//! with [`Stop::Hle`] naming it ("BIOS A0:2f not provided"); a game supplies
//! it with [`Bios::insert`].
//!
//! [`critical_sections`] answers the one system call pair PsyQ code makes
//! everywhere, EnterCriticalSection and ExitCriticalSection.
//!
//! Not here: the BIOS ROM, the kernel's exception handling and interrupts,
//! the CD-ROM, the memory card beyond "no card", and TTY output (printf
//! prints nothing).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::bus::{Bus, BusExt};
use crate::cpu::{Cpu, Stop};
use crate::heap::Heap;
use crate::machine::{Call, HookResult, Machine};

use super::R3000;

/// A BIOS function on the host: reads its arguments through the [`Call`]
/// and returns v0 (or stops the run).
pub type BiosFn<B> = Box<dyn FnMut(&mut Call<'_, R3000, B>) -> HookResult>;

/// The three table entry points.
pub const TABLES: [u32; 3] = [0xa0, 0xb0, 0xc0];

/// Where [`Bios::new`]'s GetC0Table (B0:56) says the C0 table is: in the
/// kernel's low RAM, where the real BIOS keeps it.
pub const C0_TABLE: u32 = 0x674;
/// Where [`Bios::new`]'s GetB0Table (B0:57) says the B0 table is.
pub const B0_TABLE: u32 = 0x874;
/// The first event id OpenEvent (B0:08) hands out; each call returns the
/// next, from `FIRST_EVENT + 1`.
pub const FIRST_EVENT: u32 = 0xf100_0000;
/// GP0, the GPU's command and data port.
pub const GP0: u32 = 0x1f80_1810;
/// GP1, the GPU's control port (writes) and status register (reads).
pub const GP1: u32 = 0x1f80_1814;

/// The BIOS functions a machine answers, by (table, function number): a
/// shared handle, so entries can be added or replaced after [`install`] and
/// the hooks see them.
pub struct Bios<B> {
    functions: Rc<RefCell<Table<B>>>,
}

/// The functions by (table, function number).
type Table<B> = HashMap<(u32, u32), BiosFn<B>>;

impl<B> Clone for Bios<B> {
    fn clone(&self) -> Self {
        Bios { functions: self.functions.clone() }
    }
}

/// A host-side error as the hook's answer: a bus error at `addr`.
fn bus_stop<B: ?Sized>(c: &Call<'_, R3000, B>, addr: u32) -> HookResult {
    HookResult::Stop(Stop::Bus { pc: c.cpu.pc(), addr })
}

/// The NUL-terminated bytes at `at`, raw (no UTF-8 decoding), at most 1 MB.
fn c_bytes<B: Bus + ?Sized>(c: &mut Call<'_, R3000, B>, at: u32) -> Result<Vec<u8>, HookResult> {
    let mut out = Vec::new();
    for i in 0..(1u32 << 20) {
        match c.bus.read_u8(at.wrapping_add(i)) {
            Ok(0) => return Ok(out),
            Ok(b) => out.push(b),
            Err(addr) => return Err(bus_stop(c, addr)),
        }
    }
    Err(HookResult::Stop(Stop::Hle { pc: c.cpu.pc(), what: format!("no string terminator within 1 MB of {at:#x}") }))
}

impl<B: Bus + 'static> Bios<B> {
    /// A table with no functions.
    pub fn empty() -> Bios<B> {
        Bios { functions: Rc::new(RefCell::new(HashMap::new())) }
    }

    /// hwtr's table: the functions the PsyQ libraries reach that can be
    /// answered on the host.
    ///
    /// | function | answer |
    /// |---|---|
    /// | A0:17 strcmp, A0:19 strcpy, A0:1b strlen | on the host, bytes as they are |
    /// | A0:2a memcpy, A0:2b memset | on the host; return the destination |
    /// | A0:33 malloc, A0:34 free, A0:39 InitHeap | on [`Machine::heap`] (InitHeap replaces it) |
    /// | A0:3f printf, B0:3f puts | print nothing, return 0 |
    /// | A0:44 FlushCache, A0:72 CdRemove, A0:70 _bu_init | 0 |
    /// | A0:48 SendGP1Command(word) | writes the word to GP1 (0x1f801814) |
    /// | A0:49 GPU_cw(word) | writes the word to GP0 (0x1f801810) |
    /// | A0:4a GPU_cwp(list, n) | writes n words from list to GP0 |
    /// | A0:4d GetGPUStatus | reads GP1 |
    /// | B0:08 OpenEvent | event ids from 0xf1000001 up |
    /// | B0:07, 09, 0a, 0b, 0c, 0d, 20 (DeliverEvent, CloseEvent, WaitEvent, TestEvent, EnableEvent, DisableEvent, UnDeliverEvent) | 1: every event is delivered |
    /// | B0:17 ReturnFromException | returns 0 to its caller |
    /// | B0:18 ResetEntryInt, B0:19 HookEntryInt, B0:5b ChangeClearPad | 0 |
    /// | C0:02 SysEnqIntRP, C0:03 SysDeqIntRP, C0:0a ChangeClearRCnt | 0 |
    /// | B0:4a InitCARD, B0:4b StartCARD, B0:4c StopCARD | 1 |
    /// | B0:32 open, B0:42 firstfile | -1: there is no memory card |
    /// | B0:33 lseek, 34 read, 35 write, 36 close, 43 nextfile, 4e _card_write, 4f _card_read, 50 _new_card | 0 |
    /// | B0:56 GetC0Table, B0:57 GetB0Table | 0x674, 0x874 |
    pub fn new() -> Bios<B> {
        let bios: Bios<B> = Bios::empty();
        let fixed = |bios: &Bios<B>, table: u32, functions: &[u32], v: u32| {
            for &f in functions {
                bios.insert(table, f, move |_| HookResult::Return(u64::from(v)));
            }
        };

        // strcmp(a, b)
        bios.insert(0xa0, 0x17, |c| {
            let (a, b) = (c.arg32(0), c.arg32(1));
            let x = match c_bytes(c, a) {
                Ok(x) => x,
                Err(stop) => return stop,
            };
            match c_bytes(c, b) {
                Ok(y) => HookResult::ret32(x.cmp(&y) as i32 as u32),
                Err(stop) => stop,
            }
        });
        // strcpy(dst, src)
        bios.insert(0xa0, 0x19, |c| {
            let (dst, src) = (c.arg32(0), c.arg32(1));
            let mut s = match c_bytes(c, src) {
                Ok(s) => s,
                Err(stop) => return stop,
            };
            s.push(0);
            match c.bus.write(dst, &s) {
                Ok(()) => HookResult::Return(u64::from(dst)),
                Err(addr) => bus_stop(c, addr),
            }
        });
        // strlen(s)
        bios.insert(0xa0, 0x1b, |c| {
            let s = c.arg32(0);
            match c_bytes(c, s) {
                Ok(s) => HookResult::Return(s.len() as u64),
                Err(stop) => stop,
            }
        });
        // memcpy(dst, src, n)
        bios.insert(0xa0, 0x2a, |c| {
            let (dst, src, n) = (c.arg32(0), c.arg32(1), c.arg32(2));
            let copied = c.bus.bytes(src, n as usize).and_then(|bytes| c.bus.write(dst, &bytes));
            match copied {
                Ok(()) => HookResult::Return(u64::from(dst)),
                Err(addr) => bus_stop(c, addr),
            }
        });
        // memset(dst, value, n)
        bios.insert(0xa0, 0x2b, |c| {
            let (dst, value, n) = (c.arg32(0), c.arg32(1), c.arg32(2));
            match c.bus.write(dst, &vec![value as u8; n as usize]) {
                Ok(()) => HookResult::Return(u64::from(dst)),
                Err(addr) => bus_stop(c, addr),
            }
        });
        // malloc(n), free(p), InitHeap(base, size)
        bios.insert(0xa0, 0x33, |c| {
            let n = c.arg32(0);
            HookResult::Return(u64::from(c.heap.alloc(n)))
        });
        bios.insert(0xa0, 0x34, |c| {
            let at = c.arg32(0);
            c.heap.free(at);
            HookResult::Return(0)
        });
        bios.insert(0xa0, 0x39, |c| {
            *c.heap = Heap::new(c.arg32(0), c.arg32(1), 8);
            HookResult::Return(0)
        });
        // printf and puts print nothing.
        fixed(&bios, 0xa0, &[0x3f], 0);
        fixed(&bios, 0xb0, &[0x3f], 0);
        // FlushCache, _bu_init (the memory card filing system), CdRemove
        fixed(&bios, 0xa0, &[0x44, 0x70, 0x72], 0);

        // The kernel's event and interrupt plumbing the libraries set up.
        // Events are handed out as ids and always report as delivered.
        let next_event = Rc::new(Cell::new(FIRST_EVENT));
        bios.insert(0xb0, 0x08, move |_| {
            next_event.set(next_event.get().wrapping_add(1));
            HookResult::Return(u64::from(next_event.get()))
        });
        // DeliverEvent, CloseEvent, WaitEvent, TestEvent, EnableEvent,
        // DisableEvent, UnDeliverEvent
        fixed(&bios, 0xb0, &[0x07, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20], 1);
        // ResetEntryInt, HookEntryInt, ChangeClearPad
        fixed(&bios, 0xb0, &[0x18, 0x19, 0x5b], 0);
        // SysEnqIntRP, SysDeqIntRP, ChangeClearRCnt
        fixed(&bios, 0xc0, &[0x02, 0x03, 0x0a], 0);
        // ReturnFromException never returns to its caller in a kernel; here
        // it just returns.
        fixed(&bios, 0xb0, &[0x17], 0);

        // The memory card: no card, every open fails.
        // InitCARD, StartCARD, StopCARD
        fixed(&bios, 0xb0, &[0x4a, 0x4b, 0x4c], 1);
        // open, firstfile
        fixed(&bios, 0xb0, &[0x32, 0x42], u32::MAX);
        // lseek, read, write, close, nextfile, _card_write, _card_read,
        // _new_card
        fixed(&bios, 0xb0, &[0x33, 0x34, 0x35, 0x36, 0x43, 0x4e, 0x4f, 0x50], 0);

        // GetC0Table, GetB0Table: zeros there; the libraries patch entries
        // in.
        fixed(&bios, 0xb0, &[0x56], C0_TABLE);
        fixed(&bios, 0xb0, &[0x57], B0_TABLE);

        // SendGP1Command, GPU_cw, GPU_cwp(list, n), GetGPUStatus: through
        // the ports, to whatever device is mapped there.
        bios.insert(0xa0, 0x48, |c| {
            let w = c.arg32(0);
            match c.bus.write_u32(GP1, w) {
                Ok(()) => HookResult::Return(0),
                Err(addr) => bus_stop(c, addr),
            }
        });
        bios.insert(0xa0, 0x49, |c| {
            let w = c.arg32(0);
            match c.bus.write_u32(GP0, w) {
                Ok(()) => HookResult::Return(0),
                Err(addr) => bus_stop(c, addr),
            }
        });
        bios.insert(0xa0, 0x4a, |c| {
            let (list, n) = (c.arg32(0), c.arg32(1));
            for k in 0..n {
                let sent = c.bus.read_u32(list.wrapping_add(4 * k)).and_then(|w| c.bus.write_u32(GP0, w));
                if let Err(addr) = sent {
                    return bus_stop(c, addr);
                }
            }
            HookResult::Return(0)
        });
        bios.insert(0xa0, 0x4d, |c| HookResult::Return(u64::from(c.bus.read_u32(GP1).unwrap_or(0))));
        bios
    }

    /// Answers BIOS function `function` of `table` (0xa0, 0xb0 or 0xc0)
    /// with `f`, replacing what answered it before.
    pub fn insert(&self, table: u32, function: u32, f: impl FnMut(&mut Call<'_, R3000, B>) -> HookResult + 'static) {
        self.functions.borrow_mut().insert((table, function), Box::new(f));
    }

    /// Stops answering a function: calling it then stops the run.
    pub fn remove(&self, table: u32, function: u32) {
        self.functions.borrow_mut().remove(&(table, function));
    }

    /// Whether the table answers `function` of `table`.
    pub fn provides(&self, table: u32, function: u32) -> bool {
        self.functions.borrow().contains_key(&(table, function))
    }

    /// Hooks 0xa0, 0xb0 and 0xc0 in `machine` with a dispatcher over this
    /// table, replacing any hooks there.
    pub fn hook(&self, machine: &mut Machine<R3000, B>) {
        for table in TABLES {
            let functions = self.functions.clone();
            machine.hook(table, move |c| {
                let function = c.cpu.r[9] & 0xff;
                // Taken out while it runs, so a handler that inserts into
                // the table (through its own handle) does not find it
                // borrowed.
                let Some(mut f) = functions.borrow_mut().remove(&(table, function)) else {
                    let ra = c.cpu.r[31];
                    return HookResult::Stop(Stop::Hle {
                        pc: c.cpu.pc(),
                        what: format!("BIOS {table:02X}:{function:02x} not provided (return address {ra:#010x})"),
                    });
                };
                let result = f(c);
                functions.borrow_mut().entry((table, function)).or_insert(f);
                result
            });
        }
    }
}

impl<B: Bus + 'static> Default for Bios<B> {
    fn default() -> Self {
        Bios::new()
    }
}

/// Hooks the BIOS tables in `machine` with [`Bios::new`]'s functions and
/// returns the table, for adding to it.
pub fn install<B: Bus + 'static>(machine: &mut Machine<R3000, B>) -> Bios<B> {
    let bios = Bios::new();
    bios.hook(machine);
    bios
}

/// Answers the kernel's critical-section system calls as PsyQ makes them
/// (`syscall` with a0 = 1 EnterCriticalSection, 2 ExitCriticalSection): v0
/// is whether interrupts were enabled before, and the returned flag tracks
/// whether they are now. Any other system call is passed on to the
/// machine's other handlers ([`Machine::on_syscall`]), and stops the run with
/// [`Stop::Syscall`] if none takes it.
pub fn critical_sections<B: Bus + 'static>(machine: &mut Machine<R3000, B>) -> Rc<Cell<bool>> {
    let enabled = Rc::new(Cell::new(true));
    let flag = enabled.clone();
    machine.on_syscall(move |c, code| match c.cpu.r[4] {
        a0 @ (1 | 2) => {
            let was = flag.replace(a0 == 2);
            HookResult::Return(u64::from(was))
        }
        _ => {
            let _ = code;
            HookResult::Continue
        }
    });
    enabled
}

#[cfg(test)]
mod tests {
    use crate::psx::machine;

    #[test]
    fn open_event_hands_out_ids_counting_up_from_0xf1000001() {
        let mut m = machine();
        // A function that is just the BIOS entry: call 0xb0 with t1 = 8.
        m.cpu.r[9] = 0x08;
        assert_eq!(m.call(0xb0, &[]), Ok(0xf100_0001));
        m.cpu.r[9] = 0x08;
        assert_eq!(m.call(0xb0, &[]), Ok(0xf100_0002));
    }
}
