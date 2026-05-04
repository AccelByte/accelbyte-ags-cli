//! JSON renderer for `ags doctor` output.

use serde_json::Value;

use crate::errors::CliError;
use crate::frontend::json::format_json;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::diagnostics::{CheckResult, CheckStatus, DoctorReport, DoctorResult};

/// Render doctor output as JSON
pub(crate) fn render_doctor_output(
    output: &DoctorResult,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    Ok(RenderedOutput {
        stdout: Some(render_doctor_json(output)?),
        stderr: None,
        is_stdout_first: true,
    })
}

/// Serialise the multi-profile doctor result as the top-level JSON envelope.
fn render_doctor_json(output: &DoctorResult) -> Result<String, CliError> {
    let mut json_object = serde_json::Map::new();
    let (fails, warns, _) = count_all(output);
    json_object.insert(
        "status".to_string(),
        Value::String(overall_status(fails, warns).to_string()),
    );
    let profiles: Vec<Value> = output
        .reports
        .iter()
        .map(render_doctor_report_json)
        .collect();
    json_object.insert("profiles".to_string(), Value::Array(profiles));
    format_json(&Value::Object(json_object))
}

/// Serialise a single profile's doctor report as a JSON object with status, counts, and check details.
fn render_doctor_report_json(report: &DoctorReport) -> Value {
    let mut json_object = serde_json::Map::new();
    let (fails, warns, skips) = count_issues(&report.checks);

    json_object.insert(
        "status".to_string(),
        Value::String(overall_status(fails, warns).to_string()),
    );
    json_object.insert("profile".to_string(), Value::String(report.profile.clone()));
    json_object.insert("warnings".to_string(), Value::Number(warns.into()));
    json_object.insert("errors".to_string(), Value::Number(fails.into()));
    json_object.insert("skipped".to_string(), Value::Number(skips.into()));

    let checks: Vec<Value> = report
        .checks
        .iter()
        .map(|c| serde_json::to_value(c).expect("CheckResult serialisation is infallible"))
        .collect();
    json_object.insert("checks".to_string(), Value::Array(checks));

    Value::Object(json_object)
}

/// Count fails, warnings, and skips within a single profile's checks.
fn count_issues(checks: &[CheckResult]) -> (usize, usize, usize) {
    let fails = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Fail)
        .count();
    let warns = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Warning)
        .count();
    let skips = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Skipped)
        .count();
    (fails, warns, skips)
}

/// Sum fails, warnings, and skips across every profile in the doctor result.
fn count_all(output: &DoctorResult) -> (usize, usize, usize) {
    let mut fails = 0;
    let mut warns = 0;
    let mut skips = 0;
    for report in &output.reports {
        let (f, w, s) = count_issues(&report.checks);
        fails += f;
        warns += w;
        skips += s;
    }
    (fails, warns, skips)
}

/// Reduce fail and warning counts to the worst-of-three status string for the JSON envelope.
fn overall_status(fails: usize, warns: usize) -> &'static str {
    if fails > 0 {
        "fail"
    } else if warns > 0 {
        "warning"
    } else {
        "pass"
    }
}

#[cfg(test)]
mod tests {
    use super::overall_status;

    #[test]
    fn test_overall_status_reflects_worst_result() {
        assert_eq!(overall_status(1, 0), "fail");
        assert_eq!(overall_status(1, 2), "fail");
        assert_eq!(overall_status(0, 1), "warning");
        assert_eq!(overall_status(0, 0), "pass");
    }
}
