use super::OpenRequest;
use anyhow::{bail, Context, Result};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
const REQUEST_MAGIC: &[u8; 4] = b"OCVQ";
/// Envelope carrying only file paths (the v1 layout).
const REQUEST_VERSION_PATHS_ONLY: u16 = 1;
/// Envelope that appends a forwarded window-activation token after the paths.
/// The token carries the launching process's user-interaction provenance so
/// the running instance can raise itself past WM focus-stealing prevention
/// (see `activation.rs`).
const REQUEST_VERSION_WITH_TOKEN: u16 = 2;
/// Envelope carrying native path units, with the codec tagged in the payload.
const REQUEST_VERSION_NATIVE_PATHS_ONLY: u16 = 3;
/// Native path units plus a forwarded window-activation token.
const REQUEST_VERSION_NATIVE_PATHS_WITH_TOKEN: u16 = 4;
#[cfg(unix)]
const NATIVE_PATH_CODEC_UNIX_BYTES: u8 = 1;
#[cfg(windows)]
const NATIVE_PATH_CODEC_WINDOWS_UTF16LE: u8 = 2;
/// Reserved, path-free capability probe. Older readers reject version zero.
pub(super) const CAPABILITY_PROBE: [u8; REQUEST_HEADER_BYTES] = [
    b'O', b'C', b'V', b'Q', 0, 0, 0, 0, 0, 0, 0, 0,
];
/// Fixed acknowledgements are exchanged only after the request has been
/// recognized (and, for an open request, queued by the listener).
pub(super) const CAPABILITY_ACK: &[u8; 8] = b"OCVA\x01\x00\x01\x00";
pub(super) const OPEN_REQUEST_ACK: &[u8; 8] = b"OCVA\x01\x00\x02\x00";
const FALLBACK_ACK_MAGIC: &[u8; 4] = b"OCFA";
const FALLBACK_ACK_HEADER_BYTES: usize = 8;
pub(super) const MAX_REQUEST_PATHS: usize = 128;
pub(super) const MAX_REQUEST_BYTES: usize = 256 * 1024;
const REQUEST_HEADER_BYTES: usize = 12;
const MAX_TOKEN_BYTES: usize = 4096;

