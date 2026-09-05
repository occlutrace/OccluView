//! Startup composition helpers behind the library boundary.
//!
//! Pure argument parsing, file-extension reporting, and single-instance
//! open-state decisions. No windowing, GPU, or filesystem effects here so the
//! binary can delegate without changing startup behavior; tests pin the
//! current contracts.

use std::ffi::OsStr;
use std::path::PathBuf;

/// Parsed process arguments: launcher flags plus candidate file paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupArgs {
    /// `--shell-refresh` was passed (Windows installer refresh path).
    pub shell_refresh: bool,
    /// `--version` or `-V` was passed; the process prints and exits early.
    pub version: bool,
    /// Remaining arguments, treated as files to open in order.
    pub files: Vec<PathBuf>,
}

/// Parse arguments from an iterator of values after the executable name.
///
/// Flags match anywhere in the sequence; everything else is kept as a file
/// path in order. Never touches the environment or the filesystem.
pub fn parse_args_from<I, S>(args: I) -> StartupArgs
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut parsed = StartupArgs::default();
    for arg in args {
        // Match flags on the lossy view but keep the original bytes for file
        // paths: a non-UTF8 scan name must survive verbatim on Unix.
        let os = arg.as_ref();
        match os.to_string_lossy().as_ref() {
            "--shell-refresh" => parsed.shell_refresh = true,
            "--version" | "-V" => parsed.version = true,
            _ => parsed.files.push(PathBuf::from(os)),
        }
    }
    parsed
}

/// Parse the real process arguments (skips the executable name).
pub fn parse_args() -> StartupArgs {
    parse_args_from(std::env::args_os().skip(1))
}

/// Distinct lowercase extensions of `files`, sorted.
///
/// Log/crash-report safe: describes the session shape (how many files of
/// which kind) without carrying paths that may name a case.
pub fn file_extensions(files: &[PathBuf]) -> Vec<String> {
    let mut extensions: Vec<String> = files
        .iter()
        .filter_map(|path| path.extension())
        .filter_map(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .collect();
    extensions.sort();
    extensions.dedup();
    extensions
}

/// Whether an incoming single-instance open request appends to the current
/// session instead of replacing it.
///
/// Appending preserves operator context: anything already on screen, loading,
/// or queued means the new paths join the queue.
pub fn should_append_incoming_open_state(
    has_scene: bool,
    has_active_load: bool,
    queued_load_count: usize,
) -> bool {
    has_scene || has_active_load || queued_load_count != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_shell_refresh_flags_do_not_become_files() {
        let parsed = parse_args_from(["--shell-refresh", "--version", "scan.stl"]);
        assert!(parsed.shell_refresh);
        assert!(parsed.version);
        assert_eq!(parsed.files, vec![PathBuf::from("scan.stl")]);
    }

    #[test]
    fn short_version_flag_and_flag_order_are_preserved() {
        let parsed = parse_args_from(["a.obj", "-V", "--shell-refresh", "b.stl"]);
        assert!(parsed.version);
        assert!(parsed.shell_refresh);
        assert_eq!(
            parsed.files,
            vec![PathBuf::from("a.obj"), PathBuf::from("b.stl")]
        );
    }

    #[test]
    fn no_arguments_yields_no_flags_and_no_files() {
        assert_eq!(
            parse_args_from(Vec::<String>::new()),
            StartupArgs::default()
        );
    }

    #[test]
    fn file_extensions_are_lowercase_sorted_and_deduplicated() {
        let files = vec![
            PathBuf::from("B.STL"),
            PathBuf::from("a.obj"),
            PathBuf::from("c.stl"),
            PathBuf::from("no-extension"),
        ];
        assert_eq!(file_extensions(&files), vec!["obj", "stl"]);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_file_names_survive_verbatim() {
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"scan-\xff.stl");
        let parsed = parse_args_from([raw]);
        assert!(!parsed.version && !parsed.shell_refresh);
        assert_eq!(parsed.files, vec![PathBuf::from(raw)]);
    }

    #[test]
    fn append_when_any_session_context_exists() {
        assert!(should_append_incoming_open_state(true, false, 0));
        assert!(should_append_incoming_open_state(false, true, 0));
        assert!(should_append_incoming_open_state(false, false, 2));
        assert!(!should_append_incoming_open_state(false, false, 0));
    }
}
