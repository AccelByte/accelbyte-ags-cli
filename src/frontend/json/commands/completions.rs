//! JSON completions output: `{"script": "...", "hint": "..." | null}`.
//!
//! Unlike human mode, the detection hint is embedded in the JSON body
//! rather than written to stderr — JSON consumers should read the `hint`
//! field, not the stderr stream.

use crate::errors::CliError;
use crate::frontend::{RenderOptions, RenderedOutput};
use crate::protocol::output::CompletionsOutput;

/// Emit the completions script and detection hint as a JSON envelope on stdout.
pub(crate) fn render_completions_output(
    output: &CompletionsOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let body = serde_json::json!({
        "script": output.script,
        "hint": output.hint,
    });
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::json::format_json(&body)?),
        stderr: None,
        is_stdout_first: true,
    })
}
