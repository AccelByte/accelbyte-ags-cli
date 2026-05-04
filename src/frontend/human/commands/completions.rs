//! Human-readable completions output: raw script to stdout, optional
//! detection hint to stderr.

use crate::errors::CliError;
use crate::frontend::{RenderOptions, RenderedOutput};
use crate::protocol::output::CompletionsOutput;

/// Render a completions script for human terminals: shell script on stdout, optional install hint on stderr.
pub fn render_completions_output(
    output: &CompletionsOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    Ok(RenderedOutput {
        stdout: Some(output.script.clone()),
        stderr: output.hint.clone(),
        is_stdout_first: false,
    })
}
