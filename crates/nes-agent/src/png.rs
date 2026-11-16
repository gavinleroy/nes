pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (!(crc & 1)).wrapping_add(1));
        }
    }
    !crc
}

pub fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(65535).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none() as u8;
        let len = chunk.len() as u16;
        out.push(last);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

pub fn encode(width: u32, height: u32, pixels: &[u32]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        for x in 0..width as usize {
            let px = pixels[y * width as usize + x];
            raw.push((px & 0xFF) as u8);
            raw.push((px >> 8 & 0xFF) as u8);
            raw.push((px >> 16 & 0xFF) as u8);
        }
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32_reference(data: &[u8]) -> u32 {
        let mut reg = 0xFFFF_FFFFu32;
        for &byte in data {
            for bit in 0..8 {
                let incoming = (byte >> bit) & 1;
                let feedback = (reg & 1) as u8 ^ incoming;
                reg >>= 1;
                if feedback == 1 {
                    reg ^= 0xEDB8_8320;
                }
            }
        }
        !reg
    }

    fn test_inputs() -> Vec<Vec<u8>> {
        let mut inputs: Vec<Vec<u8>> = vec![vec![]];
        for b in 0..=255u8 {
            inputs.push(vec![b]);
        }
        inputs.push((0..=255).collect());
        inputs.push((0..1000).map(|i| (i * 31 % 251) as u8).collect());
        inputs.push(vec![0xFF; 70000]);
        inputs
    }

    #[test]
    fn crc32_matches_bitwise_reference() {
        for input in test_inputs() {
            assert_eq!(crc32(&input), crc32_reference(&input));
        }
    }

    #[test]
    fn adler32_matches_definition() {
        for input in test_inputs() {
            let mut a: u64 = 1;
            let mut b: u64 = 0;
            for &byte in &input {
                a = (a + byte as u64) % 65521;
                b = (b + a) % 65521;
            }
            assert_eq!(adler32(&input) as u64, b << 16 | a);
        }
    }

    fn inflate_stored(z: &[u8]) -> Vec<u8> {
        assert_eq!(z[0] & 0x0F, 8);
        assert_eq!((((z[0] as u16) << 8) | z[1] as u16) % 31, 0);
        let mut out = Vec::new();
        let mut pos = 2;
        loop {
            let last = z[pos];
            assert!(last <= 1);
            let len = u16::from_le_bytes([z[pos + 1], z[pos + 2]]) as usize;
            let nlen = u16::from_le_bytes([z[pos + 3], z[pos + 4]]);
            assert_eq!(nlen, !(len as u16));
            pos += 5;
            out.extend_from_slice(&z[pos..pos + len]);
            pos += len;
            if last == 1 {
                break;
            }
        }
        assert_eq!(adler32(&out).to_be_bytes(), z[pos..pos + 4]);
        assert_eq!(pos + 4, z.len());
        out
    }

    #[test]
    fn zlib_stored_roundtrips_all_block_boundaries() {
        for len in [0usize, 1, 65534, 65535, 65536, 131070, 131071, 200000] {
            let data: Vec<u8> = (0..len).map(|i| (i % 253) as u8).collect();
            assert_eq!(inflate_stored(&zlib_stored(&data)), data);
        }
    }

    #[test]
    fn png_structure_and_pixels_roundtrip() {
        let width = 7u32;
        let height = 5u32;
        let pixels: Vec<u32> = (0..35)
            .map(|i| 0xFF00_0000 | ((i * 3) << 16) | ((i * 5) << 8) | (i * 7))
            .collect();
        let png = encode(width, height, &pixels);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let mut pos = 8;
        let mut idat = Vec::new();
        let mut kinds = Vec::new();
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &png[pos + 4..pos + 8];
            let payload = &png[pos + 8..pos + 8 + len];
            let crc = u32::from_be_bytes(png[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(&png[pos + 4..pos + 8 + len]));
            kinds.push(kind.to_vec());
            if kind == b"IDAT" {
                idat.extend_from_slice(payload);
            }
            if kind == b"IHDR" {
                assert_eq!(u32::from_be_bytes(payload[0..4].try_into().unwrap()), width);
                assert_eq!(
                    u32::from_be_bytes(payload[4..8].try_into().unwrap()),
                    height
                );
                assert_eq!(&payload[8..13], &[8, 2, 0, 0, 0]);
            }
            pos += 12 + len;
        }
        assert_eq!(kinds[0], b"IHDR");
        assert_eq!(kinds.last().unwrap(), b"IEND");
        let raw = inflate_stored(&idat);
        assert_eq!(raw.len(), (width as usize * 3 + 1) * height as usize);
        for y in 0..height as usize {
            let row = &raw[y * (width as usize * 3 + 1)..];
            assert_eq!(row[0], 0);
            for x in 0..width as usize {
                let px = pixels[y * width as usize + x];
                assert_eq!(row[1 + x * 3], (px & 0xFF) as u8);
                assert_eq!(row[2 + x * 3], (px >> 8 & 0xFF) as u8);
                assert_eq!(row[3 + x * 3], (px >> 16 & 0xFF) as u8);
            }
        }
    }
}
