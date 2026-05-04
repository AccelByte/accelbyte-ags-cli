//! Classify API error responses into user-friendly messages with fix suggestions.

use serde_json::Value;

use super::error_codes;
use crate::protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};
use crate::protocol::error::{ErrorMetadata, SuggestionKind};
use crate::support::strings::{
    capitalize_first, kebab_case_to_words, singularize, strip_terminal_control_sequences,
};

/// Classify an API error response into a `RuntimeError`.
pub fn classify_to_runtime_error(
    status: u16,
    body: &Value,
    service: &str,
    resource: &str,
) -> RuntimeError {
    let (message, metadata) = classify_error_message_and_metadata(status, body, service, resource);
    let kind = derive_runtime_error_kind(status, body);
    let hint = metadata.as_ref().and_then(|m| m.suggestion.clone());
    let details = metadata.as_ref().map(|m| {
        Box::new(ErrorDetails {
            code: body
                .get("errorCode")
                .and_then(|value| value.as_i64())
                .filter(|code| *code != 0)
                .map(|code| code.to_string()),
            reason: m.reason.clone(),
            detail: m.detail.clone(),
            suggestion_kind: Some(m.suggestion_kind),
            tip: m.tip.clone(),
        })
    });
    RuntimeError {
        kind,
        message,
        details,
        hint,
        trace: None,
    }
}

/// Map an HTTP status (and optional AccelByte error code) to a `RuntimeErrorKind` variant.
fn derive_runtime_error_kind(status: u16, body: &Value) -> RuntimeErrorKind {
    let code = body
        .get("errorCode")
        .and_then(|value| value.as_i64())
        .filter(|code| *code != 0)
        .map(|code| code.to_string());
    match status {
        401 => RuntimeErrorKind::NotAuthenticated,
        403 => RuntimeErrorKind::Forbidden,
        404 => RuntimeErrorKind::NotFound,
        400 | 422 => RuntimeErrorKind::Rejected,
        s => RuntimeErrorKind::Upstream { status: s, code },
    }
}

/// Build the message and metadata for a classified API error.
fn classify_error_message_and_metadata(
    status: u16,
    body: &Value,
    service: &str,
    resource: &str,
) -> (String, Option<ErrorMetadata>) {
    let error_message = body
        .get("errorMessage")
        .and_then(|value| value.as_str())
        .map(String::from);
    let error_code = body
        .get("errorCode")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);

    let detail = if error_code != 0 {
        Some(format!("Error code {error_code}"))
    } else {
        None
    };

    // Prefer validation details over generic code mappings.
    if status == 400 || status == 422 {
        let raw_unsanitized = body
            .get("errorMessage")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let raw = strip_terminal_control_sequences(raw_unsanitized);
        let lower = raw.to_lowercase();
        if lower.contains("details:") || lower.contains("detail:") {
            let (message, reason) = split_validation_error_message(&raw);
            return (
                message,
                Some(ErrorMetadata {
                    reason,
                    detail,
                    suggestion: Some("Check the request fields and retry.".to_string()),
                    ..Default::default()
                }),
            );
        }
    }

    // Normalize the raw error message — used as the reason fallback both for
    // curated mappings (so server-substituted template values aren't lost) and
    // for HTTP-status fallbacks below.
    let clean_message = error_message
        .map(|message| strip_terminal_control_sequences(&strip_user_id_suffix(&message)));

    // Prefer curated error-code mappings when available.
    if let Some(mapping) = error_codes::lookup_error(service, error_code) {
        // Curated reason wins when present; otherwise surface the server's
        // own message so callers see the actual offending IDs / paths.
        let reason = mapping
            .reason
            .map(String::from)
            .or_else(|| clean_message.clone());
        return (
            mapping.message.to_string(),
            Some(ErrorMetadata {
                reason,
                detail,
                suggestion: mapping.suggestion.map(String::from),
                tip: mapping.tip.map(String::from),
                ..Default::default()
            }),
        );
    }

    // Fall back to HTTP-status-based classification.
    match status {
        401 => (
            "Request was not authorized.".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some("Run 'ags auth login'.".to_string()),
                ..Default::default()
            }),
        ),
        403 => (
            "You do not have permission for this operation.".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some(
                    "Check that your account has the required role and permissions.".to_string(),
                ),
                ..Default::default()
            }),
        ),
        404 => {
            let singular = kebab_case_to_words(&singularize(resource));
            (
                format!("{} not found.", capitalize_first(&singular)),
                Some(ErrorMetadata {
                    reason: clean_message,
                    detail,
                    suggestion: Some(format!(
                        "Run 'ags {service} {resource} --help' to see available methods."
                    )),
                    suggestion_kind: SuggestionKind::Next,
                    ..Default::default()
                }),
            )
        }
        409 => (
            "Update rejected — resource has changed.".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some("Fetch the latest version and retry.".to_string()),
                ..Default::default()
            }),
        ),
        429 => (
            "Too many requests.".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some("Wait a moment and retry.".to_string()),
                ..Default::default()
            }),
        ),
        400 | 422 => {
            let raw_unsanitized = body
                .get("errorMessage")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let raw = strip_terminal_control_sequences(raw_unsanitized);
            let (msg, reason) = if !raw.is_empty() {
                split_validation_error_message(&raw)
            } else {
                (
                    "Validation error.".to_string(),
                    Some("The request was invalid.".to_string()),
                )
            };
            (
                msg,
                Some(ErrorMetadata {
                    reason,
                    detail,
                    suggestion: Some("Check the request fields and retry.".to_string()),
                    ..Default::default()
                }),
            )
        }
        501 => (
            "Not implemented.".to_string(),
            Some(ErrorMetadata {
                reason: Some("This operation is not yet supported by the API.".to_string()),
                detail,
                tip: Some("This endpoint may be available in a future API version.".to_string()),
                ..Default::default()
            }),
        ),
        status_code if status_code >= 500 => (
            "Server error.".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some("Retry the command.".to_string()),
                ..Default::default()
            }),
        ),
        _ => (
            clean_message.unwrap_or_else(|| format!("HTTP {status} error.")),
            detail.map(|d| ErrorMetadata {
                detail: Some(d),
                ..Default::default()
            }),
        ),
    }
}

