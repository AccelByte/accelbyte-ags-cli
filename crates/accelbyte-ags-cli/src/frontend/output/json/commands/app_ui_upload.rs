//! JSON renderer for `ags extend app-ui upload` output.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::AppUiUploadOutput;

pub(crate) fn render_app_ui_upload_output(
    output: &AppUiUploadOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    // Surface the CSM response envelope verbatim, augmented with the
    // upload metadata so callers can correlate without re-parsing stderr.
    let value = serde_json::json!({
        "name": output.name,
        "version": output.version,
        "archive_bytes": output.archive_bytes,
        "response": output.response,
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

    #[test]
    fn test_json_envelope_contains_response() {
        let output = AppUiUploadOutput {
            name: "my-app".to_string(),
            version: "abc12345".to_string(),
            archive_bytes: 2048,
            response: serde_json::json!({"ok": true}),
        };
        let rendered = render_app_ui_upload_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["name"], "my-app");
        assert_eq!(json["version"], "abc12345");
        assert_eq!(json["archive_bytes"], 2048);
        assert_eq!(json["response"]["ok"], true);
    }
}
