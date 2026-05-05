use ags::frontend::human::templates::render_error_text as render_error;
use ags::runtime::dispatch::classify::classify_to_runtime_error;
use serde_json::json;

use crate::common::error_helpers::{error_detail, error_reason, error_suggestion_kind, error_tip};

/// Known error codes produce curated messages instead of raw API text, so users get actionable guidance
#[test]
fn test_known_error_code_classifies_and_renders() {
    let body = json!({
        "errorCode": 20013,
        "errorMessage": "insufficient permissions"
    });

    let error = classify_to_runtime_error(403, &body, "iam", "users");
    let rendered = render_error(
        &error.message,
        error_reason(&error),
        error_detail(&error),
        error.hint.as_deref(),
        error_suggestion_kind(&error),
        error_tip(&error),
        false,
    );

    // Should contain the curated message from the mapping, not the raw errorMessage
    assert!(
        rendered.contains('\u{2715}'),
        "Should have error symbol: {rendered}"
    );
    // Should contain the error code detail
    assert!(
        rendered.contains("20013"),
        "Should show error code in detail: {rendered}"
    );
}

/// Unmapped error codes must still produce useful output by falling back to HTTP status classification
#[test]
fn test_unknown_error_code_falls_through_to_http_status() {
    let body = json!({
        "errorCode": 99999,
        "errorMessage": "something unexpected"
    });

    let error = classify_to_runtime_error(500, &body, "iam", "users");
    let rendered = render_error(
        &error.message,
        error_reason(&error),
        error_detail(&error),
        error.hint.as_deref(),
        error_suggestion_kind(&error),
        error_tip(&error),
        false,
    );

    assert!(
        rendered.contains('\u{2715}'),
        "Should have error symbol: {rendered}"
    );
}

/// 401 errors must suggest re-authentication so users know how to recover from expired tokens
#[test]
fn test_401_suggests_relogin() {
    let body = json!({
        "errorCode": 0,
        "errorMessage": "token expired"
    });

    let error = classify_to_runtime_error(401, &body, "iam", "users");
    let rendered = render_error(
        &error.message,
        error_reason(&error),
        error_detail(&error),
        error.hint.as_deref(),
        error_suggestion_kind(&error),
        error_tip(&error),
        false,
    );

    assert!(
        rendered.contains("auth") || rendered.contains("login") || rendered.contains("Login"),
        "401 should suggest re-authentication: {rendered}"
    );
}

/// Responses missing the errorCode field must degrade gracefully to HTTP status classification rather than panicking
#[test]
fn test_missing_error_code_field() {
    let body = json!({"message": "Not Found"});

    let error = classify_to_runtime_error(404, &body, "iam", "roles");
    let rendered = render_error(
        &error.message,
        error_reason(&error),
        error_detail(&error),
        error.hint.as_deref(),
        error_suggestion_kind(&error),
        error_tip(&error),
        false,
    );

    assert!(
        rendered.contains("404") || rendered.to_lowercase().contains("not found"),
        "Should indicate 404: {rendered}"
    );
}

// ── Error classification: status-based ──

/// 401 must produce a re-login suggestion with the exact `ags auth login` command so users can copy-paste to fix it
#[test]
fn test_classify_401_error() {
    let body = json!({
        "errorCode": 20001,
        "errorMessage": "unauthorized access"
    });
    let error = classify_to_runtime_error(401, &body, "iam", "users");
    assert_eq!(error.message, "Request was not authorized.");
    assert_eq!(
        error_reason(&error),
        Some("Your access token is invalid or expired.")
    );
    assert!(error.hint.as_deref().unwrap().contains("ags auth login"));
}

/// 404 on a mapped code must produce a resource-specific message and a list command suggestion to help discovery
#[test]
fn test_classify_404_error() {
    let body = json!({
        "errorCode": 20008,
        "errorMessage": "user not found"
    });
    let error = classify_to_runtime_error(404, &body, "iam", "users");
    assert_eq!(error.message, "User not found.");
    assert_eq!(
        error_reason(&error),
        Some("The user does not exist or is not accessible in this context.")
    );
    assert!(error
        .hint
        .as_deref()
        .unwrap_or("")
        .contains("ags iam users search"));
}

/// Unmapped 404s must singularize the resource name (e.g. "users" -> "User not found") for natural language output
#[test]
fn test_classify_404_unmapped_error_singularizes() {
    let body = json!({
        "errorCode": 99999,
        "errorMessage": "resource not found"
    });
    let error = classify_to_runtime_error(404, &body, "iam", "users");
    assert_eq!(error.message, "User not found.");
}

/// 500 errors must suggest retrying so users know the failure is transient and server-side
#[test]
fn test_classify_500_error() {
    let body = json!({
        "errorCode": 0,
        "errorMessage": "internal server error"
    });
    let error = classify_to_runtime_error(500, &body, "iam", "users");
    assert_eq!(error.message, "Server error.");
    assert!(error.hint.as_deref().unwrap().contains("Retry"));
}

/// 501 must explain the operation isn't supported and offer a tip, avoiding a confusing suggestion to "fix" something the user can't control
#[test]
fn test_classify_501_not_implemented() {
    let body = json!({
        "errorCode": 0,
        "errorMessage": "not implemented"
    });
    let error = classify_to_runtime_error(501, &body, "iam", "users");
    assert_eq!(error.message, "Not implemented.");
    assert_eq!(
        error_reason(&error),
        Some("This operation is not yet supported by the API.")
    );
    assert!(error.hint.is_none());
    assert!(error_tip(&error).is_some());
}

