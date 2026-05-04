//! JSON version output rendering.

use crate::errors::CliError;
use crate::frontend::{RenderOptions, RenderedOutput};
use crate::protocol::output::VersionOutput;

/// Render version as JSON.
pub(crate) fn render_version_output(
    output: &VersionOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::json::format_json(
            &serde_json::json!({ "version": output.version }),
        )?),
        stderr: None,
        is_stdout_first: true,
    })
}
