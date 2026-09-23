//! JSON update output rendering.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::UpdateOutput;

/// Render update output as JSON with exactly the 8 documented fields.
pub(crate) fn render_update_output(
    output: &UpdateOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    // Keys are emitted in alphabetical order like every other JSON output
    // of the CLI. The contract is the set of eight fields, not their order.
    Ok(RenderedOutput {
        stdout: Some(format_json(&serde_json::json!({
            "current": output.current,
            "latest": output.latest,
            "update_available": output.update_available,
            "install_method": output.install_method.to_string(),
            "binary_path": output.binary_path,
            "upgrade_command": output.upgrade_command,
            "download_archive": output.download_archive,
            "release_url": output.release_url,
        }))?),
        stderr: None,
        is_stdout_first: true,
    })
}
