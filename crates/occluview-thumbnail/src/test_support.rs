use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

pub(crate) fn acquire_render_test_guard() -> MutexGuard<'static, ()> {
    static RENDER_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    #[cfg(unix)]
    ensure_test_runtime_dir();
    RENDER_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(all(test, unix))]
fn ensure_test_runtime_dir() {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();
    let runtime_dir = RUNTIME_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("occluview-thumbnail-wgpu-runtime");
        let create = std::fs::create_dir_all(&dir);
        assert!(
            create.is_ok(),
            "create thumbnail test runtime dir: {create:?}"
        );

        let permissions = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        assert!(
            permissions.is_ok(),
            "set thumbnail test runtime dir permissions: {permissions:?}"
        );
        dir
    });

    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        std::env::set_var("XDG_RUNTIME_DIR", runtime_dir);
    }
}
