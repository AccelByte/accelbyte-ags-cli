//! Human-readable renderer for `ags refresh-specs` output.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use crate::protocol::output::{RefreshMode, RefreshSpecsOutput};
use crate::support::strings::pluralize;

/// Render refresh-specs output as human-readable text.
pub(crate) fn render_refresh_specs_output(
    output: &RefreshSpecsOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    if options.verbosity.is_quiet() {
        return Ok(RenderedOutput::default());
    }
    debug_assert!(
        !output.succeeded.is_empty() || !output.failed.is_empty(),
        "RefreshSpecsOutput must have at least one succeeded or failed entry"
    );
    let color = style::is_stderr_enabled();
    let mut lines: Vec<String> = Vec::new();

    for (service, error) in &output.failed {
        lines.push(format!("{service}: {error}"));
    }

    if output.failed.is_empty() {
        let success_message = match output.mode {
            RefreshMode::Single => format!(
                "Refreshed {} in {}",
                output.succeeded.first().map(String::as_str).unwrap_or(""),
                format_duration(output.duration)
            ),
            RefreshMode::All => format!(
                "Refreshed {} {} in {}",
                output.succeeded.len(),
                pluralize("service", output.succeeded.len()),
                format_duration(output.duration)
            ),
        };
        lines.push(style::success(&success_message, color));
    } else if !output.succeeded.is_empty() {
        // Single mode returns CliError on failure, so partial failure is All-only.
        debug_assert_eq!(
            output.mode,
            RefreshMode::All,
            "Single mode cannot produce partial failures"
        );
        lines.push(format!(
            "Refreshed {} {} in {}",
            output.succeeded.len(),
            pluralize("service", output.succeeded.len()),
            format_duration(output.duration)
        ));
    }

    Ok(RenderedOutput {
        stdout: None,
        stderr: Some(lines.join("\n")),
        is_stdout_first: false,
    })
}

/// Format a duration as `Nms` under one second, otherwise `N.Ns`, for refresh-spec timing lines.
fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Build a `RenderOptions` with quiet mode enabled for the suppression tests.
    fn quiet() -> RenderOptions {
        RenderOptions {
            verbosity: crate::protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        }
    }

    /// Build a default `RenderOptions` for the standard-output rendering tests.
    fn normal() -> RenderOptions {
        RenderOptions::default()
    }

    #[test]
    fn test_quiet_returns_empty_output() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::All,
            succeeded: vec!["iam".to_string()],
            failed: vec![],
            duration: Duration::from_millis(100),
        };
        let rendered = render_refresh_specs_output(&output, &quiet()).unwrap();
        assert!(rendered.stdout.is_none());
        assert!(rendered.stderr.is_none());
    }

    #[test]
    fn test_all_success_goes_to_stderr_with_count_and_duration() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::All,
            succeeded: vec!["iam".to_string(), "platform".to_string()],
            failed: vec![],
            duration: Duration::from_millis(142),
        };
        let rendered = render_refresh_specs_output(&output, &normal()).unwrap();
        assert!(rendered.stdout.is_none());
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("Refreshed 2 services"), "got: {stderr}");
        assert!(stderr.contains("142ms"), "got: {stderr}");
    }

    #[test]
    fn test_single_success_names_service_and_duration() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::Single,
            succeeded: vec!["iam".to_string()],
            failed: vec![],
            duration: Duration::from_millis(50),
        };
        let rendered = render_refresh_specs_output(&output, &normal()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("Refreshed iam"), "got: {stderr}");
        assert!(stderr.contains("50ms"), "got: {stderr}");
    }

    #[test]
    fn test_partial_failure_lists_errors_before_success_count() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::All,
            succeeded: vec!["platform".to_string()],
            failed: vec![("iam".to_string(), "parse error".to_string())],
            duration: Duration::from_millis(200),
        };
        let rendered = render_refresh_specs_output(&output, &normal()).unwrap();
        let stderr = rendered.stderr.unwrap();
        let iam_pos = stderr.find("iam: parse error").expect("missing error line");
        let count_pos = stderr
            .find("Refreshed 1 service")
            .expect("missing count line");
        assert!(
            iam_pos < count_pos,
            "failure lines should precede success count"
        );
    }

    #[test]
    fn test_duration_over_one_second_formats_as_seconds() {
        let output = RefreshSpecsOutput {
            mode: RefreshMode::All,
            succeeded: vec!["iam".to_string()],
            failed: vec![],
            duration: Duration::from_millis(1500),
        };
        let rendered = render_refresh_specs_output(&output, &normal()).unwrap();
        let stderr = rendered.stderr.unwrap();
        assert!(stderr.contains("1.5s"), "got: {stderr}");
    }
}
