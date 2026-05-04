//! JSON renderer for `ags refresh-specs` output.

use crate::errors::CliError;
use crate::frontend::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use crate::protocol::output::{RefreshMode, RefreshSpecsOutput};

/// Render refresh-specs output as JSON.
///
/// `--quiet` is intentionally ignored: machine consumers rely on the JSON
/// payload regardless of quiet mode, and structured output should not be
/// silently suppressed in automation pipelines.
pub(crate) fn render_refresh_specs_output(
    output: &RefreshSpecsOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let status = if output.failed.is_empty() {
        "pass"
    } else {
        "fail"
    };
    let mode = match output.mode {
        RefreshMode::Single => "single",
        RefreshMode::All => "all",
    };
    let failed: Vec<serde_json::Value> = output
        .failed
        .iter()
        .map(|(service, error)| serde_json::json!({ "service": service, "error": error }))
        .collect();
    let value = serde_json::json!({
        "status": status,
        "mode": mode,
        "succeeded": output.succeeded,
        "failed": failed,
        "duration_ms": output.duration.as_millis(),
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
    use std::time::Duration;

    /// Build a default `RenderOptions` shared by every JSON render test in this module.
    fn options() -> RenderOptions {
        RenderOptions::default()
    }

    #[test]
    fn test_all_success_emits_pass_status() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::All,
            succeeded: vec!["iam".to_string(), "platform".to_string()],
            failed: vec![],
            duration: Duration::from_millis(142),
        };
        let rendered = render_refresh_specs_output(&output, &options()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["status"], "pass");
        assert_eq!(json["mode"], "all");
        assert_eq!(json["succeeded"], serde_json::json!(["iam", "platform"]));
        assert_eq!(json["failed"], serde_json::json!([]));
        assert_eq!(json["duration_ms"], 142u64);
    }

    #[test]
    fn test_partial_failure_emits_fail_status_with_detail() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::All,
            succeeded: vec!["platform".to_string()],
            failed: vec![("iam".to_string(), "parse error".to_string())],
            duration: Duration::from_millis(99),
        };
        let rendered = render_refresh_specs_output(&output, &options()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["status"], "fail");
        assert_eq!(json["failed"][0]["service"], "iam");
        assert_eq!(json["failed"][0]["error"], "parse error");
    }

    #[test]
    fn test_single_success_emits_single_mode() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::Single,
            succeeded: vec!["iam".to_string()],
            failed: vec![],
            duration: Duration::from_millis(50),
        };
        let rendered = render_refresh_specs_output(&output, &options()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["mode"], "single");
        assert_eq!(json["status"], "pass");
    }

    #[test]
    fn test_output_goes_to_stdout_not_stderr() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::All,
            succeeded: vec!["iam".to_string()],
            failed: vec![],
            duration: Duration::from_millis(10),
        };
        let rendered = render_refresh_specs_output(&output, &options()).unwrap();
        assert!(rendered.stdout.is_some());
        assert!(rendered.stderr.is_none());
    }
}
