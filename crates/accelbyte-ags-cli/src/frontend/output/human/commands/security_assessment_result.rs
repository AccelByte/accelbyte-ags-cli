//! Human-readable renderer for `ags extend security-assessment result` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::SecurityAssessmentResultOutput;

pub(crate) fn render_security_assessment_result_output(
    output: &SecurityAssessmentResultOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    let color = style::is_stderr_enabled();
    let mut line = style::success(
        &format!(
            "Security assessment report downloaded for '{}'.",
            output.app
        ),
        color,
    );
    line.push_str(&format!(
        "\n  Engagement ID: {}\n  Format:        {}\n  Report:        {}",
        output.engagement_id, output.report_format, output.path
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

    fn sample() -> SecurityAssessmentResultOutput {
        SecurityAssessmentResultOutput {
            namespace: "ns1".to_string(),
            app: "my-app".to_string(),
            engagement_id: 42,
            report_format: "pdf".to_string(),
            path: "my-app-42-report.pdf".to_string(),
            bytes_written: 1024,
        }
    }

    #[test]
    fn test_render_contains_success_message_and_details() {
        let rendered =
            render_security_assessment_result_output(&sample(), &RenderOptions::default()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("Security assessment report downloaded for 'my-app'."));
        assert!(stderr.contains("42"));
        assert!(stderr.contains("pdf"));
        assert!(stderr.contains("my-app-42-report.pdf"));
    }

    #[test]
    fn test_quiet_returns_empty() {
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        };
        let rendered = render_security_assessment_result_output(&sample(), &options).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }
}
