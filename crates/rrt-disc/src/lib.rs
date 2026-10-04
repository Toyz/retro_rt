//! Game discs, read from their images.
//!
//! [`Image`] opens a disc image by its path: a CUE sheet (its first track
//! must be data, stored as raw 2352-byte sectors), a raw `.bin` with no
//! sheet, or a cooked `.iso` of 2048-byte blocks (a DVD, or a CD ripped to
//! user data only). Reads go to the file as they are asked for; a 4 GB DVD is
//! never held in memory. [`Image::sector`] gives a raw sector with its
//! header and CD-XA subheader ([`Sector`]); [`Image::read_blocks`] gives
//! Form 1 user data, which is what [`Iso`] reads the ISO 9660 file system
//! through. See `docs/crates/rrt-disc.md`.
//!
//! Not here: CD-DA audio decoding and XA-ADPCM, which a game's audio crate
//! decodes from [`Image::sector`]; the console-specific boot files
//! (`SYSTEM.CNF`) and executables.

pub mod cue;
pub mod iso;

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use cue::{Cue, Track, TrackKind};
pub use iso::{DirEntry, Iso, Pvd};

/// Bytes in a raw CD sector.
pub const RAW_SECTOR: usize = 2352;
/// User data bytes in a Mode 1 or Mode 2 Form 1 sector, and in an ISO 9660
/// logical block.
pub const FORM1_DATA: usize = 2048;
/// User data bytes in a Mode 2 Form 2 sector.
pub const FORM2_DATA: usize = 2324;
/// CD sectors a second at 1x.
pub const SECTORS_PER_SECOND: u32 = 75;
/// The sync pattern every raw data sector starts with.
pub const SYNC: [u8; 12] = [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0];

/// Why an image could not be read.
#[derive(Debug)]
pub enum Error {
    /// The file could not be opened or read.
    Io(PathBuf, std::io::Error),
    /// The CUE sheet is malformed or describes something unsupported.
    Cue(String),
    /// The ISO 9660 file system is malformed, or a path is not on it.
    Iso(String),
    /// A sector past the end of the data track.
    OutOfRange(u32),
    /// Raw sectors asked of a cooked image, which has none.
    NotRaw,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(path, e) => write!(f, "{}: {e}", path.display()),
            Error::Cue(msg) => write!(f, "cue sheet: {msg}"),
            Error::Iso(msg) => write!(f, "iso 9660: {msg}"),
            Error::OutOfRange(lba) => write!(f, "sector {lba} is past the end of the data track"),
            Error::NotRaw => write!(f, "a cooked 2048-byte image has no raw sectors"),
        }
    }
}

impl std::error::Error for Error {}

/// This crate's results.
pub type Result<T> = std::result::Result<T, Error>;

/// How the data track's sectors are stored in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// 2352 bytes a sector: sync, header, subheader, data, EDC/ECC.
    Raw,
    /// 2048 bytes a sector: Form 1 user data only.
    Cooked,
}

/// One raw sector of the data track.
#[derive(Clone)]
pub struct Sector {
    /// The 2352 bytes as stored.
    pub raw: Box<[u8; RAW_SECTOR]>,
}

/// The CD-XA subheader's submode bits.
pub mod submode {
    /// End of record.
    pub const EOR: u8 = 0x01;
    /// Video data (STR).
    pub const VIDEO: u8 = 0x02;
    /// XA-ADPCM audio.
    pub const AUDIO: u8 = 0x04;
    /// Data.
    pub const DATA: u8 = 0x08;
    /// Trigger: an interrupt for the reading program.
    pub const TRIGGER: u8 = 0x10;
    /// Form 2: 2324 bytes of user data, no ECC.
    pub const FORM2: u8 = 0x20;
    /// Real-time sector.
    pub const REALTIME: u8 = 0x40;
    /// End of file.
    pub const EOF: u8 = 0x80;
}

