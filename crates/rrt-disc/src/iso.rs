//! ISO 9660, with the CD-XA system use field when a record carries it (as
//! the PlayStation mastering tools wrote it). Plain ISO 9660 only: no Joliet,
//! no Rock Ridge, no UDF, which is what PS1 CDs and PS2 DVDs are.

use std::sync::OnceLock;

use crate::{Error, FORM1_DATA, Image, Result};

/// The CD-XA attribute bits, from the big-endian word in the system use
/// field.
pub mod xa {
    /// Holds Form 1 sectors.
    pub const FORM1: u16 = 0x0800;
    /// Holds Form 2 sectors.
    pub const FORM2: u16 = 0x1000;
    /// Interleaved (XA audio, STR video).
    pub const INTERLEAVED: u16 = 0x2000;
    /// CD-DA.
    pub const CDDA: u16 = 0x4000;
    /// A directory.
    pub const DIRECTORY: u16 = 0x8000;
}

/// One file or directory.
#[derive(Clone, Debug)]
pub struct DirEntry {
    /// The full path from the root, `/`-separated with a leading `/`,
    /// without the `;1` version.
    pub path: String,
    /// Its first sector.
    pub lba: u32,
    /// The recorded size in bytes, every extent together. For Form 2 files
    /// this counts 2048 a sector, not 2324.
    pub size: u64,
    /// Where the file lies: each extent's first sector and size in bytes,
    /// in order. One for most files; a file of 4 GiB or more is recorded as
    /// several (ISO 9660 multi-extent), each under 4 GiB.
    pub extents: Vec<(u32, u32)>,
    /// A directory.
    pub is_dir: bool,
    /// Recording date: years since 1900, month, day, hour, minute, second,
    /// GMT offset in 15-minute steps.
    pub date: [u8; 7],
    /// The CD-XA attributes ([`xa`]), if the record has the XA field.
    pub xa_attr: Option<u16>,
    /// The CD-XA file number.
    pub xa_file: Option<u8>,
}

impl DirEntry {
    /// Sectors the file covers, every extent together.
    pub fn sectors(&self) -> u32 {
        self.extents.iter().map(|(_, size)| size.div_ceil(FORM1_DATA as u32)).sum()
    }

    /// The record says the file holds Form 2 sectors (XA audio, STR video),
    /// which [`Iso::read`] cannot give whole: read them with
    /// [`Image::sector`].
    pub fn is_form2(&self) -> bool {
        self.xa_attr.is_some_and(|a| a & (xa::FORM2 | xa::INTERLEAVED) != 0)
    }
}

/// The primary volume descriptor's fields that matter.
#[derive(Clone, Debug)]
pub struct Pvd {
    /// `PLAYSTATION` on Sony's discs.
    pub system_id: String,
    /// The volume's name.
    pub volume_id: String,
    /// Logical blocks in the volume.
    pub volume_blocks: u32,
    /// Publisher identifier.
    pub publisher: String,
    /// Data preparer identifier.
    pub preparer: String,
    /// Application identifier.
    pub application: String,
    /// Creation date and time as recorded, `YYYYMMDDHHMMSScc`.
    pub created: String,
    /// The root directory's first sector.
    pub root_lba: u32,
    /// The root directory's size in bytes.
    pub root_size: u32,
}

/// The file system on an [`Image`]. The directory tree is walked once, on
/// first use, and kept.
pub struct Iso<'a> {
    /// The image it is read from.
    pub image: &'a Image,
    /// The primary volume descriptor.
    pub pvd: Pvd,
    tree: OnceLock<Vec<DirEntry>>,
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// A d-character field: padded with spaces by the standard, with NULs by
/// some mastering tools.
fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_end_matches([' ', '\0']).to_string()
}

