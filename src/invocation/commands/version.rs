//! Version command handler.

use crate::errors::CliError;
use crate::frontend;
use crate::invocation::flags::GlobalFlags;
use crate::protocol::output::{CommandOutput, VersionOutput};

/// Handle the `version` command.
pub fn handle_version(
    flags: &GlobalFlags,
    frontend: &mut dyn frontend::Frontend,
) -> Result<(), CliError> {
    let output = CommandOutput::Version(VersionOutput {
        version: env!("CARGO_PKG_VERSION").to_string(),
    });
    frontend.render(&output, &frontend::RenderOptions::from(flags))
}
