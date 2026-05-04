use ags::catalogue::Catalogue;
use ags::frontend::RenderOptions;
use ags::protocol::catalogue::OperationSchema;
use ags::protocol::output::{ApiBody, ApiOutput, CommandOutput};
use ags::protocol::result::{CommandResult, RawResult};
use ags::runtime::dispatch::shape::shape_response;
use serde_json::json;

/// Helper: render a JSON body through shape + render, returning the stdout string.
/// Matches real dispatch behavior: JSON mode wraps as Raw, human mode shapes.
fn render_response(
    body: &serde_json::Value,
    operation: &OperationSchema,
    options: &RenderOptions,
    resource_name: &str,
    is_json: bool,
) -> Result<String, ags::errors::CliError> {
    let result = if is_json {
        CommandResult::Raw(RawResult {
            value: body.clone(),
        })
    } else {
        shape_response(body, operation, resource_name, false)
    };
    let output = CommandOutput::Service(Box::new(ApiOutput {
        operation: operation.clone(),
        resource_name: resource_name.to_string(),
        body: ApiBody::Shaped(Box::new(result)),
        success: None,
        trace: None,
    }));
    let rendered = if is_json {
        ags::frontend::json::render(&output, options)?
    } else {
        ags::frontend::human::render(&output, options)?
    };
    Ok(rendered.stdout.unwrap_or_default())
}

/// Helper: find an operation by resource + method name from the parsed IAM spec.
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

// ── Single object (inspect) ──

/// Single-object responses must render as human-readable key-value output, not raw JSON, for terminal usability
#[test]
fn test_render_single_user_response() {
    let operation = iam_operation("users", "get");
    let options = RenderOptions::default();

    let body = json!({
        "userId": "abc-123",
        "displayName": "Jane Doe",
        "emailAddress": "jane@example.com",
        "namespace": "my-game",
        "enabled": true,
        "emailVerified": true,
        "createdAt": "2024-01-15T10:30:00Z"
    });

    let result = render_response(&body, &operation, &options, "users", false).unwrap();

    // Should produce human-readable output with key fields
    assert!(result.contains("Jane Doe"), "Should show display name");
    assert!(
        result.contains("abc-123") || result.contains("User ID"),
        "Should show user ID somewhere"
    );
    // Should NOT be raw JSON (no opening brace at start)
    assert!(
        !result.trim().starts_with('{'),
        "Human-readable mode should not produce raw JSON"
    );
}

// ── Paginated list ──

/// Paginated lists must display all items and a count so users can tell at a glance how many results were returned
#[test]
fn test_render_paginated_user_list() {
    let operation = iam_operation("users", "list-users-with-accelbyte-account");
    let options = RenderOptions::default();

    let body = json!({
        "data": [
            {"userId": "u1", "displayName": "Alice", "emailAddress": "alice@test.com", "namespace": "ns"},
            {"userId": "u2", "displayName": "Bob", "emailAddress": "bob@test.com", "namespace": "ns"}
        ],
        "paging": {"first": "", "last": "", "next": "", "previous": ""}
    });

    let result = render_response(&body, &operation, &options, "users", false).unwrap();

    assert!(result.contains("Alice"), "Should show first user");
    assert!(result.contains("Bob"), "Should show second user");
    assert!(result.contains('2'), "Should show item count");
}

// ── Empty list ──

/// Empty lists must show a "No ... found" message rather than blank output, so users know the query succeeded
#[test]
fn test_render_empty_list() {
    let operation = iam_operation("users", "list-users-with-accelbyte-account");
    let options = RenderOptions::default();

    let body = json!({"data": [], "paging": {}});
    let result = render_response(&body, &operation, &options, "users", false).unwrap();

    assert!(
        result.to_lowercase().contains("no "),
        "Empty list should show 'No ... found'"
    );
}

// ── JSON passthrough ──

/// --format json must pass the API response through as valid JSON for piping into jq or other tools
#[test]
fn test_render_json_format_passthrough() {
    let operation = iam_operation("users", "get");
    let options = RenderOptions::default();

    let body = json!({"userId": "abc-123", "displayName": "Jane"});
    let result = render_response(&body, &operation, &options, "users", true).unwrap();

    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("Should be valid JSON");
    assert_eq!(parsed["userId"], "abc-123");
}

// ── Action response (POST/PUT/DELETE) ──

/// Mutation responses (POST/PUT/DELETE) must produce non-empty output so users get confirmation the action succeeded
#[test]
fn test_render_action_response() {
    let operation = iam_operation("bans", "bulk-ban-users");
    let options = RenderOptions::default();

    let body = json!({
        "userId": "banned-user",
        "banId": "ban-001",
        "reason": "cheating"
    });

    let result = render_response(&body, &operation, &options, "bans", false).unwrap();

    // Action responses should show a success-style message
    assert!(!result.is_empty(), "Action response should produce output");
}
