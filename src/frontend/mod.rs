//! Presentation layer: format and render all CLI output.

pub mod human;
pub mod json;
mod presenters;
mod render;
pub mod style;
pub mod templates;

use crate::errors::CliError;
use crate::protocol::catalogue::OperationSchema;
use crate::protocol::event::ProgressSink;
use crate::protocol::output::CommandOutput;
use crate::protocol::result::CommandPreview;

/// Pagination metadata for display in list output.
#[derive(Debug, Clone)]
pub struct PaginationHint {
    pub total: Option<u64>,
    pub has_next: bool,
}

/// Frontend family to render with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendKind {
    Human,
    Json,
}

/// Final rendered text ready for emission to stdout and/or stderr
#[derive(Debug, Clone, Default)]
pub struct RenderedOutput {
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    /// When true, print stdout before stderr (e.g. status headline first).
    /// When false (default), print stderr before stdout (e.g. verbose trace first).
    pub is_stdout_first: bool,
}

/// Emit rendered output honouring the caller's `--output` flag. When
/// `output` is `Some(path)` or `Some("-")`, the stdout portion goes
/// through `OutputSink` instead of `println!`. The stderr portion
/// always goes to stderr (spinners, confirmation lines, warnings).
pub fn emit_with_options(
    rendered: RenderedOutput,
    options: &RenderOptions,
) -> Result<(), crate::errors::CliError> {
    // stderr is unaffected by --output — always goes to real stderr.
    // stdout goes to OutputSink when --output is set.
    if rendered.is_stdout_first {
        if let Some(stdout) = rendered.stdout.filter(|s| !s.is_empty()) {
            write_stdout(&stdout, options)?;
        }
        if let Some(stderr) = rendered.stderr.filter(|s| !s.is_empty()) {
            write_stderr_line(&stderr);
        }
    } else {
        if let Some(stderr) = rendered.stderr.filter(|s| !s.is_empty()) {
            write_stderr_line(&stderr);
        }
        if let Some(stdout) = rendered.stdout.filter(|s| !s.is_empty()) {
            write_stdout(&stdout, options)?;
        }
    }
    Ok(())
}

pub(crate) use render::render_output;

/// Route stdout text either to the console or to the `--output` destination resolved by `OutputSink`.
fn write_stdout(text: &str, options: &RenderOptions) -> Result<(), crate::errors::CliError> {
    use crate::support::output_sink::OutputSink;

    match options.output.as_ref() {
        // Common case: no --output flag. Writes go through anstream::stdout(),
        // which enables Windows VT mode on first use and surfaces a closed
        // pipe as ErrorKind::BrokenPipe instead of panicking. On Unix,
        // reset_sigpipe() still drives the SIGPIPE-based exit-141 path.
        None => write_stdout_line(text),
        Some(destination) => {
            let sink = OutputSink::resolve(Some(destination), false)?;
            // Append a trailing newline to match println! semantics — users
            // expect text files to end with a newline.
            //
            // This is intentionally asymmetric with the binary --output path
            // in runtime::dispatch, which writes raw bytes verbatim.
            let mut bytes = text.as_bytes().to_vec();
            if !bytes.ends_with(b"\n") {
                bytes.push(b'\n');
            }
            sink.write(&bytes)
        }
    }
}

