//! Classify API error responses into user-friendly messages with fix suggestions.

use serde_json::Value;

use super::error_codes;
use crate::support::strings::{
    capitalize_first, kebab_case_to_words, singularize, strip_terminal_control_sequences,
};
use ags_protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};
use ags_protocol::error::{ErrorMetadata, SuggestionKind};

/// Classify an API error response into a `RuntimeError`.
pub fn classify_to_runtime_error(
    status: u16,
    body: &Value,
    service: &str,
    resource: &str,
    method: &str,
) -> RuntimeError {
    let (message, metadata) =
        classify_error_message_and_metadata(status, body, service, resource, method);
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
    method: &str,
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

    // Prefer curated error-code mappings — but only as the authority when the
    // server did not send its own specific message. When the server message is
    // informative (differs from the curated label, not a placeholder), surface it
    // with an honest "<operation> failed" headline and demote the curated mapping
    // to a reference annotation, dropping the curated fix (it belongs to the
    // code's nominal meaning, which the server just refined).
    if let Some(mapping) = error_codes::lookup_error(service, error_code) {
        if let Some(clean) = clean_message.as_deref() {
            if is_informative_server_message(clean, mapping.message, mapping.reason) {
                let annotated_detail = detail.map(|d| format!("{d} ({})", mapping.message));
                return (
                    operation_failed_headline(method, resource),
                    Some(ErrorMetadata {
                        reason: Some(clean.to_string()),
                        detail: annotated_detail,
                        suggestion: None,
                        ..Default::default()
                    }),
                );
            }
        }
        // Curated authority (unchanged): curated reason wins, else the server
        // message; curated suggestion + tip retained.
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

    classify_by_http_status(status, body, service, resource, clean_message, detail)
}

/// Classify an upstream error from its HTTP status alone, after curated
/// error-code mappings have been tried. `clean_message` is the sanitized server
/// message used as the reason fallback; `detail` is the `Error code N` line.
fn classify_by_http_status(
    status: u16,
    body: &Value,
    service: &str,
    resource: &str,
    clean_message: Option<String>,
    detail: Option<String>,
) -> (String, Option<ErrorMetadata>) {
    match status {
        401 => (
            "Request was not authorized".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some("Run 'ags auth login'.".to_string()),
                ..Default::default()
            }),
        ),
        403 => (
            "You do not have permission for this operation".to_string(),
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
                format!("{} not found", capitalize_first(&singular)),
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
            "Update rejected — resource has changed".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some("Fetch the latest version and retry.".to_string()),
                ..Default::default()
            }),
        ),
        429 => (
            "Too many requests".to_string(),
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
                    "Validation error".to_string(),
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
            "Not implemented".to_string(),
            Some(ErrorMetadata {
                reason: Some("This operation is not yet supported by the API.".to_string()),
                detail,
                tip: Some("This endpoint may be available in a future API version.".to_string()),
                ..Default::default()
            }),
        ),
        status_code if status_code >= 500 => (
            "Server error".to_string(),
            Some(ErrorMetadata {
                reason: clean_message,
                detail,
                suggestion: Some("Retry the command.".to_string()),
                ..Default::default()
            }),
        ),
        _ => (
            clean_message.unwrap_or_else(|| format!("HTTP {status} error")),
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

/// Compose the failure headline `"<verb> <noun> failed"` from the operation —
/// never from the error code. Resource-inclusive so bare CRUD methods aren't
/// vague.
///
/// The noun derivation mirrors `derive_noun_from_method` (the success path) so a
/// given operation reads consistently across success and failure: the leading
/// qualifiers `by`/`for`/`with`/`from`/`of` fall back to the resource noun, and
/// `my` is stripped to keep the self-scoped object (e.g. `check-my-ownership-by-
/// app-id` → "ownership by app id", not the bare resource). It differs from that
/// helper in one respect: the resource-fallback noun is singularized here, so
/// bare CRUD failures read "Get item failed", not "Get items failed".
fn operation_failed_headline(method: &str, resource: &str) -> String {
    let verb = method.split('-').next().unwrap_or(method);
    let resource_noun = || kebab_case_to_words(&singularize(resource));
    let noun = match method.find('-') {
        None => resource_noun(),
        Some(pos) => {
            let after = &method[pos + 1..];
            let first_word = after.split('-').next().unwrap_or("");
            if matches!(first_word, "by" | "for" | "with" | "from" | "of") {
                resource_noun()
            } else if first_word == "my" {
                let rest = after.strip_prefix("my").unwrap_or(after);
                let rest = rest.strip_prefix('-').unwrap_or(rest);
                if rest.is_empty() {
                    resource_noun()
                } else {
                    rest.replace('-', " ")
                }
            } else {
                after.replace('-', " ")
            }
        }
    };
    let phrase = format!("{verb} {noun}");
    format!("{} failed", capitalize_first(&phrase))
}

/// True when the server's (already-sanitised) message adds something the curated
/// mapping does not already say: non-empty, not a normalized echo of the curated
/// message/reason, and not a known generic placeholder.
fn is_informative_server_message(
    clean: &str,
    curated_message: &str,
    curated_reason: Option<&str>,
) -> bool {
    let normalize = |s: &str| s.trim().trim_end_matches('.').to_lowercase();
    let c = normalize(clean);
    if c.is_empty() {
        return false;
    }
    const GENERIC_PLACEHOLDERS: [&str; 5] = [
        "error",
        "internal error",
        "bad request",
        "unknown error",
        "validation error",
    ];
    if GENERIC_PLACEHOLDERS.contains(&c.as_str()) {
        return false;
    }
    if c == normalize(curated_message) {
        return false;
    }
    if let Some(reason) = curated_reason {
        if c == normalize(reason) {
            return false;
        }
    }
    true
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
                "Validation error".to_string()
            } else {
                // Headline: no trailing full stop (the reason below keeps its
                // sentence punctuation).
                capitalize_first(before).trim_end_matches('.').to_string()
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
        return ("Validation error".to_string(), Some(reason));
    }

    // Default: capitalize the raw message; headline carries no trailing stop.
    let message = capitalize_first(raw).trim_end_matches('.').to_string();
    (message, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Curated code + an informative, non-echo server message → operation headline,
    /// server message as reason, code+curated-name detail, no fix.
    #[test]
    fn test_known_error_code_with_informative_message_uses_operation_headline() {
        let body = json!({ "errorCode": 10139, "errorMessage": "raw message" });
        let error = classify_to_runtime_error(401, &body, "iam", "users", "get");
        assert_eq!(error.message, "Get user failed");
        assert_eq!(
            error.details.as_ref().and_then(|d| d.reason.as_deref()),
            Some("raw message")
        );
        assert!(error.hint.is_none());
    }

    /// Error code 0 (absent) must fall through to HTTP-status-based classification
    #[test]
    fn test_error_code_zero_falls_through_to_status() {
        let body = json!({ "errorCode": 0, "errorMessage": "something went wrong" });
        let error = classify_to_runtime_error(401, &body, "iam", "users", "get");
        assert_eq!(error.message, "Request was not authorized");
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
        let error = classify_to_runtime_error(500, &body, "iam", "users", "get");
        assert_eq!(error.message, "Server error");
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
        let error = classify_to_runtime_error(400, &body, "iam", "users", "get");
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
        let error = classify_to_runtime_error(422, &body, "iam", "users", "get");
        let reason = error
            .details
            .as_ref()
            .and_then(|details| details.reason.as_deref())
            .expect("should have reason");
        assert!(reason.contains("Name too long"));
    }

    /// Empty errorMessage on 400 must produce a generic "Validation error" fallback
    #[test]
    fn test_validation_error_no_details_no_message() {
        let body = json!({ "errorCode": 0, "errorMessage": "" });
        let error = classify_to_runtime_error(400, &body, "iam", "users", "get");
        assert_eq!(error.message, "Validation error");
    }

    /// 404 responses must include the singularized resource name and a "next step" suggestion
    #[test]
    fn test_404_includes_resource_name() {
        let body = json!({ "errorCode": 0, "errorMessage": "not found" });
        let error = classify_to_runtime_error(404, &body, "iam", "users", "get");
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
        let error = classify_to_runtime_error(404, &body, "basic", "user-profile", "get");
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

    #[test]
    fn test_operation_failed_headline_no_hyphen_uses_resource() {
        assert_eq!(
            operation_failed_headline("create", "stores"),
            "Create store failed"
        );
        assert_eq!(operation_failed_headline("get", "items"), "Get item failed");
        assert_eq!(
            operation_failed_headline("delete", "stores"),
            "Delete store failed"
        );
    }

    #[test]
    fn test_operation_failed_headline_object_tail() {
        // A real object tail (publish-all) is kept.
        assert_eq!(
            operation_failed_headline("publish-all", "catalog-changes"),
            "Publish all failed"
        );
    }

    #[test]
    fn test_operation_failed_headline_by_qualifier_uses_resource() {
        // `-by-` is a qualifier, not the object: fall back to the resource noun.
        assert_eq!(
            operation_failed_headline("get-by-id", "items"),
            "Get item failed"
        );
        assert_eq!(
            operation_failed_headline("get-by-sku", "items"),
            "Get item failed"
        );
        assert_eq!(
            operation_failed_headline("delete-by-share-code", "items"),
            "Delete item failed"
        );
    }

    #[test]
    fn test_operation_failed_headline_my_qualifier_keeps_scope_noun() {
        // `-my-` is a self-scope qualifier: strip "my" and keep the remaining
        // object, mirroring `derive_noun_from_method` (the success path) rather
        // than collapsing to the resource noun and losing the scope entirely.
        assert_eq!(
            operation_failed_headline("check-my-ownership-by-app-id", "entitlements"),
            "Check ownership by app id failed"
        );
        assert_eq!(
            operation_failed_headline("claim-my-rewards-by-challenge-code", "player-rewards"),
            "Claim rewards by challenge code failed"
        );
    }

    #[test]
    fn test_operation_failed_headline_my_alone_falls_back_to_resource() {
        // `-my` with nothing after it → resource noun (singularized).
        assert_eq!(
            operation_failed_headline("get-my", "entitlements"),
            "Get entitlement failed"
        );
    }

    #[test]
    fn test_operation_failed_headline_other_qualifiers_use_resource() {
        // for/with/from/of behave like `-by-`: fall back to the resource noun,
        // matching the qualifier set `derive_noun_from_method` recognises.
        assert_eq!(
            operation_failed_headline("get-for-user", "entitlements"),
            "Get entitlement failed"
        );
        assert_eq!(
            operation_failed_headline("query-of-namespace", "stores"),
            "Query store failed"
        );
    }

    #[test]
    fn test_is_informative_true_when_specific_and_different() {
        assert!(is_informative_server_message(
            "Language/Region does not match",
            "Item not found",
            Some("The specified item does not exist in this namespace"),
        ));
        // Adds an id → not equal → informative.
        assert!(is_informative_server_message(
            "Item not found: abc123",
            "Item not found",
            None,
        ));
    }

    /// Curated code + an informative, differing server message → operation
    /// headline, server message as reason, code+curated-name detail, no fix.
    #[test]
    fn test_curated_code_with_informative_message_surfaces_server_message() {
        let body = json!({
            "errorCode": 30122,
            "errorMessage": "Language/Region does not match"
        });
        let error =
            classify_to_runtime_error(409, &body, "platform", "catalog-changes", "publish-all");
        assert_eq!(error.message, "Publish all failed");
        let details = error.details.as_ref().expect("details");
        assert_eq!(
            details.reason.as_deref(),
            Some("Language/Region does not match")
        );
        assert_eq!(
            details.detail.as_deref(),
            Some("Error code 30122 (Item not found)")
        );
        assert!(
            error.hint.is_none(),
            "curated fix must be dropped when we defer to the server"
        );
    }

    /// Curated code + server message that adds an id → still triggers; the id is
    /// preserved in the reason.
    #[test]
    fn test_curated_code_with_id_suffix_preserves_id() {
        let body = json!({ "errorCode": 30122, "errorMessage": "Item not found: abc123" });
        let error = classify_to_runtime_error(404, &body, "platform", "items", "get");
        assert_eq!(error.message, "Get item failed");
        assert_eq!(
            error.details.as_ref().and_then(|d| d.reason.as_deref()),
            Some("Item not found: abc123")
        );
        assert!(error.hint.is_none());
    }

    /// Curated code + an exact echo of the curated message → stays on the curated
    /// path (curated headline + curated fix kept).
    #[test]
    fn test_curated_code_with_echo_message_keeps_curated() {
        let body = json!({ "errorCode": 30122, "errorMessage": "Item not found" });
        let error = classify_to_runtime_error(404, &body, "platform", "items", "get");
        assert_eq!(error.message, "Item not found");
        assert_eq!(error.hint.as_deref(), Some("Verify the item ID and retry."));
    }

    /// Curated code + a generic placeholder → stays on the curated path.
    #[test]
    fn test_curated_code_with_generic_message_keeps_curated() {
        let body = json!({ "errorCode": 30122, "errorMessage": "error" });
        let error = classify_to_runtime_error(404, &body, "platform", "items", "get");
        assert_eq!(error.message, "Item not found");
        assert_eq!(error.hint.as_deref(), Some("Verify the item ID and retry."));
    }

    /// No curated mapping → unchanged HTTP-status fallback (method arg is ignored
    /// on this path).
    #[test]
    fn test_no_curated_mapping_unchanged() {
        let body = json!({ "errorCode": 0, "errorMessage": "something went wrong" });
        let error = classify_to_runtime_error(401, &body, "iam", "users", "get");
        assert_eq!(error.message, "Request was not authorized");
    }

    #[test]
    fn test_is_informative_false_when_echo_or_generic_or_empty() {
        // Exact echo of the curated message.
        assert!(!is_informative_server_message(
            "Item not found",
            "Item not found",
            None
        ));
        // Exact echo of the curated reason (case/punctuation-insensitive).
        assert!(!is_informative_server_message(
            "the specified item does not exist in this namespace.",
            "Item not found",
            Some("The specified item does not exist in this namespace"),
        ));
        // Generic placeholder.
        assert!(!is_informative_server_message(
            "error",
            "Item not found",
            None
        ));
        assert!(!is_informative_server_message(
            "Bad request",
            "Item not found",
            None
        ));
        // Empty.
        assert!(!is_informative_server_message(
            "   ",
            "Item not found",
            None
        ));
    }
}