pub(super) fn serialize_request(request: &OpenRequest) -> Result<Vec<u8>> {
    if request.paths.len() > MAX_REQUEST_PATHS {
        bail!(
            "single-instance request has {} paths; max is {}",
            request.paths.len(),
            MAX_REQUEST_PATHS
        );
    }

    let native_paths = request_needs_native_encoding(request);
    let mut payload = Vec::new();
    if native_paths {
        payload.push(native_path_codec_tag()?);
    }
    for path in &request.paths {
        let text_path = if native_paths {
            None
        } else {
            let text = path
                .to_str()
                .context("single-instance path is not valid Unicode")?;
            validate_request_text(text, "path")?;
            Some(text)
        };
        let encoded_path_len = match text_path {
            Some(text) => text.len(),
            None => native_path_encoded_len(path)?,
        };
        let len = u32::try_from(encoded_path_len).context("request path length exceeds u32")?;
        let path_payload_len = 4usize
            .checked_add(encoded_path_len)
            .context("single-instance request path payload length overflow")?;
        let next_payload_len = admit_request_payload_growth(payload.len(), path_payload_len)?;
        payload.extend_from_slice(&len.to_le_bytes());
        if let Some(text) = text_path {
            payload.extend_from_slice(text.as_bytes());
        } else {
            // Check the complete envelope budget before producing either a
            // lossy validation string or a native-unit encoding buffer.
            validate_native_path(path)?;
            let native_bytes = encode_native_path(path)?;
            if native_bytes.len() != encoded_path_len {
                bail!("native single-instance path length changed while encoding");
            }
            payload.extend_from_slice(&native_bytes);
        }
        debug_assert_eq!(payload.len(), next_payload_len);
    }

    // Only bump to the token-carrying version when a token is actually present,
    // so the common no-token request stays byte-identical to the v1 layout an
    // older running instance would understand.
    let version = match request.activation_token.as_deref() {
        Some(token) => {
            validate_request_text(token, "activation token")?;
            let bytes = token.as_bytes();
            if bytes.len() > MAX_TOKEN_BYTES {
                bail!(
                    "single-instance activation token is {} bytes; max is {MAX_TOKEN_BYTES}",
                    bytes.len()
                );
            }
            let len = u32::try_from(bytes.len()).context("activation token length exceeds u32")?;
            let token_payload_len = 4usize
                .checked_add(bytes.len())
                .context("single-instance activation token length overflow")?;
            let next_payload_len = admit_request_payload_growth(payload.len(), token_payload_len)?;
            payload.extend_from_slice(&len.to_le_bytes());
            payload.extend_from_slice(bytes);
            debug_assert_eq!(payload.len(), next_payload_len);
            if native_paths {
                REQUEST_VERSION_NATIVE_PATHS_WITH_TOKEN
            } else {
                REQUEST_VERSION_WITH_TOKEN
            }
        }
        None if native_paths => REQUEST_VERSION_NATIVE_PATHS_ONLY,
        None => REQUEST_VERSION_PATHS_ONLY,
    };

    let total_len = REQUEST_HEADER_BYTES
        .checked_add(payload.len())
        .context("single-instance request length overflow")?;
    if total_len > MAX_REQUEST_BYTES {
        bail!("single-instance request is {total_len} bytes; max is {MAX_REQUEST_BYTES}");
    }

    let mut request_bytes = Vec::with_capacity(total_len);
    request_bytes.extend_from_slice(REQUEST_MAGIC);
    request_bytes.extend_from_slice(&version.to_le_bytes());
    request_bytes.extend_from_slice(
        &u16::try_from(request.paths.len())
            .context("single-instance request path count exceeds u16")?
            .to_le_bytes(),
    );
    request_bytes.extend_from_slice(
        &u32::try_from(payload.len())
            .context("single-instance request payload exceeds u32")?
            .to_le_bytes(),
    );
    request_bytes.extend_from_slice(&payload);
    Ok(request_bytes)
}

fn admit_request_payload_growth(payload_len: usize, addition: usize) -> Result<usize> {
    let next_payload_len = payload_len
        .checked_add(addition)
        .context("single-instance request payload length overflow")?;
    let total_len = REQUEST_HEADER_BYTES
        .checked_add(next_payload_len)
        .context("single-instance request length overflow")?;
    if total_len > MAX_REQUEST_BYTES {
        bail!("single-instance request is {total_len} bytes; max is {MAX_REQUEST_BYTES}");
    }
    Ok(next_payload_len)
}

pub(super) fn request_needs_native_encoding(request: &OpenRequest) -> bool {
    request.paths.iter().any(|path| path.to_str().is_none())
}

pub(super) fn is_capability_probe(bytes: &[u8]) -> bool {
    bytes == CAPABILITY_PROBE.as_slice()
}

pub(super) fn is_native_request(bytes: &[u8]) -> bool {
    bytes.starts_with(REQUEST_MAGIC)
        && bytes.len() >= 6
        && matches!(u16::from_le_bytes([bytes[4], bytes[5]]), 3 | 4)
}

pub(super) fn serialize_fallback_ack(kind: &[u8; 8], nonce: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FALLBACK_ACK_HEADER_BYTES + nonce.len());
    bytes.extend_from_slice(FALLBACK_ACK_MAGIC);
    bytes.extend_from_slice(&kind[4..]);
    bytes.extend_from_slice(nonce.as_bytes());
    bytes
}

pub(super) fn is_fallback_ack(bytes: &[u8], kind: &[u8; 8], nonce: &str) -> bool {
    bytes.len() == FALLBACK_ACK_HEADER_BYTES + nonce.len()
        && bytes.starts_with(FALLBACK_ACK_MAGIC)
        && &bytes[4..FALLBACK_ACK_HEADER_BYTES] == &kind[4..]
        && &bytes[FALLBACK_ACK_HEADER_BYTES..] == nonce.as_bytes()
}

