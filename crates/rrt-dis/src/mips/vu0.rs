//! VU0 macro mode: the COP2 operations with bit 25 set on the R5900. The
//! tables are the VU upper and lower instructions COP2 exposes, as PCSX2's
//! COP2 tables and piney's decoder (checked against Sony's libvu0 code in a
//! retail ELF) lay them out. SPECIAL1 is keyed by the function field;
//! function 0x3c-0x3f escape to SPECIAL2, keyed by bits 6-10 and 0-1
//! together. Encodings in neither table decode as invalid. The MAC and
//! status flags every FMAC operation updates are not listed as written.

use super::decode::{A, Def, T, d, di, dz};
use super::regs::{VACC, VCLIP, VI_REG, VQ};
use crate::insn::Reg;

/// `vnop`, `vwaitq`: dest, ft and fs must be zero.
const NONE: u32 = 0x01ff_f800;

const ACC: &[Reg] = &[VACC];

/// The macro operation `w` (COP2, rs >= 0x10), if it is one.
pub(super) fn macro_op(w: u32) -> Option<Def> {
    let f = w & 63;
    if f < 0x3c { special1(f) } else { special2((((w >> 6) & 31) << 2) | (w & 3)) }
}

fn special1(f: u32) -> Option<Def> {
    use A::*;
    use T::*;
    // The broadcast forms: base op by bits 2-5, the ft field by bits 0-1.
    const BC: [&str; 28] = [
        "vaddx", "vaddy", "vaddz", "vaddw", "vsubx", "vsuby", "vsubz", "vsubw", "vmaddx", "vmaddy", "vmaddz", "vmaddw",
        "vmsubx", "vmsuby", "vmsubz", "vmsubw", "vmaxx", "vmaxy", "vmaxz", "vmaxw", "vminix", "vminiy", "vminiz",
        "vminiw", "vmulx", "vmuly", "vmulz", "vmulw",
    ];
    let bc = &[Vfd(W), Vfs(R), VftBc];
    let q = &[Vfd(W), Vfs(R), Q(R)];
    let i = &[Vfd(W), Vfs(R), I];
    let full = &[Vfd(W), Vfs(R), Vft(R)];
    let qr: &'static [Reg] = &[VQ];
    let ir: &'static [Reg] = &[VI_REG];
    // vmadd*/vmsub*/vopmsub accumulate into ACC without naming it.
    let qacc: &'static [Reg] = &[VQ, VACC];
    let iacc: &'static [Reg] = &[VI_REG, VACC];
    let def = match f {
        0x00..=0x1b => {
            let mn = BC[f as usize];
            let imp = if (0x08..0x10).contains(&f) { ACC } else { &[] };
            di(mn, bc, imp, &[]).dest()
        }
        0x1c => di("vmulq", q, qr, &[]).dest(),
        0x1d => di("vmaxi", i, ir, &[]).dest(),
        0x1e => di("vmuli", i, ir, &[]).dest(),
        0x1f => di("vminii", i, ir, &[]).dest(),
        0x20 => di("vaddq", q, qr, &[]).dest(),
        0x21 => di("vmaddq", q, qacc, &[]).dest(),
        0x22 => di("vaddi", i, ir, &[]).dest(),
        0x23 => di("vmaddi", i, iacc, &[]).dest(),
        0x24 => di("vsubq", q, qr, &[]).dest(),
        0x25 => di("vmsubq", q, qacc, &[]).dest(),
        0x26 => di("vsubi", i, ir, &[]).dest(),
        0x27 => di("vmsubi", i, iacc, &[]).dest(),
        0x28 => d("vadd", full).dest(),
        0x29 => di("vmadd", full, ACC, &[]).dest(),
        0x2a => d("vmul", full).dest(),
        0x2b => d("vmax", full).dest(),
        0x2c => d("vsub", full).dest(),
        0x2d => di("vmsub", full, ACC, &[]).dest(),
        0x2e => di("vopmsub", full, ACC, &[]).dest(),
        0x2f => d("vmini", full).dest(),
        0x30 => d("viadd", &[Vid(W), Vis(R), Vit(R)]),
        0x31 => d("visub", &[Vid(W), Vis(R), Vit(R)]),
        0x32 => d("viaddi", &[Vit(W), Vis(R), Vimm5]),
        0x34 => d("viand", &[Vid(W), Vis(R), Vit(R)]),
        0x35 => d("vior", &[Vid(W), Vis(R), Vit(R)]),
        0x38 => d("vcallms", &[Vimm15]),
        0x39 => d("vcallmsr", &[VuCtrl(R)]),
        _ => return None,
    };
    Some(def)
}

