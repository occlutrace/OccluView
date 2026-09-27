use super::error::CliError;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub(crate) const HELP: &str = "Usage: occluview-hps-export --input <FILE|-> --output-dir <DIR>\n\
\n\
Always writes surface.ply; textured input also writes surface.glb. Writes manifest.json.\n\
Use --input - to read from standard input. HPS keys are read only from the environment.\n";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InputSource {
    File(PathBuf),
    Stdin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Arguments {
    pub(crate) input: InputSource,
    pub(crate) output_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ParseOutcome {
    Convert(Arguments),
    Help,
    Version,
}

pub(crate) fn parse(arguments: Vec<OsString>) -> Result<ParseOutcome, CliError> {
    // `-h` as well as `--help`: the main CLI accepts both, and the two tools
    // accept the same help flags.
    if arguments.len() == 1
        && (arguments[0] == OsStr::new("--help") || arguments[0] == OsStr::new("-h"))
    {
        return Ok(ParseOutcome::Help);
    }
    if arguments.len() == 1 && arguments[0] == OsStr::new("--version") {
        return Ok(ParseOutcome::Version);
    }

    let mut input = None;
    let mut output_dir = None;
    let mut arguments = arguments.into_iter();
    while let Some(option) = arguments.next() {
        if option == OsStr::new("--input") {
            if input.is_some() {
                return Err(CliError::invalid_arguments());
            }
            let value = arguments.next().ok_or_else(CliError::invalid_arguments)?;
            if value.is_empty() {
                return Err(CliError::invalid_arguments());
            }
            input = Some(if value == OsStr::new("-") {
                InputSource::Stdin
            } else {
                InputSource::File(PathBuf::from(value))
            });
        } else if option == OsStr::new("--output-dir") {
            if output_dir.is_some() {
                return Err(CliError::invalid_arguments());
            }
            let value = arguments.next().ok_or_else(CliError::invalid_arguments)?;
            if value.is_empty() {
                return Err(CliError::invalid_arguments());
            }
            output_dir = Some(PathBuf::from(value));
        } else {
            return Err(CliError::invalid_arguments());
        }
    }

    Ok(ParseOutcome::Convert(Arguments {
        input: input.ok_or_else(CliError::invalid_arguments)?,
        output_dir: output_dir.ok_or_else(CliError::invalid_arguments)?,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::{parse, InputSource, ParseOutcome};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn values(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_file_input_and_explicit_output_directory() {
        let parsed = parse(values(&[
            "--input",
            "private/source.dcm",
            "--output-dir",
            "safe-output",
        ]))
        .expect("valid arguments");
        let ParseOutcome::Convert(arguments) = parsed else {
            panic!("expected conversion arguments");
        };

        assert_eq!(
            arguments.input,
            InputSource::File(PathBuf::from("private/source.dcm"))
        );
        assert_eq!(arguments.output_dir, PathBuf::from("safe-output"));
    }

    #[test]
    fn dash_selects_stdin() {
        let parsed = parse(values(&["--input", "-", "--output-dir", "safe-output"]))
            .expect("valid stdin arguments");
        let ParseOutcome::Convert(arguments) = parsed else {
            panic!("expected conversion arguments");
        };

        assert_eq!(arguments.input, InputSource::Stdin);
    }

    #[test]
    fn help_and_version_are_non_conversion_outcomes() {
        assert_eq!(
            parse(values(&["--help"])).expect("help"),
            ParseOutcome::Help
        );
        assert_eq!(
            parse(values(&["--version"])).expect("version"),
            ParseOutcome::Version
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_arguments_without_retaining_values() {
        for invalid in [
            values(&["--input", "scan.dcm"]),
            values(&["--output-dir", "out"]),
            values(&[
                "--input",
                "one.dcm",
                "--input",
                "two.dcm",
                "--output-dir",
                "out",
            ]),
            values(&["--secret-key", "must-not-be-retained"]),
            values(&["--input", "", "--output-dir", "out"]),
            values(&["--input", "scan.dcm", "--output-dir", ""]),
        ] {
            let error = parse(invalid).expect_err("arguments must be rejected");
            assert_eq!(error.code(), "invalid_arguments");
        }
    }
}
