//! Executables as the originals shipped them: the PlayStation's PS-X EXE
//! and ELF (the PS2's executables, and most consoles' toolchains since).
//! Parsed only; `rrt_emu::Load` writes them into an emulated machine.
//!
//! [`load`] recognises a file by its [`FORMATS`] and gives the same
//! [`Loaded`] shape for any of them: segments, entry, symbols, and which
//! backend ([`crate::arch`]) its code is for. A new format (an NE or PE for
//! 16- or 32-bit Windows, a cartridge header) is a parser plus one
//! [`Format`] entry.

use crate::analysis::Image;
use crate::insn::Endian;
use crate::symbols::{Kind, Symbol, Symbols};

/// A PS1 executable (`PS-X EXE`): a 2048-byte header, then the image loaded
/// at `t_addr`.
///
/// ```text
/// 0x000  [u8; 8]  "PS-X EXE"
/// 0x010  u32      pc0     entry point
/// 0x014  u32      gp0     initial $gp (0 when crt0 sets it)
/// 0x018  u32      t_addr  where the image loads
/// 0x01c  u32      t_size  its size
/// 0x020  u32      d_addr  initialised data (usually 0: in the image)
/// 0x024  u32      d_size
/// 0x028  u32      b_addr  bss, zeroed by crt0
/// 0x02c  u32      b_size
/// 0x030  u32      s_addr  stack base (0: the BIOS default)
/// 0x034  u32      s_size
/// 0x04c  ASCII    the region marker ("Sony Computer Entertainment Inc.
///                 for North America area")
/// 0x800           the image
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsxExe {
    /// The entry point.
    pub pc0: u32,
    /// The initial `$gp`, 0 when the program's crt0 sets it.
    pub gp0: u32,
    /// Where the image loads.
    pub t_addr: u32,
    /// Initialised data's address, 0 when it is in the image.
    pub d_addr: u32,
    /// Initialised data's size.
    pub d_size: u32,
    /// Bss: where crt0 zeroes.
    pub b_addr: u32,
    /// Bss's size.
    pub b_size: u32,
    /// Stack base, 0 for the BIOS's.
    pub s_addr: u32,
    /// Stack size.
    pub s_size: u32,
    /// The region marker at 0x4c, up to its NUL (empty when absent).
    pub marker: String,
    /// The image.
    pub image: Vec<u8>,
}

impl PsxExe {
    /// Parses an executable's bytes.
    pub fn parse(data: &[u8]) -> Result<PsxExe, String> {
        if data.len() < 0x800 || &data[..8] != b"PS-X EXE" {
            return Err("not a PS-X EXE (no magic, or shorter than its header)".into());
        }
        let w = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        let size = w(0x1c) as usize;
        let image = data.get(0x800..0x800 + size).ok_or("the image is shorter than t_size")?.to_vec();
        let marker = &data[0x4c..0x800];
        let marker = &marker[..marker.iter().position(|b| *b == 0).unwrap_or(marker.len())];
        let marker = String::from_utf8_lossy(marker).trim().to_string();
        Ok(PsxExe {
            pc0: w(0x10),
            gp0: w(0x14),
            t_addr: w(0x18),
            d_addr: w(0x20),
            d_size: w(0x24),
            b_addr: w(0x28),
            b_size: w(0x2c),
            s_addr: w(0x30),
            s_size: w(0x34),
            marker,
            image,
        })
    }

    /// The image as an [`Image`] for analysis.
    pub fn as_image(&self) -> Image<'_> {
        Image { base: self.t_addr, bytes: &self.image }
    }
}

/// One loadable segment of an ELF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Where it loads.
    pub vaddr: u32,
    /// Its bytes from the file; memory past them up to `mem_size` is zero
    /// (bss).
    pub data: Vec<u8>,
    /// Its size in memory.
    pub mem_size: u32,
}

/// A 32-bit ELF executable, either byte order: its entry point, loadable
/// segments and symbols.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elf {
    /// The byte order the file declares.
    pub endian: Endian,
    /// `e_machine`: 8 MIPS (PS1 homebrew, PS2), 40 ARM, 42 SuperH, 20
    /// PowerPC.
    pub machine: u16,
    /// `e_flags`: for MIPS, the ISA level (bits 28-31) and the CPU (bits
    /// 16-23, 0x92 for the R5900).
    pub flags: u32,
    /// The entry point.
    pub entry: u32,
    /// `PT_LOAD` segments, in file order.
    pub segments: Vec<Segment>,
    /// Named symbols from `.symtab` (functions as [`Kind::Func`], objects as
    /// [`Kind::Data`], untyped) with a section and their sizes, empty when the
    /// file is stripped. The first of a repeated name or address wins.
    pub symbols: Symbols,
}

