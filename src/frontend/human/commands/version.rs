//! Human-readable version output rendering.

use crate::errors::CliError;
use crate::frontend::{RenderOptions, RenderedOutput};
use crate::protocol::output::VersionOutput;

/// Render version as human-readable text.
pub fn render_version_output(
    output: &VersionOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    Ok(RenderedOutput {
        stdout: Some(format!("ags {}", output.version)),
        stderr: None,
        is_stdout_first: true,
    })
}
