#![cfg_attr(test, allow(clippy::expect_used))]

use anyhow::{bail, Context, Result};
use occluview_core::{fill_holes_in_mesh, MeshEditOptions, MeshEditReport};
use occluview_formats::dispatch::read_file_with_key_provider;
use occluview_formats::hps::RuntimeHpsKeyProvider;
use occluview_formats::write::{write_mesh_overwrite, MeshWriteFormat, MeshWriteOptions};
use std::path::Path;

/// Generous edge ceiling for whole-mesh Close Holes, mirroring the app button
/// (`app_layer_edits::whole_mesh`): the mm perimeter slider does the real
/// limiting, so the edge count is only a safety valve under the ear-clip's rim
/// limit.
// One number, owned by the kernel: a private copy here would silently pin the
// ceiling, because the selection gate takes the maximum of the two.
use occluview_core::CLOSE_HOLES_EDGE_CEILING;

/// Load a mesh (STL loads as a triangle soup), run the whole-mesh Close Holes
/// pipeline — exactly the app's button path — and write the closed result.
///
/// Returns the honest edit report so the caller can print counts.
pub(crate) fn close_holes_file(
    input: &Path,
    output: &Path,
    limit_mm: Option<f32>,
) -> Result<MeshEditReport> {
    let mesh = read_file_with_key_provider(input, &RuntimeHpsKeyProvider)
        .with_context(|| format!("loading {}", input.display()))?;
    let format = ExportFormat::from_output_path(output)?;

    // Mirror `app_layer_edits::whole_mesh::close_holes_options`: heal the cut
    // line (which also welds the STL soup back to real topology), compact the
    // welded-away duplicates, and honor the optional mm perimeter budget.
    let options = MeshEditOptions {
        compact_vertices: true,
        max_boundary_loop: CLOSE_HOLES_EDGE_CEILING,
        max_rim_perimeter_mm: limit_mm,
        heal_boundary_rims: true,
        ..MeshEditOptions::default()
    };
    let result =
        fill_holes_in_mesh(&mesh, None, options).with_context(|| "closing holes".to_string())?;

    write_mesh_overwrite(
        output,
        &result.mesh,
        format.mesh_write_format(),
        MeshWriteOptions::default(),
    )
    .with_context(|| format!("writing {}", output.display()))?;
    Ok(result.report)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExportFormat {
    Obj,
    Ply,
    Stl,
}

impl ExportFormat {
    pub(crate) fn from_output_path(path: &Path) -> Result<Self> {
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            bail!(
                "convert: output path {} has no extension; use .stl, .ply, or .obj",
                path.display()
            );
        };

        match extension.to_ascii_lowercase().as_str() {
            "obj" => Ok(Self::Obj),
            "ply" => Ok(Self::Ply),
            "stl" => Ok(Self::Stl),
            other => bail!("convert: unsupported output format .{other}; use .stl, .ply, or .obj"),
        }
    }

    fn mesh_write_format(self) -> MeshWriteFormat {
        match self {
            Self::Obj => MeshWriteFormat::Obj,
            Self::Ply => MeshWriteFormat::PlyBinaryLittleEndian,
            Self::Stl => MeshWriteFormat::StlBinary,
        }
    }
}

pub(crate) fn convert_file(input: &Path, output: &Path) -> Result<ExportFormat> {
    let mesh = read_file_with_key_provider(input, &RuntimeHpsKeyProvider)
        .with_context(|| format!("loading {}", input.display()))?;
    let format = ExportFormat::from_output_path(output)?;
    let _report = write_mesh_overwrite(
        output,
        &mesh,
        format.mesh_write_format(),
        MeshWriteOptions::default(),
    )
    .with_context(|| format!("writing {}", output.display()))?;
    Ok(format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{Mesh, Vertex};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn output_extension_maps_to_supported_formats() {
        assert_eq!(
            ExportFormat::from_output_path(Path::new("scan.obj")).expect("obj"),
            ExportFormat::Obj
        );
        assert_eq!(
            ExportFormat::from_output_path(Path::new("scan.ply")).expect("ply"),
            ExportFormat::Ply
        );
        assert_eq!(
            ExportFormat::from_output_path(Path::new("scan.stl")).expect("stl"),
            ExportFormat::Stl
        );
        let message = ExportFormat::from_output_path(Path::new("scan.glb"))
            .expect_err("glb export should not be supported yet")
            .to_string();
        assert!(message.contains("unsupported output format"));
    }

    #[test]
    fn convert_file_routes_through_writer_format_mapping() {
        let input = temp_file("obj");
        let output = temp_file("ply");
        fs::write(
            &input,
            "o sample\n\
             v 0 0 0 255 0 0\n\
             v 1 0 0 0 255 0\n\
             v 0 1 0 0 0 255\n\
             vt 0 0\n\
             vt 1 0\n\
             vt 0 1\n\
             vn 0 0 1\n\
             vn 0 0 1\n\
             vn 0 0 1\n\
             f 1/1/1 2/2/2 3/3/3\n",
        )
        .expect("seed input obj");

        let format = convert_file(&input, &output).expect("convert");
        assert_eq!(format, ExportFormat::Ply);
        assert!(output.exists());
        let _ = fs::remove_file(input);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn stl_export_rejects_point_clouds() {
        let path = temp_file("stl");
        let mesh = Mesh::point_cloud(
            Some("cloud".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
            ],
        );

        let error = write_mesh_overwrite(
            &path,
            &mesh,
            MeshWriteFormat::StlBinary,
            MeshWriteOptions::default(),
        )
        .expect_err("point cloud to stl");
        assert!(error.to_string().contains("triangle mesh"));
        let _ = fs::remove_file(path);
    }

    fn temp_file(extension: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("occluview-export-{unique}.{extension}"))
    }
}
