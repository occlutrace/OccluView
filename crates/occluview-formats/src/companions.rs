//! Textures that sit beside a mesh file.
//!
//! OBJ reaches its image through `mtllib` and a `map_Kd` line; PLY names one in
//! a `comment TextureFile` line. Neither format holds the image in the file
//! itself, which is why a file from another program arrives with a companion
//! the reader has to find, and why OccluView's own exports bake the colour into
//! the vertices rather than writing a second file.
//!
//! The reader itself takes bytes and nothing else, on purpose: a file another
//! process is replacing mid-import must not change what was parsed. Finding a
//! companion is therefore a separate step, done by the path-aware entry points
//! in [`crate::dispatch`]. A missing image leaves the mesh available; an
//! unsupported material interpretation returns an explicit error.

use crate::error::FormatError;
use crate::texture_decode::decode_embedded_raster;
use occluview_core::Mesh;
use std::path::{Path, PathBuf};

/// Largest material library read.
///
/// A material library is a few lines of text. One that is larger is not a
/// material library, and the cap keeps a crafted `mtllib` from asking for an
/// arbitrary read.
const MAX_MATERIAL_LIBRARY_BYTES: u64 = 1 << 20;

/// Largest companion image read.
///
/// The decoder bounds the *decoded* surface; this bounds the read, so a file
/// that claims to be an atlas cannot pull gigabytes through the import.
pub(crate) const MAX_COMPANION_IMAGE_BYTES: u64 = 64 << 20;

/// Image extensions tried when a mesh names no image but sits beside one.
///
/// Matched case-insensitively, because a scanner writes `scan.JPG` as readily
/// as `scan.jpg` and both name the same picture.
const SAME_STEM_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Attach the texture this mesh file points at beside itself.
///
/// A mesh that already carries a texture keeps it: a GLB or HPS holds its own
/// image, and nothing beside the file overrides it.
pub(crate) fn attach(
    mesh: &mut Mesh,
    path: &Path,
    kind: LocateKind,
    bytes: &[u8],
) -> Result<(), FormatError> {
    if mesh.texture().is_some() {
        return Ok(());
    }
    let image = locate(path, kind, bytes)?;
    // Without UVs an atlas would paint the whole layer with a single texel.
    if !mesh.has_uvs() {
        return Ok(());
    }
    let Some(image) = image else {
        return Ok(());
    };
    // A missing or undecodable image leaves the geometry available.
    let Ok(image_bytes) =
        crate::dispatch::read_file_bytes_with_limit(&image, MAX_COMPANION_IMAGE_BYTES)
    else {
        return Ok(());
    };
    let Ok(texture) = decode_embedded_raster(image_bytes.as_slice(), "texture beside the mesh")
    else {
        return Ok(());
    };
    mesh.set_texture(texture);
    Ok(())
}

/// The image a mesh file names, or the one that shares its name.
fn locate(path: &Path, kind: LocateKind, bytes: &[u8]) -> Result<Option<PathBuf>, FormatError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    // A relative path from a command line has an empty parent, and the files
    // it names are in the working directory.
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    Ok(match kind {
        LocateKind::Obj => material_image(directory, bytes)?.or_else(|| {
            used_face_material(bytes)
                .is_none()
                .then(|| same_stem_image(path, directory))
                .flatten()
        }),
        LocateKind::Ply => {
            let Ok(header) = crate::ply::header::parse(bytes) else {
                return Ok(None);
            };
            header
                .texture
                .file
                .as_deref()
                .and_then(|name| inside(directory, directory, name))
        }
        LocateKind::None => None,
    })
}

/// Which companion lookup a mesh file is entitled to.
///
/// The choice follows the format the reader actually used, not the extension
/// on the name: a `.stl` that is really a PLY must find the image its own
/// `TextureFile` comment names, which is the case the magic-first probe exists
/// for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocateKind {
    /// An OBJ, which names its image through `mtllib`/`map_Kd`.
    Obj,
    /// A PLY, which may name it in a `comment TextureFile` line.
    Ply,
    /// Neither: this format has no companion lookup.
    None,
}

impl LocateKind {
    /// The lookup a probed format is entitled to.
    pub(crate) fn for_kind(kind: crate::probe::FormatKind) -> Self {
        match kind {
            crate::probe::FormatKind::Obj => Self::Obj,
            crate::probe::FormatKind::Ply => Self::Ply,
            _ => Self::None,
        }
    }
}

