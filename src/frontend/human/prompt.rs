//! Interactive confirmation prompts for mutating operations.

use crate::errors::CliError;
use crate::frontend::style;
use crate::protocol::result::CommandPreview;

/// Display the command preview on stderr and read a yes/no answer from stdin.
pub fn confirm(preview: &CommandPreview) -> Result<bool, CliError> {
    let color_enabled = style::is_stderr_enabled();
    crate::frontend::write_stderr_line(&style::warning(&preview.summary, color_enabled));
    crate::frontend::write_stderr_line(&format!(
        "    {}",
        style::apply_tone(
            &format!("Detail: {}", preview.url),
            style::Tone::Dim,
            color_enabled,
        ),
    ));
    crate::frontend::write_stderr("Continue? [y/N] ");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|error| CliError::Usage {
            message: format!("Failed to read input: {error}"),
            metadata: None,
        })?;
    if input.trim().eq_ignore_ascii_case("y") {
        Ok(true)
    } else {
        crate::frontend::write_stderr_line("Aborted.");
        Ok(false)
    }
}
