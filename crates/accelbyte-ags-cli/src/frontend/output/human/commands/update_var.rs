//! Human-readable renderer for `ags extend update-var` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::UpdateVarOutput;

pub(crate) fn render_update_var_output(
    output: &UpdateVarOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    let color = style::is_stderr_enabled();
    let verb = if output.created { "Created" } else { "Updated" };
    let line = style::success(
        &format!(
            "{} variable '{}' (configId={})",
            verb, output.config_name, output.config_id
        ),
        color,
    );
    Ok(RenderedOutput {
        stdout: None,
        stderr: Some(line),
        is_stdout_first: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_contains_config_name_and_id() {
        let output = UpdateVarOutput {
            config_id: "id-1".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: false,
            description: None,
            created: false,
        };
        let rendered = render_update_var_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("MY_KEY"));
        assert!(stderr.contains("id-1"));
        assert!(stderr.contains("Updated"));
    }

    #[test]
    fn test_render_created_says_created() {
        let output = UpdateVarOutput {
            config_id: "new-id".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: true,
            description: Some("desc".to_string()),
            created: true,
        };
        let rendered = render_update_var_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(
            stderr.contains("Created"),
            "create path must say Created, got: {stderr}"
        );
        assert!(!stderr.contains("Updated variable"));
    }

    #[test]
    fn test_quiet_returns_empty() {
        let output = UpdateVarOutput {
            config_id: "id-1".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: false,
            description: None,
            created: false,
        };
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        };
        let rendered = render_update_var_output(&output, &options).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }
}
