//! Memory as a CPU sees it.

use std::collections::HashMap;

/// Memory a CPU reads and writes. Accesses are byte slices, little-endian
/// for the helpers in [`BusExt`], so one interface carries a byte, a word or
/// an EE quadword. An `Err` is a bus error: nothing is mapped there.
pub trait Bus {
    /// Fills `buf` from `addr` up.
    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), u32>;
    /// Writes `data` from `addr` up.
    fn write(&mut self, addr: u32, data: &[u8]) -> Result<(), u32>;

    /// Starts recording writes (address, bytes replaced) for
    /// [`Bus::take_journal`]. A bus that cannot record ignores it, and the
    /// oracle then reports no changes.
    fn start_journal(&mut self) {}

    /// The writes recorded since [`Bus::start_journal`], ending the
    /// recording; None when there was none. Addresses are canonical
    /// ([`Bus::canonical`]).
    fn take_journal(&mut self) -> Option<Vec<(u32, Vec<u8>)>> {
        None
    }

    /// The one address every alias of `addr` folds to (a mirror, another
    /// MIPS segment): what the journal records, so a byte written through
    /// two aliases is one change. The identity unless the bus knows better.
    fn canonical(&self, addr: u32) -> u32 {
        addr
    }
}

/// One bus shared by several machines: two CPUs over one memory (the PS2's
/// EE and IOP each see their own, but the Saturn's two SH-2s and a
/// 68000 + Z80 pair share). Clone it to give each machine a handle.
pub struct SharedBus<B>(pub std::rc::Rc<std::cell::RefCell<B>>);

impl<B> SharedBus<B> {
    /// Shares `bus`.
    pub fn new(bus: B) -> SharedBus<B> {
        SharedBus(std::rc::Rc::new(std::cell::RefCell::new(bus)))
    }
}

impl<B> Clone for SharedBus<B> {
    fn clone(&self) -> Self {
        SharedBus(self.0.clone())
    }
}

impl<B: Bus> Bus for SharedBus<B> {
    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), u32> {
        self.0.borrow_mut().read(addr, buf)
    }
    fn write(&mut self, addr: u32, data: &[u8]) -> Result<(), u32> {
        self.0.borrow_mut().write(addr, data)
    }
    fn start_journal(&mut self) {
        self.0.borrow_mut().start_journal();
    }
    fn take_journal(&mut self) -> Option<Vec<(u32, Vec<u8>)>> {
        self.0.borrow_mut().take_journal()
    }
    fn canonical(&self, addr: u32) -> u32 {
        self.0.borrow().canonical(addr)
    }
}

