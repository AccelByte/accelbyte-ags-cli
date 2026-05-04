//! Human-readable output rendering.

pub mod commands;
pub mod frontend;
pub mod progress;
pub mod prompt;
pub mod templates;

use crate::errors::CliError;
use crate::frontend::{FrontendKind, RenderOptions, RenderedOutput};
use crate::protocol::output::CommandOutput;

/// Render a `CommandOutput` using the human-readable frontend.
///
/// This is a thin wrapper over the shared frontend dispatch in
/// [`crate::frontend::render_output`].
pub fn render(output: &CommandOutput, options: &RenderOptions) -> Result<RenderedOutput, CliError> {
    crate::frontend::render_output(FrontendKind::Human, output, options)
}