fn special2(code: u32) -> Option<Def> {
    use A::*;
    use T::*;
    const BC: [&str; 16] = [
        "vaddax", "vadday", "vaddaz", "vaddaw", "vsubax", "vsubay", "vsubaz", "vsubaw", "vmaddax", "vmadday",
        "vmaddaz", "vmaddaw", "vmsubax", "vmsubay", "vmsubaz", "vmsubaw",
    ];
    const MULA: [&str; 4] = ["vmulax", "vmulay", "vmulaz", "vmulaw"];
    const ITOF: [&str; 4] = ["vitof0", "vitof4", "vitof12", "vitof15"];
    const FTOI: [&str; 4] = ["vftoi0", "vftoi4", "vftoi12", "vftoi15"];
    let bc_w = &[Acc(W), Vfs(R), VftBc];
    let bc_rw = &[Acc(Rw), Vfs(R), VftBc];
    let q_w = &[Acc(W), Vfs(R), Q(R)];
    let q_rw = &[Acc(Rw), Vfs(R), Q(R)];
    let i_w = &[Acc(W), Vfs(R), I];
    let i_rw = &[Acc(Rw), Vfs(R), I];
    let full_w = &[Acc(W), Vfs(R), Vft(R)];
    let full_rw = &[Acc(Rw), Vfs(R), Vft(R)];
    let move_ = &[Vft(W), Vfs(R)];
    let qr: &'static [Reg] = &[VQ];
    let ir: &'static [Reg] = &[VI_REG];
    let def = match code {
        // vmadda*/vmsuba* read the accumulator they add into.
        0x00..=0x07 => d(BC[code as usize], bc_w).dest(),
        0x08..=0x0f => d(BC[code as usize], bc_rw).dest(),
        0x10..=0x13 => d(ITOF[(code & 3) as usize], move_).dest(),
        0x14..=0x17 => d(FTOI[(code & 3) as usize], move_).dest(),
        0x18..=0x1b => d(MULA[(code & 3) as usize], bc_w).dest(),
        0x1c => di("vmulaq", q_w, qr, &[]).dest(),
        0x1d => d("vabs", move_).dest(),
        0x1e => di("vmulai", i_w, ir, &[]).dest(),
        0x1f => di("vclipw", &[Vfs(R), VftBc], &[], &[VCLIP]).dest(),
        0x20 => di("vaddaq", q_w, qr, &[]).dest(),
        0x21 => di("vmaddaq", q_rw, qr, &[]).dest(),
        0x22 => di("vaddai", i_w, ir, &[]).dest(),
        0x23 => di("vmaddai", i_rw, ir, &[]).dest(),
        0x24 => di("vsubaq", q_w, qr, &[]).dest(),
        0x25 => di("vmsubaq", q_rw, qr, &[]).dest(),
        0x26 => di("vsubai", i_w, ir, &[]).dest(),
        0x27 => di("vmsubai", i_rw, ir, &[]).dest(),
        0x28 => d("vadda", full_w).dest(),
        0x29 => d("vmadda", full_rw).dest(),
        0x2a => d("vmula", full_w).dest(),
        0x2c => d("vsuba", full_w).dest(),
        0x2d => d("vmsuba", full_rw).dest(),
        0x2e => d("vopmula", full_w).dest(),
        0x2f => dz("vnop", &[], NONE),
        0x30 => d("vmove", move_).dest(),
        0x31 => d("vmr32", move_).dest(),
        0x34 => d("vlqi", &[Vft(W), VisInc]).dest(),
        0x35 => d("vsqi", &[Vfs(R), VitInc]).dest(),
        0x36 => d("vlqd", &[Vft(W), VisDec]).dest(),
        0x37 => d("vsqd", &[Vfs(R), VitDec]).dest(),
        0x38 => d("vdiv", &[Q(W), VfsF, VftF]),
        0x39 => d("vsqrt", &[Q(W), VftF]),
        0x3a => d("vrsqrt", &[Q(W), VfsF, VftF]),
        0x3b => di("vwaitq", &[], qr, &[]).zero(NONE),
        0x3c => d("vmtir", &[Vit(W), VfsF]),
        0x3d => d("vmfir", &[Vft(W), Vis(R)]).dest(),
        0x3e => d("vilwr", &[Vit(W), VisAt]).dest(),
        0x3f => d("viswr", &[Vit(R), VisAt]).dest(),
        0x40 => d("vrnext", &[Vft(W), Rand(Rw)]).dest(),
        0x41 => d("vrget", &[Vft(W), Rand(R)]).dest(),
        0x42 => d("vrinit", &[Rand(W), VfsF]),
        0x43 => d("vrxor", &[Rand(Rw), VfsF]),
        _ => return None,
    };
    Some(def)
}
