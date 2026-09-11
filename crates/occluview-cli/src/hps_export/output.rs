use super::error::CliError;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn write_artifacts(
    output_dir: &Path,
    geometry: &[u8],
    preview: Option<&[u8]>,
    manifest: &[u8],
) -> Result<(), CliError> {
    let parent = output_dir
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| CliError::OutputDirectoryFailed)?;
    if output_directory_exists(output_dir)? {
        return Err(CliError::OutputExists);
    }

    let staging = reserve_staging_directory(output_dir)?;
    let result = write_staged_artifacts(&staging, geometry, preview, manifest);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = fs::rename(&staging, output_dir) {
        let _ = fs::remove_dir_all(&staging);
        return if error.kind() == std::io::ErrorKind::AlreadyExists {
            Err(CliError::OutputExists)
        } else {
            Err(CliError::OutputWriteFailed)
        };
    }
    Ok(())
}

fn output_directory_exists(path: &Path) -> Result<bool, CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(CliError::OutputDirectoryFailed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(CliError::OutputDirectoryFailed),
    }
}

static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);

fn reserve_staging_directory(output_dir: &Path) -> Result<std::path::PathBuf, CliError> {
    let parent = output_dir
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output_name = output_dir
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("artifacts"));
    for _ in 0..16 {
        let id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(output_name);
        name.push(format!(".occluview-{id}.staging"));
        let staging = parent.join(name);
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(CliError::OutputDirectoryFailed),
        }
    }
    Err(CliError::OutputWriteFailed)
}

fn write_staged_artifacts(
    staging: &Path,
    geometry: &[u8],
    preview: Option<&[u8]>,
    manifest: &[u8],
) -> Result<(), CliError> {
    let geometry_path = staging.join("surface.ply");
    write_new(&geometry_path, geometry)?;
    let preview_path = staging.join("surface.glb");
    if let Some(preview) = preview {
        if let Err(error) = write_new(&preview_path, preview) {
            return Err(error);
        }
    }
    write_new(&staging.join("manifest.json"), manifest)?;
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                CliError::OutputExists
            } else {
                CliError::OutputWriteFailed
            }
        })?;
    if file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(CliError::OutputWriteFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{write_artifacts, CliError};

    #[test]
    fn artifact_group_is_published_as_one_directory() {
        let temp = tempfile::tempdir().expect("temp directory");
        let output = temp.path().join("artifacts");

        write_artifacts(&output, b"geometry", Some(b"preview"), b"manifest")
            .expect("publish artifact group");

        assert_eq!(
            std::fs::read(output.join("surface.ply")).unwrap(),
            b"geometry"
        );
        assert_eq!(
            std::fs::read(output.join("surface.glb")).unwrap(),
            b"preview"
        );
        assert_eq!(
            std::fs::read(output.join("manifest.json")).unwrap(),
            b"manifest"
        );
        assert!(std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".staging")));
    }

    #[test]
    fn an_existing_empty_directory_is_not_reused_for_a_partial_export() {
        let temp = tempfile::tempdir().expect("temp directory");
        let output = temp.path().join("artifacts");
        std::fs::create_dir(&output).expect("seed output directory");

        assert_eq!(
            write_artifacts(&output, b"geometry", None, b"manifest"),
            Err(CliError::OutputExists)
        );
        assert!(std::fs::read_dir(&output).unwrap().next().is_none());
    }
}