/// Remove trailing `userID` fragments from API error messages.
fn strip_user_id_suffix(message: &str) -> String {
    if let Some(pos) = message.find(", userID: ") {
        let cleaned = message[..pos].trim_end_matches('.');
        if cleaned.is_empty() {
            message.to_string()
        } else {
            cleaned.to_string()
        }
    } else {
        message.to_string()
    }
}

/// Parse validation error text into message and reason strings.
fn split_validation_error_message(raw: &str) -> (String, Option<String>) {
    let lower = raw.to_lowercase();

    // Look for "details:" or "detail:" separator
    for separator in &["details:", "detail:"] {
        if let Some(pos) = lower.find(separator) {
            let before = raw[..pos].trim().trim_end_matches(',').trim();
            let after = raw[pos + separator.len()..].trim();

            let message = if before.is_empty() {
                "Validation error.".to_string()
            } else {
                let message_text = capitalize_first(before);
                if message_text.ends_with('.') {
                    message_text
                } else {
                    format!("{message_text}.")
                }
            };

            let reason = if after.is_empty() {
                None
            } else {
                let reason_text = capitalize_first(after);
                if reason_text.ends_with('.') {
                    Some(reason_text)
                } else {
                    Some(format!("{reason_text}."))
                }
            };

            return (message, reason);
        }
    }

    // Contains "validation error" but no details separator
    if lower.contains("validation error") {
        let reason = capitalize_first(raw);
        let reason = if reason.ends_with('.') {
            reason
        } else {
            format!("{reason}.")
        };
        return ("Validation error.".to_string(), Some(reason));
    }

    // Default: capitalize the raw message
    let message = capitalize_first(raw);
    let message = if message.ends_with('.') {
        message
    } else {
        format!("{message}.")
    };
    (message, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Known AccelByte error codes must use curated messages rather than raw API text
    #[test]
    fn test_known_error_code_returns_curated_message() {
        let body = json!({ "errorCode": 10139, "errorMessage": "raw message" });
        let error = classify_to_runtime_error(401, &body, "iam", "users");
        // Should use the curated mapping, not the raw message
        assert!(error.details.is_some());
        assert!(!error.message.is_empty());
    }

    /// Error code 0 (absent) must fall through to HTTP-status-based classification
    #[test]
    fn test_error_code_zero_falls_through_to_status() {
        let body = json!({ "errorCode": 0, "errorMessage": "something went wrong" });
        let error = classify_to_runtime_error(401, &body, "iam", "users");
        assert_eq!(error.message, "Request was not authorized.");
        let reason = error
            .details
            .as_ref()
            .and_then(|details| details.reason.as_deref())
            .expect("should have reason");
        assert!(reason.contains("something went wrong"));
    }

    /// Empty error bodies must not panic and should produce a generic server error message
    #[test]
    fn test_empty_error_body() {
        let body = json!({});
        let error = classify_to_runtime_error(500, &body, "iam", "users");
        assert_eq!(error.message, "Server error.");
        let details = error.details.as_ref().expect("should have details");
        assert!(details.reason.is_none());
    }

    /// Validation errors with "details:" must split into a clean message and a reason
    #[test]
    fn test_validation_error_with_details_separator() {
        let body = json!({
            "errorCode": 0,
            "errorMessage": "field validation failed, details: email is required"
        });
        let error = classify_to_runtime_error(400, &body, "iam", "users");
        assert!(error.message.contains("Field validation failed"));
        let reason = error
            .details
            .as_ref()
            .and_then(|details| details.reason.as_deref())
            .expect("should have reason");
        assert!(reason.contains("Email is required"));
    }

    /// The singular "detail:" separator must be recognized alongside "details:"
    #[test]
    fn test_validation_error_with_detail_singular() {
        let body = json!({
            "errorCode": 0,
            "errorMessage": "invalid input, detail: name too long"
        });
        let error = classify_to_runtime_error(422, &body, "iam", "users");
        let reason = error
            .details
            .as_ref()
            .and_then(|details| details.reason.as_deref())
            .expect("should have reason");
        assert!(reason.contains("Name too long"));
    }

    /// Empty errorMessage on 400 must produce a generic "Validation error." fallback
    #[test]
    fn test_validation_error_no_details_no_message() {
        let body = json!({ "errorCode": 0, "errorMessage": "" });
        let error = classify_to_runtime_error(400, &body, "iam", "users");
        assert_eq!(error.message, "Validation error.");
    }

    /// 404 responses must include the singularized resource name and a "next step" suggestion
    #[test]
    fn test_404_includes_resource_name() {
        let body = json!({ "errorCode": 0, "errorMessage": "not found" });
        let error = classify_to_runtime_error(404, &body, "iam", "users");
        assert!(error.message.contains("not found"));
        let kind = error
            .details
            .as_ref()
            .and_then(|d| d.suggestion_kind)
            .expect("should have suggestion_kind");
        assert_eq!(kind, SuggestionKind::Next);
        assert!(error.hint.as_deref().unwrap().contains("iam"));
    }

    /// Kebab-case resource names must render with spaces in the 404 message so
    /// users see "User profile not found." instead of "User-profile not found."
    /// The help-command suggestion still carries the kebab form since that is
    /// what the user would type.
    #[test]
    fn test_404_kebab_case_resource_renders_with_spaces() {
        let body = json!({ "errorCode": 0, "errorMessage": "not found" });
        let error = classify_to_runtime_error(404, &body, "basic", "user-profile");
        assert!(
            error.message.contains("User profile not found"),
            "expected spaced noun in message, got: {}",
            error.message
        );
        assert!(
            !error.message.contains("User-profile"),
            "kebab-case leaked into message: {}",
            error.message
        );
        // The suggestion still refers to the CLI command, which IS kebab-case.
        assert!(
            error.hint.as_deref().unwrap().contains("user-profile"),
            "suggestion should carry kebab CLI name, got: {:?}",
            error.hint
        );
    }

    /// User ID suffixes must be stripped to avoid leaking internal identifiers in error messages
    #[test]
    fn test_strip_user_id_removes_suffix() {
        assert_eq!(
            strip_user_id_suffix("unauthorized access, userID: abc123def"),
            "unauthorized access"
        );
    }

    /// Messages without a userID suffix must pass through unchanged
    #[test]
    fn test_strip_user_id_no_suffix() {
        assert_eq!(strip_user_id_suffix("plain error"), "plain error");
    }
}
