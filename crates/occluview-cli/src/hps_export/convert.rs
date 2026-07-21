use super::args::{Arguments, InputSource};
use super::error::CliError;
use super::manifest::Manifest;
use super::output;
use occluview_formats::{MeshWriteFormat, MeshWriteOptions};
use std::fs::File;
use std::io::{Read, Write};

const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) fn convert(
    arguments: Arguments,
    stdin: &mut impl Read,
    stdout: &mut impl Write,
) -> Result<(), CliError> {
    let input = read_input(&arguments.input, stdin)?;
    let surface = match occluview_formats::hps::read_decoded_surface_bytes_with_runtime_key_provider(
        &input,
    ) {
        Ok(surface) => surface,
        Err(occluview_formats::hps::HpsDecodedReadError::Parser(error)) => {
            return Err(CliError::from_parser(error));
        }
        Err(occluview_formats::hps::HpsDecodedReadError::KeyProvider(error)) => {
            return Err(CliError::from_key_provider(error));
        }
    };
    let geometry_mesh = occluview_formats::hps::geometry_mesh_from_decoded_surface(&surface)
        .map_err(|_| CliError::SurfaceConversionFailed)?;
    let preview_mesh = occluview_formats::hps::mesh_from_decoded_surface(surface)
        .map_err(|_| CliError::SurfaceConversionFailed)?;
    let textured = preview_mesh.texture().is_some();
    let mut geometry = Vec::new();
    occluview_formats::write_mesh(
        &mut geometry,
        &geometry_mesh,
        MeshWriteFormat::PlyBinaryLittleEndian,
        MeshWriteOptions::default(),
    )
    .map_err(|_| CliError::ArtifactEncodingFailed)?;
    let preview = textured
        .then(|| occluview_formats::write_textured_glb(&preview_mesh))
        .transpose()
        .map_err(|_| CliError::ArtifactEncodingFailed)?;
    let manifest = Manifest::new(&geometry, preview.as_deref());
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|_| CliError::ManifestEncodingFailed)?;
    manifest_bytes.push(b'\n');

    output::write_artifacts(
        &arguments.output_dir,
        &geometry,
        preview.as_deref(),
        &manifest_bytes,
    )?;
    stdout
        .write_all(&manifest_bytes)
        .map_err(|_| CliError::StdoutWriteFailed)
}

fn read_input(source: &InputSource, stdin: &mut impl Read) -> Result<Vec<u8>, CliError> {
    match source {
        InputSource::File(path) => {
            let mut file = File::open(path).map_err(|_| CliError::InputReadFailed)?;
            read_limited(&mut file)
        }
        InputSource::Stdin => read_limited(stdin),
    }
}

fn read_limited(reader: &mut impl Read) -> Result<Vec<u8>, CliError> {
    read_limited_with_max(reader, MAX_INPUT_BYTES)
}

fn read_limited_with_max(reader: &mut impl Read, max_bytes: u64) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::InputReadFailed)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(CliError::InputTooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::super::error::CliError;
    use super::read_limited_with_max;
    use std::io::Cursor;

    #[test]
    fn bounded_reader_accepts_limit_and_rejects_one_byte_over() {
        let accepted = read_limited_with_max(&mut Cursor::new(b"1234"), 4);
        assert_eq!(accepted, Ok(b"1234".to_vec()));

        let rejected = read_limited_with_max(&mut Cursor::new(b"12345"), 4);
        assert_eq!(rejected, Err(CliError::InputTooLarge));
    }
}
