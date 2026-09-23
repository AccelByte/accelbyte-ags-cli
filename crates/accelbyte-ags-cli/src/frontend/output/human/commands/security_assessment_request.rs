//! Human-readable renderer for `ags extend security-assessment request` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::SecurityAssessmentRequestOutput;

pub(crate) fn render_security_assessment_request_output(
    output: &SecurityAssessmentRequestOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    let color = style::is_stderr_enabled();
    // Verbatim from the Admin Portal's `pentest.request.toast.success`.
    let mut line = style::success(
        &format!("Security assessment requested for '{}'.", output.app),
        color,
    );
    line.push_str(&format!(
        "\n  Engagement ID: {}\n  Status:        {}\n  Endpoints:     {}",
        output.engagement_id, output.status, output.endpoint_count
    ));
    Ok(RenderedOutput {
        stdout: None,
        stderr: Some(line),
        is_stdout_first: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SecurityAssessmentRequestOutput {
        SecurityAssessmentRequestOutput {
            namespace: "ns1".to_string(),
            app: "my-app".to_string(),
            engagement_id: 42,
            status: "SUBMITTED".to_string(),
            endpoint_count: 3,
        }
    }

    #[test]
    fn test_render_contains_success_message_and_details() {
        let rendered =
            render_security_assessment_request_output(&sample(), &RenderOptions::default())
                .unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("Security assessment requested for 'my-app'."));
        assert!(stderr.contains("42"));
        assert!(stderr.contains("SUBMITTED"));
        assert!(stderr.contains('3'));
    }

    #[test]
    fn test_quiet_returns_empty() {
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        };
        let rendered = render_security_assessment_request_output(&sample(), &options).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }
}
