use super::error::HpsError;

/// Decode standard base64 used inside HPS XML binary blocks.
pub(super) fn decode(encoded: &str) -> Result<Vec<u8>, HpsError> {
    let mut out = Vec::with_capacity(encoded.len() / 4 * 3);
    let mut value = 0_u32;
    let mut bits = -8_i32;
    let mut symbols = 0_usize;
    let mut padding = 0_usize;

    for ch in encoded.bytes() {
        if ch.is_ascii_whitespace() {
            continue;
        }
        if ch == b'=' {
            padding += 1;
            if padding > 2 {
                return Err(super::malformed("base64 has surplus padding"));
            }
            continue;
        }
        if padding != 0 {
            return Err(super::malformed("base64 data appears after padding"));
        }
        let Some(decoded) = decode_char(ch) else {
            return Err(super::malformed("base64 contains an invalid character"));
        };
        value = (value << 6) | u32::from(decoded);
        symbols += 1;
        bits += 6;
        if bits >= 0 {
            out.push(((value >> bits) & 0xff) as u8);
            bits -= 8;
        }
    }

    let remainder = symbols % 4;
    if remainder == 1 || (padding != 0 && padding != (4 - remainder) % 4) {
        return Err(super::malformed(
            "base64 has an incomplete quantum or invalid padding",
        ));
    }
    let unused_bits = match remainder {
        2 => 4,
        3 => 2,
        _ => 0,
    };
    if value & ((1 << unused_bits) - 1) != 0 {
        return Err(super::malformed("base64 has nonzero trailing pad bits"));
    }
    Ok(out)
}

fn decode_char(ch: u8) -> Option<u8> {
    match ch {
        b'A'..=b'Z' => Some(ch - b'A'),
        b'a'..=b'z' => Some(ch - b'a' + 26),
        b'0'..=b'9' => Some(ch - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rejects_incomplete_quanta_and_invalid_padding() {
        for text in [
            "A", "AAAAA", "=", "AAAA=", "AA=", "AA===", "AB==", "AAB=", "AB", "AAB",
        ] {
            assert!(decode(text).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn base64_keeps_whitespace_and_complete_unpadded_data() {
        for text in ["YQ==", "YQ", " Y Q = = \r\n"] {
            assert_eq!(decode(text).ok(), Some(b"a".to_vec()));
        }
        for text in ["YWI=", "YWI"] {
            assert_eq!(decode(text).ok(), Some(b"ab".to_vec()));
        }
        assert_eq!(decode("YWJj").ok(), Some(b"abc".to_vec()));
        assert_eq!(decode("").ok(), Some(Vec::new()));
    }
}
