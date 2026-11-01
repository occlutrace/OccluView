use std::path::PathBuf;

const APP_STATE_DIR_NAME: &str = "OccluView";

/// Redirect the application state directory in tests.
#[cfg(test)]
pub(crate) const TEST_STATE_DIR_ENV: &str = "OCCLUVIEW_TEST_STATE_DIR";

pub(crate) fn app_state_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(directory) = std::env::var_os(TEST_STATE_DIR_ENV) {
        return Some(PathBuf::from(directory));
    }
    platform_state_base_dir().map(|base| base.join(APP_STATE_DIR_NAME))
}

#[cfg(windows)]
fn platform_state_base_dir() -> Option<PathBuf> {
    windows_state_base_dir_from_env(
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    )
}

#[cfg(windows)]
fn windows_state_base_dir_from_env(
    appdata: Option<PathBuf>,
    local_appdata: Option<PathBuf>,
) -> Option<PathBuf> {
    appdata.or(local_appdata)
}

#[cfg(target_os = "macos")]
fn platform_state_base_dir() -> Option<PathBuf> {
    macos_state_base_dir_from_home(std::env::var_os("HOME").map(PathBuf::from))
}

#[cfg(target_os = "macos")]
fn macos_state_base_dir_from_home(home: Option<PathBuf>) -> Option<PathBuf> {
    home.map(|home| home.join("Library/Application Support"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_state_base_dir() -> Option<PathBuf> {
    unix_state_base_dir_from_env(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn unix_state_base_dir_from_env(
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    xdg_state_home.or_else(|| home.map(|home| home.join(".local/state")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_state_dir_prefers_xdg_state_home() {
        assert_eq!(
            unix_state_base_dir_from_env(
                Some(PathBuf::from("/tmp/xdg-state")),
                Some(PathBuf::from("/home/user")),
            ),
            Some(PathBuf::from("/tmp/xdg-state"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_state_dir_falls_back_to_home_local_state() {
        assert_eq!(
            unix_state_base_dir_from_env(None, Some(PathBuf::from("/home/user"))),
            Some(PathBuf::from("/home/user/.local/state"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_state_dir_uses_application_support() {
        assert_eq!(
            macos_state_base_dir_from_home(Some(PathBuf::from("/Users/operator"))),
            Some(PathBuf::from("/Users/operator/Library/Application Support"))
        );
        assert_eq!(macos_state_base_dir_from_home(None), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_state_dir_prefers_roaming_appdata_to_preserve_existing_state() {
        assert_eq!(
            windows_state_base_dir_from_env(
                Some(PathBuf::from(r"C:\Users\me\AppData\Roaming")),
                Some(PathBuf::from(r"C:\Users\me\AppData\Local")),
            ),
            Some(PathBuf::from(r"C:\Users\me\AppData\Roaming"))
        );
    }
}