impl Elf {
    /// Parses a 32-bit ELF.
    pub fn parse(data: &[u8]) -> Result<Elf, String> {
        if data.len() < 52 || &data[..4] != b"\x7fELF" {
            return Err("not an ELF".into());
        }
        if data[4] != 1 {
            return Err("not a 32-bit ELF".into());
        }
        let endian = match data[5] {
            1 => Endian::Little,
            2 => Endian::Big,
            e => return Err(format!("unknown ELF byte order {e}")),
        };
        let get = |at: usize, n: usize| -> Result<u32, String> {
            let b =
                at.checked_add(n).and_then(|end| data.get(at..end)).ok_or_else(|| format!("truncated at {at:#x}"))?;
            Ok(match endian {
                Endian::Little => b.iter().rev().fold(0u32, |a, x| (a << 8) | u32::from(*x)),
                Endian::Big => b.iter().fold(0u32, |a, x| (a << 8) | u32::from(*x)),
            })
        };
        let machine = get(18, 2)? as u16;
        let flags = get(36, 4)?;
        let entry = get(24, 4)?;
        let (phoff, shoff) = (get(28, 4)? as usize, get(32, 4)? as usize);
        let (phentsize, phnum) = (get(42, 2)? as usize, get(44, 2)? as usize);
        let (shentsize, shnum) = (get(46, 2)? as usize, get(48, 2)? as usize);
        let mut segments = Vec::new();
        for i in 0..phnum {
            let h = phoff + i * phentsize;
            if get(h, 4)? != 1 {
                continue;
            }
            let (offset, vaddr, filesz, memsz) =
                (get(h + 4, 4)? as usize, get(h + 8, 4)?, get(h + 16, 4)?, get(h + 20, 4)?);
            let data = data
                .get(offset..offset + filesz as usize)
                .ok_or_else(|| format!("segment {i} runs past the file"))?
                .to_vec();
            segments.push(Segment { vaddr, data, mem_size: memsz });
        }
        let mut symbols = Symbols::new();
        for i in 0..shnum {
            let h = shoff + i * shentsize;
            if get(h + 4, 4)? != 2 {
                continue;
            }
            let (off, size, link, entsize) = (
                get(h + 16, 4)? as usize,
                get(h + 20, 4)? as usize,
                get(h + 24, 4)? as usize,
                get(h + 36, 4)? as usize,
            );
            let strh = shoff + link * shentsize;
            let (stroff, strsize) = (get(strh + 16, 4)? as usize, get(strh + 20, 4)? as usize);
            let strtab = data.get(stroff..stroff + strsize).ok_or("string table runs past the file")?;
            for k in 0..size / entsize.max(16) {
                let e = off + k * entsize.max(16);
                let (name, value, size, info, shndx) =
                    (get(e, 4)? as usize, get(e + 4, 4)?, get(e + 8, 4)?, get(e + 12, 1)?, get(e + 14, 2)?);
                let kind = info & 15;
                if name == 0 || shndx == 0 || kind > 2 {
                    continue;
                }
                let rest = strtab.get(name..).ok_or_else(|| format!("symbol {k} names past the string table"))?;
                let name = String::from_utf8_lossy(&rest[..rest.iter().position(|b| *b == 0).unwrap_or(rest.len())]);
                // A name the symbol file could not hold (empty, or with
                // whitespace) is not a name to analyse by.
                if name.is_empty() || name.contains(char::is_whitespace) {
                    continue;
                }
                let name = name.into_owned();
                if symbols.get(&name).is_some() || symbols.at(value).is_some() {
                    // Local statics share names across files, and aliases
                    // share addresses: the first one wins.
                    continue;
                }
                symbols.mark(Symbol {
                    kind: match kind {
                        1 => Kind::Data,
                        2 => Kind::Func,
                        _ => Kind::Unknown,
                    },
                    size: (size > 0).then_some(size),
                    ..Symbol::new(name, value)
                });
            }
        }
        Ok(Elf { endian, machine, flags, entry, segments, symbols })
    }

    /// The segments' bytes from the file laid out as one image: see
    /// [`flatten`].
    pub fn flatten(&self) -> Result<(u32, Vec<u8>), String> {
        flatten(&self.segments)
    }