/// The frontend abstraction: renders output and errors to the appropriate medium.
pub trait Frontend {
    /// Render a command's structured output to stdout/stderr in this frontend's format.
    fn render(&mut self, output: &CommandOutput, options: &RenderOptions) -> Result<(), CliError>;
    /// Render a fatal error in this frontend's format and signal the caller via exit code.
    fn render_error(&mut self, err: &CliError);
    /// Emit a non-fatal warning as a stderr annotation (e.g. flag-conflict hints).
    /// JSON frontends drop these to keep automation pipelines clean.
    fn render_warning(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>);
    /// Emit the verbose resolution trace shown for `--dry-run --verbose`.
    /// JSON frontends drop these to keep automation pipelines clean.
    fn render_resolution_trace(&mut self, trace: &crate::protocol::output::ResolutionTrace);
    /// Ask the user to confirm a mutating operation; return whether to proceed.
    fn confirm(&mut self, preview: &CommandPreview) -> Result<bool, CliError>;
    /// Hand the runtime a sink for streaming progress events while a command executes.
    fn progress_sink(&mut self) -> &mut dyn ProgressSink;
    /// Collect a JSON request body from the user (interactive editor or stdin).
    #[allow(dead_code)]
    fn gather_body(&mut self, operation: &OperationSchema) -> Result<serde_json::Value, CliError>;
    /// Run any frontend setup needed before the first render call.
    fn enter(&mut self) -> Result<(), CliError> {
        Ok(())
    }
    /// Run any frontend teardown after the final render call.
    fn exit(&mut self) -> Result<(), CliError> {
        Ok(())
    }
}

/// Construct the appropriate frontend based on the requested frontend kind.
pub fn select(
    kind: FrontendKind,
    verbosity: crate::protocol::request::Verbosity,
) -> Box<dyn Frontend> {
    match kind {
        FrontendKind::Json => Box::new(crate::frontend::json::frontend::JsonFrontend::new()),
        FrontendKind::Human => Box::new(crate::frontend::human::frontend::HumanFrontend::new(
            verbosity,
        )),
    }
}

/// Render-layer options extracted from global CLI flags.
///
/// Only the fields the render layer actually needs — keeps the render module
/// independent of invocation types. Format is not here: each frontend
/// already embodies its own format.
#[derive(Debug, Default)]
pub struct RenderOptions {
    pub verbosity: crate::protocol::request::Verbosity,
    pub is_page_all: bool,
    pub output: Option<crate::protocol::request::OutputDestination>,
}

/// Internal stdout helper parameterised by writer for unit testing.
/// Translates `BrokenPipe` to `Ok(())` (matches Unix SIGPIPE-driven exit
/// semantics — a downstream consumer closing the pipe is a stop-signal,
/// not an error). All other I/O errors are mapped to `CliError::Usage`.
/// Flushes after a successful write so downstream consumers see the
/// bytes without depending on platform-specific stream-buffering
/// behaviour. On a `BrokenPipe` from either `writeln!` or the trailing
/// `flush`, the helper returns `Ok(())` without retrying — the stream
/// is broken and any further write would also fail.
fn write_stdout_line_into<W: std::io::Write>(
    mut w: W,
    text: &str,
) -> Result<(), crate::errors::CliError> {
    match writeln!(w, "{text}").and_then(|_| w.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(crate::errors::CliError::Usage {
            message: format!("Cannot write to stdout: {e}."),
            metadata: None,
        }),
    }
}

/// Internal stderr helper parameterised by writer for unit testing.
/// Stderr writes are best-effort: every I/O error (including `BrokenPipe`)
/// is silently swallowed so the program never panics or surfaces a stderr
/// failure to the user. Always flushes after writing so output appears
/// promptly even when stderr is fully or line-buffered.
fn write_stderr_line_into<W: std::io::Write>(mut w: W, text: &str) {
    let _ = writeln!(w, "{text}").and_then(|_| w.flush());
}

/// Internal no-newline stderr helper parameterised by writer for unit testing.
/// Used for inline prompts (e.g. "Continue? [y/N] ") where a trailing newline
/// would break the input flow. Always flushes after writing so the prompt
/// is visible before the program blocks on stdin. Same best-effort error
/// policy as the line variant.
fn write_stderr_into<W: std::io::Write>(mut w: W, text: &str) {
    let _ = w.write_all(text.as_bytes()).and_then(|_| w.flush());
}

/// Write `text` followed by a newline to stdout via `anstream::stdout()`.
/// On Windows, `anstream` enables the console's virtual terminal mode on
/// first use, so escape codes produced upstream render correctly.
pub(crate) fn write_stdout_line(text: &str) -> Result<(), crate::errors::CliError> {
    write_stdout_line_into(anstream::stdout().lock(), text)
}

