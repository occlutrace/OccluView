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
//! in [`crate::dispatch`], and it never fails the import — a scan whose texture
//! is missing is still a scan.

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
pub(crate) fn attach(mesh: &mut Mesh, path: &Path, kind: LocateKind, bytes: &[u8]) {
    // A mesh with no texture coordinates samples one texel for every fragment,
    // so an image attached to one paints the whole layer a single flat colour
    // and takes the tint with it. Better no texture than that.
    if mesh.texture().is_some() || !mesh.has_uvs() {
        return;
    }
    let Some(image) = locate(path, kind, bytes) else {
        return;
    };
    // Bounded like every other import read: an image that grows between the
    // metadata check and the read is refused rather than pulled into memory.
    let Ok(image_bytes) =
        crate::dispatch::read_file_bytes_with_limit(&image, MAX_COMPANION_IMAGE_BYTES)
            .map(|bytes| bytes.as_slice().to_vec())
    else {
        return;
    };
    let Ok(texture) = decode_embedded_raster(&image_bytes, "texture beside the mesh") else {
        return;
    };
    mesh.set_texture(texture);
}

/// The image a mesh file names, or the one that shares its name.
fn locate(path: &Path, kind: LocateKind, bytes: &[u8]) -> Option<PathBuf> {
    // A relative path from a command line has an empty parent, and the files
    // it names are in the working directory.
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    match kind {
        LocateKind::Obj => {
            material_image(directory, bytes).or_else(|| same_stem_image(path, directory))
        }
        LocateKind::Ply => {
            let header = crate::ply::header::parse(bytes).ok()?;
            header
                .texture
                .file
                .as_deref()
                .and_then(|name| inside(directory, name))
        }
        LocateKind::None => None,
    }
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

/// The first `map_Kd` image that exists, over the libraries the OBJ names.
///
/// An OBJ may name several libraries, and a library may carry several
/// materials of which only some have an image. Stopping at the first value
/// found loses the texture whenever that value is the one without a file.
fn material_image(directory: &Path, obj: &[u8]) -> Option<PathBuf> {
    for library in directives(obj, "mtllib") {
        let Some(library) = inside(directory, &library) else {
            continue;
        };
        // The library is read through the same bounded helper as everything
        // else this import touches.
        let Ok(text) =
            crate::dispatch::read_file_bytes_with_limit(&library, MAX_MATERIAL_LIBRARY_BYTES)
                .map(|bytes| bytes.as_slice().to_vec())
        else {
            continue;
        };
        for image in directives(&text, "map_Kd") {
            if let Some(found) = inside(directory, &image) {
                return Some(found);
            }
        }
    }
    None
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
/// The MTL specification gives each option a fixed number of arguments, and a
/// file name is whatever is left — which is how a name containing spaces
/// survives. An unknown option is dropped on its own: guessing how many
/// arguments it takes would eat the file name.
fn strip_options(tokens: &mut Vec<&str>) {
    loop {
        let Some(option) = tokens.first().copied() else {
            return;
        };
        if !option.starts_with('-') {
            return;
        }
        tokens.remove(0);
        let arguments = match option {
            "-s" | "-o" | "-t" => 3,
            "-mm" => 2,
            "-bm" | "-clamp" | "-blendu" | "-blendv" | "-texres" | "-imfchan" | "-type"
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

/// Resolve a name the file mentions against the folder the file is in.
///
/// A name that climbs out of that folder is refused. The mesh came from
/// somewhere — a lab partner, a download — and a line inside it must not be
/// able to read an arbitrary file from the operator's disk, whatever the
/// program that wrote it intended.
fn inside(directory: &Path, name: &str) -> Option<PathBuf> {
    // Windows separators in a file written on Windows.
    let name = name.replace('\\', "/");
    let candidate = directory.join(name);
    let directory = directory.canonicalize().ok()?;
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
    found.into_iter().next()
}

/// Errors from a companion are not errors of the import: the mesh is what the
/// operator opened, and a texture that could not be read is reported by the
/// format's own loss warning when it is written back out.
#[allow(dead_code)]
fn _error_type_is_shared(_: FormatError) {}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{MeshTexture, Vertex};

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
        attach(&mut mesh, &path, LocateKind::Obj, obj);

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
        attach(&mut mesh, &path, LocateKind::Obj, obj);

        assert!(
            mesh.texture().is_none(),
            "JPG is not in the tried list, so nothing is attached"
        );

        std::fs::write(directory.join("scan.jpg"), textured_png()).expect("the image");
        attach(&mut mesh, &path, LocateKind::Obj, obj);
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
        attach(&mut mesh, &path, LocateKind::Ply, header.as_bytes());
        assert!(mesh.texture().is_some(), "the named image is used");

        // A header with no name at all leaves the mesh alone.
        header = header.replace("comment TextureFile atlas.png\n", "");
        let mut untextured = triangle();
        attach(&mut untextured, &path, LocateKind::Ply, header.as_bytes());
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

        attach(&mut mesh, &path, LocateKind::Obj, obj);

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
        attach(&mut mesh, &path, LocateKind::Obj, obj);

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
        attach(&mut mesh, &path, LocateKind::Obj, obj);

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
        attach(&mut mesh, &path, LocateKind::Obj, obj);
        assert!(mesh.texture().is_none());

        // A file that is not an image at all is refused by the decoder.
        std::fs::write(directory.join("gone.png"), b"not an image").expect("the file");
        attach(&mut mesh, &path, LocateKind::Obj, obj);
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
