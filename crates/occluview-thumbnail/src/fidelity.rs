//! Full-fidelity parse cutoffs: one table, keyed by [`FormatKind`].
//!
//! `occluview-formats`' canonical reader materialises every triangle, so a
//! surface that draws a file bounds the size it hands over; a larger file goes
//! to the decimating `fast_thumb` reader instead. Two Explorer surfaces make
//! that decision — the thumbnail and the preview — and each used to carry its
//! own copy of the numbers. The copies had drifted, so one file could be read in
//! full in the preview and decimated in the thumbnail.
//!
//! The table is keyed by [`FormatKind`], never by an extension string: two
//! spellings of one format must not answer differently, and `.dcm` is a name
//! whose correct kind comes from the probe rather than from the text.
//! [`full_fidelity_file_bytes_for_extension`] is the single place an extension
//! becomes a kind for this policy.
//!
//! # The two surfaces, and why STL still differs
//!
//! The preview is one interactive pane and takes [`full_fidelity_file_bytes`] as
//! written. The thumbnail surrogate hosts several renders inside a process it
//! does not own, so it takes the flat [`THUMBNAIL_FULL_FIDELITY_FILE_BYTES`].
//! That leaves STL deliberately different between them — 128 MiB against
//! 40 MiB — and every other format equal. The difference is a recorded decision,
//! not the residue of two tables.

use occluview_formats::{probe, FormatKind};

/// The largest STL the preview reads with the canonical reader: about 2.7
/// million triangles, one file at a time.
pub const FULL_FIDELITY_STL_FILE_BYTES: u64 = 128 * 1024 * 1024;

/// The cutoff every other format uses. A full read at this size is roughly
/// 420 MB of resident vertices, which is the cost the limit bounds.
pub const FULL_FIDELITY_FILE_BYTES: u64 = 40 * 1024 * 1024;

/// The thumbnail's cutoff, flat for every kind: this surrogate hosts several
/// renders at once, and the format does not change that cost.
pub const THUMBNAIL_FULL_FIDELITY_FILE_BYTES: u64 = FULL_FIDELITY_FILE_BYTES;

/// The preview's cutoff for `kind`.
#[must_use]
pub fn full_fidelity_file_bytes(kind: FormatKind) -> u64 {
    match kind {
        FormatKind::Stl => FULL_FIDELITY_STL_FILE_BYTES,
        _ => FULL_FIDELITY_FILE_BYTES,
    }
}

/// The preview's cutoff for a file known only by its extension.
///
/// An unknown extension takes the default cutoff rather than an unlimited one,
/// so a file the probe cannot name is not also a file the reader is told to take
/// whole.
#[must_use]
pub fn full_fidelity_file_bytes_for_extension(extension: &str) -> u64 {
    probe::by_extension(extension).map_or(FULL_FIDELITY_FILE_BYTES, full_fidelity_file_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_KIND: [FormatKind; 7] = [
        FormatKind::Stl,
        FormatKind::Ply,
        FormatKind::Obj,
        FormatKind::Gltf,
        FormatKind::Threemf,
        FormatKind::Off,
        FormatKind::Hps,
    ];

    #[test]
    fn only_stl_gets_the_larger_preview_cutoff() {
        for kind in EVERY_KIND {
            let expected = if kind == FormatKind::Stl {
                FULL_FIDELITY_STL_FILE_BYTES
            } else {
                FULL_FIDELITY_FILE_BYTES
            };
            assert_eq!(full_fidelity_file_bytes(kind), expected, "{kind:?}");
        }
    }

    /// The cross-surface rule the two separate tables could not state: equal
    /// everywhere except the one format the preview is allowed to hold whole.
    #[test]
    fn the_surfaces_differ_for_stl_alone() {
        for kind in EVERY_KIND {
            let preview = full_fidelity_file_bytes(kind);
            if kind == FormatKind::Stl {
                assert!(preview > THUMBNAIL_FULL_FIDELITY_FILE_BYTES, "{kind:?}");
            } else {
                assert_eq!(preview, THUMBNAIL_FULL_FIDELITY_FILE_BYTES, "{kind:?}");
            }
        }
    }

    #[test]
    fn an_unknown_extension_takes_the_default_cutoff() {
        assert_eq!(
            full_fidelity_file_bytes_for_extension("not-a-format"),
            FULL_FIDELITY_FILE_BYTES
        );
    }

    /// Every extension the viewer opens resolves to a kind, so no shipped format
    /// silently falls through to the default.
    #[test]
    fn every_open_extension_resolves_to_a_kind() {
        for extension in occluview_formats::V1_OPEN_EXTENSIONS {
            let kind = probe::by_extension(extension);
            assert!(kind.is_some(), "{extension} must resolve to a kind");
            assert_eq!(
                full_fidelity_file_bytes_for_extension(extension),
                full_fidelity_file_bytes(kind.expect("checked above")),
                "{extension}"
            );
        }
    }
}