impl Sector {
    /// The header's mode byte: 1, or 2 for CD-XA.
    pub fn mode(&self) -> u8 {
        self.raw[15]
    }

    /// The header's address, as minutes, seconds and frames, decoded from BCD.
    pub fn msf(&self) -> (u8, u8, u8) {
        let bcd = |b: u8| (b >> 4) * 10 + (b & 15);
        (bcd(self.raw[12]), bcd(self.raw[13]), bcd(self.raw[14]))
    }

    /// The CD-XA subheader: file number, channel, submode, coding info.
    pub fn subheader(&self) -> [u8; 4] {
        [self.raw[16], self.raw[17], self.raw[18], self.raw[19]]
    }

    /// The subheader's file number.
    pub fn file(&self) -> u8 {
        self.raw[16]
    }

    /// The subheader's channel number.
    pub fn channel(&self) -> u8 {
        self.raw[17]
    }

    /// The subheader's submode ([`submode`] bits).
    pub fn submode(&self) -> u8 {
        self.raw[18]
    }

    /// The subheader's coding info (for XA audio: rate, bits, stereo).
    pub fn coding(&self) -> u8 {
        self.raw[19]
    }

    /// Mode 2 with the Form 2 bit set.
    pub fn is_form2(&self) -> bool {
        self.mode() == 2 && self.submode() & submode::FORM2 != 0
    }

    /// The user data: 2048 bytes for Mode 1 and Form 1, 2324 for Form 2.
    pub fn data(&self) -> &[u8] {
        match self.mode() {
            1 => &self.raw[16..16 + FORM1_DATA],
            _ if self.is_form2() => &self.raw[24..24 + FORM2_DATA],
            _ => &self.raw[24..24 + FORM1_DATA],
        }
    }

    /// Form 1 user data whatever the submode says, as an ISO 9660 reader
    /// sees the sector.
    pub fn block(&self) -> &[u8] {
        match self.mode() {
            1 => &self.raw[16..16 + FORM1_DATA],
            _ => &self.raw[24..24 + FORM1_DATA],
        }
    }
}

/// A disc image: its data track, read from the file on demand.
pub struct Image {
    /// The file the data track is in.
    pub path: PathBuf,
    /// The CUE sheet, when the image was opened through one.
    pub cue: Option<Cue>,
    /// How the data track's sectors are stored.
    pub layout: Layout,
    sectors: u32,
    file: Mutex<File>,
}

