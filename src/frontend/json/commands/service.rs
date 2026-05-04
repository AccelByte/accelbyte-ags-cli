//! JSON rendering for API service responses.
//!
//! The dispatch layer shapes the server body as `CommandResult::Raw(value)`
//! in JSON mode, so the renderer's common case is just running that value
//! through `format_json`. The other `CommandResult` variants are covered
//! defensively in case the dispatch layer's shape ever diverges.

use crate::errors::CliError;
use crate::frontend::presenters::service as service_presenter;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::output::ApiOutput;
use crate::protocol::result::DryRunResult;

/// Render API output as JSON. Does not emit trace or success messages to
/// stderr — those are human-decorative and would contaminate automation
/// pipelines consuming `--format json`.
pub(crate) fn render_api_output(
    output: &ApiOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let stdout = service_presenter::api_body_json_value(&output.body)
        .map(|value| crate::frontend::json::format_json(&value))
        .transpose()?;

    Ok(RenderedOutput {
        stdout,
        stderr: None,
        ..Default::default()
    })
}

/// Render a dry-run report as a JSON envelope.
pub(crate) fn render_dry_run_output(report: &DryRunResult) -> Result<RenderedOutput, CliError> {
    let request_view = service_presenter::present_dry_run(report);
    let headers: serde_json::Map<String, serde_json::Value> = request_view
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    let query: serde_json::Map<String, serde_json::Value> = request_view
        .query
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    let value = serde_json::json!({
        "method": request_view.http_method,
        "url": request_view.url,
        "headers": headers,
        "query": query,
        "body": request_view.body,
    });
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::json::format_json(&value)?),
        stderr: None,
        is_stdout_first: true,
    })
}
