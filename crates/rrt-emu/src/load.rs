//! Getting an original's code into memory: the PlayStation's PS-X EXE and
//! ELF (the PS2's executables, and most consoles' toolchains since), parsed
//! by [`rrt_dis::exe`] and written into a [`Bus`] by [`Load`].

use crate::bus::Bus;
pub use rrt_dis::exe::{Elf, PsxExe, Segment};

/// An executable that writes itself into memory.
pub trait Load {
    /// Writes the program's bytes into `bus`: a PS-X EXE's image at
    /// `t_addr`; every ELF segment, zero-filled up to its memory size.
    fn load(&self, bus: &mut dyn Bus) -> Result<(), String>;
}

impl Load for PsxExe {
    fn load(&self, bus: &mut dyn Bus) -> Result<(), String> {
        bus.write(self.t_addr, &self.image).map_err(|a| format!("nothing mapped at {a:#x} for the image"))
    }
}

impl Load for Elf {
    fn load(&self, bus: &mut dyn Bus) -> Result<(), String> {
        for s in &self.segments {
            let mut bytes = s.data.clone();
            bytes.resize(s.mem_size.max(s.data.len() as u32) as usize, 0);
            bus.write(s.vaddr, &bytes).map_err(|a| format!("nothing mapped at {a:#x} for a segment"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{BusExt, Memory};
    use crate::cpu::Endian;
    use crate::symbols::Symbols;

    #[test]
    fn a_psx_exe_loads_at_t_addr() {
        let exe = PsxExe { pc0: 0x8001_0000, gp0: 0, t_addr: 0x8001_0000, s_addr: 0, image: vec![0, 0, 0, 8] };
        let mut m = Memory::psx();
        exe.load(&mut m).unwrap();
        assert_eq!(m.read_u32(0x8001_0000), Ok(0x0800_0000));
    }

    #[test]
    fn an_elf_loads_its_segments_with_bss_zeroed() {
        let elf = Elf {
            endian: Endian::Little,
            machine: 8,
            entry: 0x0010_0000,
            segments: vec![Segment { vaddr: 0x0010_0000, data: vec![1, 2, 3, 4], mem_size: 8 }],
            symbols: Symbols::new(),
        };
        let mut m = Memory::ps2();
        m.write_u32(0x0010_0004, 0xffff_ffff).unwrap();
        elf.load(&mut m).unwrap();
        assert_eq!(m.bytes(0x0010_0000, 8).unwrap(), [1, 2, 3, 4, 0, 0, 0, 0]);
    }
}