// ── Error classification: validation ──

/// "details:" suffix in error messages must be extracted into the reason field so the validation detail isn't buried in the main message
#[test]
fn test_classify_400_with_details_pattern() {
    let body = json!({
        "errorCode": 20002,
        "errorMessage": "unable to update client: validation error, details: all request field is empty"
    });
    let error = classify_to_runtime_error(400, &body, "iam", "clients");
    assert_eq!(error.message, "Unable to update client: validation error.");
    assert_eq!(error_reason(&error), Some("All request field is empty."));
    assert_eq!(error_detail(&error), Some("Error code 20002"));
    assert_eq!(
        error.hint.as_deref(),
        Some("Check the request fields and retry.")
    );
}

/// 422 validation errors without a "details:" suffix must still show the raw message as the reason
#[test]
fn test_classify_422_validation_error_no_details() {
    let body = json!({
        "errorCode": 99998,
        "errorMessage": "validation error on field X"
    });
    let error = classify_to_runtime_error(422, &body, "iam", "users");
    assert_eq!(error.message, "Validation error.");
    assert_eq!(error_reason(&error), Some("Validation error on field X."));
    assert_eq!(
        error.hint.as_deref(),
        Some("Check the request fields and retry.")
    );
}

/// Generic 400 errors must capitalize the API message and suggest checking request fields, giving a consistent UX
#[test]
fn test_classify_400_generic_message() {
    let body = json!({
        "errorCode": 20004,
        "errorMessage": "invalid request body"
    });
    let error = classify_to_runtime_error(400, &body, "iam", "users");
    assert_eq!(error.message, "Invalid request body.");
    assert!(error_reason(&error).is_none());
    assert_eq!(
        error.hint.as_deref(),
        Some("Check the request fields and retry.")
    );
}

// ── Error classification: mapped error codes ──

/// Email conflict (10133) must suggest using a different address, turning a cryptic code into recovery steps
#[test]
fn test_classify_mapped_10133_email_conflict() {
    let body = json!({
        "errorCode": 10133,
        "errorMessage": "email already used"
    });
    let error = classify_to_runtime_error(409, &body, "iam", "users");
    assert_eq!(error.message, "Email address already in use.");
    assert_eq!(
        error_reason(&error),
        Some("Another account is already registered with this email.")
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Use a different email address or recover the existing account.")
    );
}

/// Role not found (10456) must suggest the list command so users can discover valid role IDs
#[test]
fn test_classify_mapped_10456_role_not_found() {
    let body = json!({
        "errorCode": 10456,
        "errorMessage": "role not found"
    });
    let error = classify_to_runtime_error(404, &body, "iam", "roles");
    assert_eq!(error.message, "Role not found.");
    assert_eq!(
        error.hint.as_deref(),
        Some("Run 'ags iam roles list' to see available roles.")
    );
}

/// Client not found (10365) must prompt the user to verify their client ID, the most common cause of this error
#[test]
fn test_classify_mapped_10365_client_not_found() {
    let body = json!({
        "errorCode": 10365,
        "errorMessage": "client not found"
    });
    let error = classify_to_runtime_error(404, &body, "iam", "clients");
    assert_eq!(error.message, "Client not found.");
    assert_eq!(
        error.hint.as_deref(),
        Some("Check the client ID and retry.")
    );
}

/// Password reuse (1015073) must rewrite the awkward API grammar and include the error code in the detail field
#[test]
fn test_classify_mapped_1015073_password_reuse() {
    let body = json!({
        "errorCode": 1015073,
        "errorMessage": "new password cannot same with old password"
    });
    let error = classify_to_runtime_error(400, &body, "iam", "users");
    assert_eq!(
        error.message,
        "New password cannot be the same as the old password."
    );
    assert_eq!(error.hint.as_deref(), Some("Choose a different password."));
    assert_eq!(error_detail(&error), Some("Error code 1015073"));
}

// ── Error classification: detail format & stripping ──

/// Internal identifiers like userID suffixes must be stripped from reasons to avoid leaking opaque IDs to users
#[test]
fn test_classify_error_strips_user_id() {
    let body = json!({
        "errorCode": 99999,
        "errorMessage": "unable to get user verification code: user not found, userID: abc123def456"
    });
    let error = classify_to_runtime_error(404, &body, "iam", "users");
    assert_eq!(
        error_reason(&error),
        Some("unable to get user verification code: user not found")
    );
}

/// Mapped error codes must appear in the detail field as "Error code NNNNN" for support ticket references
#[test]
fn test_detail_format_with_mapped_code() {
    let body = json!({
        "errorCode": 10139,
        "errorMessage": "platform account not found"
    });
    let error = classify_to_runtime_error(404, &body, "iam", "users");
    assert_eq!(error_detail(&error), Some("Error code 10139"));
}

/// Error code 0 (absent) must omit the detail field entirely to avoid showing a meaningless "Error code 0"
#[test]
fn test_detail_format_no_error_code() {
    let body = json!({
        "errorCode": 0,
        "errorMessage": "internal server error"
    });
    let error = classify_to_runtime_error(500, &body, "iam", "users");
    // No error code → no detail
    assert_eq!(error_detail(&error), None);
}