pub(super) fn parse_request(bytes: &[u8]) -> Result<OpenRequest> {
    if bytes.len() > MAX_REQUEST_BYTES {
        bail!("single-instance request exceeds the byte limit");
    }
    if bytes.starts_with(REQUEST_MAGIC) {
        parse_enveloped_request(bytes)
    } else {
        parse_legacy_request(bytes)
    }
}

fn parse_enveloped_request(bytes: &[u8]) -> Result<OpenRequest> {
    if bytes.len() < REQUEST_HEADER_BYTES {
        bail!(
            "single-instance request header truncated: {} < {}",
            bytes.len(),
            REQUEST_HEADER_BYTES
        );
    }
    if &bytes[..REQUEST_MAGIC.len()] != REQUEST_MAGIC {
        bail!("single-instance request magic mismatch");
    }

    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if !matches!(
        version,
        REQUEST_VERSION_PATHS_ONLY
            | REQUEST_VERSION_WITH_TOKEN
            | REQUEST_VERSION_NATIVE_PATHS_ONLY
            | REQUEST_VERSION_NATIVE_PATHS_WITH_TOKEN
    ) {
        bail!("single-instance request version {version} is unsupported");
    }

    let path_count = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    if path_count > MAX_REQUEST_PATHS {
        bail!("single-instance request has {path_count} paths; max is {MAX_REQUEST_PATHS}");
    }

    let payload_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let total_len = REQUEST_HEADER_BYTES
        .checked_add(payload_len)
        .context("single-instance request payload length overflow")?;
    if total_len != bytes.len() {
        bail!(
            "single-instance request length mismatch: header says {total_len}, actual {}",
            bytes.len()
        );
    }
    if total_len > MAX_REQUEST_BYTES {
        bail!("single-instance request is {total_len} bytes; max is {MAX_REQUEST_BYTES}");
    }

    let native_paths = matches!(
        version,
        REQUEST_VERSION_NATIVE_PATHS_ONLY | REQUEST_VERSION_NATIVE_PATHS_WITH_TOKEN
    );
    let mut cursor = REQUEST_HEADER_BYTES;
    if native_paths {
        let codec = *bytes
            .get(cursor)
            .context("single-instance request is missing its native path codec")?;
        cursor += 1;
        let expected_codec = native_path_codec_tag()?;
        if codec != expected_codec {
            bail!("single-instance native path codec does not match this operating system");
        }
    }
    let mut paths = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        let len_end = cursor
            .checked_add(4)
            .context("single-instance request path length overflow")?;
        if len_end > bytes.len() {
            bail!("single-instance request truncated before path length");
        }
        let path_len = u32::from_le_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        cursor = len_end;
        let path_end = cursor
            .checked_add(path_len)
            .context("single-instance request path bytes overflow")?;
        if path_end > bytes.len() {
            bail!("single-instance request truncated inside path payload");
        }
        if native_paths {
            let path = decode_native_path(&bytes[cursor..path_end])?;
            validate_native_path(&path)?;
            paths.push(path);
        } else {
            let text = std::str::from_utf8(&bytes[cursor..path_end])
                .context("single-instance request path is not valid utf-8")?;
            validate_request_text(text, "path")?;
            paths.push(PathBuf::from(text));
        }
        cursor = path_end;
    }

    let activation_token = if matches!(
        version,
        REQUEST_VERSION_WITH_TOKEN | REQUEST_VERSION_NATIVE_PATHS_WITH_TOKEN
    ) {
        Some(parse_token(bytes, &mut cursor)?)
    } else {
        None
    };

    if cursor != bytes.len() {
        bail!("single-instance request has trailing bytes");
    }

    Ok(OpenRequest {
        paths,
        activation_token,
    })
}

