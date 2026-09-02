//! Human-readable renderer for `ags extend update-secret` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::UpdateSecretOutput;

pub(crate) fn render_update_secret_output(
    output: &UpdateSecretOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    let color = style::is_stderr_enabled();
    let verb = if output.created { "Created" } else { "Updated" };
    let line = style::success(
        &format!(
            "{} secret '{}' (configId={})",
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
    fn test_render_says_updated_on_update_path() {
        let output = UpdateSecretOutput {
            config_id: "id-1".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: false,
            description: None,
            created: false,
        };
        let rendered = render_update_secret_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("MY_KEY"));
        assert!(stderr.contains("id-1"));
        assert!(stderr.contains("Updated"));
    }

    #[test]
    fn test_render_says_created_on_create_path() {
        let output = UpdateSecretOutput {
            config_id: "new-id".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: true,
            description: Some("desc".to_string()),
            created: true,
        };
        let rendered = render_update_secret_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("Created"), "got: {stderr}");
        assert!(!stderr.contains("Updated secret"));
    }

    #[test]
    fn test_quiet_returns_empty() {
        let output = UpdateSecretOutput {
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
        let rendered = render_update_secret_output(&output, &options).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }
}