impl<'a> Iso<'a> {
    /// Reads the primary volume descriptor at sector 16.
    pub fn open(image: &'a Image) -> Result<Iso<'a>> {
        let block = image.read_blocks(16, FORM1_DATA as u32)?;
        if block[0] != 1 || &block[1..6] != b"CD001" {
            return Err(Error::Iso("sector 16 is not a primary volume descriptor".into()));
        }
        let root = &block[156..156 + 34];
        let pvd = Pvd {
            system_id: text(&block[8..40]),
            volume_id: text(&block[40..72]),
            volume_blocks: le32(&block, 80),
            publisher: text(&block[318..446]),
            preparer: text(&block[446..574]),
            application: text(&block[574..702]),
            created: text(&block[813..830]),
            root_lba: le32(root, 2),
            root_size: le32(root, 10),
        };
        Ok(Iso { image, pvd, tree: OnceLock::new() })
    }

    /// Every file and directory, depth first, a directory before its
    /// contents.
    pub fn walk(&self) -> Result<&[DirEntry]> {
        if let Some(t) = self.tree.get() {
            return Ok(t);
        }
        let mut out = Vec::new();
        self.walk_dir(self.pvd.root_lba, self.pvd.root_size, "", &mut out, 0)?;
        Ok(self.tree.get_or_init(|| out))
    }

    fn walk_dir(&self, lba: u32, size: u32, prefix: &str, out: &mut Vec<DirEntry>, depth: u32) -> Result<()> {
        if depth > 16 {
            return Err(Error::Iso(format!("directory nesting too deep at {prefix}")));
        }
        let data = self.image.read_blocks(lba, size)?;
        let mut children: Vec<DirEntry> = Vec::new();
        let mut continues = false;
        for chunk in data.chunks(FORM1_DATA) {
            let mut at = 0;
            while at < chunk.len() {
                let len = chunk[at] as usize;
                if len == 0 {
                    break;
                }
                let rec = chunk
                    .get(at..at + len)
                    .filter(|r| r.len() >= 34)
                    .ok_or_else(|| Error::Iso(format!("record overruns its sector in {prefix}/")))?;
                at += len;
                let name_len = rec[32] as usize;
                let name = rec
                    .get(33..33 + name_len)
                    .ok_or_else(|| Error::Iso(format!("name overruns its record in {prefix}/")))?;
                if name == [0] || name == [1] {
                    continue;
                }
                let mut name = String::from_utf8_lossy(name).to_string();
                if let Some(semi) = name.find(';') {
                    name.truncate(semi);
                }
                let su = (33 + name_len + 1) & !1;
                let (xa_attr, xa_file) = match rec.get(su..su + 14) {
                    Some(f) if &f[6..8] == b"XA" => (Some(u16::from_be_bytes([f[4], f[5]])), Some(f[8])),
                    _ => (None, None),
                };
                let (lba, size, flags) = (le32(rec, 2), le32(rec, 10), rec[25]);
                let path = format!("{prefix}/{name}");
                // Flag 0x80: not the final extent; the next record of the
                // same name continues the file.
                if let Some(last) = children.last_mut()
                    && continues
                    && last.path == path
                {
                    last.extents.push((lba, size));
                    last.size += u64::from(size);
                    continues = flags & 0x80 != 0;
                    continue;
                }
                continues = flags & 0x80 != 0;
                children.push(DirEntry {
                    path,
                    lba,
                    size: u64::from(size),
                    extents: vec![(lba, size)],
                    is_dir: flags & 2 != 0,
                    date: rec[18..25].try_into().unwrap(),
                    xa_attr,
                    xa_file,
                });
            }
        }
        for child in children {
            let (is_dir, lba, size, path) = (child.is_dir, child.lba, child.extents[0].1, child.path.clone());
            out.push(child);
            if is_dir {
                self.walk_dir(lba, size, &path, out, depth + 1)?;
            }
        }
        Ok(())
    }

    /// A file by path, case-insensitively, with or without the leading `/`
    /// and the `;1`.
    pub fn find(&self, path: &str) -> Result<DirEntry> {
        let want = format!("/{}", path.trim_start_matches('/').trim_end_matches(";1"));
        self.walk()?
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case(&want))
            .cloned()
            .ok_or_else(|| Error::Iso(format!("no {want} on the disc")))
    }

    /// A file's Form 1 contents, every extent in order.
    pub fn read(&self, entry: &DirEntry) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for &(lba, size) in &entry.extents {
            out.extend(self.image.read_blocks(lba, size)?);
        }
        Ok(out)
    }

    /// A file's contents by path: [`Iso::find`] then [`Iso::read`].
    pub fn read_path(&self, path: &str) -> Result<Vec<u8>> {
        self.read(&self.find(path)?)
    }
}

#[cfg(test)]
mod tests {
    use crate::Image;
    use crate::tests::{scratch, tiny_iso, tiny_raw};

    /// The same file system read from a cooked image and a raw one.
    fn both(test: &str) -> [Image; 2] {
        let dir = scratch(&format!("iso-{test}"));
        [Image::open(tiny_iso(&dir)).unwrap(), Image::open(tiny_raw(&dir)).unwrap()]
    }

    #[test]
    fn finds_and_reads_a_file_case_insensitively() {
        for image in both("find") {
            let iso = image.iso().unwrap();
            assert_eq!((iso.pvd.system_id.as_str(), iso.pvd.volume_id.as_str()), ("PLAYSTATION", "TEST"));
            assert_eq!(iso.read_path("hello.txt;1").unwrap(), b"hello");
            assert_eq!(iso.read_path("/HELLO.TXT").unwrap(), b"hello");
            assert!(iso.find("/NOPE").is_err());
        }
    }

    #[test]
    fn the_cd_xa_field_is_read() {
        for image in both("xa") {
            let e = image.iso().unwrap().find("HELLO.TXT").unwrap();
            assert_eq!((e.xa_attr, e.xa_file), (Some(super::xa::FORM1), Some(1)));
            assert!(!e.is_form2());
        }
    }

    /// `BIG.DAT` is two records of one name, the first flagged not final:
    /// one entry, both extents, read end to end.
    #[test]
    fn a_multi_extent_file_reads_as_one() {
        for image in both("extents") {
            let iso = image.iso().unwrap();
            assert_eq!(iso.walk().unwrap().len(), 2, "BIG.DAT once, HELLO.TXT");
            let big = iso.find("BIG.DAT").unwrap();
            assert_eq!((big.size, big.extents.clone(), big.sectors()), (2053, vec![(21, 2048), (22, 5)], 2));
            let data = iso.read(&big).unwrap();
            assert_eq!(data.len(), 2053);
            assert!(data[..2048].iter().all(|b| *b == b'A'));
            assert_eq!(&data[2048..], b"tail!");
        }
    }

    #[test]
    fn a_bad_volume_descriptor_is_refused() {
        let dir = scratch("iso-no-pvd");
        std::fs::write(dir.join("blank.iso"), vec![0u8; 24 * crate::FORM1_DATA]).unwrap();
        let image = Image::open(dir.join("blank.iso")).unwrap();
        assert!(matches!(image.iso(), Err(crate::Error::Iso(_))));
    }
}
