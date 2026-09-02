//! Human-readable renderer for `ags extend app-ui upload` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::AppUiUploadOutput;

pub(crate) fn render_app_ui_upload_output(
    output: &AppUiUploadOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    let color = style::is_stderr_enabled();
    let size_kib = output.archive_bytes / 1024;
    let line = style::success(
        &format!(
            "Uploaded {} (version {}, {} KiB)",
            output.name, output.version, size_kib
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
    fn test_render_upload_contains_name_and_version() {
        let output = AppUiUploadOutput {
            name: "my-app".to_string(),
            version: "abc12345".to_string(),
            archive_bytes: 10240,
            response: serde_json::json!({"ok": true}),
        };
        let rendered = render_app_ui_upload_output(&output, &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(
            stderr.contains("my-app"),
            "expected app name in output: {stderr}"
        );
        assert!(
            stderr.contains("abc12345"),
            "expected version in output: {stderr}"
        );
        assert!(stderr.contains("10"), "expected size in output: {stderr}");
    }

    #[test]
    fn test_quiet_returns_empty() {
        let output = AppUiUploadOutput {
            name: "my-app".to_string(),
            version: "v1".to_string(),
            archive_bytes: 1024,
            response: serde_json::json!({}),
        };
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        };
        let rendered = render_app_ui_upload_output(&output, &options).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }
}
