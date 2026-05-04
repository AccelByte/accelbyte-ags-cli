//! JSON output rendering.

pub mod commands;
pub mod frontend;
pub mod progress;

use serde_json::Value;

use crate::errors::CliError;
use crate::frontend::{FrontendKind, RenderOptions, RenderedOutput};
use crate::protocol::output::CommandOutput;

/// Pretty-print a JSON value, returning a formatted string.
pub fn format_json(value: &Value) -> Result<String, CliError> {
    serde_json::to_string_pretty(value)
        .map_err(|error| CliError::Internal(anyhow::anyhow!("Failed to format JSON: {error}")))
}

/// Render a `CommandOutput` using the JSON frontend.
///
/// This is a thin wrapper over the shared frontend dispatch in
/// [`crate::frontend::render_output`].
pub fn render(output: &CommandOutput, options: &RenderOptions) -> Result<RenderedOutput, CliError> {
    crate::frontend::render_output(FrontendKind::Json, output, options)
}
