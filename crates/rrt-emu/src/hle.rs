//! High-level stand-ins that work on any CPU: they read their arguments
//! through the CPU's calling convention, so one `memcpy` serves the PS1, the
//! PS2 and whatever comes next.
//!
//! [`libc`] installs the C string and memory functions and the heap
//! functions by symbol name - the set piney_apples' harness replaces,
//! because games link vectorised versions an interpreter runs slowly. A
//! function the symbol table does not name is skipped. Comparisons return
//! what newlib's do: the difference of the first differing bytes as
//! unsigned chars, not just its sign.

use crate::bus::{Bus, BusExt};
use crate::cpu::{Cpu, Stop};
use crate::machine::{Call, HookResult, Machine};

/// Reads the NUL-terminated string at `at`, or stops: an HLE string function
/// handed a pointer to no string.
fn string<C: Cpu + ?Sized, B: Bus + ?Sized>(c: &mut Call<'_, C, B>, at: u32) -> Result<Vec<u8>, HookResult> {
    let mut out = Vec::new();
    for i in 0..(1u32 << 20) {
        match c.bus.read_u8(at.wrapping_add(i)) {
            Ok(0) => return Ok(out),
            Ok(b) => out.push(b),
            Err(addr) => return Err(HookResult::Stop(Stop::Bus { pc: c.cpu.pc(), addr })),
        }
    }
    Err(HookResult::Stop(Stop::Hle { pc: c.cpu.pc(), what: format!("no terminator within 1 MB of {at:#x}") }))
}

/// Copies `n` bytes as `memmove` does (overlap-safe), or stops on a bus
/// error.
fn copy<C: Cpu + ?Sized, B: Bus + ?Sized>(c: &mut Call<'_, C, B>, dst: u32, src: u32, n: u32) -> Option<HookResult> {
    let pc = c.cpu.pc();
    let bytes = match c.bus.bytes(src, n as usize) {
        Ok(b) => b,
        Err(addr) => return Some(HookResult::Stop(Stop::Bus { pc, addr })),
    };
    c.bus.write(dst, &bytes).err().map(|addr| HookResult::Stop(Stop::Bus { pc, addr }))
}

/// A C comparison result as newlib's `strcmp` and `memcmp` give it: the
/// difference of the first bytes that differ, as unsigned chars (0 when none
/// do within `a` and `b`, which include a string's terminator).
fn diff(a: &[u8], b: &[u8]) -> HookResult {
    let d = a.iter().zip(b).find(|(x, y)| x != y).map_or(0, |(x, y)| i32::from(*x) - i32::from(*y));
    HookResult::ret32(d as u32)
}

/// The functions [`libc`] installs, by symbol name.
pub const LIBC: [&str; 12] = [
    "memcpy", "memmove", "memset", "memcmp", "strlen", "strcpy", "strcat", "strcmp", "strncmp", "malloc", "free",
    "printf",
];

/// Hooks every function in [`LIBC`] that `machine.symbols` names: string and
/// memory functions done on the host, `malloc` and `free` on
/// [`Machine::heap`], `printf` returning 0. Returns the names installed.
pub fn libc<C: Cpu + 'static, B: Bus + 'static>(machine: &mut Machine<C, B>) -> Vec<&'static str> {
    let mut installed = Vec::new();
    for name in LIBC {
        let Some(addr) = machine.symbols.addr(name) else { continue };
        install(machine, addr, name);
        installed.push(name);
    }
    installed
}