/// Little-endian loads and stores of every width, on any [`Bus`].
pub trait BusExt: Bus {
    /// A byte.
    fn read_u8(&mut self, addr: u32) -> Result<u8, u32> {
        let mut b = [0; 1];
        self.read(addr, &mut b)?;
        Ok(b[0])
    }
    /// A halfword.
    fn read_u16(&mut self, addr: u32) -> Result<u16, u32> {
        let mut b = [0; 2];
        self.read(addr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }
    /// A word.
    fn read_u32(&mut self, addr: u32) -> Result<u32, u32> {
        let mut b = [0; 4];
        self.read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    /// A doubleword.
    fn read_u64(&mut self, addr: u32) -> Result<u64, u32> {
        let mut b = [0; 8];
        self.read(addr, &mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    /// A quadword (the EE's `lq`).
    fn read_u128(&mut self, addr: u32) -> Result<u128, u32> {
        let mut b = [0; 16];
        self.read(addr, &mut b)?;
        Ok(u128::from_le_bytes(b))
    }
    /// A byte.
    fn write_u8(&mut self, addr: u32, v: u8) -> Result<(), u32> {
        self.write(addr, &[v])
    }
    /// A halfword.
    fn write_u16(&mut self, addr: u32, v: u16) -> Result<(), u32> {
        self.write(addr, &v.to_le_bytes())
    }
    /// A word.
    fn write_u32(&mut self, addr: u32, v: u32) -> Result<(), u32> {
        self.write(addr, &v.to_le_bytes())
    }
    /// A doubleword.
    fn write_u64(&mut self, addr: u32, v: u64) -> Result<(), u32> {
        self.write(addr, &v.to_le_bytes())
    }
    /// A quadword.
    fn write_u128(&mut self, addr: u32, v: u128) -> Result<(), u32> {
        self.write(addr, &v.to_le_bytes())
    }
    /// `len` bytes from `addr`.
    fn bytes(&mut self, addr: u32, len: usize) -> Result<Vec<u8>, u32> {
        let mut v = vec![0; len];
        self.read(addr, &mut v)?;
        Ok(v)
    }
    /// An unsigned value `bytes` wide (1, 2, 4 or 8) in byte order `endian`.
    ///
    /// # Panics
    ///
    /// When `bytes` is more than 8.
    fn read_uint(&mut self, addr: u32, bytes: usize, endian: crate::cpu::Endian) -> Result<u64, u32> {
        assert!(bytes <= 8, "{bytes}-byte value");
        let mut b = [0u8; 8];
        self.read(addr, &mut b[..bytes])?;
        let b = &b[..bytes];
        Ok(match endian {
            crate::cpu::Endian::Little => b.iter().rev().fold(0u64, |acc, x| (acc << 8) | u64::from(*x)),
            crate::cpu::Endian::Big => b.iter().fold(0u64, |acc, x| (acc << 8) | u64::from(*x)),
        })
    }
    /// Writes the low `bytes` bytes of `v` in byte order `endian`.
    ///
    /// # Panics
    ///
    /// When `bytes` is more than 8.
    fn write_uint(&mut self, addr: u32, bytes: usize, endian: crate::cpu::Endian, v: u64) -> Result<(), u32> {
        assert!(bytes <= 8, "{bytes}-byte value");
        let le = v.to_le_bytes();
        let mut b = [0u8; 8];
        for i in 0..bytes {
            b[i] = match endian {
                crate::cpu::Endian::Little => le[i],
                crate::cpu::Endian::Big => le[bytes - 1 - i],
            };
        }
        self.write(addr, &b[..bytes])
    }
    /// A NUL-terminated string from `addr`, at most 4096 bytes, lossy UTF-8.
    fn cstr(&mut self, addr: u32) -> Result<String, u32> {
        let mut out = Vec::new();
        for i in 0..4096 {
            match self.read_u8(addr.wrapping_add(i))? {
                0 => break,
                b => out.push(b),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

impl<B: Bus + ?Sized> BusExt for B {}

/// Hardware behind a region of I/O addresses. Offsets are from the region's
/// start.
pub trait Device {
    /// A read of `buf.len()` bytes at `offset`.
    fn read(&mut self, offset: u32, buf: &mut [u8]);
    /// A write of `data` at `offset`.
    fn write(&mut self, offset: u32, data: &[u8]);
}

/// A [`Device`] that is no hardware at all: it records every access and
/// answers reads from a table of values (0 otherwise). For running code that
/// touches I/O the test does not care about, and for seeing that it did.
#[derive(Clone, Debug, Default)]
pub struct IoLog {
    /// Every access, in order: offset, bytes, the value written (None for a
    /// read).
    pub accesses: Vec<(u32, usize, Option<u64>)>,
    /// What a read at an offset returns, little-endian.
    pub values: HashMap<u32, u64>,
}

impl Device for IoLog {
    fn read(&mut self, offset: u32, buf: &mut [u8]) {
        self.accesses.push((offset, buf.len(), None));
        let v = self.values.get(&offset).copied().unwrap_or(0).to_le_bytes();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = v.get(i).copied().unwrap_or(0);
        }
    }

    fn write(&mut self, offset: u32, data: &[u8]) {
        let mut v = [0u8; 8];
        for (i, b) in data.iter().take(8).enumerate() {
            v[i] = *b;
        }
        self.accesses.push((offset, data.len(), Some(u64::from_le_bytes(v))));
    }
}

/// What a region holds.
pub enum Backing {
    /// Bytes; an address past the end wraps (a mirror).
    Ram(Vec<u8>),
    /// Hardware.
    Device(Box<dyn Device>),
    /// Reads 0, writes vanish (cache control and the like).
    Ignore,
}

/// A range of addresses: matched after `addr & mask` (0x1fff_ffff folds the
/// MIPS segments KUSEG/KSEG0/KSEG1 onto one physical address), from `start`
/// for `len` bytes.
pub struct Region {
    /// A name for messages.
    pub name: &'static str,
    /// The first address, after masking.
    pub start: u32,
    /// Bytes the region spans (a RAM region may be smaller: it mirrors).
    pub len: u32,
    /// Applied to an address before matching.
    pub mask: u32,
    /// What is there.
    pub backing: Backing,
}

impl Region {
    /// RAM of `size` bytes at `start`, spanning `len` (mirrored past `size`),
    /// matched under `mask`.
    pub fn ram(name: &'static str, start: u32, size: usize, len: u32, mask: u32) -> Region {
        Region { name, start, len, mask, backing: Backing::Ram(vec![0; size]) }
    }

    /// A device at `start`, `len` bytes, under `mask`.
    pub fn device(name: &'static str, start: u32, len: u32, mask: u32, d: impl Device + 'static) -> Region {
        Region { name, start, len, mask, backing: Backing::Device(Box::new(d)) }
    }

    /// Addresses that read 0 and ignore writes.
    pub fn ignore(name: &'static str, start: u32, len: u32, mask: u32) -> Region {
        Region { name, start, len, mask, backing: Backing::Ignore }
    }

    fn offset(&self, addr: u32) -> Option<u32> {
        let a = addr & self.mask;
        (a >= self.start && a - self.start < self.len).then(|| a - self.start)
    }
}

/// A [`Bus`] of [`Region`]s, searched in order. An access that crosses out
/// of its region is a bus error at the first unmapped byte. With
/// [`Memory::journal`] on, every RAM write is recorded with the bytes it
/// replaced, which is what [`crate::Machine::call_recorded`] diffs.
pub struct Memory {
    /// The regions, first match wins.
    pub regions: Vec<Region>,
    /// When `Some`, RAM writes are logged here: address, the bytes before.
    pub journal: Option<Vec<(u32, Vec<u8>)>>,
}

impl Memory {
    /// No regions.
    pub fn new() -> Memory {
        Memory { regions: Vec::new(), journal: None }
    }

    /// The PS1: 2 MB of RAM mirrored through the first 8 MB, the 1 KB
    /// scratchpad at 0x1f80_0000, I/O ports 0x1f80_1000-0x1f80_2fff on an
    /// [`IoLog`], the BIOS ROM area 0x1fc0_0000 (512 KB) as RAM to load into,
    /// and the cache control at 0xfffe_0000 ignored. Addresses are folded
    /// by 0x1fff_ffff (KUSEG, KSEG0 and KSEG1 alike).
    pub fn psx() -> Memory {
        const PHYS: u32 = 0x1fff_ffff;
        Memory {
            regions: vec![
                Region::ignore("cache control", 0xfffe_0000, 0x200, u32::MAX),
                Region::ram("ram", 0, 2 << 20, 8 << 20, PHYS),
                Region::ram("scratchpad", 0x1f80_0000, 1024, 1024, PHYS),
                Region::device("io", 0x1f80_1000, 0x2000, PHYS, IoLog::default()),
                Region::ram("bios", 0x1fc0_0000, 512 << 10, 512 << 10, PHYS),
            ],
            journal: None,
        }
    }

    /// The PS2's EE: 32 MB of RAM (folded by 0x1fff_ffff), the 16 KB
    /// scratchpad at 0x7000_0000 (not folded), and the EE's I/O registers
    /// 0x1000_0000-0x1000_ffff on an [`IoLog`].
    pub fn ps2() -> Memory {
        const PHYS: u32 = 0x1fff_ffff;
        Memory {
            regions: vec![
                Region::ram("scratchpad", 0x7000_0000, 16 << 10, 16 << 10, u32::MAX),
                Region::ram("ram", 0, 32 << 20, 32 << 20, PHYS),
                Region::device("io", 0x1000_0000, 0x1_0000, PHYS, IoLog::default()),
            ],
            journal: None,
        }
    }

    /// The region named `name`.
    pub fn region(&mut self, name: &str) -> Option<&mut Region> {
        self.regions.iter_mut().find(|r| r.name == name)
    }

    /// The bytes of the RAM region named `name`.
    pub fn ram(&mut self, name: &str) -> Option<&mut Vec<u8>> {
        match &mut self.region(name)?.backing {
            Backing::Ram(v) => Some(v),
            _ => None,
        }
    }

    /// Writes `data` at `addr`, panicking on a bus error: for setting up a
    /// test.
    ///
    /// # Panics
    ///
    /// When any byte is unmapped.
    pub fn load(&mut self, addr: u32, data: &[u8]) {
        if let Err(at) = self.write(addr, data) {
            panic!("load at {addr:#x}: nothing mapped at {at:#x}");
        }
    }

    /// Visits each byte range of an access within one region: (region
    /// index, offset, range of the access buffer).
    fn spans(&self, addr: u32, len: usize) -> Result<Vec<(usize, u32, std::ops::Range<usize>)>, u32> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < len {
            let a = addr.wrapping_add(i as u32);
            let (ri, off) =
                self.regions.iter().enumerate().find_map(|(ri, r)| r.offset(a).map(|o| (ri, o))).ok_or(a)?;
            let room = (self.regions[ri].len - off) as usize;
            let n = room.min(len - i);
            out.push((ri, off, i..i + n));
            i += n;
        }
        Ok(out)
    }
}

impl Default for Memory {
    fn default() -> Self {
        Memory::new()
    }
}

impl Bus for Memory {
    fn start_journal(&mut self) {
        self.journal = Some(Vec::new());
    }

    fn take_journal(&mut self) -> Option<Vec<(u32, Vec<u8>)>> {
        self.journal.take()
    }

    /// The region's start plus the offset, folded into the RAM's size for
    /// a mirrored region: PS1 RAM at 0x8001_0000, 0xa001_0000 and
    /// 0x0021_0000 is all 0x0001_0000.
    fn canonical(&self, addr: u32) -> u32 {
        for r in &self.regions {
            if let Some(off) = r.offset(addr) {
                return match &r.backing {
                    Backing::Ram(v) => r.start + (off as usize % v.len()) as u32,
                    _ => r.start + off,
                };
            }
        }
        addr
    }

    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), u32> {
        for (ri, off, range) in self.spans(addr, buf.len())? {
            let dst = &mut buf[range];
            match &mut self.regions[ri].backing {
                Backing::Ram(v) => {
                    let size = v.len();
                    for (k, b) in dst.iter_mut().enumerate() {
                        *b = v[(off as usize + k) % size];
                    }
                }
                Backing::Device(d) => d.read(off, dst),
                Backing::Ignore => dst.fill(0),
            }
        }
        Ok(())
    }

    fn write(&mut self, addr: u32, data: &[u8]) -> Result<(), u32> {
        let spans = self.spans(addr, data.len())?;
        for (ri, off, range) in spans {
            let src = &data[range];
            let start = self.regions[ri].start;
            match &mut self.regions[ri].backing {
                Backing::Ram(v) => {
                    let size = v.len();
                    if let Some(j) = &mut self.journal {
                        // Canonical addresses, one entry per run that does
                        // not wrap past the end of the RAM's mirror.
                        let mut k = 0;
                        while k < src.len() {
                            let at = (off as usize + k) % size;
                            let n = (size - at).min(src.len() - k);
                            j.push((start + at as u32, v[at..at + n].to_vec()));
                            k += n;
                        }
                    }
                    for (k, b) in src.iter().enumerate() {
                        v[(off as usize + k) % size] = *b;
                    }
                }
                Backing::Device(d) => d.write(off, src),
                Backing::Ignore => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psx_ram_mirrors_and_folds_the_segments() {
        let mut m = Memory::psx();
        m.write_u32(0x8001_0000, 0xdead_beef).unwrap();
        assert_eq!(m.read_u32(0xa001_0000), Ok(0xdead_beef), "KSEG1 sees KSEG0");
        assert_eq!(m.read_u32(0x0001_0000), Ok(0xdead_beef), "and KUSEG");
        assert_eq!(m.read_u32(0x0021_0000), Ok(0xdead_beef), "2 MB mirrors through 8 MB");
        assert_eq!(m.read_u32(0x0080_0000), Err(0x0080_0000), "past 8 MB is unmapped");
    }

    #[test]
    fn widths_from_a_byte_to_a_quadword_are_little_endian() {
        let mut m = Memory::ps2();
        m.write_u128(0x0010_0000, 0x0f0e_0d0c_0b0a_0908_0706_0504_0302_0100).unwrap();
        assert_eq!(m.read_u8(0x0010_0001), Ok(1));
        assert_eq!(m.read_u16(0x0010_0002), Ok(0x0302));
        assert_eq!(m.read_u64(0x0010_0008), Ok(0x0f0e_0d0c_0b0a_0908));
        m.write_u32(0x7000_0010, 7).unwrap();
        assert_eq!(m.read_u32(0x7000_0010), Ok(7), "the EE scratchpad is not folded");
    }

    #[test]
    fn io_is_logged_and_answers_from_its_table() {
        let mut m = Memory::psx();
        m.write_u32(0x1f80_1810, 0xe100_0000).unwrap();
        assert_eq!(m.read_u32(0x1f80_1814), Ok(0));
        assert!(matches!(m.region("io").unwrap().backing, Backing::Device(_)));
    }

    #[test]
    fn the_journal_records_what_a_write_replaced_at_its_canonical_address() {
        let mut m = Memory::psx();
        m.write_u32(0x8000_0100, 0x1111_1111).unwrap();
        m.start_journal();
        m.write_u16(0x8000_0102, 0xabcd).unwrap();
        m.write_u8(0xa020_0104, 0x22).unwrap();
        let j = m.take_journal().unwrap();
        assert_eq!(j, [(0x0000_0102, vec![0x11, 0x11]), (0x0000_0104, vec![0])], "KSEG1 and the mirror fold too");
        assert_eq!(m.canonical(0x801f_0000), 0x001f_0000);
        assert_eq!(m.canonical(0x1f80_0010), 0x1f80_0010, "the scratchpad keeps its address");
    }

    #[test]
    fn either_byte_order_reads_and_writes() {
        use crate::cpu::Endian;
        let mut m = Memory::psx();
        m.write_uint(0x8000_0000, 4, Endian::Big, 0x1122_3344).unwrap();
        assert_eq!(m.bytes(0x8000_0000, 4).unwrap(), [0x11, 0x22, 0x33, 0x44]);
        assert_eq!(m.read_uint(0x8000_0000, 4, Endian::Little), Ok(0x4433_2211));
        assert_eq!(m.read_uint(0x8000_0000, 2, Endian::Big), Ok(0x1122));
    }

    #[test]
    fn a_shared_bus_is_one_memory() {
        let mut a = SharedBus::new(Memory::psx());
        let mut b = a.clone();
        a.write_u32(0x8000_0000, 9).unwrap();
        assert_eq!(b.read_u32(0x8000_0000), Ok(9));
    }

    #[test]
    fn strings_and_byte_runs() {
        let mut m = Memory::psx();
        m.load(0x8000_1000, b"PSX\0junk");
        assert_eq!(m.cstr(0x8000_1000).unwrap(), "PSX");
        assert_eq!(m.bytes(0x8000_1004, 4).unwrap(), b"junk");
    }
}