/// The used material's image, or the first atlas for legacy OBJs without usemtl.
///
/// An OBJ may name several libraries, and a library may carry several
/// materials of which only some have an image. Stopping at the first value
/// found loses the texture whenever that value is the one without a file.
fn material_image(directory: &Path, obj: &[u8]) -> Result<Option<PathBuf>, FormatError> {
    let used = used_face_material(obj);
    for library in directives(obj, "mtllib") {
        let Some(library) = inside(directory, directory, &library) else {
            continue;
        };
        let Ok(text) =
            crate::dispatch::read_file_bytes_with_limit(&library, MAX_MATERIAL_LIBRARY_BYTES)
        else {
            continue;
        };
        let Ok(material_text) = std::str::from_utf8(text.as_slice()) else {
            return Ok(None);
        };
        let mut sections = vec![(None, Vec::new())];
        for line in material_text.trim_start_matches('\u{feff}').lines() {
            if let Some(name) = directives(line.as_bytes(), "newmtl").first() {
                sections.push((Some(name.clone()), Vec::new()));
            } else if let Some((_, lines)) = sections.last_mut() {
                lines.push(line);
            }
        }
        for (name, lines) in sections {
            if used
                .as_ref()
                .is_some_and(|used| name.as_ref() != Some(used))
            {
                continue;
            }
            let image = lines
                .iter()
                .flat_map(|line| directives(line.as_bytes(), "map_Kd"))
                .find_map(|image| inside(directory, library.parent()?, &image));
            if used.is_some() || image.is_some() {
                validate_material_properties(&lines)?;
            }
            if image.is_some() {
                return Ok(image);
            }
        }
    }
    Ok(None)
}

fn unsupported_material(property: &str) -> FormatError {
    FormatError::Deferred {
        format: "OBJ material",
        reason: format!("unsupported material property {property}; appearance cannot be preserved"),
    }
}

/// Validate the selected section before attaching an atlas from that section.
fn validate_material_properties(lines: &[&str]) -> Result<(), FormatError> {
    for line in lines {
        let tokens: Vec<_> = line
            .split('#')
            .next()
            .unwrap_or("")
            .split_ascii_whitespace()
            .collect();
        match tokens.first().copied() {
            Some("Kd")
                if tokens.len() != 4
                    || tokens[1..]
                        .iter()
                        .any(|value| value.parse::<f64>().ok() != Some(1.0)) =>
            {
                return Err(unsupported_material("Kd"));
            }
            Some("map_Kd") => validate_map_properties(&tokens[1..])?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_map_properties(mut tokens: &[&str]) -> Result<(), FormatError> {
    while let Some(option) = tokens
        .first()
        .copied()
        .filter(|value| value.starts_with('-'))
    {
        tokens = &tokens[1..];
        if option == "-mm" {
            if tokens.len() < 2
                || tokens[0].parse::<f64>().ok() != Some(0.0)
                || tokens[1].parse::<f64>().ok() != Some(1.0)
            {
                return Err(unsupported_material(option));
            }
            tokens = &tokens[2..];
            continue;
        }
        let expected = match option {
            "-s" => 1.0,
            "-o" | "-t" => 0.0,
            _ => return Err(unsupported_material(option)),
        };
        let mut count = 0;
        while count < 3 {
            let Some(value) = tokens.first().and_then(|value| value.parse::<f64>().ok()) else {
                break;
            };
            if value != expected {
                return Err(unsupported_material(option));
            }
            tokens = &tokens[1..];
            count += 1;
        }
        if count == 0 {
            return Err(unsupported_material(option));
        }
    }
    Ok(())
}

/// Material active at the first face; the OBJ parser refuses mixed identities.
fn used_face_material(obj: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(obj).ok()?;
    let mut current = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        if let Some(name) = directives(line.as_bytes(), "usemtl").first() {
            current = Some(name.clone());
        }
        if line.split_ascii_whitespace().next() == Some("f") {
            return current;
        }
    }
    current
}

/// Every value of `keyword` in a text file, in the order they appear.
///
/// OBJ and MTL are line-oriented and lenient: leading whitespace, tabs and
/// trailing comments all occur in files from scanners. A value is everything
/// after the keyword, minus a trailing comment and surrounding quotes. Options
/// that precede a file name in `map_Kd` (`-s`, `-o`, `-clamp`) are dropped, and
/// a file name containing spaces survives because only the leading option
/// tokens are removed.
fn directives(bytes: &[u8], keyword: &str) -> Vec<String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let Some(text) = std::str::from_utf8(bytes).ok() else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(keyword) else {
            continue;
        };
        // `mtllib` must not match `mtllibfoo`.
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.split('#').next().unwrap_or(rest).trim();
        let mut tokens: Vec<&str> = rest.split_ascii_whitespace().collect();
        strip_options(&mut tokens);
        let value = tokens.join(" ").trim_matches('"').trim().to_string();
        if !value.is_empty() {
            values.push(value);
        }
    }
    values
}