    /// The backend for its code: `r5900` for a MIPS ELF built for the R5900
    /// or at ISA level III or above, `r3000` for other MIPS; None for other
    /// machines.
    pub fn arch(&self) -> Option<&'static str> {
        const EF_MIPS_MACH_5900: u32 = 0x0092_0000;
        match self.machine {
            8 if self.flags & 0x00ff_0000 == EF_MIPS_MACH_5900 || self.flags >> 28 >= 2 => Some("r5900"),
            8 => Some("r3000"),
            _ => None,
        }
    }
}
/// Segments' bytes from the file laid out as one image, from the lowest
/// address to the highest end of file data, gaps zero: `(base, bytes)`. Bss
/// and segments with no file bytes (overlays loaded later) are left out:
/// they hold no code or initial data. Errors when the span is over 64 MiB
/// (segments scattered across the address space) or no segment has bytes.
pub fn flatten(segments: &[Segment]) -> Result<(u32, Vec<u8>), String> {
    let filled = || segments.iter().filter(|s| !s.data.is_empty());
    let base = filled().map(|s| s.vaddr).min().ok_or("no segment has bytes in the file")?;
    let end = filled().map(|s| u64::from(s.vaddr) + s.data.len() as u64).max().unwrap_or(0);
    let span = end - u64::from(base);
    if span > 64 << 20 {
        return Err(format!("segments span {span:#x} bytes; analyse them one at a time"));
    }
    let mut bytes = vec![0u8; span as usize];
    for s in filled() {
        let at = (s.vaddr - base) as usize;
        bytes[at..at + s.data.len()].copy_from_slice(&s.data);
    }
    Ok((base, bytes))
}

/// An executable, whatever its format: what [`load`] gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// The format's name, as [`Format::name`].
    pub format: &'static str,
    /// The backend its code is for ([`crate::arch::by_name`]), when the file
    /// says.
    pub arch: Option<&'static str>,
    /// The entry point.
    pub entry: u32,
    /// The global pointer's value, when the file gives it.
    pub gp: Option<u32>,
    /// What is loaded where.
    pub segments: Vec<Segment>,
    /// The file's own symbols, empty when it has none.
    pub symbols: Symbols,
}

impl Loaded {
    /// The segments as one image: see [`flatten`].
    pub fn flatten(&self) -> Result<(u32, Vec<u8>), String> {
        flatten(&self.segments)
    }
}

/// An executable format [`load`] recognises.
#[derive(Clone, Copy, Debug)]
pub struct Format {
    /// Its name: `psx-exe`, `elf`.
    pub name: &'static str,
    /// What it is, for a listing.
    pub about: &'static str,
    /// Whether `bytes` look like this format (a magic number); cheap, and
    /// never wrong in the other direction for a file of this format.
    pub sniff: fn(&[u8]) -> bool,
    /// Parses `bytes`.
    pub load: fn(&[u8]) -> Result<Loaded, String>,
}

/// The formats [`load`] knows, tried in order.
pub const FORMATS: &[Format] = &[
    Format {
        name: "psx-exe",
        about: "PlayStation executable (PS-X EXE)",
        sniff: |b| b.starts_with(b"PS-X EXE"),
        load: |b| {
            let exe = PsxExe::parse(b)?;
            let mem_size = exe.image.len() as u32;
            Ok(Loaded {
                format: "psx-exe",
                arch: Some("r3000"),
                entry: exe.pc0,
                gp: (exe.gp0 != 0).then_some(exe.gp0),
                segments: vec![Segment { vaddr: exe.t_addr, data: exe.image, mem_size }],
                symbols: Symbols::new(),
            })
        },
    },
    Format {
        name: "elf",
        about: "32-bit ELF, either byte order (PS2 executables, homebrew)",
        sniff: |b| b.starts_with(b"\x7fELF"),
        load: |b| {
            let elf = Elf::parse(b)?;
            Ok(Loaded {
                format: "elf",
                arch: elf.arch(),
                entry: elf.entry,
                gp: None,
                symbols: elf.symbols.clone(),
                segments: elf.segments,
            })
        },
    },
];

/// The format `bytes` are in, by [`Format::sniff`].
pub fn format_of(bytes: &[u8]) -> Option<&'static Format> {
    FORMATS.iter().find(|f| (f.sniff)(bytes))
}

