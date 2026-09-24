//! Base64 for the texture a PLY carries inside itself.
//!
//! PLY has no texture element, and the de-facto convention — a
//! `comment TextureFile <name>` line plus an image beside the file — leaves the
//! pair splittable: move the `.ply` alone and the texture is gone. An export
//! therefore carries the encoded PNG in its own header comments, which every
//! reader that does not know the key skips as text, and this reader turns back
//! into a texture.
//!
//! Standard alphabet with padding (RFC 4648), encoded here rather than pulled
//! from a dependency: the encoder and decoder must agree on exactly this
//! dialect, and the whole of it is thirty lines.
//!
//! Decoding shifts a four-sextet group down to its three bytes, so each `as u8`
//! below keeps only the low eight bits of a value already masked by the shift.
#![allow(clippy::cast_possible_truncation)]

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode `bytes` as standard base64 with padding.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let triple = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let index = |shift: u32| ALPHABET[((triple >> shift) & 0x3f) as usize] as char;
        out.push(index(18));
        out.push(index(12));
        out.push(if chunk.len() > 1 { index(6) } else { '=' });
        out.push(if chunk.len() > 2 { index(0) } else { '=' });
    }
    out
}

/// Decode standard base64 with padding. Whitespace is ignored; anything else
/// outside the alphabet — including a wrong padding — yields `None`.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut sextets = 0u32;
    let mut padding = 0u32;
    for byte in text.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            padding += 1;
            continue;
        }
        // Data after the first padding character is malformed, not merely
        // ignored: accepting it would let two different strings decode to the
        // same bytes.
        if padding > 0 {
            return None;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        sextets += 1;
        if sextets == 4 {
            out.push((accumulator >> 16) as u8);
            out.push((accumulator >> 8) as u8);
            out.push(accumulator as u8);
            accumulator = 0;
            sextets = 0;
        }
    }
    match (sextets, padding) {
        // A whole number of four-character groups.
        (0, 0) => Some(out),
        // Two sextets and two padding characters carry one byte, and the four
        // bits the byte does not use must be zero: without that, two different
        // strings decode to the same bytes.
        (2, 2) if accumulator.trailing_zeros() >= 4 => {
            out.push((accumulator >> 4) as u8);
            Some(out)
        }
        // Three sextets and one padding character carry two bytes, with the two
        // unused bits zero for the same reason.
        (3, 1) if accumulator.trailing_zeros() >= 2 => {
            out.push((accumulator >> 10) as u8);
            out.push((accumulator >> 2) as u8);
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn every_remainder_round_trips() {
        for length in 0..64usize {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 7 + 3) as u8).collect();
            let text = encode(&bytes);
            assert_eq!(
                decode(&text).as_deref(),
                Some(bytes.as_slice()),
                "a {length}-byte payload must survive its own encoding"
            );
        }
    }

    #[test]
    fn encoding_matches_the_standard_alphabet() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        // Bytes above 0x7f are data, not text: the encoder must not treat them
        // as UTF-8.
        assert_eq!(encode(&[0xff, 0x00, 0x80]), "/wCA");
    }

    #[test]
    fn malformed_input_is_refused_rather_than_guessed() {
        assert_eq!(decode("Zm9vYmFy!"), None, "an outside-alphabet byte");
        assert_eq!(
            decode("Zh=="),
            None,
            "a non-canonical tail: the bits the byte does not use must be zero"
        );
        assert_eq!(decode("Zm9vYmF"), None, "an incomplete group");
        assert_eq!(decode("Zg==Zg=="), None, "data after padding");
        assert_eq!(decode("Zg="), None, "too little padding");
        assert_eq!(
            decode("Zm9v\nYmFy"),
            Some(b"foobar".to_vec()),
            "line breaks inside the payload are how the comment wraps it"
        );
    }
}
