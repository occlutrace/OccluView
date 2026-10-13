//! Apartment-local adapter from COM streams to the bounded byte reader.
//!
//! Preview and thumbnail handlers share this adapter without sharing their
//! activation state. All COM calls stay on the calling apartment.

use crate::stream_read::{
    read_capped_stream, read_capped_stream_until, StreamRead, StreamReadBounds,
};
use occluview_thumbnail::render_thumb::MAX_THUMBNAIL_INPUT_BYTES;
use std::time::Instant;
use windows::Win32::System::Com::{IStream, STATFLAG, STATSTG, STREAM_SEEK_SET};

const MIN_STREAM_BUFFER_BYTES: usize = 16 * 1024;
const STREAM_READ_CHUNK_BYTES: usize = 1024 * 1024;

pub(super) fn rewind(stream: &IStream) -> windows::core::Result<()> {
    // SAFETY: the caller owns a valid COM IStream reference for the
    // duration of this synchronous helper.
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None)? };
    Ok(())
}

/// Reads a stream synchronously, with the normal shell size cap but no
/// elapsed-time budget. Preview Pane uses this during `DoPreview`; `Stat`
/// and `Read` can block without a deadline. The caller's preceding `Seek`
/// is also synchronous and unbounded.
pub(super) fn read(stream: &IStream) -> windows::core::Result<StreamRead> {
    read_inner(stream, None)
}

/// Reads the stream into a byte buffer under the caller's absolute budget.
///
/// The budget is checked before `Stat` and between read chunks; it cannot
/// interrupt a synchronous `Stat` or `Read` already in progress. Use
/// [`rewind_and_read_until`] when the caller also needs a deadline check
/// before the preceding `Seek`.
pub(super) fn read_until(stream: &IStream, deadline: Instant) -> windows::core::Result<StreamRead> {
    if Instant::now() >= deadline {
        return Ok(StreamRead::TimedOut);
    }
    read_inner(stream, Some(deadline))
}

/// Rewinds and reads a stream under the caller's absolute request budget.
///
/// The budget is checked before `Seek` here, again before `Stat` through
/// [`read_until`], and between read chunks. A synchronous `Seek`, `Stat`, or
/// `Read` already in progress is not cancelled: an arbitrary `IStream` can
/// still block this apartment inside any one of those COM calls.
pub(super) fn rewind_and_read_until(
    stream: &IStream,
    deadline: Instant,
) -> windows::core::Result<StreamRead> {
    if Instant::now() >= deadline {
        return Ok(StreamRead::TimedOut);
    }
    rewind(stream)?;
    read_until(stream, deadline)
}

fn read_inner(stream: &IStream, deadline: Option<Instant>) -> windows::core::Result<StreamRead> {
    // SAFETY: `stat` is a stack-local zeroed STATSTG owned by us; the
    // STATFLAG_NONAME flag avoids an internal allocation we'd have to free.
    let mut stat: STATSTG = unsafe { std::mem::zeroed() };
    // SAFETY: passing a valid pointer to our zeroed STATSTG.
    unsafe { stream.Stat(&mut stat, STATFLAG(1))? };
    let declared = if stat.cbSize > 0 {
        // STATSTG.cbSize is a u64 union member (_ULARGE_INTEGER QuadPart).
        #[allow(clippy::cast_possible_truncation)]
        let n = stat.cbSize as u64;
        n
    } else {
        0
    };
    let bounds = StreamReadBounds {
        declared_len: (declared != 0).then_some(declared),
        max_bytes: MAX_THUMBNAIL_INPUT_BYTES,
        min_buffer_bytes: MIN_STREAM_BUFFER_BYTES,
        chunk_bytes: STREAM_READ_CHUNK_BYTES,
    };
    let mut read_chunk = |buf: &mut [u8]| {
        let mut read = 0u32;
        // SAFETY: `buf` is a valid write region; `read` is a stack out-param.
        let result = unsafe {
            stream.Read(
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                buf.len() as u32,
                Some(std::ptr::from_mut(&mut read)),
            )
        };
        if result.is_err() {
            tracing::warn!(hresult = ?result, "shell stream read failed");
            return Err(());
        }
        Ok(read as usize)
    };
    Ok(match deadline {
        Some(deadline) => read_capped_stream_until(bounds, deadline, &mut read_chunk),
        None => read_capped_stream(bounds, &mut read_chunk),
    })
}
