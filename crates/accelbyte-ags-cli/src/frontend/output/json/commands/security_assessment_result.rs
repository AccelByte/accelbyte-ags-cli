//! JSON renderer for `ags extend security-assessment result` output.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::SecurityAssessmentResultOutput;

pub(crate) fn render_security_assessment_result_output(
    output: &SecurityAssessmentResultOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let value = serde_json::json!({
        "namespace": output.namespace,
        "app": output.app,
        "engagementId": output.engagement_id,
        "reportFormat": output.report_format,
        "path": output.path,
        "bytesWritten": output.bytes_written,
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
    fn test_json_envelope_contains_all_fields() {
        let output = SecurityAssessmentResultOutput {
            namespace: "ns1".to_string(),
            app: "my-app".to_string(),
            engagement_id: 42,
            report_format: "pdf".to_string(),
            path: "my-app-42-report.pdf".to_string(),
            bytes_written: 1024,
        };
        let rendered =
            render_security_assessment_result_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["namespace"], "ns1");
        assert_eq!(json["app"], "my-app");
        assert_eq!(json["engagementId"], 42);
        assert_eq!(json["reportFormat"], "pdf");
        assert_eq!(json["path"], "my-app-42-report.pdf");
        assert_eq!(json["bytesWritten"], 1024);
    }
}
