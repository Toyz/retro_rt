---
title: rrt-disc, game disc images
status: solid
crates: rrt-disc
covers: rrt_disc::Image, rrt_disc::Image::open, rrt_disc::Image::with_layout, rrt_disc::Image::find, rrt_disc::Image::sectors, rrt_disc::Image::sector, rrt_disc::Image::read_blocks, rrt_disc::Image::iso, rrt_disc::Layout, rrt_disc::Sector, rrt_disc::submode, rrt_disc::Cue, rrt_disc::Track, rrt_disc::TrackKind, rrt_disc::Iso, rrt_disc::Iso::walk, rrt_disc::Iso::find, rrt_disc::Iso::read, rrt_disc::Iso::read_path, rrt_disc::DirEntry, rrt_disc::DirEntry::sectors, rrt_disc::DirEntry::is_form2, rrt_disc::Pvd
---

# rrt-disc

Disc images and the ISO 9660 file system on them. Reads go to the file as
asked; nothing loads the image whole, so a 4 GB PS2 DVD costs no memory.

## Opening

`Image::open(path)` by extension:

| path | layout | notes |
| --- | --- | --- |
| `.cue` | `Layout::Raw` | track 1 must be data (`MODE1/2352` or `MODE2/2352`); `Image::cue` keeps the sheet |
| `.iso` | `Layout::Cooked` | 2048-byte blocks: a DVD, or a CD ripped to user data |
| other | `Raw` if the size divides by 2352 and the file starts with the sync pattern, else `Cooked` | a bare `.bin` |

A file that is not a whole number of sectors is refused. `Image::find(dir)`
returns the one `.cue` in a directory, else the one `.iso`; several of a
kind, or none, is an error.

## Sectors

```
raw sector, 2352 bytes
  0x000  [u8; 12]  sync           00 ff*10 00
  0x00c  [u8; 3]   address        minutes, seconds, frames, BCD (150-sector lead-in included)
  0x00f  u8        mode           1, or 2 for CD-XA
  mode 1:  0x010  user data, 2048
  mode 2:  0x010  subheader [file, channel, submode, coding] twice
           0x018  user data: 2048 (Form 1) or 2324 (Form 2, submode 0x20)
```

`Image::sector(lba)` gives a `Sector` (`mode`, `msf`, `subheader`, `file`,
`channel`, `submode`, `coding`, `is_form2`, `data`, `block`); a cooked image
has none and returns `Error::NotRaw`. `Image::read_blocks(lba, size)` gives
Form 1 user data from either layout, which is what the file system is read
through.

## ISO 9660

`Image::iso()` reads the primary volume descriptor at sector 16 (`Pvd`) and
refuses a sector that is not one. The directory tree is walked once on first
use and kept (`Iso::walk`). `Iso::find` matches a path case-insensitively
with or without the leading `/` and the `;1`; `Iso::read_path` finds and
reads. A record's CD-XA field, when present, gives `DirEntry::xa_attr` and
`xa_file`; `is_form2` marks files (XA audio, STR video) that `Iso::read`
cannot give whole - read their sectors raw. Identifier fields are trimmed of
trailing spaces and NULs (some mastering tools pad with NULs).

A file of 4 GiB or more is recorded as several records of one name, each
but the last flagged not final (flag 0x80). They become one `DirEntry`:
`extents` lists each part's sector and size, `size` is the total (`u64`),
and `Iso::read` reads them end to end.

## Tests

On one synthetic disc written both cooked and as raw Mode 2 sectors with a
CUE sheet: `a_cooked_image_reads_blocks_and_refuses_raw_sectors`,
`a_cue_sheet_opens_its_raw_data_track`, `a_form2_sector_gives_2324_bytes`,
`a_bare_bin_is_recognised_as_raw_by_its_sync`, `find_prefers_a_cue_and_refuses_two`,
`a_sheet_whose_first_track_is_audio_is_refused`,
`a_file_that_is_not_whole_sectors_is_refused`, `bcd_msf_decodes`,
`parses_data_and_audio_tracks`, and for both layouts
`finds_and_reads_a_file_case_insensitively`, `the_cd_xa_field_is_read`,
`a_multi_extent_file_reads_as_one`, `a_bad_volume_descriptor_is_refused`.
The raw path is the one hwtr-disc reads Hot Wheels Turbo Racing's US disc
with.

## Not here

CD-DA and XA-ADPCM decoding (a game's audio crate decodes them from
`Image::sector`), `SYSTEM.CNF`, executables, and the extensions no PS1 or PS2
disc uses: Joliet, Rock Ridge, UDF.

## Gaps

Nothing known.
