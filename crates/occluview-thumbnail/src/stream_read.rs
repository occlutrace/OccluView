#[derive(Debug, PartialEq, Eq)]
/// Result of a bounded stream copy.
pub enum StreamRead {
    /// The stream ended and returned all bytes successfully.
    Complete(Vec<u8>),
    /// The declared or observed stream length exceeded the configured cap.
    OverCap {
        /// The observed or declared byte length that exceeded the cap.
        byte_len: usize,
    },
    /// A read operation failed before the stream completed.
    ReadFailed,
}

/// Read a stream in bounded chunks without exceeding `max_bytes`.
pub fn read_capped_stream(
    declared_len: Option<u64>,
    max_bytes: usize,
    min_buffer_bytes: usize,
    chunk_bytes: usize,
    mut read_chunk: impl FnMut(&mut [u8]) -> Result<usize, ()>,
) -> StreamRead {
    let declared_cap = declared_len
        .and_then(|len| usize::try_from(len).ok())
        .unwrap_or(usize::MAX);
    if declared_cap > max_bytes {
        return StreamRead::OverCap {
            byte_len: declared_cap,
        };
    }

    let initial_capacity = if declared_cap == 0 {
        min_buffer_bytes
    } else {
        declared_cap.clamp(min_buffer_bytes, max_bytes)
    };
    let mut buf = Vec::with_capacity(initial_capacity);
    while buf.len() <= max_bytes {
        let want = (max_bytes + 1 - buf.len()).min(chunk_bytes);
        let write_offset = buf.len();
        buf.resize(write_offset + want, 0);
        let Ok(read) = read_chunk(&mut buf[write_offset..write_offset + want]) else {
            return StreamRead::ReadFailed;
        };
        if read == 0 {
            buf.truncate(write_offset);
            break;
        }
        buf.truncate(write_offset + read);
        if buf.len() > max_bytes {
            return StreamRead::OverCap {
                byte_len: buf.len(),
            };
        }
    }
    StreamRead::Complete(buf)
}

#[cfg(test)]
mod tests {
    use super::{read_capped_stream, StreamRead};

    #[test]
    fn declared_oversize_stream_returns_overcap_without_reading() {
        let mut called = false;
        let result = read_capped_stream(Some(1025), 1024, 16, 64, |_buf| {
            called = true;
            Ok(0)
        });

        assert_eq!(result, StreamRead::OverCap { byte_len: 1025 });
        assert!(!called, "oversize declaration should fail before any read");
    }

    #[test]
    fn mid_stream_read_error_does_not_become_truncated_success() {
        let mut reads = 0;
        let result = read_capped_stream(Some(32), 1024, 16, 16, |buf| {
            reads += 1;
            match reads {
                1 => {
                    buf[..4].copy_from_slice(&[1, 2, 3, 4]);
                    Ok(4)
                }
                _ => Err(()),
            }
        });

        assert_eq!(result, StreamRead::ReadFailed);
    }

    #[test]
    fn chunked_stream_that_crosses_limit_returns_overcap() {
        let mut remaining = 33usize;
        let result = read_capped_stream(Some(0), 32, 8, 16, |buf| {
            if remaining == 0 {
                return Ok(0);
            }
            let take = remaining.min(buf.len());
            for byte in &mut buf[..take] {
                *byte = 7;
            }
            remaining -= take;
            Ok(take)
        });

        assert_eq!(result, StreamRead::OverCap { byte_len: 33 });
    }

    #[test]
    fn successful_stream_returns_complete_bytes() {
        let data = *b"hello mesh";
        let mut cursor = 0usize;
        let result = read_capped_stream(Some(data.len() as u64), 1024, 4, 5, |buf| {
            if cursor >= data.len() {
                return Ok(0);
            }
            let take = (data.len() - cursor).min(buf.len());
            buf[..take].copy_from_slice(&data[cursor..cursor + take]);
            cursor += take;
            Ok(take)
        });

        assert_eq!(result, StreamRead::Complete(data.to_vec()));
    }
}
