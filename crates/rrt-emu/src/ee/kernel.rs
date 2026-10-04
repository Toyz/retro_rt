//! PS2 kernel and library stand-ins, hooked by symbol name: the semaphore and
//! interrupt calls, and the GS / DMA library calls, that piney_apples'
//! harnesses replace with constant returns when running a game's own code.
//!
//! The names and values are what `tools/test_stream_rs.py` stubs (its `Game`
//! set-up and `run_scene`), nothing more. Not here: the EE kernel's system
//! calls by number (an interpreted `syscall` still stops the run, or goes to
//! [`Machine::on_syscall`]), threads, real semaphores, DMA or the GS. The C
//! library is [`crate::hle::libc`].

use crate::bus::Bus;
use crate::machine::Machine;

use super::Ee;

/// Semaphore and interrupt-control calls that return 1 without doing
/// anything: `tools/test_stream_rs.py`'s `Game.__init__` hooks each to
/// `lambda mm, *a: 1`. Code under test sees every semaphore created,
/// signalled and waited on at once, and interrupts toggled.
pub const SEMA_AND_INTR: [&str; 7] =
    ["CreateSema", "DeleteSema", "SignalSema", "WaitSema", "iSignalSema", "DIntr", "EIntr"];

/// What each of [`SEMA_AND_INTR`] returns.
pub const SEMA_AND_INTR_RETURN: u64 = 1;

/// GS image uploads, cache flushing and DMA calls that return 0 without
/// doing anything: `tools/test_stream_rs.py`'s `run_scene` hooks each to
/// `lambda mm, *a: 0` ("the texture uploads' GS packets and DMA: not part of
/// the draw list").
pub const GS_DMA: [&str; 14] = [
    "sceGsSetDefLoadImage",
    "sceGsExecLoadImage",
    "sceGsSyncPath",
    "FlushCache",
    "sceDmaGetChan",
    "sceDmaReset",
    "sceDmaPutEnv",
    "sceDmaSend",
    "sceDmaSendN",
    "sceDmaSendI",
    "sceDmaSync",
    "sceDmaWatch",
    "sceDmaPause",
    "sceDmaRestart",
];

/// What each of [`GS_DMA`] returns.
pub const GS_DMA_RETURN: u64 = 0;

/// Stubs every function in [`SEMA_AND_INTR`] and [`GS_DMA`] that
/// `machine.symbols` names, each returning its constant; a name the table
/// lacks is skipped. Returns the names installed, in table order.
pub fn install<B: Bus + 'static>(machine: &mut Machine<Ee, B>) -> Vec<&'static str> {
    let table =
        SEMA_AND_INTR.iter().map(|n| (*n, SEMA_AND_INTR_RETURN)).chain(GS_DMA.iter().map(|n| (*n, GS_DMA_RETURN)));
    let mut installed = Vec::new();
    for (name, v) in table {
        let Some(addr) = machine.symbols.addr(name) else { continue };
        machine.stub(addr, v);
        installed.push(name);
    }
    installed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BusExt;
    use crate::ee::{Features, machine};

    const JAL: u32 = 0x0c00_0000;

    #[test]
    fn install_stubs_the_named_kernel_functions_and_skips_the_rest() {
        let mut m = machine(Features::default());
        m.symbols.insert("CreateSema", 0x0001_0000);
        m.symbols.insert("sceDmaSend", 0x0001_0100);
        m.symbols.insert("main", 0x0002_0000);
        assert_eq!(install(&mut m), ["CreateSema", "sceDmaSend"]);
        assert!(m.is_hooked(0x0001_0000) && m.is_hooked(0x0001_0100) && !m.is_hooked(0x0002_0000));
        // The stubs' addresses hold `break`: reaching the code would stop.
        m.bus.write_u32(0x0001_0000, 0x0000_000d).unwrap();
        m.bus.write_u32(0x0001_0100, 0x0000_000d).unwrap();
        assert_eq!(m.call(0x0001_0000, &[]), Ok(1));
        assert_eq!(m.call(0x0001_0100, &[1, 2, 3]), Ok(0));
        // From code: move s0, ra; jal CreateSema; nop; jr s0; nop.
        let prog = [0x03e0_8021, JAL | (0x0001_0000 >> 2), 0, 0x0200_0008, 0];
        for (i, w) in prog.iter().enumerate() {
            m.bus.write_u32(0x0002_0000 + 4 * i as u32, *w).unwrap();
        }
        assert_eq!(m.call(0x0002_0000, &[]), Ok(1));
    }
}
