//! macOS single-instance ownership.
//!
//! A kernel-managed advisory lock replaces Linux PID liveness checks. File-open
//! handoff itself uses the shared request protocol and state-directory queue.

use super::SingleInstance;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::path::PathBuf;

const LOCK_FILE_NAME: &str = "single-instance.lock";

pub(super) fn acquire() -> Result<SingleInstance> {
    let lock_path = lock_file_path().context("locating single-instance lock file")?;
    acquire_lock_file(lock_path)
}

fn lock_file_path() -> Option<PathBuf> {
    crate::app_paths::app_state_dir().map(|base| base.join(LOCK_FILE_NAME))
}

fn acquire_lock_file(lock_path: PathBuf) -> Result<SingleInstance> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("creating single-instance directory")?;
    }

    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening single-instance lock {}", lock_path.display()))?;

    match lock_file.try_lock() {
        Ok(()) => Ok(SingleInstance {
            lock_file: Some(lock_file),
            secondary: false,
        }),
        Err(std::fs::TryLockError::WouldBlock) => Ok(SingleInstance {
            lock_file: None,
            secondary: true,
        }),
        Err(std::fs::TryLockError::Error(error)) => {
            Err(error).context("acquiring single-instance file lock")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn kernel_lock_selects_secondary_and_releases_with_file_handle() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time follows the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "occluview-single-instance-{}-{nonce}.lock",
            std::process::id()
        ));

        let primary = acquire_lock_file(path.clone()).expect("first process acquires lock");
        assert!(!primary.is_secondary());

        let secondary = acquire_lock_file(path.clone()).expect("a duplicate is not an error");
        assert!(secondary.is_secondary());
        drop(secondary);

        drop(primary);
        let restarted = acquire_lock_file(path.clone()).expect("lock is released on drop");
        assert!(!restarted.is_secondary());
        drop(restarted);

        assert!(std::fs::remove_file(path).is_ok());
    }
}
