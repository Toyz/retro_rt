//! A minimal PNG writer: 8-bit RGBA, one IDAT, filter 0 on every row.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

fn crc32(chunks: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for data in chunks {
        for &b in *data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// `rgba` as a PNG file: `width * height * 4` bytes, top row first.
///
/// # Panics
///
/// When `rgba` is shorter than `width * height * 4`.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = width as usize * 4;
    assert!(rgba.len() >= stride * height as usize, "{} bytes for a {width}x{height} picture", rgba.len());
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgba.chunks_exact(stride.max(1)).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&raw).expect("zlib into memory");
    let idat = z.finish().expect("zlib into memory");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_iend_is_the_fixed_value() {
        // Every PNG ends with this chunk; its CRC never changes.
        assert_eq!(crc32(&[b"IEND"]), 0xae42_6082);
    }

    #[test]
    fn a_one_pixel_png_has_signature_header_and_end() {
        let png = encode(1, 1, &[1, 2, 3, 4]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }
}
