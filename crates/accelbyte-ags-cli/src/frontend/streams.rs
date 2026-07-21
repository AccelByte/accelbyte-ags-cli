//! UiSink: stderr-only helper for ad-hoc UI chrome writes (spinners,
//! prompts, banners, hint lines, transition messages).
//!
//! Stdout / result emission is NOT covered by this module — results must
//! always route through `emit_with_options` (`crate::frontend::mod`) so
//! `--output <path>` is honoured. There is intentionally no `ResultSink`.

use std::io::Write;

pub struct UiSink;

impl UiSink {
    /// Write raw bytes to stderr and flush.
    pub fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut handle = std::io::stderr().lock();
        handle.write_all(bytes)?;
        handle.flush()
    }

    /// Write `line` to stderr followed by a newline.
    pub fn write_line(&self, line: &str) -> std::io::Result<()> {
        self.write_all(line.as_bytes())?;
        self.write_all(b"\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_sink_writes_line_with_newline() {
        // Structural smoke test — verifies the API compiles and accepts
        // the inputs we expect. Real stderr capture happens in subprocess
        // integration tests (Task 5.4).
        let sink = UiSink;
        let _ = sink.write_line("hello");
    }
}
