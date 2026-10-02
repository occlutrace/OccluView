use super::{
    ensure_format_can_represent, write_mesh_unchecked, MeshWriteFormat, MeshWriteOptions,
    MeshWriteReport,
};
use crate::error::FormatError;
use occluview_core::Mesh;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Write a mesh to a newly-created file.
///
/// Uses `create_new` semantics so an existing path is treated as an error.
///
/// # Errors
///
/// Returns a [`FormatError`] if the file cannot be opened or the write fails.
pub fn write_mesh_to_new_file(
    path: &Path,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
) -> Result<MeshWriteReport, FormatError> {
    write_mesh_file(path, mesh, format, options, true)
}

/// Write a mesh to a file, truncating any existing content.
///
/// A `path` that is a symbolic link is followed: the file it points at receives
/// the new mesh and the link itself survives.
///
/// # Errors
///
/// Returns a [`FormatError`] if the file cannot be opened or the write fails.
pub fn write_mesh_overwrite(
    path: &Path,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
) -> Result<MeshWriteReport, FormatError> {
    write_mesh_file(path, mesh, format, options, false)
}

fn write_mesh_file(
    path: &Path,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
    create_new: bool,
) -> Result<MeshWriteReport, FormatError> {
    ensure_format_can_represent(mesh, format, &options)?;
    if create_new {
        // Write beside the destination and publish with a no-replace hard
        // link. Opening the destination with `create_new` first would expose
        // a partially written file to Explorer and crash recovery; a hard
        // link makes the completed inode visible in one operation while
        // retaining create-new collision semantics.
        let (temporary, file) = create_export_temp(path)?;
        let result = write_mesh_to_file(file, mesh, format, options);
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        };
        if let Err(error) = publish_new_export_file(&temporary, path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error.into());
        }
        return Ok(report);
    }

    // An overwrite is a transaction: the existing destination remains readable
    // until the complete new mesh has been flushed and the same-directory
    // rename commits it. Writing the target directly would turn a disk-full
    // or interrupted export into an empty/partial scan.
    //
    // Resolve a symlink destination first. `rename` replaces the link itself
    // rather than the file it points at, so publishing straight onto the
    // operator's `CASE/upper.ply` shortcut would leave the archive copy
    // untouched while the app reports a successful export. Resolving also
    // keeps the temporary beside the file the rename lands on.
    let destination = resolve_overwrite_destination(path)?;
    let (temporary, file) = create_export_temp(&destination)?;
    let result = write_mesh_to_file(file, mesh, format, options);
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = replace_export_file(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(report)
}

/// Follow a destination symlink chain to the file an overwrite targets.
///
/// A regular file costs one `symlink_metadata` probe and is returned
/// unchanged.
///
/// A chain that is still pointing at a link when the bound runs out - a loop,
/// or deeper than any case-folder shortcut should be - is an error. The publish
/// step is a rename, which would replace the link inode and leave the file it
/// pointed at with the previous geometry while reporting success.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidInput`] when the chain cannot be
/// resolved to a regular path.
pub fn resolve_overwrite_destination(path: &Path) -> std::io::Result<PathBuf> {
    /// Enough for the "case folder is a link into the archive" layouts this
    /// exists for, without letting a long chain walk somewhere unexpected.
    const MAX_DESTINATION_LINKS: usize = 8;
    let mut current = path.to_path_buf();
    for depth in 0..=MAX_DESTINATION_LINKS {
        // A missing path is not an error: `create_export_temp` and the rename
        // create it, and that is what the caller wants for a new file.
        let Ok(metadata) = std::fs::symlink_metadata(&current) else {
            return Ok(current);
        };
        if !metadata.file_type().is_symlink() {
            return Ok(current);
        }
        if depth == MAX_DESTINATION_LINKS {
            break;
        }
        let target = std::fs::read_link(&current)?;
        current = if target.is_absolute() {
            target
        } else {
            match current.parent() {
                Some(parent) => parent.join(target),
                None => return Ok(current),
            }
        };
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "destination {} is a symbolic-link chain that does not resolve to a file after {MAX_DESTINATION_LINKS} steps",
            path.display()
        ),
    ))
}

fn write_mesh_to_file(
    file: File,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
) -> Result<MeshWriteReport, FormatError> {
    let mut writer = BufWriter::new(file);
    let report = write_mesh_unchecked(&mut writer, mesh, format, options)?;
    writer.flush()?;
    writer
        .into_inner()
        .map_err(std::io::IntoInnerError::into_error)?
        .sync_all()?;
    Ok(report)
}

