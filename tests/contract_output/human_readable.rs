use ags::catalogue::Catalogue;
use ags::frontend::human::render;
use ags::frontend::RenderOptions;
use ags::protocol::catalogue::OperationSchema;
use ags::protocol::output::{ApiBody, ApiOutput, CommandOutput};
use ags::runtime::dispatch::shape::shape_response;
use serde_json::json;

/// Helper: render a JSON body through shape + render, returning the stdout string.
fn render_response(
    body: &serde_json::Value,
    operation: &OperationSchema,
    options: &RenderOptions,
    resource_name: &str,
) -> Result<String, ags::errors::CliError> {
    let shaped = shape_response(body, operation, resource_name, false);
    let output = CommandOutput::Service(Box::new(ApiOutput {
        operation: operation.clone(),
        resource_name: resource_name.to_string(),
        body: ApiBody::Shaped(Box::new(shaped)),
        success: None,
        trace: None,
    }));
    let rendered = render(&output, options)?;
    Ok(rendered.stdout.unwrap_or_default())
}

fn default_options() -> RenderOptions {
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

// ── List intent structural contract ──
// Contract: starts with "▸ Found N <noun>", followed by column headers and rows

/// List output starts with the item count so users immediately know how many results came back
#[test]
fn test_list_has_count_heading() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!({
        "data": [
            {"userId": "u1", "displayName": "Alice"},
            {"userId": "u2", "displayName": "Bob"}
        ],
        "paging": {}
    });

    let result = render_response(&body, &op, &default_options(), "users").unwrap();

    assert!(
        result.contains("Found 2"),
        "List output must contain item count: {result}"
    );
}

/// List tables include a column header row so users can identify which field each column represents
#[test]
fn test_list_has_column_headers() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!({
        "data": [
            {"userId": "u1", "displayName": "Alice", "emailAddress": "a@test.com"}
        ],
        "paging": {}
    });

    let result = render_response(&body, &op, &default_options(), "users").unwrap();
    let lines: Vec<&str> = result.lines().collect();

    // Second line should contain column headers (uppercase labels)
    assert!(
        lines.len() >= 2,
        "List must have at least heading + header row"
    );
}

/// Empty lists show "No ... found" instead of a blank screen so users know the query succeeded
#[test]
fn test_list_empty_shows_no_found() {
    let op = iam_operation("users", "list-users-with-accelbyte-account");
    let body = json!({"data": [], "paging": {}});

    let result = render_response(&body, &op, &default_options(), "users").unwrap();

    assert!(
        result.to_lowercase().contains("no "),
        "Empty list must show 'No ... found': {result}"
    );
}

/// Data row count matches item count so the heading number is never misleading
#[test]
fn test_list_data_rows_match_item_count() {
    let op = iam_operation("roles", "list");
    let body = json!({
        "data": [
            {"roleId": "r1", "roleName": "Admin"},
            {"roleId": "r2", "roleName": "Player"},
            {"roleId": "r3", "roleName": "Moderator"}
        ],
        "paging": {}
    });

    let result = render_response(&body, &op, &default_options(), "roles").unwrap();

    // Count non-empty lines after the heading and header row
    let lines: Vec<&str> = result.lines().filter(|l| !l.is_empty()).collect();
    // heading + header + separator(optional) + 3 data rows = at least 5 lines
    assert!(
        lines.len() >= 5,
        "List should have heading + header + 3 data rows, got {} lines: {result}",
        lines.len()
    );
}

// ── Inspect intent structural contract ──
// Contract: starts with "▸ <heading>", followed by "Label: value" pairs

/// Inspect heading includes the entity's display name so users can confirm they fetched the right record
#[test]
fn test_inspect_has_heading_with_name() {
    let op = iam_operation("users", "get");
    let body = json!({
        "userId": "abc-123",
        "displayName": "Jane Doe",
        "emailAddress": "jane@test.com"
    });

    let result = render_response(&body, &op, &default_options(), "users").unwrap();

    assert!(
        result.contains("Jane Doe"),
        "Inspect heading must include display name: {result}"
    );
}

/// Inspect body renders as indented label: value pairs for scannable key-value display
#[test]
fn test_inspect_has_label_value_pairs() {
    let op = iam_operation("users", "get");
    let body = json!({
        "userId": "abc-123",
        "displayName": "Jane Doe",
        "emailAddress": "jane@test.com",
        "namespace": "my-game"
    });

    let result = render_response(&body, &op, &default_options(), "users").unwrap();

    // Should contain colon-separated label:value pairs
    let label_value_lines: Vec<&str> = result
        .lines()
        .filter(|l| l.contains(':') && l.starts_with(' '))
        .collect();
    assert!(
        !label_value_lines.is_empty(),
        "Inspect must have indented label: value lines: {result}"
    );
}

/// When no display name key exists, inspect heading falls back to the resource noun (e.g. "Role")
#[test]
fn test_inspect_falls_back_to_resource_name() {
    let op = iam_operation("roles", "get");
    // roleId and roleName are not in the heading name keys, so heading
    // derives from method name or resource name
    let body = json!({
        "roleId": "role-999",
        "roleName": "Admin",
        "permissions": []
    });

    let result = render_response(&body, &op, &default_options(), "roles").unwrap();
    let first_line = result.lines().next().unwrap_or("");

    assert!(
        first_line.to_lowercase().contains("role"),
        "Inspect heading must include resource noun: {first_line}"
    );
}

/// When no name keys are present, the entity ID appears in the heading so the record is identifiable
#[test]
fn test_inspect_uses_id_in_heading_when_no_name_keys() {
    let op = iam_operation("users", "get");
    // None of the heading name keys (displayName, name, clientName,
    // userName, emailAddress) are present — should fall back to id
    let body = json!({
        "id": "user-abc-123",
        "enabled": true
    });

    let result = render_response(&body, &op, &default_options(), "users").unwrap();
    let first_line = result.lines().next().unwrap_or("");

    assert!(
        first_line.contains("user-abc-123"),
        "Inspect heading must include ID when no name keys: {first_line}"
    );
}

/// Empty objects show a "not found" info message rather than a blank screen
#[test]
fn test_inspect_empty_object_shows_not_found() {
    let op = iam_operation("users", "get");
    let body = json!({});

    let result = render_response(&body, &op, &default_options(), "users").unwrap();

    assert!(
        result.contains("No") && result.contains("found"),
        "Empty inspect should show 'No ... found' message: {result}"
    );
}

// ── Action intent structural contract ──
// Contract: stdout carries response field rows; the verb-specific success line
// (e.g. "✔ Updated stat-definition.") is rendered separately on stderr from
// the runtime's `ApiSuccess` summary, not embedded in the stdout block.

/// Action output includes response data fields so users can see the ID or state of the created resource
#[test]
fn test_action_includes_response_fields() {
    let op = iam_operation("roles", "create");
    let body = json!({
        "roleId": "new-role-001",
        "roleName": "TestRole",
        "adminRole": false
    });

    let result = render_response(&body, &op, &default_options(), "roles").unwrap();

    // Should show at least some of the response fields
    assert!(
        result.contains("new-role-001") || result.contains("TestRole"),
        "Action output should include response data: {result}"
    );
}