impl Image {
    /// Opens an image by its extension: `.cue` through its sheet (track 1
    /// must be raw data), `.iso` as cooked 2048-byte blocks, anything else as
    /// raw if it divides into 2352-byte sectors starting with the sync
    /// pattern, else cooked.
    pub fn open(path: impl AsRef<Path>) -> Result<Image> {
        let path = path.as_ref();
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("cue") => {
                let cue = Cue::load(path)?;
                let first = cue.tracks.first().ok_or_else(|| Error::Cue("no tracks".into()))?;
                if first.kind == TrackKind::Audio {
                    return Err(Error::Cue("track 1 is audio, not a data track".into()));
                }
                let mut image = Image::with_layout(&first.file.clone(), Layout::Raw)?;
                image.cue = Some(cue);
                Ok(image)
            }
            Some("iso") => Image::with_layout(path, Layout::Cooked),
            _ => {
                let mut f = File::open(path).map_err(|e| Error::Io(path.to_path_buf(), e))?;
                let len = f.metadata().map_err(|e| Error::Io(path.to_path_buf(), e))?.len();
                let mut head = [0u8; 12];
                let raw = len % RAW_SECTOR as u64 == 0 && f.read_exact(&mut head).is_ok() && head == SYNC;
                Image::with_layout(path, if raw { Layout::Raw } else { Layout::Cooked })
            }
        }
    }

    /// Opens `path` as a data track stored in `layout`.
    pub fn with_layout(path: &Path, layout: Layout) -> Result<Image> {
        let file = File::open(path).map_err(|e| Error::Io(path.to_path_buf(), e))?;
        let len = file.metadata().map_err(|e| Error::Io(path.to_path_buf(), e))?.len();
        let size = match layout {
            Layout::Raw => RAW_SECTOR,
            Layout::Cooked => FORM1_DATA,
        } as u64;
        if len % size != 0 {
            return Err(Error::Cue(format!("{} is not a whole number of {size}-byte sectors", path.display())));
        }
        Ok(Image { path: path.to_path_buf(), cue: None, layout, sectors: (len / size) as u32, file: Mutex::new(file) })
    }

    /// The one disc image in `dir`: a `.cue`, else an `.iso`. An error when
    /// there are none or several of the kind found.
    pub fn find(dir: &Path) -> Result<PathBuf> {
        let entries = std::fs::read_dir(dir).map_err(|e| Error::Io(dir.to_path_buf(), e))?;
        let files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        for ext in ["cue", "iso"] {
            let mut found: Vec<&PathBuf> =
                files.iter().filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext))).collect();
            found.sort();
            match found.len() {
                0 => continue,
                1 => return Ok(found[0].clone()),
                _ => return Err(Error::Cue(format!("several .{ext} files in {}", dir.display()))),
            }
        }
        Err(Error::Cue(format!("no .cue or .iso in {}", dir.display())))
    }

    /// Sectors on the data track.
    pub fn sectors(&self) -> u32 {
        self.sectors
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        f.seek(SeekFrom::Start(offset)).and_then(|_| f.read_exact(buf)).map_err(|e| Error::Io(self.path.clone(), e))
    }

    /// The raw sector at `lba`. [`Error::NotRaw`] for a cooked image.
    pub fn sector(&self, lba: u32) -> Result<Sector> {
        if self.layout != Layout::Raw {
            return Err(Error::NotRaw);
        }
        if lba >= self.sectors {
            return Err(Error::OutOfRange(lba));
        }
        let mut raw = Box::new([0u8; RAW_SECTOR]);
        self.read_at(u64::from(lba) * RAW_SECTOR as u64, &mut raw[..])?;
        Ok(Sector { raw })
    }

    /// `size` bytes of Form 1 user data from `lba` on, across as many
    /// sectors as it takes.
    pub fn read_blocks(&self, lba: u32, size: u32) -> Result<Vec<u8>> {
        let count = (size as usize).div_ceil(FORM1_DATA) as u32;
        if count > 0 && lba.checked_add(count - 1).is_none_or(|last| last >= self.sectors) {
            return Err(Error::OutOfRange(lba.saturating_add(count.saturating_sub(1))));
        }
        let mut out = Vec::with_capacity(count as usize * FORM1_DATA);
        match self.layout {
            Layout::Cooked => {
                out.resize(count as usize * FORM1_DATA, 0);
                self.read_at(u64::from(lba) * FORM1_DATA as u64, &mut out)?;
            }
            Layout::Raw => {
                for i in 0..count {
                    out.extend_from_slice(self.sector(lba + i)?.block());
                }
            }
        }
        out.truncate(size as usize);
        Ok(out)
    }

    /// The ISO 9660 file system on the data track.
    pub fn iso(&self) -> Result<Iso<'_>> {
        Iso::open(self)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One directory record: name, first sector, size, flags, and the CD-XA
    /// field's attributes and file number when given.
    fn record(name: &[u8], lba: u32, size: u32, flags: u8, xa: Option<(u16, u8)>) -> Vec<u8> {
        let mut r = vec![0u8; 33];
        r[2..6].copy_from_slice(&lba.to_le_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[25] = flags;
        r[32] = name.len() as u8;
        r.extend_from_slice(name);
        if r.len() % 2 == 1 {
            r.push(0);
        }
        if let Some((attr, file)) = xa {
            let mut f = [0u8; 14];
            f[4..6].copy_from_slice(&attr.to_be_bytes());
            f[6..8].copy_from_slice(b"XA");
            f[8] = file;
            r.extend_from_slice(&f);
        }
        r[0] = r.len() as u8;
        r
    }

    /// A disc's 24 user-data blocks: a PVD at 16, the root directory at 18
    /// holding `HELLO.TXT` (block 20, with a CD-XA field) and `BIG.DAT`, a
    /// file of two extents (block 21, 2048 bytes of `A`, flagged not final;
    /// then block 22, `tail!`).
    pub fn blocks() -> Vec<u8> {
        let mut img = vec![0u8; 24 * FORM1_DATA];
        let pvd = &mut img[16 * FORM1_DATA..17 * FORM1_DATA];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[8..19].copy_from_slice(b"PLAYSTATION");
        pvd[40..44].copy_from_slice(b"TEST");
        pvd[80..84].copy_from_slice(&24u32.to_le_bytes());
        pvd[156..190].copy_from_slice(&record(&[0], 18, 2048, 2, None)[..34]);
        let mut dir = Vec::new();
        dir.extend(record(b"BIG.DAT;1", 21, 2048, 0x80, None));
        dir.extend(record(b"BIG.DAT;1", 22, 5, 0, None));
        dir.extend(record(b"HELLO.TXT;1", 20, 5, 0, Some((iso::xa::FORM1, 1))));
        img[18 * FORM1_DATA..18 * FORM1_DATA + dir.len()].copy_from_slice(&dir);
        img[20 * FORM1_DATA..20 * FORM1_DATA + 5].copy_from_slice(b"hello");
        img[21 * FORM1_DATA..22 * FORM1_DATA].fill(b'A');
        img[22 * FORM1_DATA..22 * FORM1_DATA + 5].copy_from_slice(b"tail!");
        img
    }

    /// A scratch directory for one test.
    pub fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rrt-disc-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// [`blocks`] as a cooked `.iso`.
    pub fn tiny_iso(dir: &Path) -> PathBuf {
        let path = dir.join("tiny.iso");
        std::fs::write(&path, blocks()).unwrap();
        path
    }

    /// One raw Mode 2 sector at `lba`: sync, BCD address (150 sectors of
    /// lead-in added), the subheader twice, `data`.
    fn raw_sector(lba: u32, submode: u8, data: &[u8]) -> Vec<u8> {
        let mut raw = vec![0u8; RAW_SECTOR];
        raw[..12].copy_from_slice(&SYNC);
        let at = lba + 150;
        let bcd = |v: u32| (((v / 10) << 4) | (v % 10)) as u8;
        raw[12..15].copy_from_slice(&[bcd(at / 75 / 60), bcd(at / 75 % 60), bcd(at % 75)]);
        raw[15] = 2;
        let sub = [1, 0, submode, 0];
        raw[16..20].copy_from_slice(&sub);
        raw[20..24].copy_from_slice(&sub);
        raw[24..24 + data.len()].copy_from_slice(data);
        raw
    }

    /// [`blocks`] as raw Form 1 sectors, then one Form 2 sector (lba 24)
    /// full of `F`, in `tiny.bin`, with a sheet `tiny.cue` naming it as track
    /// 1 and an audio track 2.
    pub fn tiny_raw(dir: &Path) -> PathBuf {
        let mut bin = Vec::new();
        for (lba, block) in blocks().chunks(FORM1_DATA).enumerate() {
            bin.extend(raw_sector(lba as u32, submode::DATA, block));
        }
        bin.extend(raw_sector(24, submode::FORM2 | submode::AUDIO, &[b'F'; FORM2_DATA]));
        std::fs::write(dir.join("tiny.bin"), bin).unwrap();
        std::fs::write(dir.join("tiny (Track 2).bin"), vec![0u8; RAW_SECTOR * 4]).unwrap();
        let cue = "FILE \"tiny.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n\
                   FILE \"tiny (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n";
        let path = dir.join("tiny.cue");
        std::fs::write(&path, cue).unwrap();
        path
    }

    #[test]
    fn a_cooked_image_reads_blocks_and_refuses_raw_sectors() {
        let image = Image::open(tiny_iso(&scratch("cooked"))).unwrap();
        assert_eq!((image.layout, image.sectors()), (Layout::Cooked, 24));
        assert_eq!(image.read_blocks(20, 5).unwrap(), b"hello");
        assert!(matches!(image.sector(0), Err(Error::NotRaw)));
        assert!(matches!(image.read_blocks(24, 1), Err(Error::OutOfRange(24))));
    }

    #[test]
    fn a_cue_sheet_opens_its_raw_data_track() {
        let image = Image::open(tiny_raw(&scratch("cue"))).unwrap();
        assert_eq!((image.layout, image.sectors()), (Layout::Raw, 25));
        assert_eq!(image.cue.as_ref().unwrap().tracks.len(), 2);
        let s = image.sector(16).unwrap();
        assert_eq!((s.mode(), s.msf(), s.file(), s.submode()), (2, (0, 2, 16), 1, submode::DATA));
        assert_eq!(&s.block()[1..6], b"CD001");
        assert!(!s.is_form2());
        assert_eq!(image.read_blocks(20, 5).unwrap(), b"hello");
        assert!(matches!(image.sector(25), Err(Error::OutOfRange(25))));
    }

    #[test]
    fn a_form2_sector_gives_2324_bytes() {
        let image = Image::open(tiny_raw(&scratch("form2"))).unwrap();
        let s = image.sector(24).unwrap();
        assert!(s.is_form2());
        assert_eq!(s.data(), [b'F'; FORM2_DATA]);
        assert_eq!(s.block().len(), FORM1_DATA);
    }

    #[test]
    fn a_bare_bin_is_recognised_as_raw_by_its_sync() {
        let dir = scratch("bare");
        tiny_raw(&dir);
        assert_eq!(Image::open(dir.join("tiny.bin")).unwrap().layout, Layout::Raw);
        let cooked = dir.join("cooked.img");
        std::fs::write(&cooked, blocks()).unwrap();
        assert_eq!(Image::open(cooked).unwrap().layout, Layout::Cooked);
    }

    #[test]
    fn find_prefers_a_cue_and_refuses_two() {
        let dir = scratch("find");
        tiny_iso(&dir);
        assert_eq!(Image::find(&dir).unwrap(), dir.join("tiny.iso"));
        tiny_raw(&dir);
        assert_eq!(Image::find(&dir).unwrap(), dir.join("tiny.cue"));
        std::fs::copy(dir.join("tiny.cue"), dir.join("other.cue")).unwrap();
        assert!(matches!(Image::find(&dir), Err(Error::Cue(_))));
        assert!(matches!(Image::find(&scratch("empty")), Err(Error::Cue(_))));
    }

    #[test]
    fn a_sheet_whose_first_track_is_audio_is_refused() {
        let dir = scratch("audio-first");
        std::fs::write(dir.join("a.bin"), vec![0u8; RAW_SECTOR]).unwrap();
        std::fs::write(dir.join("a.cue"), "FILE \"a.bin\" BINARY\n  TRACK 01 AUDIO\n    INDEX 01 00:00:00\n").unwrap();
        assert!(matches!(Image::open(dir.join("a.cue")), Err(Error::Cue(_))));
    }

    #[test]
    fn a_file_that_is_not_whole_sectors_is_refused() {
        let dir = scratch("ragged");
        std::fs::write(dir.join("r.iso"), vec![0u8; FORM1_DATA + 1]).unwrap();
        assert!(matches!(Image::open(dir.join("r.iso")), Err(Error::Cue(_))));
    }

    #[test]
    fn bcd_msf_decodes() {
        let mut raw = Box::new([0u8; RAW_SECTOR]);
        raw[12..16].copy_from_slice(&[0x00, 0x02, 0x16, 2]);
        let s = Sector { raw };
        assert_eq!((s.msf(), s.mode()), ((0, 2, 16), 2));
    }
}
