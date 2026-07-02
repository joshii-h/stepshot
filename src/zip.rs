//! Minimal **store-only** ZIP writer (no compression) — enough to build ODT
//! containers. Dependency-free, matching the hand-rolled base64/json/toml code.
//!
//! ODT allows stored (uncompressed) entries, and requires the `mimetype` entry
//! to be stored and first — so a store-only writer is exactly sufficient.

/// A ZIP archive being assembled in memory.
pub struct Zip {
    data: Vec<u8>,
    entries: Vec<Entry>,
}

struct Entry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

impl Zip {
    pub fn new() -> Self {
        Zip {
            data: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Append one stored file. Order is preserved (call `mimetype` first for ODT).
    pub fn add(&mut self, name: &str, content: &[u8]) {
        let crc = crc32(content);
        let offset = self.data.len() as u32;
        let size = content.len() as u32;

        // Local file header.
        self.data.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.data.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.data.extend_from_slice(&0u16.to_le_bytes()); // flags
        self.data.extend_from_slice(&0u16.to_le_bytes()); // method 0 = store
        self.data.extend_from_slice(&0u16.to_le_bytes()); // mod time
        self.data.extend_from_slice(&0x0021u16.to_le_bytes()); // mod date (1980-01-01)
        self.data.extend_from_slice(&crc.to_le_bytes());
        self.data.extend_from_slice(&size.to_le_bytes()); // compressed size
        self.data.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        self.data
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes()); // extra len
        self.data.extend_from_slice(name.as_bytes());
        self.data.extend_from_slice(content);

        self.entries.push(Entry {
            name: name.to_string(),
            crc,
            size,
            offset,
        });
    }

    /// Finish the archive: write the central directory + EOCD and return bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.data.len() as u32;
        for e in &self.entries {
            self.data.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            self.data.extend_from_slice(&20u16.to_le_bytes()); // version made by
            self.data.extend_from_slice(&20u16.to_le_bytes()); // version needed
            self.data.extend_from_slice(&0u16.to_le_bytes()); // flags
            self.data.extend_from_slice(&0u16.to_le_bytes()); // method store
            self.data.extend_from_slice(&0u16.to_le_bytes()); // time
            self.data.extend_from_slice(&0x0021u16.to_le_bytes()); // date
            self.data.extend_from_slice(&e.crc.to_le_bytes());
            self.data.extend_from_slice(&e.size.to_le_bytes());
            self.data.extend_from_slice(&e.size.to_le_bytes());
            self.data
                .extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            self.data.extend_from_slice(&0u16.to_le_bytes()); // extra
            self.data.extend_from_slice(&0u16.to_le_bytes()); // comment
            self.data.extend_from_slice(&0u16.to_le_bytes()); // disk number
            self.data.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            self.data.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            self.data.extend_from_slice(&e.offset.to_le_bytes());
            self.data.extend_from_slice(e.name.as_bytes());
        }
        let cd_size = self.data.len() as u32 - cd_offset;
        let count = self.entries.len() as u16;

        // End of central directory.
        self.data.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes()); // disk
        self.data.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        self.data.extend_from_slice(&count.to_le_bytes());
        self.data.extend_from_slice(&count.to_le_bytes());
        self.data.extend_from_slice(&cd_size.to_le_bytes());
        self.data.extend_from_slice(&cd_offset.to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes()); // comment len
        self.data
    }
}

/// Standard CRC-32 (IEEE), computed without a lookup table.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vector() {
        // CRC-32 of "123456789" is 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn produces_a_valid_zip_shell() {
        let mut z = Zip::new();
        z.add("mimetype", b"application/x");
        z.add("content.xml", b"<x/>");
        let bytes = z.finish();
        // Starts with a local file header, ends with the EOCD signature.
        assert_eq!(&bytes[0..4], &0x0403_4b50u32.to_le_bytes());
        let eocd = &bytes[bytes.len() - 22..bytes.len() - 18];
        assert_eq!(eocd, &0x0605_4b50u32.to_le_bytes());
        // First stored name is mimetype, right after the 30-byte header.
        assert_eq!(&bytes[30..38], b"mimetype");
    }
}