fn parse_token(bytes: &[u8], cursor: &mut usize) -> Result<String> {
    let len_end = cursor
        .checked_add(4)
        .context("single-instance request token length overflow")?;
    if len_end > bytes.len() {
        bail!("single-instance request truncated before token length");
    }
    let token_len = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]) as usize;
    *cursor = len_end;
    if token_len > MAX_TOKEN_BYTES {
        bail!("single-instance activation token is {token_len} bytes; max is {MAX_TOKEN_BYTES}");
    }
    let token_end = cursor
        .checked_add(token_len)
        .context("single-instance request token bytes overflow")?;
    if token_end > bytes.len() {
        bail!("single-instance request truncated inside token payload");
    }
    let text = std::str::from_utf8(&bytes[*cursor..token_end])
        .context("single-instance request token is not valid utf-8")?;
    validate_request_text(text, "activation token")?;
    *cursor = token_end;
    Ok(text.to_string())
}

fn parse_legacy_request(bytes: &[u8]) -> Result<OpenRequest> {
    let text = std::str::from_utf8(bytes).context("legacy single-instance request is not utf-8")?;
    let mut paths = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if paths.len() >= MAX_REQUEST_PATHS {
            bail!("legacy single-instance request exceeds the path limit");
        }
        validate_request_text(line, "path")?;
        paths.push(PathBuf::from(line));
    }
    Ok(OpenRequest {
        paths,
        activation_token: None,
    })
}

fn validate_request_text(text: &str, kind: &str) -> Result<()> {
    if text.is_empty() {
        bail!("single-instance request {kind} is empty");
    }
    if text.chars().any(char::is_control) {
        bail!("single-instance request {kind} contains control characters");
    }
    Ok(())
}

fn validate_native_path(path: &std::path::Path) -> Result<()> {
    let text = path.to_string_lossy();
    validate_request_text(&text, "path")
}

fn native_path_codec_tag() -> Result<u8> {
    #[cfg(unix)]
    {
        Ok(NATIVE_PATH_CODEC_UNIX_BYTES)
    }
    #[cfg(windows)]
    {
        Ok(NATIVE_PATH_CODEC_WINDOWS_UTF16LE)
    }
    #[cfg(not(any(unix, windows)))]
    {
        bail!("native single-instance paths are unsupported on this operating system")
    }
}

fn encode_native_path(path: &std::path::Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        Ok(path.as_os_str().as_bytes().to_vec())
    }
    #[cfg(windows)]
    {
        let units = path.as_os_str().encode_wide();
        let mut bytes = Vec::new();
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        Ok(bytes)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        bail!("native single-instance paths are unsupported on this operating system")
    }
}

fn native_path_encoded_len(path: &std::path::Path) -> Result<usize> {
    #[cfg(unix)]
    {
        Ok(path.as_os_str().as_bytes().len())
    }
    #[cfg(windows)]
    {
        path.as_os_str()
            .encode_wide()
            .count()
            .checked_mul(2)
            .context("native single-instance path length overflow")
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        bail!("native single-instance paths are unsupported on this operating system")
    }
}

