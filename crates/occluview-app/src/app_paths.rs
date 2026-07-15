use std::path::PathBuf;

const APP_STATE_DIR_NAME: &str = "OccluView";

pub(crate) fn app_state_dir() -> Option<PathBuf> {
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

#[cfg(not(windows))]
fn platform_state_base_dir() -> Option<PathBuf> {
    unix_state_base_dir_from_env(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

#[cfg(not(windows))]
fn unix_state_base_dir_from_env(
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    xdg_state_home.or_else(|| home.map(|home| home.join(".local/state")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
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

    #[cfg(not(windows))]
    #[test]
    fn unix_state_dir_falls_back_to_home_local_state() {
        assert_eq!(
            unix_state_base_dir_from_env(None, Some(PathBuf::from("/home/user"))),
            Some(PathBuf::from("/home/user/.local/state"))
        );
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
