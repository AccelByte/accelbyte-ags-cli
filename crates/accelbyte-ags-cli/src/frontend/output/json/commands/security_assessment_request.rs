//! JSON renderer for `ags extend security-assessment request` output.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::SecurityAssessmentRequestOutput;

pub(crate) fn render_security_assessment_request_output(
    output: &SecurityAssessmentRequestOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let value = serde_json::json!({
        "namespace": output.namespace,
        "app": output.app,
        "engagementId": output.engagement_id,
        "status": output.status,
        "endpointCount": output.endpoint_count,
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
        let output = SecurityAssessmentRequestOutput {
            namespace: "ns1".to_string(),
            app: "my-app".to_string(),
            engagement_id: 42,
            status: "SUBMITTED".to_string(),
            endpoint_count: 3,
        };
        let rendered =
            render_security_assessment_request_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["namespace"], "ns1");
        assert_eq!(json["app"], "my-app");
        assert_eq!(json["engagementId"], 42);
        assert_eq!(json["status"], "SUBMITTED");
        assert_eq!(json["endpointCount"], 3);
    }
}
