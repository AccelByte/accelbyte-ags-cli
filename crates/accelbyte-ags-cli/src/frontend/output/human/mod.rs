//! Human-readable output rendering.

pub mod commands;
pub mod templates;

use crate::errors::CliError;
use crate::frontend::{RenderFormat, RenderOptions, RenderedOutput};
use ags_protocol::output::CommandOutput;

/// Render a `CommandOutput` using the human-readable frontend.
///
/// This is a thin wrapper over the shared frontend dispatch in
/// [`crate::frontend::render_output`].
pub fn render(output: &CommandOutput, options: &RenderOptions) -> Result<RenderedOutput, CliError> {
    crate::frontend::render_output(RenderFormat::Human, output, options)
}
