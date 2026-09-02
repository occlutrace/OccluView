use std::time::Instant;

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
    /// The caller's absolute request deadline elapsed before another bounded
    /// shell-stream read could begin.
    TimedOut,
}

/// Size policy and allocation limits for one shell stream copy.
#[derive(Clone, Copy, Debug)]
pub struct StreamReadBounds {
    /// Length reported by the shell, if it supplied a usable value.
    pub declared_len: Option<u64>,
    /// Largest accepted payload, in bytes.
    pub max_bytes: usize,
    /// Smallest initial allocation for a size-silent stream.
    pub min_buffer_bytes: usize,
    /// Largest synchronous read issued to the shell stream.
    pub chunk_bytes: usize,
}

/// Read a stream in bounded chunks without exceeding `max_bytes`.
///
/// `declared_len` is a *hint*, not a gate: `Some(len)` beyond the cap rejects
/// up front (no wasted copy), while `None` means the source did not report a
/// size — cloud placeholders and pipe-like shell streams legitimately `Stat`
/// as zero — and the only honest answer is to read until EOF or the cap.
/// Treating "unknown" as "infinite" here once turned every size-silent stream
/// into a permanent oversize placeholder.
pub fn read_capped_stream(
    bounds: StreamReadBounds,
    read_chunk: impl FnMut(&mut [u8]) -> Result<usize, ()>,
) -> StreamRead {
    read_capped_stream_inner(bounds, None, read_chunk)
}

/// Read a stream in bounded chunks until `deadline` expires.
///
/// The caller owns the absolute budget. A synchronous COM `Read` that is
/// already in progress cannot be safely cancelled, but this helper prevents a
/// slow or cloud-backed stream from beginning another chunk after its request
/// has expired.
pub fn read_capped_stream_until(
    bounds: StreamReadBounds,
    deadline: Instant,
    read_chunk: impl FnMut(&mut [u8]) -> Result<usize, ()>,
) -> StreamRead {
    read_capped_stream_inner(bounds, Some(deadline), read_chunk)
}

fn read_capped_stream_inner(
    bounds: StreamReadBounds,
    deadline: Option<Instant>,
    mut read_chunk: impl FnMut(&mut [u8]) -> Result<usize, ()>,
) -> StreamRead {
    let declared_cap = match bounds.declared_len {
        Some(len) => {
            let len = usize::try_from(len).unwrap_or(usize::MAX);
            if len > bounds.max_bytes {
                return StreamRead::OverCap { byte_len: len };
            }
            len
        }
        None => 0,
    };

    let initial_capacity = if declared_cap == 0 {
        bounds.min_buffer_bytes
    } else {
        declared_cap.clamp(bounds.min_buffer_bytes, bounds.max_bytes)
    };
    let mut buf = Vec::with_capacity(initial_capacity);
    while buf.len() <= bounds.max_bytes {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return StreamRead::TimedOut;
        }
        let want = (bounds.max_bytes + 1 - buf.len()).min(bounds.chunk_bytes);
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
        if buf.len() > bounds.max_bytes {
            return StreamRead::OverCap {
                byte_len: buf.len(),
            };
        }
    }
    StreamRead::Complete(buf)
}

#[cfg(test)]
mod tests {
    use super::{read_capped_stream, read_capped_stream_until, StreamRead, StreamReadBounds};
    use std::time::Instant;

    const BOUNDS: StreamReadBounds = StreamReadBounds {
        declared_len: Some(8),
        max_bytes: 1024,
        min_buffer_bytes: 4,
        chunk_bytes: 8,
    };

    #[test]
    fn expired_deadline_prevents_the_next_shell_stream_read() {
        let mut read_started = false;
        let result = read_capped_stream_until(BOUNDS, Instant::now(), |_buf| {
            read_started = true;
            Ok(0)
        });

        assert_eq!(result, StreamRead::TimedOut);
        assert!(
            !read_started,
            "an expired request must not issue a shell read"
        );
    }

    #[test]
    fn unknown_length_stream_reads_to_completion_instead_of_overcap() {
        // Some shell streams report zero bytes; read them until EOF or the cap.
        let data = *b"stream without a declared size";
        let mut cursor = 0usize;
        let result = read_capped_stream(
            StreamReadBounds {
                declared_len: None,
                ..BOUNDS
            },
            |buf| {
                if cursor >= data.len() {
                    return Ok(0);
                }
                let take = (data.len() - cursor).min(buf.len());
                buf[..take].copy_from_slice(&data[cursor..cursor + take]);
                cursor += take;
                Ok(take)
            },
        );

        assert_eq!(result, StreamRead::Complete(data.to_vec()));
    }

    #[test]
    fn unknown_length_stream_still_detects_overcap_while_reading() {
        let result = read_capped_stream(
            StreamReadBounds {
                declared_len: None,
                max_bytes: 32,
                min_buffer_bytes: 8,
                chunk_bytes: 16,
            },
            |buf| {
                for byte in buf.iter_mut() {
                    *byte = 9;
                }
                Ok(buf.len())
            },
        );

        assert_eq!(result, StreamRead::OverCap { byte_len: 33 });
    }

    #[test]
    fn declared_oversize_stream_returns_overcap_without_reading() {
        let mut called = false;
        let result = read_capped_stream(
            StreamReadBounds {
                declared_len: Some(1025),
                max_bytes: 1024,
                min_buffer_bytes: 16,
                chunk_bytes: 64,
            },
            |_buf| {
                called = true;
                Ok(0)
            },
        );

        assert_eq!(result, StreamRead::OverCap { byte_len: 1025 });
        assert!(!called, "oversize declaration should fail before any read");
    }

    #[test]
    fn mid_stream_read_error_does_not_become_truncated_success() {
        let mut reads = 0;
        let result = read_capped_stream(
            StreamReadBounds {
                declared_len: Some(32),
                max_bytes: 1024,
                min_buffer_bytes: 16,
                chunk_bytes: 16,
            },
            |buf| {
                reads += 1;
                match reads {
                    1 => {
                        buf[..4].copy_from_slice(&[1, 2, 3, 4]);
                        Ok(4)
                    }
                    _ => Err(()),
                }
            },
        );

        assert_eq!(result, StreamRead::ReadFailed);
    }

    #[test]
    fn chunked_stream_that_crosses_limit_returns_overcap() {
        let mut remaining = 33usize;
        let result = read_capped_stream(
            StreamReadBounds {
                declared_len: Some(0),
                max_bytes: 32,
                min_buffer_bytes: 8,
                chunk_bytes: 16,
            },
            |buf| {
                if remaining == 0 {
                    return Ok(0);
                }
                let take = remaining.min(buf.len());
                for byte in &mut buf[..take] {
                    *byte = 7;
                }
                remaining -= take;
                Ok(take)
            },
        );

        assert_eq!(result, StreamRead::OverCap { byte_len: 33 });
    }

    #[test]
    fn successful_stream_returns_complete_bytes() {
        let data = *b"hello mesh";
        let mut cursor = 0usize;
        let result = read_capped_stream(
            StreamReadBounds {
                declared_len: Some(data.len() as u64),
                max_bytes: 1024,
                min_buffer_bytes: 4,
                chunk_bytes: 5,
            },
            |buf| {
                if cursor >= data.len() {
                    return Ok(0);
                }
                let take = (data.len() - cursor).min(buf.len());
                buf[..take].copy_from_slice(&data[cursor..cursor + take]);
                cursor += take;
                Ok(take)
            },
        );

        assert_eq!(result, StreamRead::Complete(data.to_vec()));
    }
}