fn decode_native_path(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(windows)]
    {
        if bytes.len() % 2 != 0 {
            bail!("single-instance UTF-16 path has an odd byte length");
        }
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&units)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = bytes;
        bail!("native single-instance paths are unsupported on this operating system")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(paths: &[&str], token: Option<&str>) -> OpenRequest {
        OpenRequest {
            paths: paths.iter().map(PathBuf::from).collect(),
            activation_token: token.map(str::to_string),
        }
    }

    /// A secondary launch forwards open requests only: the handoff carries
    /// paths and an activation token, never a language, so it cannot change
    /// the primary instance's UI language.
    #[test]
    fn request_envelope_round_trips_multiple_paths() {
        let original = request(&["/tmp/a.stl", "/tmp/b/scan.ply"], None);

        let payload = serialize_request(&original);
        assert!(payload.is_ok(), "serialize request failed: {payload:?}");
        let Ok(payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        // No token: the envelope must stay on the v1 layout for back-compat.
        assert_eq!(&payload[4..6], &REQUEST_VERSION_PATHS_ONLY.to_le_bytes());
        let parsed = parse_request(&payload);
        assert!(parsed.is_ok(), "parse request failed: {parsed:?}");
        let Ok(parsed) = parsed else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(parsed.paths, original.paths);
        assert_eq!(parsed.activation_token, None);
    }

    #[test]
    fn request_envelope_round_trips_activation_token() {
        let original = request(&["/tmp/a.stl"], Some("occluview/host/12-3_TIME9876543"));

        let payload = serialize_request(&original);
        assert!(payload.is_ok(), "serialize request failed: {payload:?}");
        let Ok(payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        // A token bumps the envelope version so the reader knows to parse it.
        assert_eq!(&payload[4..6], &REQUEST_VERSION_WITH_TOKEN.to_le_bytes());
        let parsed = parse_request(&payload);
        assert!(parsed.is_ok(), "parse request failed: {parsed:?}");
        let Ok(parsed) = parsed else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(parsed.paths, original.paths);
        assert_eq!(
            parsed.activation_token.as_deref(),
            Some("occluview/host/12-3_TIME9876543")
        );
    }

    #[test]
    fn legacy_text_request_is_still_accepted() {
        let parsed = parse_request(b"/tmp/a.stl\n/tmp/b.obj\n");
        assert!(parsed.is_ok(), "parse legacy request failed: {parsed:?}");
        let Ok(parsed) = parsed else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(
            parsed.paths,
            vec![PathBuf::from("/tmp/a.stl"), PathBuf::from("/tmp/b.obj")]
        );
        assert_eq!(parsed.activation_token, None);
    }

    #[test]
    fn legacy_requests_obey_the_same_path_and_byte_limits() {
        let at_limit = "scan.stl\n".repeat(MAX_REQUEST_PATHS);
        let parsed = parse_request(at_limit.as_bytes());
        assert!(parsed.is_ok());
        let Ok(parsed) = parsed else {
            panic!("a legacy request at the path limit must be accepted");
        };
        assert_eq!(parsed.paths.len(), MAX_REQUEST_PATHS);

        let too_many = "scan.stl\n".repeat(MAX_REQUEST_PATHS + 1);
        assert!(parse_request(too_many.as_bytes()).is_err());
        assert!(parse_request(&vec![b'x'; MAX_REQUEST_BYTES + 1]).is_err());
    }

    #[test]
    fn request_parser_rejects_control_characters() {
        let payload = serialize_request(&request(&["/tmp/a.stl"], None));
        assert!(payload.is_ok(), "serialize failed: {payload:?}");
        let Ok(mut payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        assert!(!payload.is_empty(), "payload should not be empty");
        let Some(last) = payload.last_mut() else {
            panic!("required test setup or expected result was missing");
        };
        *last = b'\n';

        let parsed = parse_request(&payload);
        assert!(
            parsed.is_err(),
            "control characters must be rejected: {parsed:?}"
        );
        let Err(error) = parsed else {
            panic!("required test setup or expected result was missing");
        };
        assert!(
            error.to_string().contains("control characters"),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn request_parser_rejects_token_with_control_characters() {
        let payload = serialize_request(&request(&["/tmp/a.stl"], Some("token")));
        assert!(payload.is_ok(), "serialize failed: {payload:?}");
        let Ok(mut payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        // Corrupt the final token byte into a control character.
        let Some(last) = payload.last_mut() else {
            panic!("required test setup or expected result was missing");
        };
        *last = 0x07;

        let parsed = parse_request(&payload);
        assert!(parsed.is_err(), "token control chars must fail: {parsed:?}");
        let Err(error) = parsed else {
            panic!("required test setup or expected result was missing");
        };
        assert!(
            error.to_string().contains("control characters"),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn request_parser_rejects_truncated_envelope() {
        let payload = serialize_request(&request(&["/tmp/a.stl"], None));
        assert!(payload.is_ok(), "serialize failed: {payload:?}");
        let Ok(payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        let truncated = &payload[..payload.len() - 1];

        let parsed = parse_request(truncated);
        assert!(parsed.is_err(), "truncated envelope must fail: {parsed:?}");
        let Err(error) = parsed else {
            panic!("required test setup or expected result was missing");
        };
        assert!(
            error.to_string().contains("truncated") || error.to_string().contains("mismatch"),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn request_parser_rejects_truncated_token() {
        let payload = serialize_request(&request(&["/tmp/a.stl"], Some("token")));
        assert!(payload.is_ok(), "serialize failed: {payload:?}");
        let Ok(payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        // Drop the last token byte but keep the header's declared length,
        // producing a length mismatch the parser must reject.
        let truncated = &payload[..payload.len() - 1];

        let parsed = parse_request(truncated);
        assert!(parsed.is_err(), "truncated token must fail: {parsed:?}");
    }

    #[test]
    fn request_serializer_rejects_path_count_over_limit() {
        let paths = (0..=MAX_REQUEST_PATHS)
            .map(|index| format!("/tmp/{index}.stl"))
            .collect::<Vec<_>>();
        let request = OpenRequest {
            paths: paths.iter().map(PathBuf::from).collect(),
            activation_token: None,
        };

        let result = serialize_request(&request);
        assert!(result.is_err(), "too many paths must fail: {result:?}");
        let Err(error) = result else {
            panic!("required test setup or expected result was missing");
        };
        assert!(
            error.to_string().contains("max is"),
            "unexpected error: {error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_request_serializer_admits_only_envelopes_within_the_byte_limit() {
        use std::os::unix::ffi::OsStringExt;

        let max_path_bytes = MAX_REQUEST_BYTES - REQUEST_HEADER_BYTES - 1 - 4;
        let mut native_bytes = vec![b'x'; max_path_bytes];
        native_bytes[0] = 0xff;
        let request = OpenRequest {
            paths: vec![PathBuf::from(std::ffi::OsString::from_vec(native_bytes.clone()))],
            activation_token: None,
        };

        let encoded = serialize_request(&request);
        assert!(encoded.is_ok(), "an envelope at the limit must fit: {encoded:?}");
        let Ok(encoded) = encoded else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(encoded.len(), MAX_REQUEST_BYTES);

        native_bytes.push(b'x');
        let oversized = serialize_request(&OpenRequest {
            paths: vec![PathBuf::from(std::ffi::OsString::from_vec(native_bytes))],
            activation_token: None,
        });
        assert!(oversized.is_err(), "a path over the envelope budget must fail");
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_request_serializer_admits_only_bounded_utf16_paths() {
        use std::os::windows::ffi::OsStringExt;

        let max_path_bytes = MAX_REQUEST_BYTES - REQUEST_HEADER_BYTES - 1 - 4;
        let mut units = vec![u16::from(b'x'); max_path_bytes / 2];
        units[0] = 0xD800;
        let request = OpenRequest {
            paths: vec![PathBuf::from(std::ffi::OsString::from_wide(&units))],
            activation_token: None,
        };

        let encoded = serialize_request(&request);
        assert!(encoded.is_ok(), "a bounded UTF-16 envelope must fit: {encoded:?}");
        let Ok(encoded) = encoded else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(encoded.len(), MAX_REQUEST_BYTES - 1);

        let mut oversized_units = units;
        oversized_units.push(u16::from(b'x'));
        let oversized = serialize_request(&OpenRequest {
            paths: vec![PathBuf::from(std::ffi::OsString::from_wide(&oversized_units))],
            activation_token: None,
        });
        assert!(oversized.is_err(), "a path over the envelope budget must fail");
    }

    #[test]
    fn unicode_requests_keep_the_exact_v1_and_v2_wire_bytes() {
        let no_token = serialize_request(&request(&["/tmp/π.stl"], None));
        assert!(no_token.is_ok());
        let Ok(no_token) = no_token else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(
            no_token.as_slice(),
            b"OCVQ\x01\x00\x01\x00\x0f\x00\x00\x00\x0b\x00\x00\x00/tmp/\xcf\x80.stl"
        );

        let with_token = serialize_request(&request(&["/tmp/π.stl"], Some("t")));
        assert!(with_token.is_ok());
        let Ok(with_token) = with_token else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(
            with_token.as_slice(),
            b"OCVQ\x02\x00\x01\x00\x14\x00\x00\x00\x0b\x00\x00\x00/tmp/\xcf\x80.stl\x01\x00\x00\x00t"
        );
    }

    #[test]
    fn capability_probe_is_path_free_and_not_an_open_request() {
        assert!(is_capability_probe(&CAPABILITY_PROBE));
        let parsed = parse_request(&CAPABILITY_PROBE);
        assert!(parsed.is_err(), "a probe must not be accepted as an open request");
        let Err(error) = parsed else {
            panic!("a capability probe unexpectedly parsed as a request");
        };
        assert!(error.to_string().contains("version 0"));
    }

    #[test]
    fn fallback_ack_requires_the_matching_nonce_and_kind() {
        let ack = serialize_fallback_ack(CAPABILITY_ACK, "123-45-6");
        assert!(is_fallback_ack(&ack, CAPABILITY_ACK, "123-45-6"));
        assert!(!is_fallback_ack(&ack, CAPABILITY_ACK, "123-45-7"));
        assert!(!is_fallback_ack(&ack, OPEN_REQUEST_ACK, "123-45-6"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_native_paths_round_trip_without_loss_and_reject_other_codec_tags() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let native_path = PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/scan-\xff.stl".to_vec(),
        ));
        let original = OpenRequest {
            paths: vec![PathBuf::from("/tmp/π.stl"), native_path.clone()],
            activation_token: Some("launch-token".to_owned()),
        };
        let encoded = serialize_request(&original);
        assert!(encoded.is_ok(), "serialize failed: {encoded:?}");
        let Ok(encoded) = encoded else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(&encoded[4..6], &REQUEST_VERSION_NATIVE_PATHS_WITH_TOKEN.to_le_bytes());
        assert_eq!(encoded[REQUEST_HEADER_BYTES], NATIVE_PATH_CODEC_UNIX_BYTES);
        let decoded = parse_request(&encoded);
        assert!(decoded.is_ok(), "parse failed: {decoded:?}");
        let Ok(decoded) = decoded else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(decoded.paths[0], original.paths[0]);
        assert_eq!(
            decoded.paths[1].as_os_str().as_bytes(),
            native_path.as_os_str().as_bytes()
        );
        assert_eq!(decoded.activation_token, original.activation_token);

        let mut mismatched = encoded;
        mismatched[REQUEST_HEADER_BYTES] = 2;
        assert!(parse_request(&mismatched).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unix_native_paths_still_reject_control_characters() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/a\n\xff".to_vec()));
        let request = OpenRequest {
            paths: vec![path],
            activation_token: None,
        };
        assert!(serialize_request(&request).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_paths_round_trip_unpaired_utf16_units() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let original_units = [u16::from(b'\\'), 0xD800, u16::from(b'.'), u16::from(b's')];
        let path = PathBuf::from(std::ffi::OsString::from_wide(&original_units));
        let encoded = serialize_request(&OpenRequest {
            paths: vec![path.clone()],
            activation_token: None,
        });
        assert!(encoded.is_ok());
        let Ok(encoded) = encoded else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(&encoded[4..6], &REQUEST_VERSION_NATIVE_PATHS_ONLY.to_le_bytes());
        assert_eq!(encoded[REQUEST_HEADER_BYTES], NATIVE_PATH_CODEC_WINDOWS_UTF16LE);
        let decoded = parse_request(&encoded);
        assert!(decoded.is_ok());
        let Ok(decoded) = decoded else {
            panic!("required test setup or expected result was missing");
        };
        assert_eq!(
            decoded.paths[0]
                .as_os_str()
                .encode_wide()
                .collect::<Vec<_>>(),
            original_units
        );
    }
}