/// Drop the options that may precede a `map_Kd` file name.
///
/// Vector options take one to three numeric components; other known options
/// have fixed arity. The remaining tokens form the filename, including spaces.
/// An unknown option is dropped on its own.
fn strip_options(tokens: &mut Vec<&str>) {
    loop {
        let Some(option) = tokens.first().copied() else {
            return;
        };
        if !option.starts_with('-') {
            return;
        }
        tokens.remove(0);
        if matches!(option, "-s" | "-o" | "-t") {
            for _ in 0..3 {
                if tokens
                    .first()
                    .is_none_or(|value| value.parse::<f64>().is_err())
                {
                    break;
                }
                tokens.remove(0);
            }
            continue;
        }
        let arguments = match option {
            "-mm" => 2,
            "-bm" | "-cc" | "-clamp" | "-blendu" | "-blendv" | "-texres" | "-imfchan" | "-type"
            | "-boost" => 1,
            _ => 0,
        };
        for _ in 0..arguments {
            if tokens.is_empty() {
                return;
            }
            tokens.remove(0);
        }
    }
}

/// Resolve a name against its source folder, confined to the mesh folder.
///
/// A name that climbs out of that folder is refused. The mesh came from
/// somewhere — a lab partner, a download — and a line inside it must not be
/// able to read an arbitrary file from the operator's disk, whatever the
/// program that wrote it intended.
fn inside(root: &Path, directory: &Path, name: &str) -> Option<PathBuf> {
    // Windows separators in a file written on Windows.
    let name = name.replace('\\', "/");
    let candidate = directory.join(name);
    let directory = root.canonicalize().ok()?;
    let resolved = candidate.canonicalize().ok()?;
    resolved.starts_with(&directory).then_some(resolved)
}

