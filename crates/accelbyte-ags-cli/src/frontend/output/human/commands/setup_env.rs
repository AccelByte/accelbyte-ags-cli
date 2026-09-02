//! Human-readable renderer for `ags extend app-ui setup-env` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::{SetupEnvOutput, SetupEnvStatus};

pub(crate) fn render_setup_env_output(
    output: &SetupEnvOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    let color = style::is_stderr_enabled();
    let line = match output.status {
        SetupEnvStatus::Written => style::success(&format!("Wrote {}", output.env_path), color),
        SetupEnvStatus::Skipped => {
            let msg = style::warning(
                &format!(
                    "Skipped {} (already exists). Use --force to overwrite.",
                    output.env_path
                ),
                color,
            );
            return Ok(RenderedOutput {
                stdout: None,
                stderr: Some(msg),
                is_stdout_first: false,
            });
        }
    };
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
    fn test_render_written_contains_path() {
        let output = SetupEnvOutput {
            status: SetupEnvStatus::Written,
            env_path: "/tmp/project/.env.local".to_string(),
        };
        let rendered = render_setup_env_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(
            stderr.contains(".env.local"),
            "expected env path in output: {stderr}"
        );
    }

    #[test]
    fn test_render_skipped_contains_path_and_force_hint() {
        let output = SetupEnvOutput {
            status: SetupEnvStatus::Skipped,
            env_path: "/tmp/project/.env.local".to_string(),
        };
        let rendered = render_setup_env_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(
            stderr.contains(".env.local"),
            "expected env path in output: {stderr}"
        );
        assert!(
            stderr.contains("already exists"),
            "expected 'already exists' in output: {stderr}"
        );
        assert!(
            stderr.contains("--force"),
            "expected '--force' hint in skip output: {stderr}"
        );
    }

    #[test]
    fn test_quiet_returns_empty() {
        let output = SetupEnvOutput {
            status: SetupEnvStatus::Written,
            env_path: "/tmp/.env.local".to_string(),
        };
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        };
        let rendered = render_setup_env_output(&output, &options).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }
}
