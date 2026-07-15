use super::OpenRequest;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

const REQUEST_MAGIC: &[u8; 4] = b"OCVQ";
/// Envelope carrying only file paths (the historical layout).
const REQUEST_VERSION_PATHS_ONLY: u16 = 1;
/// Envelope that appends a forwarded window-activation token after the paths.
/// The token carries the launching process's user-interaction provenance so
/// the running instance can raise itself past WM focus-stealing prevention
/// (see `activation.rs`).
const REQUEST_VERSION_WITH_TOKEN: u16 = 2;
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

    let mut payload = Vec::new();
    for path in &request.paths {
        let text = path.display().to_string();
        validate_request_text(&text, "path")?;
        let bytes = text.as_bytes();
        let len = u32::try_from(bytes.len()).context("request path length exceeds u32")?;
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(bytes);
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
            payload.extend_from_slice(&len.to_le_bytes());
            payload.extend_from_slice(bytes);
            REQUEST_VERSION_WITH_TOKEN
        }
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

pub(super) fn parse_request(bytes: &[u8]) -> Result<OpenRequest> {
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
    if version != REQUEST_VERSION_PATHS_ONLY && version != REQUEST_VERSION_WITH_TOKEN {
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

    let mut cursor = REQUEST_HEADER_BYTES;
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
        let text = std::str::from_utf8(&bytes[cursor..path_end])
            .context("single-instance request path is not valid utf-8")?;
        validate_request_text(text, "path")?;
        paths.push(PathBuf::from(text));
        cursor = path_end;
    }

    let activation_token = if version == REQUEST_VERSION_WITH_TOKEN {
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
    let paths = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            validate_request_text(line, "path")?;
            Ok(PathBuf::from(line))
        })
        .collect::<Result<Vec<_>>>()?;
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

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    fn request(paths: &[&str], token: Option<&str>) -> OpenRequest {
        OpenRequest {
            paths: paths.iter().map(PathBuf::from).collect(),
            activation_token: token.map(str::to_string),
        }
    }

    #[test]
    fn request_envelope_round_trips_multiple_paths() {
        let original = request(&["/tmp/a.stl", "/tmp/b/scan.ply"], None);

        let payload = serialize_request(&original);
        assert!(payload.is_ok(), "serialize request failed: {payload:?}");
        let Ok(payload) = payload else {
            return;
        };
        // No token: the envelope must stay on the v1 layout for back-compat.
        assert_eq!(&payload[4..6], &REQUEST_VERSION_PATHS_ONLY.to_le_bytes());
        let parsed = parse_request(&payload);
        assert!(parsed.is_ok(), "parse request failed: {parsed:?}");
        let Ok(parsed) = parsed else {
            return;
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
            return;
        };
        // A token bumps the envelope version so the reader knows to parse it.
        assert_eq!(&payload[4..6], &REQUEST_VERSION_WITH_TOKEN.to_le_bytes());
        let parsed = parse_request(&payload);
        assert!(parsed.is_ok(), "parse request failed: {parsed:?}");
        let Ok(parsed) = parsed else {
            return;
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
            return;
        };
        assert_eq!(
            parsed.paths,
            vec![PathBuf::from("/tmp/a.stl"), PathBuf::from("/tmp/b.obj")]
        );
        assert_eq!(parsed.activation_token, None);
    }

    #[test]
    fn request_parser_rejects_control_characters() {
        let payload = serialize_request(&request(&["/tmp/a.stl"], None));
        assert!(payload.is_ok(), "serialize failed: {payload:?}");
        let Ok(mut payload) = payload else {
            return;
        };
        assert!(!payload.is_empty(), "payload should not be empty");
        let Some(last) = payload.last_mut() else {
            return;
        };
        *last = b'\n';

        let parsed = parse_request(&payload);
        assert!(
            parsed.is_err(),
            "control characters must be rejected: {parsed:?}"
        );
        let Err(error) = parsed else {
            return;
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
            return;
        };
        // Corrupt the final token byte into a control character.
        let Some(last) = payload.last_mut() else {
            return;
        };
        *last = 0x07;

        let parsed = parse_request(&payload);
        assert!(parsed.is_err(), "token control chars must fail: {parsed:?}");
        let Err(error) = parsed else {
            return;
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
            return;
        };
        let truncated = &payload[..payload.len() - 1];

        let parsed = parse_request(truncated);
        assert!(parsed.is_err(), "truncated envelope must fail: {parsed:?}");
        let Err(error) = parsed else {
            return;
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
            return;
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
            return;
        };
        assert!(
            error.to_string().contains("max is"),
            "unexpected error: {error:?}"
        );
    }
}
