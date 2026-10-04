//! Binary-coded decimal: CD addresses, real-time clocks, memory card dates.

/// A BCD byte's value: 0x59 is 59. None when either digit is over 9.
pub const fn decode(b: u8) -> Option<u8> {
    let (hi, lo) = (b >> 4, b & 15);
    if hi > 9 || lo > 9 { None } else { Some(hi * 10 + lo) }
}

/// `v` as a BCD byte: 59 is 0x59. None when `v` is over 99.
pub const fn encode(v: u8) -> Option<u8> {
    if v > 99 { None } else { Some(((v / 10) << 4) | (v % 10)) }
}

/// A CD address (minutes, seconds, frames, 75 frames a second) as a sector
/// number from the start of the disc's data, with the 150-sector lead-in
/// taken off. None before the lead-in ends.
pub const fn msf_to_lba(m: u8, s: u8, f: u8) -> Option<u32> {
    let at = (m as u32 * 60 + s as u32) * 75 + f as u32;
    at.checked_sub(150)
}

/// A sector number as the CD address it is read at: the lead-in added.
pub const fn lba_to_msf(lba: u32) -> (u8, u8, u8) {
    let at = lba + 150;
    ((at / 75 / 60) as u8, (at / 75 % 60) as u8, (at % 75) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_round_trips_and_refuses_non_digits() {
        for v in 0..=99 {
            assert_eq!(decode(encode(v).unwrap()), Some(v));
        }
        assert_eq!(decode(0x59), Some(59));
        assert_eq!(decode(0x5a), None);
        assert_eq!(encode(100), None);
    }

    #[test]
    fn sector_16_is_read_at_00_02_16() {
        assert_eq!(lba_to_msf(16), (0, 2, 16));
        assert_eq!(msf_to_lba(0, 2, 16), Some(16));
        assert_eq!(msf_to_lba(0, 1, 0), None, "inside the lead-in");
        assert_eq!(msf_to_lba(74, 59, 74), Some(337_349), "the last sector of an 80-minute disc's address space");
    }
}
