//! Base64 for a texture embedded in a PLY header.
//!
//! PLY has no texture element. An earlier OccluView release wrote the encoded
//! image into `OccluViewTexture*` header comments; this decoder still reads that
//! form so those files keep opening in colour. Current exports bake the colour
//! into the vertices and write no image.
//!
//! Standard alphabet with padding (RFC 4648).
//!
//! Decoding shifts a four-sextet group down to its three bytes, so each `as u8`
//! below keeps only the low eight bits of a value already masked by the shift.
#![allow(clippy::cast_possible_truncation)]

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
    use super::decode;

    /// The payloads an older release wrote must still decode, because a reader
    /// has to keep opening the files that release produced.
    #[test]
    fn the_standard_alphabet_decodes() {
        assert_eq!(decode("").as_deref(), Some([].as_slice()));
        assert_eq!(decode("Zg==").as_deref(), Some(b"f".as_slice()));
        assert_eq!(decode("Zm8=").as_deref(), Some(b"fo".as_slice()));
        assert_eq!(decode("Zm9v").as_deref(), Some(b"foo".as_slice()));
        assert_eq!(decode("Zm9vYg==").as_deref(), Some(b"foob".as_slice()));
        assert_eq!(decode("Zm9vYmE=").as_deref(), Some(b"fooba".as_slice()));
        assert_eq!(decode("Zm9vYmFy").as_deref(), Some(b"foobar".as_slice()));
        // Bytes above 0x7f are data, not text.
        assert_eq!(
            decode("/wCA").as_deref(),
            Some([0xff, 0x00, 0x80].as_slice())
        );
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
