//! Run-length encodings: PackBits and Nintendo RL.

use super::Error;

/// Decodes PackBits: a header byte `n` then, for 0-127, `n + 1` literal
/// bytes; for 129-255, the next byte repeated `257 - n` times; 128 is
/// skipped.
pub fn packbits_decode(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut at = 0;
    while at < data.len() {
        let n = data[at];
        at += 1;
        match n {
            0..=127 => {
                let len = usize::from(n) + 1;
                let lit = data.get(at..at + len).ok_or(Error { at, what: "a literal run cut short" })?;
                out.extend_from_slice(lit);
                at += len;
            }
            128 => {}
            _ => {
                let b = *data.get(at).ok_or(Error { at, what: "a repeat with no byte" })?;
                out.extend(std::iter::repeat_n(b, 257 - usize::from(n)));
                at += 1;
            }
        }
    }
    Ok(out)
}

/// Encodes as PackBits: runs of 3 or more as repeats (up to 128), the rest
/// as literal runs (up to 128).
pub fn packbits_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let flush = |out: &mut Vec<u8>, lit: &mut Vec<u8>| {
        for chunk in lit.chunks(128) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        lit.clear();
    };
    let mut i = 0;
    while i < data.len() {
        let run = data[i..].iter().take(128).take_while(|&&b| b == data[i]).count();
        if run >= 3 {
            flush(&mut out, &mut lit);
            out.push((257 - run) as u8);
            out.push(data[i]);
            i += run;
        } else {
            lit.push(data[i]);
            i += 1;
        }
    }
    flush(&mut out, &mut lit);
    out
}

/// Decodes Nintendo RL (BIOS type 0x30): a 4-byte header (0x30, then the
/// size in 24 bits, little-endian), then flag bytes: bit 7 set, the next byte
/// repeated `(flag & 0x7f) + 3` times; clear, `(flag & 0x7f) + 1` literal
/// bytes.
pub fn rl_decode(data: &[u8]) -> Result<Vec<u8>, Error> {
    let head = data.get(..4).ok_or(Error { at: 0, what: "no RL header" })?;
    if head[0] != 0x30 {
        return Err(Error { at: 0, what: "not RL (type byte is not 0x30)" });
    }
    let size = usize::from(head[1]) | usize::from(head[2]) << 8 | usize::from(head[3]) << 16;
    let mut out = Vec::with_capacity(size);
    let mut at = 4;
    while out.len() < size {
        let flag = *data.get(at).ok_or(Error { at, what: "the data ends before the size it declares" })?;
        at += 1;
        if flag & 0x80 != 0 {
            let b = *data.get(at).ok_or(Error { at, what: "a repeat with no byte" })?;
            out.extend(std::iter::repeat_n(b, usize::from(flag & 0x7f) + 3));
            at += 1;
        } else {
            let len = usize::from(flag & 0x7f) + 1;
            let lit = data.get(at..at + len).ok_or(Error { at, what: "a literal run cut short" })?;
            out.extend_from_slice(lit);
            at += len;
        }
    }
    out.truncate(size);
    Ok(out)
}

/// Encodes as Nintendo RL with its header: runs of 3-130 as repeats, the
/// rest as literal runs of up to 128.
///
/// # Panics
///
/// When `data` is 16 MiB or longer (the header's 24-bit size).
pub fn rl_encode(data: &[u8]) -> Vec<u8> {
    assert!(data.len() < 1 << 24, "RL holds under 16 MiB");
    let n = data.len() as u32;
    let mut out = vec![0x30, n as u8, (n >> 8) as u8, (n >> 16) as u8];
    let mut lit: Vec<u8> = Vec::new();
    let flush = |out: &mut Vec<u8>, lit: &mut Vec<u8>| {
        for chunk in lit.chunks(128) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        lit.clear();
    };
    let mut i = 0;
    while i < data.len() {
        let run = data[i..].iter().take(130).take_while(|&&b| b == data[i]).count();
        if run >= 3 {
            flush(&mut out, &mut lit);
            out.push(0x80 | (run - 3) as u8);
            out.push(data[i]);
            i += run;
        } else {
            lit.push(data[i]);
            i += 1;
        }
    }
    flush(&mut out, &mut lit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example from Apple's Technical Note TN1023.
    #[test]
    fn packbits_decodes_apples_published_example() {
        let packed = [0xfe, 0xaa, 0x02, 0x80, 0x00, 0x2a, 0xfd, 0xaa, 0x03, 0x80, 0x00, 0x2a, 0x22, 0xf7, 0xaa];
        let want = [
            0xaa, 0xaa, 0xaa, 0x80, 0x00, 0x2a, 0xaa, 0xaa, 0xaa, 0xaa, 0x80, 0x00, 0x2a, 0x22, 0xaa, 0xaa, 0xaa, 0xaa,
            0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
        ];
        assert_eq!(packbits_decode(&packed).unwrap(), want);
        assert_eq!(packbits_decode(&[0x80, 0x00, 0x41]).unwrap(), b"A", "128 is skipped");
    }

    /// Worked from GBATEK: 5 'a's are one repeat (flag 0x82); "xy" one
    /// literal run (flag 0x01).
    #[test]
    fn rl_decodes_repeats_and_literals() {
        let data = [0x30, 0x07, 0x00, 0x00, 0x82, b'a', 0x01, b'x', b'y'];
        assert_eq!(rl_decode(&data).unwrap(), b"aaaaaxy");
        assert_eq!(rl_encode(b"aaaaaxy"), data);
    }

    #[test]
    fn corrupt_runs_are_errors() {
        assert!(packbits_decode(&[0x05, 1, 2]).is_err());
        assert!(packbits_decode(&[0xff]).is_err());
        assert!(rl_decode(&[0x30, 0x09, 0, 0, 0x82, b'a']).is_err());
        assert!(rl_decode(&[0x31, 0, 0, 0]).is_err());
    }

    #[test]
    fn both_encoders_round_trip() {
        let cases: [&[u8]; 6] = [b"", b"a", b"aab", &[0u8; 1000], &[1, 2, 3, 4, 5, 6, 7, 8, 9], &[9u8; 131]];
        for data in cases {
            assert_eq!(packbits_decode(&packbits_encode(data)).unwrap(), data);
            assert_eq!(rl_decode(&rl_encode(data)).unwrap(), data);
        }
        let mixed: Vec<u8> = (0..2000).map(|i| if i % 50 < 30 { 0 } else { (i % 7) as u8 }).collect();
        assert_eq!(packbits_decode(&packbits_encode(&mixed)).unwrap(), mixed);
        assert_eq!(rl_decode(&rl_encode(&mixed)).unwrap(), mixed);
        assert!(packbits_encode(&[0u8; 1000]).len() < 20);
    }
}
