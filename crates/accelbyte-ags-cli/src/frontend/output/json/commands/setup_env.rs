//! JSON renderer for `ags extend app-ui setup-env` output.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::SetupEnvOutput;

pub(crate) fn render_setup_env_output(
    output: &SetupEnvOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let value = serde_json::json!({
        "status": output.status,
        "env_path": output.env_path,
    });
    Ok(RenderedOutput {
        stdout: Some(format_json(&value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::output::SetupEnvStatus;

    #[test]
    fn test_json_written_status() {
        let output = SetupEnvOutput {
            status: SetupEnvStatus::Written,
            env_path: "/tmp/project/.env.local".to_string(),
        };
        let rendered = render_setup_env_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["status"], "written");
        assert_eq!(json["env_path"], "/tmp/project/.env.local");
    }

    #[test]
    fn test_json_skipped_status() {
        let output = SetupEnvOutput {
            status: SetupEnvStatus::Skipped,
            env_path: "/tmp/project/.env.local".to_string(),
        };
        let rendered = render_setup_env_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["status"], "skipped");
        assert_eq!(json["env_path"], "/tmp/project/.env.local");
    }
}
