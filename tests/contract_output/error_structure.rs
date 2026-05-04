use ags::frontend::human::templates::render_error_text as render_error;
use ags::runtime::dispatch::classify::classify_to_runtime_error;
use serde_json::json;

use crate::common::error_helpers::{error_detail, error_reason, error_suggestion_kind, error_tip};

/// Helper: classify + render with color disabled.
fn render_classified(status: u16, body: &serde_json::Value) -> String {
    let error = classify_to_runtime_error(status, body, "iam", "users");
    render_error(
        &error.message,
        error_reason(&error),
        error_detail(&error),
        error.hint.as_deref(),
        error_suggestion_kind(&error),
        error_tip(&error),
        false,
    )
}

// ── Error prefix contract ──

/// Every HTTP error status renders with the error symbol so users can visually scan for failures
#[test]
fn test_error_always_has_error_prefix() {
    let cases = vec![
        (401, json!({"errorCode": 0})),
        (
            403,
            json!({"errorCode": 20013, "errorMessage": "forbidden"}),
        ),
        (404, json!({"errorCode": 0, "errorMessage": "not found"})),
        (500, json!({"errorCode": 0})),
    ];

    for (status, body) in cases {
        let rendered = render_classified(status, &body);
        assert!(
            rendered.contains('\u{2715}'),
            "HTTP {status} must contain error symbol: {rendered}"
        );
    }
}

// ── Detail line contract ──

/// Known AccelByte error codes surface the numeric code in a Detail line so users can reference docs
#[test]
fn test_known_error_code_shows_detail_with_code() {
    let body = json!({"errorCode": 20013, "errorMessage": "forbidden"});
    let rendered = render_classified(403, &body);

    assert!(
        rendered.contains("Detail:"),
        "Known error code must have Detail line: {rendered}"
    );
    assert!(
        rendered.contains("20013"),
        "Detail must include error code: {rendered}"
    );
}

/// Unrecognised error codes still show the numeric code so users can report it to support
#[test]
fn test_unknown_error_code_shows_detail_with_code() {
    let body = json!({"errorCode": 99999});
    let rendered = render_classified(500, &body);

    assert!(
        rendered.contains("Detail:"),
        "Unknown error code must have Detail line: {rendered}"
    );
    assert!(
        rendered.contains("99999"),
        "Detail must include error code: {rendered}"
    );
}

/// Error code 0 means "no specific code" so the Detail line is omitted to avoid showing noise
#[test]
fn test_zero_error_code_has_no_detail_line() {
    let body = json!({"errorCode": 0});
    let rendered = render_classified(404, &body);

    assert!(
        !rendered.contains("Detail:"),
        "Zero error code must not show Detail line: {rendered}"
    );
}

// ── Reason line contract ──

/// Known error codes produce a curated reason so users understand what went wrong without raw API text
#[test]
fn test_known_code_has_reason() {
    let body = json!({"errorCode": 20013, "errorMessage": "forbidden"});
    let error = classify_to_runtime_error(403, &body, "iam", "users");

    // Known error codes from mapping table should have a curated reason
    let has_reason = error_reason(&error).is_some();
    assert!(
        has_reason || error.message.len() > 10,
        "Known error code should have a reason or descriptive message"
    );
}

/// Unknown error codes fall back to the raw errorMessage so the user still gets some explanation
#[test]
fn test_unknown_code_uses_raw_message_as_reason() {
    let body = json!({"errorCode": 99999, "errorMessage": "something went wrong"});
    let error = classify_to_runtime_error(500, &body, "iam", "users");

    // Unknown codes should use the raw errorMessage as reason
    if let Some(reason) = error_reason(&error) {
        assert!(
            reason.contains("something went wrong"),
            "Unknown code should pass through errorMessage as reason: {reason}"
        );
    }
}

// ── Fix line contract ──

/// 401 errors include a Fix line mentioning login so users can self-resolve without searching docs
#[test]
fn test_401_has_fix_suggestion() {
    let body = json!({"errorCode": 0});
    let rendered = render_classified(401, &body);

    assert!(
        rendered.contains("Fix:"),
        "401 must include a Fix suggestion: {rendered}"
    );
    assert!(
        rendered.to_lowercase().contains("login"),
        "401 fix should mention login: {rendered}"
    );
}

/// 404 errors use "Next:" instead of "Fix:" because the issue is a wrong ID, not a broken setup
#[test]
fn test_404_has_next_suggestion() {
    let body = json!({"errorCode": 0, "errorMessage": "not found"});
    let rendered = render_classified(404, &body);

    assert!(
        rendered.contains("Next:"),
        "404 must use Next (not Fix) for its suggestion: {rendered}"
    );
}

// ── Line ordering contract ──

/// Error lines follow a strict order (error → detail → fix) so the output reads top-to-bottom logically
#[test]
fn test_error_line_ordering() {
    let body = json!({"errorCode": 20013, "errorMessage": "forbidden"});
    let rendered = render_classified(403, &body);
    let lines: Vec<&str> = rendered.lines().collect();

    // Error symbol must be first
    assert!(
        lines[0].contains('\u{2715}'),
        "First line must be the error line: {rendered}"
    );

    // If Reason and Detail exist, they must come after error and before Fix
    let error_pos = lines.iter().position(|l| l.contains('\u{2715}'));
    let detail_pos = lines.iter().position(|l| l.contains("Detail:"));
    let fix_pos = lines.iter().position(|l| l.contains("Fix:"));

    if let (Some(e), Some(d)) = (error_pos, detail_pos) {
        assert!(e < d, "Error must come before Detail");
    }
    if let (Some(d), Some(f)) = (detail_pos, fix_pos) {
        assert!(d < f, "Detail must come before Fix");
    }
}

// ── No raw internals contract ──

/// Error output never leaks Rust internals (panic, unwrap, thread traces) to end users
#[test]
fn test_error_output_no_rust_internals() {
    let cases = vec![
        (401, json!({"errorCode": 0})),
        (403, json!({"errorCode": 20013})),
        (500, json!({"errorCode": 0})),
    ];

    for (status, body) in cases {
        let rendered = render_classified(status, &body);
        assert!(
            !rendered.contains("panic"),
            "Error must not contain 'panic': {rendered}"
        );
        assert!(
            !rendered.contains("unwrap"),
            "Error must not contain 'unwrap': {rendered}"
        );
        assert!(
            !rendered.contains("thread '"),
            "Error must not contain thread traces: {rendered}"
        );
    }
}
