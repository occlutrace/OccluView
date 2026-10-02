use super::error::HpsError;

/// Decode standard base64 used inside HPS XML binary blocks.
pub(super) fn decode(encoded: &str) -> Result<Vec<u8>, HpsError> {
    let mut out = Vec::with_capacity((encoded.len() * 3) / 4);
    let mut value = 0_u32;
    let mut bits = -8_i32;
    let mut seen_padding = false;

    for ch in encoded.bytes() {
        if ch.is_ascii_whitespace() {
            continue;
        }
        if ch == b'=' {
            seen_padding = true;
            continue;
        }
        if seen_padding {
            return Err(super::malformed("base64 data appears after padding"));
        }
        let Some(decoded) = decode_char(ch) else {
            return Err(super::malformed("base64 contains an invalid character"));
        };
        value = (value << 6) | u32::from(decoded);
        bits += 6;
        if bits >= 0 {
            out.push(((value >> bits) & 0xff) as u8);
            bits -= 8;
        }
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