static NEXT_EXPORT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn create_export_temp(path: &Path) -> Result<(PathBuf, File), FormatError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("mesh"));
    for _ in 0..16 {
        let id = NEXT_EXPORT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".occluview-{id}.tmp"));
        let temporary = parent.join(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve a temporary export path",
    )
    .into())
}

#[cfg(not(windows))]
fn publish_new_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    // `hard_link` fails with AlreadyExists instead of replacing a file, which
    // is the filesystem-level equivalent of the public create-new contract.
    match std::fs::hard_link(temporary, destination) {
        Ok(()) => {
            // The destination now owns the complete inode. A cleanup failure
            // must not turn a successfully published export into a false
            // failure.
            let _ = std::fs::remove_file(temporary);
            Ok(())
        }
        Err(error) if !linkless_publish_required(&error) => Err(error),
        // exFAT, vfat, and link-disabled network mounts have no `link(2)` at
        // all, so the link-based publish fails for a reason that has nothing to
        // do with the destination name. The create-new contract is about the
        // name, not about how it is claimed: fall back to reserving the
        // destination exclusively and copying the finished bytes in.
        Err(_) => publish_by_exclusive_copy(temporary, destination),
    }
}

/// Whether a failed link-based publish has to take the link-free path.
///
/// A collision is the create-new contract working: it must stay classified so
/// the batch exporter can advance to the next numbered name. Every other
/// failure means the filesystem could not link at all.
#[cfg(not(windows))]
fn linkless_publish_required(error: &std::io::Error) -> bool {
    error.kind() != std::io::ErrorKind::AlreadyExists
}

/// Publish a finished temporary into a reserved destination name, without
/// links.
///
/// This is the fallback for filesystems that cannot hard-link. It keeps the
/// contract that matters — an existing destination is never replaced, and a
/// collision still reports [`std::io::ErrorKind::AlreadyExists`] — at the cost
/// of atomicity: the destination becomes visible while it is being filled, so
/// a crash mid-copy can leave a partial export where the link path would have
/// left none. Refusing to export into a writable folder the operator picked is
/// the worse failure.
#[cfg(not(windows))]
fn publish_by_exclusive_copy(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    let mut source = File::open(temporary)?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let copied = std::io::copy(&mut source, &mut target).and_then(|_| target.sync_all());
    drop(target);
    match copied {
        Ok(()) => {
            let _ = std::fs::remove_file(temporary);
            Ok(())
        }
        Err(error) => {
            // A half-written file must not be mistaken for the export.
            let _ = std::fs::remove_file(destination);
            Err(error)
        }
    }
}

#[cfg(windows)]
fn publish_new_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    // MoveFileEx without REPLACE_EXISTING preserves create-new semantics while
    // avoiding the hard-link requirement on FAT, network, and restricted
    // Windows volumes. The temporary file is already complete and is moved
    // within the destination directory.
    move_export_file(temporary, destination, false)
}

#[cfg(not(windows))]
fn replace_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    // Both paths are created in the destination directory, so rename is an
    // atomic replacement on the supported Unix filesystems.
    std::fs::rename(temporary, destination)
}

#[cfg(windows)]
fn replace_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    move_export_file(temporary, destination, true)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn move_export_file(
    temporary: &Path,
    destination: &Path,
    replace_existing: bool,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary: Vec<u16> = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut flags = MOVEFILE_WRITE_THROUGH;
    if replace_existing {
        flags |= MOVEFILE_REPLACE_EXISTING;
    }
    // SAFETY: both paths are NUL-terminated wide strings that outlive the call,
    // and `MoveFileExW` only reads them.
    unsafe {
        MoveFileExW(
            PCWSTR(temporary.as_ptr()),
            PCWSTR(destination.as_ptr()),
            flags,
        )
    }
    // Without `MOVEFILE_REPLACE_EXISTING`, an existing destination fails here
    // with ERROR_ALREADY_EXISTS (or ERROR_FILE_EXISTS). Callers detect a
    // create-new collision by `ErrorKind::AlreadyExists`, so a flat
    // `ErrorKind::Other` would make the batch retry treat every collision as a
    // hard failure on Windows. Keep the Win32 text for the operator either way.
    .map_err(|error| {
        let code = error.code();
        if code == ERROR_ALREADY_EXISTS.to_hresult() || code == ERROR_FILE_EXISTS.to_hresult() {
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, error.to_string())
        } else {
            std::io::Error::other(error.to_string())
        }
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
