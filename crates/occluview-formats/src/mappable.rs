//! Storage policy for memory-mapped input.
//!
//! A mapped file must not change or disappear while it is read: page faults
//! surface as `EXCEPTION_IN_PAGE_ERROR` (Windows) or `SIGBUS` (Unix), outside
//! Rust panic handling.
//!
//! Only local, stable storage is mapped; other inputs are copied through
//! `read_to_end`.
use std::path::Path;

/// Whether `path` may be memory-mapped.
#[must_use]
pub(crate) fn is_mappable_storage(path: &Path) -> bool {
    platform::is_mappable_storage(path)
}

#[cfg(windows)]
#[allow(unsafe_code)] // see lib.rs: the drive-type query is kernel FFI.
mod platform {
    use std::path::{Component, Path, Prefix};
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;

    /// `GetDriveTypeW` returns a bare `u32`; these are the stable local types.
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_RAMDISK: u32 = 6;

    pub(super) fn is_mappable_storage(path: &Path) -> bool {
        // A UNC path is a network share by definition, and `GetDriveTypeW`
        // reports DRIVE_NO_ROOT_DIR for one, so answer directly.
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return false;
        };
        let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
            return false;
        };
        let root = HSTRING::from(format!("{}:\\", char::from(letter)));
        // SAFETY: `root` is a NUL-terminated wide string that outlives the call,
        // and GetDriveTypeW only reads it.
        let kind = unsafe { GetDriveTypeW(&root) };
        kind == DRIVE_FIXED || kind == DRIVE_RAMDISK
    }
}

/// Local filesystems whose pages remain available while mapped.
///
/// This is an allow list. Overlay and network filesystems are intentionally
/// absent because their backing layers may disappear during a parse.
#[cfg(any(unix, test))]
const MAPPABLE_FILESYSTEMS: &[&str] = &[
    "bcachefs", "btrfs", "ext2", "ext3", "ext4", "f2fs", "jfs", "ramfs", "reiserfs", "tmpfs",
    "xfs", "zfs",
];

/// Whether a mount of type `kind` may be mapped.
///
/// Kept apart from the mount-table lookup: the parsing tests can all pass
/// while the list says "yes" to nfs4.
#[cfg(any(unix, test))]
fn is_mappable_filesystem(kind: &str) -> bool {
    MAPPABLE_FILESYSTEMS.contains(&kind)
}

#[cfg(unix)]
mod platform {
    use std::path::Path;

    pub(super) fn is_mappable_storage(path: &Path) -> bool {
        let Ok(mountinfo) = std::fs::read_to_string("/proc/self/mountinfo") else {
            // No mountinfo (a non-Linux Unix, or a sandbox without /proc):
            // there is no way to tell, so copy.
            return false;
        };
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        super::filesystem_type_for(&mountinfo, &resolved).is_some_and(super::is_mappable_filesystem)
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use std::path::Path;

    pub(super) fn is_mappable_storage(_path: &Path) -> bool {
        false
    }
}

/// The filesystem type of the mount that `path` resolves into, per a
/// `/proc/self/mountinfo` document.
///
/// Split out from the platform module so the parsing can be tested anywhere:
/// the mount table is an input, not an ambient fact.
#[cfg(any(unix, test))]
fn filesystem_type_for<'a>(mountinfo: &'a str, path: &Path) -> Option<&'a str> {
    let mut best: Option<(usize, &'a str)> = None;
    for line in mountinfo.lines() {
        let (before, after) = line.split_once(" - ")?;
        let mount_point = before.split(' ').nth(4)?;
        let filesystem = after.split(' ').next()?;
        let mount_point = unescape_mount_point(mount_point);
        if !path_is_under(path, &mount_point) {
            continue;
        }
        // The deepest matching mount point is the one that governs the path.
        if best.is_none_or(|(depth, _)| mount_point.len() >= depth) {
            best = Some((mount_point.len(), filesystem));
        }
    }
    best.map(|(_, filesystem)| filesystem)
}

/// `mountinfo` octal-escapes space, tab, newline and backslash in paths.
#[cfg(any(unix, test))]
fn unescape_mount_point(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find('\\') {
        out.push_str(&rest[..index]);
        let escape = rest.get(index..index + 4);
        match escape {
            Some("\\040") => out.push(' '),
            Some("\\011") => out.push('\t'),
            Some("\\012") => out.push('\n'),
            Some("\\134") => out.push('\\'),
            _ => {
                out.push('\\');
                rest = &rest[index + 1..];
                continue;
            }
        }
        rest = &rest[index + 4..];
    }
    out.push_str(rest);
    out
}