/// Write `text` followed by a newline to stderr via `anstream::stderr()`.
/// Errors are silently ignored — stderr is best-effort.
pub(crate) fn write_stderr_line(text: &str) {
    write_stderr_line_into(anstream::stderr().lock(), text);
}

/// Write `text` to stderr without a trailing newline. Used for inline
/// prompts where the next character on the line is user input.
pub(crate) fn write_stderr(text: &str) {
    write_stderr_into(anstream::stderr().lock(), text);
}

#[cfg(test)]
mod tests {
    use super::{write_stderr_into, write_stderr_line_into, write_stdout_line_into};
    use std::io::{self, Write};

    /// A `Write` that records bytes, tracks flush calls, and optionally returns
    /// a configured error from `write`.
    struct Mock {
        buf: Vec<u8>,
        err: Option<io::ErrorKind>,
        flushed: usize,
    }

    impl Mock {
        /// Build a Mock writer that accepts every write — used for the success-path tests.
        fn ok() -> Self {
            Self {
                buf: Vec::new(),
                err: None,
                flushed: 0,
            }
        }

        /// Build a Mock writer that fails every write with the given `io::ErrorKind`.
        fn failing(kind: io::ErrorKind) -> Self {
            Self {
                buf: Vec::new(),
                err: Some(kind),
                flushed: 0,
            }
        }
    }

    impl Write for Mock {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if let Some(kind) = self.err {
                return Err(io::Error::from(kind));
            }
            self.buf.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed += 1;
            Ok(())
        }
    }

    #[test]
    fn test_write_stdout_line_into_writes_text_with_trailing_newline() {
        let mut m = Mock::ok();
        write_stdout_line_into(&mut m, "hello").unwrap();
        assert_eq!(m.buf, b"hello\n");
    }

    #[test]
    fn test_write_stdout_line_into_returns_ok_on_broken_pipe() {
        let mut m = Mock::failing(io::ErrorKind::BrokenPipe);
        assert!(write_stdout_line_into(&mut m, "x").is_ok());
    }

    #[test]
    fn test_write_stdout_line_into_returns_clierror_on_other_error() {
        let mut m = Mock::failing(io::ErrorKind::Other);
        let err = write_stdout_line_into(&mut m, "x").unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(message.contains("stdout"));
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_write_stderr_line_into_does_not_panic_on_broken_pipe() {
        let mut m = Mock::failing(io::ErrorKind::BrokenPipe);
        write_stderr_line_into(&mut m, "x"); // returns ()
    }

    #[test]
    fn test_write_stderr_line_into_does_not_panic_on_other_error() {
        let mut m = Mock::failing(io::ErrorKind::Other);
        write_stderr_line_into(&mut m, "x"); // returns ()
    }

    #[test]
    fn test_write_stderr_into_writes_text_without_trailing_newline() {
        let mut m = Mock::ok();
        write_stderr_into(&mut m, "Continue? [y/N] ");
        assert_eq!(m.buf, b"Continue? [y/N] ");
    }

    #[test]
    fn test_write_stderr_into_does_not_panic_on_broken_pipe() {
        let mut m = Mock::failing(io::ErrorKind::BrokenPipe);
        write_stderr_into(&mut m, "x"); // returns ()
    }

    #[test]
    fn test_write_stdout_line_into_flushes_after_write() {
        let mut m = Mock::ok();
        write_stdout_line_into(&mut m, "hello").unwrap();
        assert_eq!(m.flushed, 1);
    }

    #[test]
    fn test_write_stderr_line_into_flushes_after_write() {
        let mut m = Mock::ok();
        write_stderr_line_into(&mut m, "hello");
        assert_eq!(m.flushed, 1);
    }

    #[test]
    fn test_write_stderr_into_flushes_after_write() {
        let mut m = Mock::ok();
        write_stderr_into(&mut m, "Continue? [y/N] ");
        assert_eq!(m.flushed, 1);
    }
}
