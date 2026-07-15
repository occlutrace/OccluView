//! Shared helpers for render integration tests.

#![allow(clippy::panic)]

#[cfg(unix)]
use std::{path::PathBuf, sync::OnceLock};

pub(crate) fn ensure_test_runtime_dir() {
    #[cfg(unix)]
    {
        static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();
        let runtime_dir = RUNTIME_DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join("occluview-wgpu-runtime");
            std::fs::create_dir_all(&dir)
                .unwrap_or_else(|error| panic!("create test runtime dir: {error}"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                    .unwrap_or_else(|error| panic!("set test runtime dir permissions: {error}"));
            }
            dir
        });

        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            std::env::set_var("XDG_RUNTIME_DIR", runtime_dir);
        }
    }
}
