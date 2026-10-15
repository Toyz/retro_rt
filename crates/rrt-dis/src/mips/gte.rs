//! The PS1 GTE's commands (COP2 with bit 25 set on the R3000A): their names
//! by function field, and the GTE registers each reads and writes, from the
//! command descriptions in psx-spx. The views of other registers (`irgb`,
//! `orgb` over ir1-ir3, `sxyp` over the screen XY FIFO, `lzcr`) are not
//! listed; a command's writes name the registers it computes.

use crate::insn::{Reg, RegFile};

/// One command: name and the data/control registers it touches, as bit
/// masks over COP2 data and control registers 0-31.
#[derive(Clone, Copy, Debug)]
pub(super) struct Command {
    /// The mnemonic.
    pub name: &'static str,
    /// Data registers read.
    pub data_r: u32,
    /// Control registers read.
    pub ctrl_r: u32,
    /// Data registers written.
    pub data_w: u32,
    /// Control registers written.
    pub ctrl_w: u32,
}

const fn bits(lo: u32, hi: u32) -> u32 {
    let mut m = 0;
    let mut i = lo;
    while i <= hi {
        m |= 1 << i;
        i += 1;
    }
    m
}

// Data registers.
const V0: u32 = bits(0, 1);
const V1: u32 = bits(2, 3);
const V2: u32 = bits(4, 5);
const RGBC: u32 = 1 << 6;
const OTZ: u32 = 1 << 7;
const IR0: u32 = 1 << 8;
const IR: u32 = bits(9, 11);
const SXY_R: u32 = bits(13, 14);
const SXY_W: u32 = bits(12, 14);
const SZ_R: u32 = bits(17, 19);
const SZ_W: u32 = bits(16, 19);
const RGB_R: u32 = bits(21, 22);
const RGB_W: u32 = bits(20, 22);
const MAC0: u32 = 1 << 24;
const MAC: u32 = bits(25, 27);

// Control registers.
const RT: u32 = bits(0, 4);
const TR: u32 = bits(5, 7);
const LLM: u32 = bits(8, 12);
const BK: u32 = bits(13, 15);
const LCM: u32 = bits(16, 20);
const FC: u32 = bits(21, 23);
const OF: u32 = bits(24, 25);
const H: u32 = 1 << 26;
const DQ: u32 = bits(27, 28);
const ZSF3: u32 = 1 << 29;
const ZSF4: u32 = 1 << 30;
const FLAG: u32 = 1 << 31;

const fn cmd(name: &'static str, data_r: u32, ctrl_r: u32, data_w: u32) -> Command {
    Command { name, data_r, ctrl_r, data_w, ctrl_w: FLAG }
}

// The perspective transforms push the screen XY and Z FIFOs; the colour
// commands push the colour FIFO and copy the code byte from rgbc.
const RTP_W: u32 = SXY_W | SZ_W | IR0 | IR | MAC0 | MAC;
const COLOR_W: u32 = MAC | IR | RGB_W;

/// The command with function field `funct`, if it is one.
pub(super) fn command(funct: u32) -> Option<Command> {
    Some(match funct {
        0x01 => cmd("rtps", V0 | SXY_R | SZ_R, RT | TR | OF | H | DQ, RTP_W),
        0x06 => cmd("nclip", bits(12, 14), 0, MAC0),
        0x0c => cmd("op", IR, (1 << 0) | (1 << 2) | (1 << 4), MAC | IR),
        0x10 => cmd("dpcs", RGBC | IR0 | RGB_R, FC, COLOR_W),
        0x11 => cmd("intpl", IR | IR0 | RGBC | RGB_R, FC, COLOR_W),
        0x12 => cmd("mvmva", 0, 0, MAC | IR),
        0x13 => cmd("ncds", V0 | RGBC | IR0 | RGB_R, LLM | LCM | BK | FC, COLOR_W),
        0x14 => cmd("cdp", IR | RGBC | IR0 | RGB_R, LCM | BK | FC, COLOR_W),
        0x16 => cmd("ncdt", V0 | V1 | V2 | RGBC | IR0 | RGB_R, LLM | LCM | BK | FC, COLOR_W),
        0x1b => cmd("nccs", V0 | RGBC | RGB_R, LLM | LCM | BK, COLOR_W),
        0x1c => cmd("cc", IR | RGBC | RGB_R, LCM | BK, COLOR_W),
        0x1e => cmd("ncs", V0 | RGBC | RGB_R, LLM | LCM | BK, COLOR_W),
        0x20 => cmd("nct", V0 | V1 | V2 | RGBC | RGB_R, LLM | LCM | BK, COLOR_W),
        0x28 => cmd("sqr", IR, 0, MAC | IR),
        0x29 => cmd("dcpl", IR | RGBC | IR0 | RGB_R, FC, COLOR_W),
        0x2a => cmd("dpct", (1 << 20) | RGBC | IR0 | RGB_R, FC, COLOR_W),
        0x2d => cmd("avsz3", SZ_R, ZSF3, MAC0 | OTZ),
        0x2e => cmd("avsz4", SZ_R | (1 << 16), ZSF4, MAC0 | OTZ),
        0x30 => cmd("rtpt", V0 | V1 | V2 | SXY_R | SZ_R, RT | TR | OF | H | DQ, RTP_W),
        0x3d => cmd("gpf", IR | IR0 | RGBC | RGB_R, 0, COLOR_W),
        0x3e => cmd("gpl", IR | IR0 | MAC | RGBC | RGB_R, 0, COLOR_W),
        0x3f => cmd("ncct", V0 | V1 | V2 | RGBC | RGB_R, LLM | LCM | BK, COLOR_W),
        _ => return None,
    })
}

/// What `mvmva` reads, by its mx (matrix), v (vector) and cv (translation)
/// fields: (data, control) masks. mx 3 is the hardware's garbage matrix,
/// built from rgbc, ir0 and parts of the rotation matrix.
pub(super) fn mvmva_reads(word: u32) -> (u32, u32) {
    let (mx, v, cv) = ((word >> 17) & 3, (word >> 15) & 3, (word >> 13) & 3);
    let (mut data, mut ctrl) = match mx {
        0 => (0, RT),
        1 => (0, LLM),
        2 => (0, LCM),
        _ => (RGBC | IR0, bits(1, 2)),
    };
    data |= [V0, V1, V2, IR][v as usize];
    ctrl |= [TR, BK, FC, 0][cv as usize];
    (data, ctrl)
}

/// The registers a mask names, in index order.
pub(super) fn regs(mask: u32, file: RegFile) -> impl Iterator<Item = Reg> {
    (0..32u8).filter(move |i| mask & (1 << i) != 0).map(move |index| Reg { file, index })
}

#[cfg(test)]
mod tests {
    use super::super::regs::GTE_DATA;
    use super::*;

    #[test]
    fn masks_name_the_registers_psx_spx_names() {
        let rtps = command(0x01).unwrap();
        let names: Vec<_> = regs(rtps.data_r, RegFile::Cop2Data).map(|r| GTE_DATA[r.index as usize]).collect();
        assert_eq!(names, ["vxy0", "vz0", "sxy1", "sxy2", "sz1", "sz2", "sz3"]);
        assert!(command(0x00).is_none() && command(0x3c).is_none());
        assert_eq!(mvmva_reads((1 << 17) | (3 << 15) | (3 << 13)), (IR, LLM));
    }
}