/// An image that shares the mesh's name, which is how many dental exports ship
/// an OBJ with no usable material library.
fn same_stem_image(path: &Path, directory: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_string_lossy().to_lowercase();
    let entries = std::fs::read_dir(directory).ok()?;
    // Case-insensitive over the directory rather than over a fixed list:
    // `scan.PNG`, `scan.JPG` and `scan.JPEG` name the same picture as their
    // lower-case spellings, and a fixed list always misses one of them.
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            let Some(name) = candidate.file_name().and_then(|name| name.to_str()) else {
                return false;
            };
            let lower = name.to_lowercase();
            let Some((candidate_stem, candidate_extension)) = lower.rsplit_once('.') else {
                return false;
            };
            candidate_stem == stem
                && SAME_STEM_EXTENSIONS.contains(&candidate_extension)
                && candidate.is_file()
        })
        .collect();
    // Deterministic when several spellings exist side by side.
    found.sort();
    found
        .into_iter()
        .find_map(|candidate| inside(directory, directory, candidate.file_name()?.to_str()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{MeshTexture, Vertex};

    #[test]
    fn obj_file_refuses_unrepresented_used_material_properties() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("scan.obj");
        let obj = b"mtllib scan.mtl\nusemtl used\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n";
        std::fs::write(&path, obj).expect("OBJ");
        std::fs::write(directory.path().join("scan.png"), textured_png()).expect("image");
        for properties in [
            "map_Kd -s 2 1 1 scan.png",
            "map_Kd scan.png\nKd 0.5 1 1",
            "map_Kd -o 0.25 scan.png",
            "map_Kd -clamp on scan.png",
        ] {
            std::fs::write(
                directory.path().join("scan.mtl"),
                format!("newmtl unused\nmap_Kd -s 9 unused.png\nnewmtl used\n{properties}\n"),
            )
            .expect("MTL");
            assert!(
                crate::read_file(&path).is_err(),
                "silently discarded {properties}"
            );
        }
        std::fs::write(directory.path().join("scan.mtl"), "newmtl unused\nmap_Kd -s 9 unused.png\nnewmtl used\nmap_Kd -s 1 1 1 -o 0 0 0 -t 0 -mm 0 1 scan.png\nKd 1 1 1\n").expect("identity MTL");
        let mesh = crate::read_file(&path).expect("identity appearance");
        assert!(mesh.texture().is_some());
    }

    #[test]
    fn unnamed_obj_faces_keep_legacy_companions_after_an_unused_material_switch() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("scan.obj");
        std::fs::write(directory.path().join("scan.png"), textured_png()).expect("image");
        std::fs::write(
            &path,
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\nusemtl unused\n",
        )
        .expect("OBJ");
        assert!(crate::read_file(&path)
            .expect("unnamed faces")
            .texture()
            .is_some());
    }

    #[test]
    fn obj_companion_uses_the_faces_material_identity() {
        let directory = tempfile::tempdir().expect("directory");
        for name in ["first.png", "used.png", "scan.png"] {
            std::fs::write(directory.path().join(name), textured_png()).expect("image");
        }
        std::fs::write(
            directory.path().join("scan.mtl"),
            "newmtl first\nmap_Kd first.png\nnewmtl used\nmap_Kd used.png\n",
        )
        .expect("MTL");
        let obj = b"mtllib scan.mtl\nusemtl used\nf 1/1 2/2 3/3\n";
        assert_eq!(
            material_image(directory.path(), obj).expect("material lookup"),
            Some(
                directory
                    .path()
                    .join("used.png")
                    .canonicalize()
                    .expect("path")
            )
        );
        let missing = b"mtllib scan.mtl\nusemtl absent\nf 1/1 2/2 3/3\n";
        assert!(
            locate(&directory.path().join("scan.obj"), LocateKind::Obj, missing)
                .expect("material lookup")
                .is_none(),
            "an unrelated atlas must not replace a missing named material"
        );
    }

    #[test]
    fn a_material_library_bom_does_not_hide_its_first_texture() {
        assert_eq!(
            directives(b"\xef\xbb\xbfmap_Kd atlas.png\n", "map_Kd"),
            ["atlas.png"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn same_stem_images_cannot_escape_through_a_symlink() {
        let directory = tempfile::tempdir().expect("directory");
        let folder = directory.path().join("mesh");
        std::fs::create_dir(&folder).expect("mesh folder");
        let external = directory.path().join("external.png");
        std::fs::write(&external, textured_png()).expect("image");
        std::os::unix::fs::symlink(external, folder.join("scan.png")).expect("link");
        assert!(same_stem_image(&folder.join("scan.obj"), &folder).is_none());
    }

    #[test]
    fn material_images_are_relative_to_their_library_within_the_mesh_folder() {
        let directory = tempfile::tempdir().expect("directory");
        let materials = directory.path().join("materials");
        std::fs::create_dir(&materials).expect("materials directory");
        std::fs::write(materials.join("atlas.png"), textured_png()).expect("nested image");
        std::fs::write(directory.path().join("atlas.png"), textured_png()).expect("root image");
        for (name, expected) in [
            ("atlas.png", materials.join("atlas.png")),
            ("../atlas.png", directory.path().join("atlas.png")),
        ] {
            std::fs::write(materials.join("scan.mtl"), format!("map_Kd {name}\n")).expect("MTL");
            assert_eq!(
                material_image(directory.path(), b"mtllib materials/scan.mtl\n")
                    .expect("material lookup"),
                Some(expected.canonicalize().expect("image path")),
                "{name}"
            );
        }
    }

    #[test]
    fn material_options_keep_optional_components_out_of_the_filename() {
        for (line, expected) in [
            ("map_Kd -s 2 atlas.png", "atlas.png"),
            (
                "map_Kd -o -1 0 textures/atlas file.png",
                "textures/atlas file.png",
            ),
            ("map_Kd -t 0 -clamp on atlas.png", "atlas.png"),
            ("map_Kd -cc on atlas.png", "atlas.png"),
        ] {
            assert_eq!(directives(line.as_bytes(), "map_Kd"), [expected], "{line}");
        }
    }

    #[test]
    fn a_bom_prefixed_obj_loads_its_named_companion() {
        let directory = tempfile::tempdir().expect("directory");
        std::fs::write(directory.path().join("atlas.png"), textured_png()).expect("image");
        std::fs::write(directory.path().join("atlas.mtl"), "map_Kd atlas.png\n").expect("MTL");
        let bytes = b"\xef\xbb\xbfmtllib atlas.mtl\n\
            v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 1\nvt 1 1\nvt 0 0\nf 1/1 2/2 3/3\n";
        let path = directory.path().join("scan.obj");
        std::fs::write(&path, bytes).expect("OBJ");
        let mesh = crate::read_file(&path).expect("valid BOM OBJ");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(
            mesh.texture().expect("named companion").rgba,
            [255, 0, 0, 255, 0, 0, 255, 255]
        );
    }

    #[test]
    fn a_bom_prefixed_ply_loads_its_named_companion() {
        let directory = tempfile::tempdir().expect("directory");
        std::fs::write(directory.path().join("atlas.png"), textured_png()).expect("image");
        let bytes = b"\xef\xbb\xbfply\nformat ascii 1.0\ncomment TextureFile atlas.png\n\
            element vertex 3\nproperty float x\nproperty float y\nproperty float z\n\
            property float s\nproperty float t\nelement face 1\n\
            property list uchar int vertex_indices\nend_header\n\
            0 0 0 0 1\n1 0 0 1 1\n0 1 0 0 0\n3 0 1 2\n";
        let path = directory.path().join("scan.ply");
        std::fs::write(&path, bytes).expect("PLY");
        let mesh = crate::read_file(&path).expect("valid BOM PLY");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(
            mesh.texture().expect("named companion").rgba,
            [255, 0, 0, 255, 0, 0, 255, 255]
        );
    }

    fn textured_png() -> Vec<u8> {
        crate::glb_writer::encode_png(&MeshTexture::new(
            2,
            1,
            vec![255, 0, 0, 255, 0, 0, 255, 255],
        ))
        .expect("a PNG")
    }

    fn triangle() -> Mesh {
        Mesh::new(
            Some("scan".to_string()),
            vec![
                Vertex::at(glam::Vec3::ZERO).with_uv([0.0, 1.0]),
                Vertex::at(glam::Vec3::X).with_uv([1.0, 1.0]),
                Vertex::at(glam::Vec3::Y).with_uv([0.0, 0.0]),
            ],
            vec![0, 1, 2],
        )
        .expect("a triangle")
    }

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "occluview-companion-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        path
    }

    #[test]
    fn an_obj_gets_the_image_its_material_library_names() {
        let directory = directory("mtl");
        std::fs::write(directory.join("scan.png"), textured_png()).expect("the image");
        std::fs::write(directory.join("scan.mtl"), "newmtl scan\nmap_Kd scan.png\n")
            .expect("the material library");
        let obj =
            b"mtllib scan.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 1\nvt 1 1\nvt 0 0\nf 1/1 2/2 3/3\n";
        let path = directory.join("scan.obj");
        std::fs::write(&path, obj).expect("the obj");

        let mut mesh = triangle();
        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");

        let texture = mesh.texture().expect("the texture was found");
        assert_eq!((texture.width, texture.height), (2, 1));
        assert_eq!(texture.rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_obj_without_a_usable_library_gets_the_image_beside_it() {
        let directory = directory("same-stem");
        std::fs::write(directory.join("scan.tif"), textured_png()).expect("the image");
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let path = directory.join("scan.obj");
        std::fs::write(&path, obj).expect("the obj");

        let mut mesh = triangle();
        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");

        assert!(
            mesh.texture().is_none(),
            "JPG is not in the tried list, so nothing is attached"
        );

        std::fs::write(directory.join("scan.jpg"), textured_png()).expect("the image");
        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");
        assert!(
            mesh.texture().is_some(),
            "the image beside the mesh is used"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_ply_gets_the_image_its_header_names() {
        let directory = directory("ply");
        std::fs::write(directory.join("atlas.png"), textured_png()).expect("the image");
        let mut header = String::from(
            "ply\nformat ascii 1.0\ncomment TextureFile atlas.png\nelement vertex 3\n\
             property float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n1 0 0\n0 1 0\n",
        );
        let path = directory.join("scan.ply");
        std::fs::write(&path, &header).expect("the ply");

        let mut mesh = triangle();
        attach(&mut mesh, &path, LocateKind::Ply, header.as_bytes()).expect("companion attachment");
        assert!(mesh.texture().is_some(), "the named image is used");

        // A header with no name at all leaves the mesh alone.
        header = header.replace("comment TextureFile atlas.png\n", "");
        let mut untextured = triangle();
        attach(&mut untextured, &path, LocateKind::Ply, header.as_bytes())
            .expect("companion attachment");
        assert!(untextured.texture().is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_mesh_without_coordinates_is_not_given_an_image() {
        // Every fragment would sample one texel: the layer would render as a
        // single flat colour and lose the tint it had.
        let directory = directory("no-uvs");
        std::fs::write(directory.join("scan.png"), textured_png()).expect("the image");
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let path = directory.join("scan.obj");
        std::fs::write(&path, obj).expect("the obj");

        let mut mesh = Mesh::new(
            Some("scan".to_string()),
            vec![
                Vertex::at(glam::Vec3::ZERO),
                Vertex::at(glam::Vec3::X),
                Vertex::at(glam::Vec3::Y),
            ],
            vec![0, 1, 2],
        )
        .expect("a triangle without coordinates");
        assert!(!mesh.has_uvs());

        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");

        assert!(
            mesh.texture().is_none(),
            "an image no coordinate can sample must not be attached"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_first_library_with_an_image_wins() {
        // The first material has no image, the second one does, and the OBJ
        // names both libraries: the texture is in the second of each.
        let directory = directory("several");
        std::fs::write(directory.join("atlas.png"), textured_png()).expect("the image");
        std::fs::write(directory.join("first.mtl"), "newmtl a\nKd 1 1 1\n")
            .expect("the first library");
        std::fs::write(
            directory.join("second.mtl"),
            "newmtl b\nmap_Kd missing.png\nnewmtl c\nmap_Kd atlas.png\n",
        )
        .expect("the second library");
        let obj = b"mtllib first.mtl\nmtllib second.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 1\nvt 1 1\nvt 0 0\nf 1/1 2/2 3/3\n";
        let path = directory.join("scan.obj");
        std::fs::write(&path, obj).expect("the obj");

        let mut mesh = triangle();
        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");

        assert!(
            mesh.texture().is_some(),
            "a library later in the list still carries the image"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_image_outside_the_mesh_folder_is_refused() {
        let directory = directory("escape");
        let elsewhere = directory.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("a folder");
        std::fs::write(elsewhere.join("secret.png"), textured_png()).expect("the image");
        let obj = b"mtllib scan.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        std::fs::write(
            directory.join("scan.mtl"),
            "newmtl scan\nmap_Kd ../elsewhere/secret.png\n",
        )
        .expect("the material library");
        let path = directory.join("scan.obj");
        std::fs::write(&path, obj).expect("the obj");

        let mut mesh = triangle();
        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");

        assert!(
            mesh.texture().is_none(),
            "a line inside the mesh must not read a file outside its folder"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_or_unreadable_image_is_not_an_error() {
        let directory = directory("missing");
        let obj = b"mtllib scan.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        std::fs::write(directory.join("scan.mtl"), "newmtl scan\nmap_Kd gone.png\n")
            .expect("the material library");
        let path = directory.join("scan.obj");
        std::fs::write(&path, obj).expect("the obj");

        let mut mesh = triangle();
        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");
        assert!(mesh.texture().is_none());

        // A file that is not an image at all is refused by the decoder.
        std::fs::write(directory.join("gone.png"), b"not an image").expect("the file");
        attach(&mut mesh, &path, LocateKind::Obj, obj).expect("companion attachment");
        assert!(mesh.texture().is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_material_library_is_read_for_the_image_and_nothing_else() {
        // Options before the file name are dropped, and the name survives.
        let mtl = b"newmtl scan\nmap_Kd -s 1 1 1 -o 0 0 0 textures/atlas file.png\n";
        assert_eq!(
            directives(mtl, "map_Kd"),
            vec!["textures/atlas file.png".to_string()]
        );
        assert!(directives(b"newmtl scan\n", "map_Kd").is_empty());
        assert!(
            directives(b"mtllibfoo bar.mtl\n", "mtllib").is_empty(),
            "a keyword must not match a longer word"
        );
    }
}
