/// The installed GUI binary name used by shell "Open with" registration.
pub const APP_EXE_NAME: &str = "occluview.exe";

/// The CLSID string for the OccluView thumbnail provider.
///
/// Registered under
/// `HKCR\.<ext>\ShellEx\{E357FCCD-A995-4576-B01F-234630154E96}` for each
/// supported extension. (The literal `{E357FCCD-A995-4576-B01F-234630154E96}`
/// is the shell's `IThumbnailProvider` category, not our own CLSID — our own
/// CLSID is generated when the COM class lands.)
pub const THUMBNAIL_PROVIDER_CATEGORY: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";

/// The shell preview handler category used by Explorer's Preview Pane.
pub const PREVIEW_HANDLER_CATEGORY: &str = "{8895B1C6-B41F-4C1C-A562-0D564250836F}";

/// File extensions OccluView registers a thumbnail provider and Open-with
/// `ProgID` for.
///
/// JSON `.gltf` and `.3mf` are deliberately absent until their stream-safe
/// readers exist. HPS are included because private builds can provide the
/// HPS key at runtime while public builds safely fall back to placeholders for
/// encrypted CE sources.
pub const SUPPORTED_EXTENSIONS: &[&str] = occluview_formats::V1_OPEN_EXTENSIONS;

/// Formats that ship a dedicated file-type icon asset in the MSI.
pub const DEDICATED_FILE_ICON_EXTENSIONS: &[&str] = SUPPORTED_EXTENSIONS;

/// Extensions OccluView reads but must never claim as the machine default.
///
/// `.dcm` is two things at once: the extension 3Shape writes its HPS
/// containers under, and the extension medical DICOM has used for decades. A
/// dental workstation holds both, and this reader rejects DICOM by design
/// ([`occluview_formats::FormatError`] carries a dedicated variant for it).
/// Writing the machine-wide handler entries for `.dcm` -- the unnamed
/// `HKCR\.dcm` `ProgID`, its `DefaultIcon`, and the `ShellEx` providers under
/// either the bare key or `SystemFileAssociations\.dcm` -- would take every
/// CBCT file on the machine away from its viewer and hand it a thumbnail that
/// cannot render. OccluView is still offered for `.dcm` through
/// `OpenWithProgids`, `OpenWithList`, `SupportedTypes` and the Default Apps
/// capabilities, so a user with genuine 3Shape `.dcm` scans can choose it.
pub const OFFERED_ONLY_EXTENSIONS: &[&str] = &[occluview_formats::LEGACY_HPS_EXTENSION];

/// Whether OccluView may write the machine-wide handler entries for `ext`.
///
/// Unregistration ignores this, and must: entries written by an older build
/// that did claim `.dcm` still have to be cleaned up.
#[must_use]
pub fn owns_extension(extension: &str) -> bool {
    !OFFERED_ONLY_EXTENSIONS.contains(&extension)
}