/// Hooks the C library function `name` (one of [`LIBC`]) at `addr`: for a
/// game whose symbol has another name (`_memcpy`, `ccMemcpy`).
///
/// # Panics
///
/// When `name` is not in [`LIBC`].
pub fn install<C: Cpu + 'static, B: Bus + 'static>(machine: &mut Machine<C, B>, addr: u32, name: &str) {
    match name {
        "memcpy" | "memmove" => machine.hook(addr, |c| {
            let (d, s, n) = (c.arg32(0), c.arg32(1), c.arg32(2));
            copy(c, d, s, n).unwrap_or(HookResult::ret32(d))
        }),
        "memset" => machine.hook(addr, |c| {
            let (d, v, n) = (c.arg32(0), c.arg32(1) as u8, c.arg32(2));
            let pc = c.cpu.pc();
            match c.bus.write(d, &vec![v; n as usize]) {
                Ok(()) => HookResult::ret32(d),
                Err(addr) => HookResult::Stop(Stop::Bus { pc, addr }),
            }
        }),
        "memcmp" => machine.hook(addr, |c| {
            let (x, y, n) = (c.arg32(0), c.arg32(1), c.arg32(2) as usize);
            let pc = c.cpu.pc();
            match (c.bus.bytes(x, n), c.bus.bytes(y, n)) {
                (Ok(a), Ok(b)) => diff(&a, &b),
                (Err(addr), _) | (_, Err(addr)) => HookResult::Stop(Stop::Bus { pc, addr }),
            }
        }),
        "strlen" => machine.hook(addr, |c| {
            let s = c.arg32(0);
            string(c, s).map_or_else(|stop| stop, |b| HookResult::ret32(b.len() as u32))
        }),
        "strcpy" => machine.hook(addr, |c| {
            let (d, s) = (c.arg32(0), c.arg32(1));
            match string(c, s) {
                Ok(b) => copy(c, d, s, b.len() as u32 + 1).unwrap_or(HookResult::ret32(d)),
                Err(stop) => stop,
            }
        }),
        "strcat" => machine.hook(addr, |c| {
            let (d, s) = (c.arg32(0), c.arg32(1));
            let end = match string(c, d) {
                Ok(b) => d + b.len() as u32,
                Err(stop) => return stop,
            };
            match string(c, s) {
                Ok(b) => copy(c, end, s, b.len() as u32 + 1).unwrap_or(HookResult::ret32(d)),
                Err(stop) => stop,
            }
        }),
        "strcmp" => machine.hook(addr, |c| {
            let (x, y) = (c.arg32(0), c.arg32(1));
            match (string(c, x), string(c, y)) {
                (Ok(mut a), Ok(mut b)) => {
                    a.push(0);
                    b.push(0);
                    diff(&a, &b)
                }
                (Err(stop), _) | (_, Err(stop)) => stop,
            }
        }),
        "strncmp" => machine.hook(addr, |c| {
            let (x, y, n) = (c.arg32(0), c.arg32(1), c.arg32(2) as usize);
            match (string(c, x), string(c, y)) {
                (Ok(mut a), Ok(mut b)) => {
                    a.push(0);
                    b.push(0);
                    diff(&a[..a.len().min(n)], &b[..b.len().min(n)])
                }
                (Err(stop), _) | (_, Err(stop)) => stop,
            }
        }),
        "malloc" => machine.hook(addr, |c| {
            let n = c.arg32(0);
            HookResult::ret32(c.heap.alloc(n))
        }),
        "free" => machine.hook(addr, |c| {
            let p = c.arg32(0);
            c.heap.free(p);
            HookResult::Return(0)
        }),
        "printf" => machine.hook(addr, |_| HookResult::Return(0)),
        other => panic!("{other} is not one of hle::LIBC"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ee;
    use crate::machine::Machine;

    /// The EE machine with every libc function given an address of its own
    /// and a call stub (`jr ra; nop` would never run: the hook answers).
    fn machine() -> Machine<ee::Ee, crate::Memory> {
        let mut m = ee::machine(ee::Features::default());
        for (i, name) in LIBC.iter().enumerate() {
            m.symbols.insert(*name, 0x0010_0000 + 0x100 * i as u32);
        }
        assert_eq!(libc(&mut m).len(), LIBC.len());
        m.bus.load(0x0020_0000, b"abc\0abd\0ab\0");
        m
    }

    fn at(name: &str) -> u32 {
        0x0010_0000 + 0x100 * LIBC.iter().position(|n| *n == name).unwrap() as u32
    }

    fn signed(v: u64) -> i64 {
        v as i64
    }

    #[test]
    fn comparisons_return_the_first_byte_difference_as_newlib_does() {
        let mut m = machine();
        let (abc, abd, ab) = (0x0020_0000u64, 0x0020_0004, 0x0020_0008);
        assert_eq!(signed(m.call(at("strcmp"), &[abc, abd]).unwrap()), i64::from(b'c') - i64::from(b'd'));
        assert_eq!(signed(m.call(at("strcmp"), &[abc, ab]).unwrap()), i64::from(b'c'), "against the terminator");
        assert_eq!(m.call(at("strcmp"), &[abc, abc]).unwrap(), 0);
        assert_eq!(m.call(at("strncmp"), &[abc, abd, 2]).unwrap(), 0);
        assert_eq!(signed(m.call(at("memcmp"), &[abd, abc, 3]).unwrap()), 1);
    }

    #[test]
    fn copies_lengths_and_the_heap() {
        let mut m = machine();
        assert_eq!(m.call(at("strlen"), &[0x0020_0000]).unwrap(), 3);
        m.call(at("strcpy"), &[0x0030_0000, 0x0020_0000]).unwrap();
        m.call(at("strcat"), &[0x0030_0000, 0x0020_0004]).unwrap();
        assert_eq!(m.bus.cstr(0x0030_0000).unwrap(), "abcabd");
        m.call(at("memset"), &[0x0030_0010, 0x1ff, 4]).unwrap();
        m.call(at("memmove"), &[0x0030_0012, 0x0030_0010, 4]).unwrap();
        assert_eq!(m.bus.bytes(0x0030_0010, 6).unwrap(), [0xff; 6], "overlap-safe");
        let a = m.call(at("malloc"), &[24]).unwrap();
        let b = m.call(at("malloc"), &[8]).unwrap();
        assert_eq!(b, a + 32, "16-byte blocks");
        m.call(at("free"), &[a]).unwrap();
        assert_eq!(m.call(at("malloc"), &[16]).unwrap(), a, "freed memory is reused");
        assert_eq!(m.call(at("printf"), &[0]).unwrap(), 0);
    }

    #[test]
    fn a_string_with_no_terminator_or_a_bad_pointer_stops_the_run() {
        let mut m = machine();
        let err = m.call(at("strlen"), &[0x0300_0000]).unwrap_err();
        assert!(matches!(err, Stop::Bus { .. }), "{err}");
    }
}
