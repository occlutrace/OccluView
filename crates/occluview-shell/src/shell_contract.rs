/// The installed GUI binary name used by shell "Open with" registration.
pub const APP_EXE_NAME: &str = "occluview.exe";

/// The shell's `IThumbnailProvider` **category** CLSID, not a CLSID of ours.
///
/// Registered under `HKCR\.[ext]\ShellEx\<this>` for each supported extension,
/// which is how Explorer finds whichever handler claims the category. OccluView's
/// own CLSID is the separate one `occluview_shell.dll` registers; naming this
/// constant the provider's CLSID pointed every reader at the wrong key.
pub const THUMBNAIL_PROVIDER_CATEGORY: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";

/// The shell preview handler category used by Explorer's Preview Pane.
pub const PREVIEW_HANDLER_CATEGORY: &str = "{8895B1C6-B41F-4C1C-A562-0D564250836F}";

/// File extensions OccluView registers a thumbnail provider and Open-with
/// `ProgID` for.
///
/// JSON `.gltf` and `.3mf` are absent because they have no stream-safe readers.
pub const SUPPORTED_EXTENSIONS: &[&str] = occluview_formats::V1_OPEN_EXTENSIONS;

/// Extensions whose MSI registration names the shipped `occluview-3d` icon.
///
/// Kept separate from the supported extension list so installer icon metadata
/// can be checked independently.
pub const DEDICATED_FILE_ICON_EXTENSIONS: &[&str] = &["stl", "ply", "obj", "glb", "off"];

/// Supported extensions offered without claiming machine-wide ownership.
pub const OFFERED_ONLY_EXTENSIONS: &[&str] = &[];

/// Whether OccluView may write the machine-wide handler entries for `ext`.
///
/// Unregistration removes registrations owned by this application.
#[must_use]
pub fn owns_extension(extension: &str) -> bool {
    SUPPORTED_EXTENSIONS.contains(&extension) && !OFFERED_ONLY_EXTENSIONS.contains(&extension)
}
