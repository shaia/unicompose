//! Minimal PNG and ICO writers: enough for icons, with no image crates.

/// A PNG of `width` × `height` RGBA pixels, uncompressed (zlib "stored" blocks).
pub fn png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(rgba.len(), (width * height * 4) as usize, "RGBA buffer size");
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA, deflate, no filter, no interlace
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// An ICO holding each image: 32-bit bitmaps up to 64 pixels, PNG above, as Windows
/// expects.
pub fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let encoded: Vec<Vec<u8>> = images
        .iter()
        .map(|(size, rgba)| if *size > 64 { png(rgba, *size, *size) } else { dib(rgba, *size) })
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type 1 = icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for ((size, _), data) in images.iter().zip(&encoded) {
        let dimension = if *size >= 256 { 0 } else { *size as u8 }; // 0 means 256
        out.extend_from_slice(&[dimension, dimension, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in encoded {
        out.extend_from_slice(&data);
    }
    out
}

/// An icon's bitmap: a BITMAPINFOHEADER with doubled height, BGRA rows bottom-up, then
/// an all-clear AND mask (the alpha channel does the masking).
fn dib(rgba: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for value in [40u32, size, size * 2] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]); // BI_RGB, size, resolution, palette
    for row in rgba.chunks_exact(size as usize * 4).rev() {
        for px in row.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    let mask_row = (size as usize).div_ceil(32) * 4;
    out.extend(std::iter::repeat_n(0u8, mask_row * size as usize));
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() { vec![&[]] } else { data.chunks(65_535).collect() };
    for (i, block) in blocks.iter().enumerate() {
        out.push(u8::from(i + 1 == blocks.len())); // BFINAL, BTYPE 00
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_has_its_signature_and_chunks() {
        let png = png(&[255; 2 * 2 * 4], 2, 2);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&png[12..16], b"IHDR");
        assert!(png.ends_with(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]), "IEND and its CRC");
    }

    #[test]
    fn ico_directory_points_at_each_image() {
        let ico = ico(&[(16, vec![0; 16 * 16 * 4]), (256, vec![0; 256 * 256 * 4])]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        let entry = |i: usize| &ico[6 + 16 * i..6 + 16 * (i + 1)];
        assert_eq!(entry(0)[0], 16);
        assert_eq!(entry(1)[0], 0, "256 is written as 0");
        let second = u32::from_le_bytes(entry(1)[12..16].try_into().unwrap()) as usize;
        assert!(ico[second..].starts_with(b"\x89PNG"));
        let first = u32::from_le_bytes(entry(0)[12..16].try_into().unwrap()) as usize;
        assert_eq!(&ico[first..first + 4], &40u32.to_le_bytes(), "a BITMAPINFOHEADER");
    }
}
