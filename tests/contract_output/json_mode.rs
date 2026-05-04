use ags::catalogue::Catalogue;
use ags::frontend::json::render;
use ags::frontend::RenderOptions;
use ags::protocol::catalogue::OperationSchema;
use ags::protocol::output::{ApiBody, ApiOutput, CommandOutput};
use ags::protocol::result::{CommandResult, RawResult};
use serde_json::json;

/// Helper: render a JSON body in JSON mode (wraps as Raw, matching real dispatch behavior).
fn render_response(
    body: &serde_json::Value,
    operation: &OperationSchema,
    options: &RenderOptions,
    resource_name: &str,
) -> Result<String, ags::errors::CliError> {
    let raw = CommandResult::Raw(RawResult {
        value: body.clone(),
    });
    let output = CommandOutput::Service(Box::new(ApiOutput {
        operation: operation.clone(),
        resource_name: resource_name.to_string(),
        body: ApiBody::Shaped(Box::new(raw)),
        success: None,
        trace: None,
    }));
    let rendered = render(&output, options)?;
    Ok(rendered.stdout.unwrap_or_default())
}

fn json_options() -> RenderOptions {
    RenderOptions::default()
}

fn iam_operation(resource: &str, method: &str) -> OperationSchema {
    let service = Catalogue::load_bundled("iam").expect("load IAM spec");
    let resource_entry = service
        .resources
        .into_iter()
        .find(|resource_entry| resource_entry.name == resource)
        .unwrap_or_else(|| panic!("resource '{resource}' not found"));
    let op = resource_entry
        .operations()
        .find(|operation| operation.name == method)
        .cloned();
    op.unwrap_or_else(|| panic!("method '{method}' not found on '{resource}'"))
}

// ── Paginated list shape ──

/// JSON mode preserves the full paginated envelope (data + paging) so scripts can handle pagination
#[test]
fn test_json_passthrough_paginated_list() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!({
        "data": [{"userId": "u1", "displayName": "Alice"}],
        "paging": {"first": "", "next": ""}
    });

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");

    assert!(parsed["data"].is_array(), "data field preserved");
    assert_eq!(parsed["data"][0]["userId"], "u1", "nested fields preserved");
    assert!(parsed["paging"].is_object(), "paging field preserved");
}

// ── Raw array shape ──

/// Raw arrays pass through unchanged so scripts get the exact shape the API returned
#[test]
fn test_json_passthrough_array() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!([
        {"userId": "u1"},
        {"userId": "u2"}
    ]);

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");

    assert!(parsed.is_array(), "array shape preserved");
    assert_eq!(parsed.as_array().unwrap().len(), 2, "all items preserved");
}

// ── Single object shape ──

/// Single objects preserve all fields including nested structures for lossless scripting
#[test]
fn test_json_passthrough_single_object() {
    let op = iam_operation("users", "get");
    let body = json!({
        "userId": "abc-123",
        "displayName": "Jane Doe",
        "emailAddress": "jane@example.com",
        "enabled": true,
        "nested": {"key": "value"}
    });

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");

    assert_eq!(parsed["userId"], "abc-123", "top-level string preserved");
    assert_eq!(parsed["enabled"], true, "boolean preserved");
    assert_eq!(parsed["nested"]["key"], "value", "nested object preserved");
}

// ── Scalar shape ──

/// Scalar string responses pass through as valid JSON strings, not bare text
#[test]
fn test_json_passthrough_scalar_string() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!("plain string");

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");
    assert_eq!(parsed, "plain string");
}

/// Numeric scalars pass through as JSON numbers so type information is preserved
#[test]
fn test_json_passthrough_scalar_number() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!(42);

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");
    assert_eq!(parsed, 42);
}

/// Null responses pass through as JSON null instead of being silently dropped
#[test]
fn test_json_passthrough_null() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!(null);

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");
    assert!(parsed.is_null());
}

// ── Structural guarantees ──

/// JSON output is pretty-printed so it remains readable when viewed directly in a terminal
#[test]
fn test_json_output_is_pretty_printed() {
    let op = iam_operation("users", "get");
    let body = json!({"userId": "abc", "name": "Jane"});

    let result = render_response(&body, &op, &json_options(), "users").unwrap();

    assert!(
        result.contains('\n'),
        "JSON output should be pretty-printed (multi-line)"
    );
}

/// Empty objects render as `{}` so JSON consumers always get valid parseable output
#[test]
fn test_json_mode_empty_object() {
    let op = iam_operation("users", "get");
    let body = json!({});

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");
    assert!(parsed.is_object());
    assert!(parsed.as_object().unwrap().is_empty());
}

/// Empty arrays render as `[]` so JSON consumers always get valid parseable output
#[test]
fn test_json_mode_empty_array() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!([]);

    let result = render_response(&body, &op, &json_options(), "users").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("must be valid JSON");
    assert!(parsed.is_array());
    assert!(parsed.as_array().unwrap().is_empty());
}
