//! Human-readable renderer for `ags extend clone-template` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::CloneTemplateOutput;

pub(crate) fn render_clone_template_output(
    output: &CloneTemplateOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    let color = style::is_stderr_enabled();
    let mut lines = Vec::new();
    lines.push(style::success(
        &format!(
            "Cloned '{}' to {}",
            output.template_name, output.destination
        ),
        color,
    ));
    lines.push(format!(
        "{} Next: cd {}",
        style::fix_prefix(),
        output.destination,
    ));
    Ok(RenderedOutput {
        stdout: None,
        stderr: Some(lines.join("\n")),
        is_stdout_first: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_contains_template_name_and_destination() {
        let output = CloneTemplateOutput {
            template_name: "Extend Override :: Lootbox Roll :: Go".to_string(),
            destination: "/tmp/test".to_string(),
            source_path: None,
        };
        let rendered = render_clone_template_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(
            stderr.contains("Lootbox Roll"),
            "expected template name in output: {stderr}"
        );
        assert!(
            stderr.contains("/tmp/test"),
            "expected destination in output: {stderr}"
        );
        assert!(
            stderr.contains("Next:"),
            "expected next-step hint in output: {stderr}"
        );
    }

    #[test]
    fn test_quiet_returns_empty() {
        let output = CloneTemplateOutput {
            template_name: "test".to_string(),
            destination: "/tmp".to_string(),
            source_path: None,
        };
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        };
        let rendered = render_clone_template_output(&output, &options).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }
}