/// Loads an executable in any of the [`FORMATS`]. Errors when none
/// recognises it, or when the one that does cannot parse it.
pub fn load(bytes: &[u8]) -> Result<Loaded, String> {
    match format_of(bytes) {
        Some(f) => (f.load)(bytes),
        None => {
            let names: Vec<&str> = FORMATS.iter().map(|f| f.name).collect();
            Err(format!("not a known executable format ({})", names.join(", ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_psx_exe_parses_its_header_and_image() {
        let mut data = vec![0u8; 0x804];
        data[..8].copy_from_slice(b"PS-X EXE");
        data[0x10..0x14].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        data[0x18..0x1c].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        data[0x1c..0x20].copy_from_slice(&4u32.to_le_bytes());
        data[0x28..0x2c].copy_from_slice(&0x800c_4910u32.to_le_bytes());
        data[0x2c..0x30].copy_from_slice(&0x2_66a0u32.to_le_bytes());
        data[0x4c..0x4c + 21].copy_from_slice(b"Sony Computer Enterta");
        data[0x800..].copy_from_slice(&0x0800_0000u32.to_le_bytes());
        let exe = PsxExe::parse(&data).unwrap();
        assert_eq!((exe.pc0, exe.t_addr, exe.image.len()), (0x8001_0000, 0x8001_0000, 4));
        assert_eq!((exe.b_addr, exe.b_size, exe.marker.as_str()), (0x800c_4910, 0x2_66a0, "Sony Computer Enterta"));
        assert_eq!(exe.as_image().word(0x8001_0000, Endian::Little), Some(0x0800_0000));
        assert!(PsxExe::parse(b"nope").is_err());
        data[0x1c] = 0xff;
        assert!(PsxExe::parse(&data).is_err(), "t_size past the file");
    }

    /// A minimal ELF: one PT_LOAD of 4 bytes plus 4 of bss, and a symtab
    /// with `main` and `buf`, in either byte order.
    fn tiny_elf(endian: Endian) -> Vec<u8> {
        let mut f = vec![0u8; 0x200];
        let put = |f: &mut Vec<u8>, at: usize, n: usize, v: u32| {
            let b = v.to_le_bytes();
            for i in 0..n {
                f[at + i] = match endian {
                    Endian::Little => b[i],
                    Endian::Big => b[n - 1 - i],
                };
            }
        };
        f[..4].copy_from_slice(b"\x7fELF");
        f[4] = 1;
        f[5] = if endian == Endian::Little { 1 } else { 2 };
        put(&mut f, 18, 2, 8);
        put(&mut f, 24, 4, 0x0010_0008);
        put(&mut f, 28, 4, 0x34);
        put(&mut f, 32, 4, 0x100);
        put(&mut f, 42, 2, 32);
        put(&mut f, 44, 2, 1);
        put(&mut f, 46, 2, 40);
        put(&mut f, 48, 2, 3);
        // PT_LOAD: offset 0x80, vaddr 0x100000, filesz 4, memsz 8.
        put(&mut f, 0x34, 4, 1);
        put(&mut f, 0x38, 4, 0x80);
        put(&mut f, 0x3c, 4, 0x0010_0000);
        put(&mut f, 0x44, 4, 4);
        put(&mut f, 0x48, 4, 8);
        f[0x80..0x84].copy_from_slice(&[1, 2, 3, 4]);
        // Section 1: symtab at 0x90, 3 entries (null, main, buf); section 2:
        // strtab at 0xc0.
        let sh1 = 0x100 + 40;
        put(&mut f, sh1 + 4, 4, 2);
        put(&mut f, sh1 + 16, 4, 0x90);
        put(&mut f, sh1 + 20, 4, 48);
        put(&mut f, sh1 + 24, 4, 2);
        put(&mut f, sh1 + 36, 4, 16);
        let sh2 = 0x100 + 80;
        put(&mut f, sh2 + 4, 4, 3);
        put(&mut f, sh2 + 16, 4, 0xc0);
        put(&mut f, sh2 + 20, 4, 10);
        f[0xc0..0xca].copy_from_slice(b"\0main\0buf\0");
        put(&mut f, 0xa0, 4, 1);
        put(&mut f, 0xa4, 4, 0x0010_0000);
        f[0xa0 + 12] = 2;
        put(&mut f, 0xa0 + 14, 2, 1);
        put(&mut f, 0xb0, 4, 6);
        put(&mut f, 0xb4, 4, 0x0010_0004);
        f[0xb0 + 12] = 1;
        put(&mut f, 0xb0 + 14, 2, 1);
        f
    }

    #[test]
    fn an_elf_of_either_byte_order_parses_with_its_symbols() {
        for endian in [Endian::Little, Endian::Big] {
            let elf = Elf::parse(&tiny_elf(endian)).unwrap();
            assert_eq!((elf.endian, elf.machine, elf.entry), (endian, 8, 0x0010_0008));
            assert_eq!(elf.symbols.get("main").map(|s| (s.addr, s.kind)), Some((0x0010_0000, Kind::Func)));
            assert_eq!(elf.symbols.get("buf").map(|s| (s.addr, s.kind)), Some((0x0010_0004, Kind::Data)));
            assert_eq!(elf.flatten().unwrap(), (0x0010_0000, vec![1, 2, 3, 4]), "file bytes only, no bss");
        }
        assert!(Elf::parse(b"\x7fELF\x02").is_err());
    }
}