#[cfg(any(unix, test))]
fn path_is_under(path: &Path, mount_point: &str) -> bool {
    let mount = Path::new(mount_point);
    path == mount || path.starts_with(mount)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
25 29 0:23 / /proc rw,nosuid shared:12 - proc proc rw
29 1 254:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw
36 29 0:31 / /home/clinic/scans rw,relatime shared:9 - nfs4 nas:/scans rw
48 29 0:44 / /media/usb\\040stick rw,nosuid shared:2 - vfat /dev/sdb1 rw
52 29 0:52 / /mnt/clinic rw,relatime shared:3 - cifs //fileserver/clinic rw
60 29 0:60 / /tmp rw,nosuid shared:4 - tmpfs tmpfs rw
";

    #[test]
    fn the_deepest_mount_point_wins() {
        assert_eq!(
            filesystem_type_for(MOUNTINFO, Path::new("/home/clinic/scans/upper.stl")),
            Some("nfs4"),
            "a share mounted below / must not be read as the root filesystem"
        );
        assert_eq!(
            filesystem_type_for(MOUNTINFO, Path::new("/home/clinic/local.stl")),
            Some("ext4")
        );
    }

    #[test]
    fn octal_escaped_mount_points_are_decoded() {
        assert_eq!(
            filesystem_type_for(MOUNTINFO, Path::new("/media/usb stick/scan.stl")),
            Some("vfat"),
            "mountinfo escapes spaces as \\\\040"
        );
    }

    #[test]
    fn network_and_removable_filesystems_are_not_mappable() {
        // Exactly the mounts a dental workstation reads scans from, and
        // exactly the ones that can be withdrawn mid-parse. Both halves are
        // asserted: the type is read right, and then refused. Check only the
        // type and the allow list can gain "nfs4" with nothing going red.
        for (path, expected) in [
            ("/home/clinic/scans/upper.stl", "nfs4"),
            ("/mnt/clinic/case.stl", "cifs"),
            ("/media/usb stick/scan.stl", "vfat"),
        ] {
            let kind = filesystem_type_for(MOUNTINFO, Path::new(path));
            assert_eq!(kind, Some(expected));
            assert!(
                !is_mappable_filesystem(expected),
                "{expected} pages can be withdrawn mid-parse; mapping one takes \
                 the process down with SIGBUS, which no catch_unwind sees"
            );
        }
    }

    #[test]
    fn local_filesystems_are_mappable_so_the_fast_path_still_exists() {
        // The counterweight: a policy that refuses everything is safe and
        // useless. These are where scans actually live on a workstation.
        for kind in ["ext4", "btrfs", "xfs", "zfs", "tmpfs"] {
            assert!(is_mappable_filesystem(kind), "{kind} should be mappable");
        }
        assert_eq!(
            filesystem_type_for(MOUNTINFO, Path::new("/home/clinic/local.stl")),
            Some("ext4"),
            "the root filesystem is the common case and must reach the fast path"
        );
    }

    #[test]
    fn an_overlay_mount_is_not_mappable_because_its_layers_are_unknown() {
        // overlayfs reports "overlay" whatever it is stacked on, including an
        // NFS or CIFS export. The type carries no information about whether
        // the pages can be withdrawn, so it cannot be trusted.
        assert!(!is_mappable_filesystem("overlay"));
    }

    #[test]
    fn an_unlisted_path_maps_to_nothing_rather_than_guessing() {
        assert_eq!(filesystem_type_for("", Path::new("/anything")), None);
    }
}

#[cfg(test)]
mod platform_tests {
    use super::is_mappable_storage;
    use std::path::Path;

    /// The filesystem this checkout sits on, as `/proc/self/mountinfo` reports
    /// it. `None` off Linux, or wherever `/proc` is not mounted.
    fn filesystem_under_the_checkout() -> Option<String> {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
        let here = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR"))).ok()?;
        super::filesystem_type_for(&mountinfo, &here).map(str::to_owned)
    }

    #[test]
    fn a_file_on_this_checkout_is_still_mapped() {
        // The point of the gate is to refuse storage that can vanish, not to
        // disable mapping. If this ever fails on a normal developer machine or
        // CI runner, the allow list has lost a mainstream local filesystem and
        // every read silently became a copy.
        //
        // Inside a container the checkout is on overlayfs, which this policy
        // refuses on purpose because an overlay says nothing about the layers
        // under it. There the correct answer is "copy", and this test has
        // nothing left to prove.
        if filesystem_under_the_checkout().as_deref() == Some("overlay") {
            return;
        }
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(
            is_mappable_storage(&here),
            "a file in the working tree should be mappable; \
             the filesystem it lives on is missing from the allow list"
        );
    }

    #[test]
    fn a_path_that_does_not_exist_is_answered_without_panicking() {
        let _ = is_mappable_storage(Path::new("/nonexistent/occluview/probe.stl"));
    }
}
