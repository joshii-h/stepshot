//! Minimal, dependency-free base64 (standard alphabet).
//!
//! The encoder inlines every screenshot into the self-contained HTML report and
//! the editor page; the decoder reads the `data:` URIs the editor sends back
//! for manual-step images. Hand-rolled like the JSON/TOML/ZIP code — no dep.

/// Minimal base64 encoder (standard alphabet, dependency-free).
pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Minimal base64 decoder (standard alphabet, `=`/whitespace ignored). Returns
/// `None` on an invalid character. Counterpart to [`base64`], used to decode the
/// data-URI images the editor sends for manual steps.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lut = [255u8; 256];
    for (i, &c) in A.iter().enumerate() {
        lut[c as usize] = i as u8;
    }
    let (mut buf, mut bits) = (0u32, 0u32);
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for &b in s.as_bytes() {
        if matches!(b, b'=' | b'\n' | b'\r' | b' ' | b'\t') {
            continue;
        }
        let v = lut[b as usize];
        if v == 255 {
            return None;
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 test vectors — the hand-rolled encoder feeds every embedded
    /// image in the final report, so it gets the canonical vectors.
    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_binary_roundtrip_length() {
        let data: Vec<u8> = (0..=255).collect();
        let enc = base64(&data);
        assert_eq!(enc.len(), data.len().div_ceil(3) * 4);
        assert_eq!(&enc[..8], "AAECAwQF");
    }

    #[test]
    fn base64_decode_roundtrips_and_rejects_junk() {
        for v in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foobar",
            &(0u8..=255).collect::<Vec<u8>>()[..],
        ] {
            assert_eq!(base64_decode(&base64(v)).unwrap(), v);
        }
        assert_eq!(base64_decode("Zm9v\nYmFy").unwrap(), b"foobar"); // whitespace ok
        assert!(base64_decode("not base64!!").is_none());
    }
}
