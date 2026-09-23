//! JSON `ags update --install` output rendering.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::UpdateInstallOutput;

/// Render update-install output as JSON with exactly the six documented fields.
pub(crate) fn render_update_install_output(
    output: &UpdateInstallOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    // Keys are emitted in alphabetical order (serde_json::to_value uses a
    // BTreeMap). The contract is the set of six fields, not their order.
    Ok(RenderedOutput {
        stdout: Some(format_json(&serde_json::to_value(output).map_err(
            |e| {
                CliError::Internal(anyhow::anyhow!(
                    "Failed to serialize update-install output: {e}"
                ))
            },
        )?)?),
        stderr: None,
        is_stdout_first: true,
    })
}
