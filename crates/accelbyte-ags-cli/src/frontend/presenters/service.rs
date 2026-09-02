//! Shared service presentation helpers.

use ags_protocol::output::ApiBody;
use ags_protocol::result::{CommandResult, DryRunResult};

/// Format-neutral dry-run request view shared by human and JSON renderers.
#[derive(Debug, Clone)]
pub(crate) struct DryRunRequestView {
    pub http_method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
    pub body: Option<ags_protocol::request::RequestBody>,
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

/// Render a `RequestBody` as a single human-readable line per part, for
/// `--dry-run` preview output. A JSON body renders as pretty-printed JSON
/// (unchanged from today); a multipart body renders one line per part —
/// `name: value` for text, `name: <file path> (N bytes)` for a file (the
/// file is never read here; only its already-known local path is shown).
pub(crate) fn render_dry_run_body(body: &ags_protocol::request::RequestBody) -> String {
    use ags_protocol::request::{FormPart, RequestBody};
    match body {
        RequestBody::Json(value) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        RequestBody::Multipart(parts) => parts
            .iter()
            .map(|part| match part {
                FormPart::Text { name, value } => format!("{name}: {value}"),
                FormPart::File { name, path, .. } => format!("{name}: {}", path.display()),
            })
            .collect::<Vec<_>>()
            .join("\n"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::catalogue::HttpMethod;
    use ags_protocol::request::{FormPart, RequestBody};

    /// `present_dry_run` passes a `RequestBody::Multipart` value through
    /// unchanged (rendering decisions happen at the human/JSON output layer,
    /// not here).
    #[test]
    fn test_present_dry_run_carries_multipart_body_through() {
        let report = DryRunResult {
            http_method: HttpMethod::Post,
            url: "https://example.test/upload".to_string(),
            headers: vec![],
            query: vec![],
            body: Some(RequestBody::Multipart(vec![FormPart::Text {
                name: "strategy".to_string(),
                value: "REPLACE".to_string(),
            }])),
        };
        let view = present_dry_run(&report);
        assert!(matches!(view.body, Some(RequestBody::Multipart(_))));
    }
}
