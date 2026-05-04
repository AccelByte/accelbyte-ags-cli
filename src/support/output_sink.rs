//! Output destination handling for rendered text and raw response bodies.
use std::io::Write;
use std::path::PathBuf;

use crate::errors::CliError;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OutputSink {
    Stdout,
    File(PathBuf),
}

impl OutputSink {
    /// Resolve the output destination from the `--output` flag.
    pub(crate) fn resolve(
        destination: Option<&crate::protocol::request::OutputDestination>,
        is_binary_body: bool,
    ) -> Result<Self, CliError> {
        resolve_output_sink(destination, is_binary_body, super::is_stdout_tty())
    }

    /// Write bytes to the resolved destination.
    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), CliError> {
        match self {
            OutputSink::Stdout => {
                let mut output = std::io::stdout().lock();
                if let Err(error) = output.write_all(bytes) {
                    map_stdout_write_error(error, "write to")?;
                }
                if let Err(error) = output.flush() {
                    map_stdout_write_error(error, "flush")?;
                }
                Ok(())
            }
            OutputSink::File(path) => {
                std::fs::write(path, bytes)
                    .map_err(anyhow::Error::from)
                    .map_err(|error| {
                        CliError::Internal(
                            error.context(format!("Cannot write to '{}'", path.display())),
                        )
                    })?;
                Ok(())
            }
        }
    }
}

/// Map stdout write failures to CLI-facing errors.
fn map_stdout_write_error(error: std::io::Error, operation: &str) -> Result<(), CliError> {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        return Ok(());
    }
    Err(CliError::Usage {
        message: format!("Cannot {operation} stdout: {error}."),
        metadata: None,
    })
}

/// Resolve an output destination with an injected TTY state.
fn resolve_output_sink(
    destination: Option<&crate::protocol::request::OutputDestination>,
    is_binary_body: bool,
    is_stdout_tty: bool,
) -> Result<OutputSink, CliError> {
    use crate::protocol::request::OutputDestination;
    match destination {
        Some(OutputDestination::Stdout) => Ok(OutputSink::Stdout),
        Some(OutputDestination::File(path)) => Ok(OutputSink::File(path.clone())),
        None => {
            if is_binary_body && is_stdout_tty {
                Err(CliError::Usage {
                    message: "Response body is binary. Use --output <path> to \
                              write it to a file, or redirect stdout."
                        .to_string(),
                    metadata: None,
                })
            } else {
                Ok(OutputSink::Stdout)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::request::OutputDestination;

    #[test]
    fn test_binary_plus_tty_plus_no_flag_errors() {
        let error = resolve_output_sink(None, true, true).unwrap_err();
        match error {
            CliError::Usage { message, .. } => {
                assert!(message.contains("binary"));
                assert!(message.contains("--output"));
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_binary_plus_non_tty_plus_no_flag_returns_stdout() {
        assert_eq!(
            resolve_output_sink(None, true, false).unwrap(),
            OutputSink::Stdout
        );
    }

    #[test]
    fn test_text_plus_tty_plus_no_flag_returns_stdout() {
        assert_eq!(
            resolve_output_sink(None, false, true).unwrap(),
            OutputSink::Stdout
        );
    }

    #[test]
    fn test_dash_is_stdout() {
        assert_eq!(
            resolve_output_sink(Some(&OutputDestination::Stdout), true, true).unwrap(),
            OutputSink::Stdout
        );
        assert_eq!(
            resolve_output_sink(Some(&OutputDestination::Stdout), false, false).unwrap(),
            OutputSink::Stdout
        );
    }

    #[test]
    fn test_path_becomes_file() {
        assert_eq!(
            resolve_output_sink(
                Some(&OutputDestination::File(PathBuf::from("foo.png"))),
                true,
                true
            )
            .unwrap(),
            OutputSink::File(PathBuf::from("foo.png"))
        );
    }

    #[test]
    fn test_binary_plus_flag_bypasses_tty_guard() {
        assert_eq!(
            resolve_output_sink(
                Some(&OutputDestination::File(PathBuf::from("foo.bin"))),
                true,
                true
            )
            .unwrap(),
            OutputSink::File(PathBuf::from("foo.bin"))
        );
        assert_eq!(
            resolve_output_sink(Some(&OutputDestination::Stdout), true, true).unwrap(),
            OutputSink::Stdout
        );
    }

    #[test]
    fn test_file_write_reports_os_error_for_missing_dir() {
        let sink = OutputSink::File(PathBuf::from("/definitely/does/not/exist/out.bin"));
        let error = sink.write(b"hello").unwrap_err();
        match error {
            CliError::Internal(inner) => {
                assert!(inner.to_string().starts_with("Cannot write to"));
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[test]
    fn test_file_write_creates_and_matches_bytes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("out.bin");
        let bytes = b"\x89PNG\r\n\x1a\n";
        OutputSink::File(path.clone()).write(bytes).unwrap();
        let read = std::fs::read(&path).unwrap();
        assert_eq!(read, bytes);
    }

    #[test]
    fn test_broken_pipe_on_write_returns_ok() {
        let broken_pipe = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        assert!(map_stdout_write_error(broken_pipe, "write to").is_ok());
    }

    #[test]
    fn test_broken_pipe_on_flush_returns_ok() {
        let broken_pipe = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        assert!(map_stdout_write_error(broken_pipe, "flush").is_ok());
    }

    #[test]
    fn test_non_broken_pipe_write_error_includes_operation_in_message() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        match map_stdout_write_error(error, "write to") {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("write to stdout")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[test]
    fn test_non_broken_pipe_flush_error_includes_operation_in_message() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        match map_stdout_write_error(error, "flush") {
            Err(CliError::Usage { message, .. }) => assert!(message.contains("flush stdout")),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[test]
    fn test_file_write_overwrites_existing_silently() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("out.bin");
        std::fs::write(&path, b"old").unwrap();
        OutputSink::File(path.clone()).write(b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }
}
