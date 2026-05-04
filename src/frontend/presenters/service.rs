//! Shared service presentation helpers.

use crate::protocol::output::ApiBody;
use crate::protocol::result::{CommandResult, DryRunResult};

/// Format-neutral dry-run request view shared by human and JSON renderers.
#[derive(Debug, Clone)]
pub(crate) struct DryRunRequestView {
    pub http_method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
}

/// Build a shared dry-run request view.
pub(crate) fn present_dry_run(report: &DryRunResult) -> DryRunRequestView {
    DryRunRequestView {
        http_method: report.http_method.as_str().to_string(),
        url: report.url.clone(),
        headers: report.headers.clone(),
        query: report.query.clone(),
        body: report.body.clone(),
    }
}

/// Convert a shaped `CommandResult` into the JSON value that JSON mode should emit.
pub(crate) fn command_result_json_value(result: &CommandResult) -> Option<serde_json::Value> {
    match result {
        CommandResult::Raw(raw) => Some(raw.value.clone()),
        CommandResult::Empty(_) => None,
        CommandResult::Entity(entity) => Some(
            serde_json::to_value(entity).expect("serializing EntityResult should be infallible"),
        ),
        CommandResult::Collection(collection) => Some(
            serde_json::to_value(collection)
                .expect("serializing CollectionResult should be infallible"),
        ),
    }
}

/// Convert an API body into the JSON value that JSON mode should emit.
pub(crate) fn api_body_json_value(body: &ApiBody) -> Option<serde_json::Value> {
    match body {
        ApiBody::Shaped(result) => command_result_json_value(result),
        ApiBody::Text(body) => Some(serde_json::Value::String(body.clone())),
        ApiBody::Empty => None,
    }
}
