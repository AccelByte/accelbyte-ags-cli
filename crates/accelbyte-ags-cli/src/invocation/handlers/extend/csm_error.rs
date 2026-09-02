//! Shared CSM API helpers for `ags extend` subcommands.
//!
//! CSM error responses carry `errorCode` (integer) and `errorMessage`
//! (string). This module extracts only those recognised fields, never
//! echoing the raw body — which for secret endpoints may contain the
//! submitted plaintext value in request-echo or `attributes` fields.
//!
//! Also provides shared constants and the submitted-value redaction
//! helper used by both `update-secret` and `update-var`.

/// Warning emitted when `--value` is used instead of `--value-stdin`.
///
/// Both `update-secret` and `update-var` share this exact message so a
/// wording change needs one edit. The `auth login` command has its own
/// variant mentioning `--client-secret` / `--client-secret-stdin`.
pub(crate) const VALUE_FLAG_SHELL_HISTORY_WARNING: &str =
    "--value is visible in shell history. Use --value-stdin for better security.";

/// Extract structured error detail from a CSM error response body.
///
/// Returns a suffix like `: errorCode 20004: validation error` when
/// fields are present, or an empty string when the body is not JSON
/// or carries no recognised field.
pub(crate) fn extract_csm_error_detail(body: &str) -> String {
    #[derive(serde::Deserialize)]
    struct CsmError {
        #[serde(rename = "errorCode")]
        error_code: Option<serde_json::Number>,
        #[serde(rename = "errorMessage")]
        error_message: Option<String>,
    }

    match serde_json::from_str::<CsmError>(body) {
        Ok(csm) => {
            let mut parts = Vec::new();
            if let Some(code) = csm.error_code {
                parts.push(format!("errorCode {code}"));
            }
            if let Some(msg) = csm.error_message {
                if !msg.is_empty() {
                    parts.push(msg);
                }
            }
            if parts.is_empty() {
                String::new()
            } else {
                format!(": {}", parts.join(": "))
            }
        }
        Err(_) => String::new(),
    }
}

/// Minimum length for a submitted value to be redacted as a bare
/// (unquoted) substring. Values shorter than this are only redacted
/// when they appear in the delimited form CSM echoes (`'value'`).
///
/// Rationale: a 1–7 character value can plausibly collide with
/// ordinary message text — a digit inside an HTTP status code, a
/// boolean like `"true"`, or a short word that happens to appear in
/// the error description. 8 characters is long enough to be
/// unlikely incidental while still catching short passwords and
/// tokens.
const BARE_REDACTION_MIN_LENGTH: usize = 8;

/// Redact occurrences of a submitted value from a `CliError` message.
///
/// CSM validation-error responses may echo the submitted value inside
/// `errorMessage` — typically in single quotes (e.g.
/// `"value 'super-secret-123' failed validation"`).
///
/// Two-layer strategy:
/// 1. **Delimited form** (`'value'`): always redacted, regardless of
///    length. This is the shape CSM actually echoes and the real leak
///    path.
/// 2. **Bare occurrences**: redacted only when the value is at least
///    [`BARE_REDACTION_MIN_LENGTH`] characters, avoiding corruption of
///    short substrings that collide with HTTP status codes or other
///    message infrastructure.
///
/// Applied unconditionally regardless of `--sensitive`: a value's
/// sensitivity is the operator's knowledge, not the CLI's, and an
/// operator who forgot `--sensitive` is exactly who needs the redaction.
///
/// Returns the error untouched when `submitted_value` is empty — an empty
/// pattern matches at every byte boundary in `str::replace`, turning the
/// message into confetti. There is no credential to hide in that case.
///
/// **Residual risk:** a short bare value (< 8 chars) that appears
/// unquoted in the error message will NOT be redacted. This is an
/// accepted trade-off: corrupting HTTP status codes and other message
/// infrastructure is worse than leaking a 1–7 character value that
/// the server chose to echo without quoting.
pub(crate) fn redact_submitted_value_in_error(
    error: crate::errors::CliError,
    submitted_value: &str,
) -> crate::errors::CliError {
    if submitted_value.is_empty() {
        return error;
    }
    match error {
        crate::errors::CliError::Api {
            message,
            metadata,
            category,
        } => {
            // Always redact the delimited form CSM echoes in single quotes.
            let quoted = format!("'{submitted_value}'");
            let mut redacted = message.replace(&quoted, "'[REDACTED]'");

            // Also redact bare occurrences when the value is long enough
            // not to be plausibly incidental text.
            if submitted_value.len() >= BARE_REDACTION_MIN_LENGTH {
                redacted = redacted.replace(submitted_value, "[REDACTED]");
            }

            crate::errors::CliError::Api {
                message: redacted,
                metadata,
                category,
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_fields_present() {
        let body = r#"{"errorCode": 20004, "errorMessage": "validation error"}"#;
        assert_eq!(
            extract_csm_error_detail(body),
            ": errorCode 20004: validation error"
        );
    }

    #[test]
    fn only_error_message() {
        let body = r#"{"errorMessage": "something went wrong"}"#;
        assert_eq!(extract_csm_error_detail(body), ": something went wrong");
    }

    #[test]
    fn only_error_code() {
        let body = r#"{"errorCode": 20003}"#;
        assert_eq!(extract_csm_error_detail(body), ": errorCode 20003");
    }

    #[test]
    fn empty_error_message_ignored() {
        let body = r#"{"errorCode": 20003, "errorMessage": ""}"#;
        assert_eq!(extract_csm_error_detail(body), ": errorCode 20003");
    }

    #[test]
    fn no_recognised_fields() {
        let body = r#"{"detail": "some other shape"}"#;
        assert_eq!(extract_csm_error_detail(body), "");
    }

    #[test]
    fn not_json() {
        assert_eq!(extract_csm_error_detail("not json at all"), "");
    }

    #[test]
    fn empty_body() {
        assert_eq!(extract_csm_error_detail(""), "");
    }

    // ── redact_submitted_value_in_error ──

    #[test]
    fn redact_replaces_value_in_api_error() {
        let error = crate::errors::CliError::Api {
            message: "value 'secret-123' failed validation".to_string(),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        };
        let redacted = redact_submitted_value_in_error(error, "secret-123");
        match redacted {
            crate::errors::CliError::Api { ref message, .. } => {
                assert!(
                    !message.contains("secret-123"),
                    "value must be redacted: {message}"
                );
                assert!(
                    message.contains("[REDACTED]"),
                    "placeholder must appear: {message}"
                );
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
    }

    #[test]
    fn redact_empty_value_is_noop() {
        let error = crate::errors::CliError::Api {
            message: "some error message".to_string(),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        };
        let redacted = redact_submitted_value_in_error(error, "");
        match redacted {
            crate::errors::CliError::Api { ref message, .. } => {
                assert_eq!(message, "some error message");
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
    }

    #[test]
    fn redact_non_api_error_is_passthrough() {
        let error = crate::errors::CliError::Usage {
            message: "usage error with secret-123".to_string(),
            metadata: None,
        };
        let redacted = redact_submitted_value_in_error(error, "secret-123");
        match redacted {
            crate::errors::CliError::Usage { ref message, .. } => {
                // Non-Api variants pass through unchanged — redaction targets
                // only API response errors where the CSM echo behaviour occurs.
                assert!(message.contains("secret-123"));
            }
            other => panic!("expected CliError::Usage, got: {other:?}"),
        }
    }

    // ── Delimiter-aware redaction (short-value safety) ──

    /// A short bare value like `"3"` must NOT corrupt the HTTP status code
    /// or other incidental occurrences of the same character sequence in the
    /// error message. Only the delimited form `'3'` is redacted.
    ///
    /// Contract: security/redaction — "redact unconditionally" applies to
    /// the delimited token shape CSM actually echoes, not to bare substrings
    /// that collide with message infrastructure.
    #[test]
    fn redact_short_value_does_not_corrupt_status_code() {
        let error = crate::errors::CliError::Api {
            message: "CSM returned HTTP 403 for variable '3' in namespace 'ns'".to_string(),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        };
        let redacted = redact_submitted_value_in_error(error, "3");
        match redacted {
            crate::errors::CliError::Api { ref message, .. } => {
                assert!(
                    message.contains("403"),
                    "HTTP status must survive redaction of short value: {message}"
                );
                assert!(
                    !message.contains("'3'"),
                    "quoted value must be redacted: {message}"
                );
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
    }

    /// A short value that appears in single quotes (the CSM echo shape)
    /// must still be redacted regardless of length.
    ///
    /// Contract: security/redaction — the delimited form is always the
    /// real leak path.
    #[test]
    fn redact_short_quoted_value_is_redacted() {
        let error = crate::errors::CliError::Api {
            message: "value '3' failed validation".to_string(),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        };
        let redacted = redact_submitted_value_in_error(error, "3");
        match redacted {
            crate::errors::CliError::Api { ref message, .. } => {
                assert!(
                    !message.contains("'3'"),
                    "quoted short value must be redacted: {message}"
                );
                assert!(
                    message.contains("[REDACTED]"),
                    "placeholder must appear: {message}"
                );
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
    }

    /// A long bare value (above the incidental-collision threshold) must
    /// still be redacted, preserving the existing behaviour for typical
    /// secret/variable values.
    ///
    /// Contract: security/redaction — long values are unlikely to collide
    /// with message infrastructure and must be scrubbed.
    #[test]
    fn redact_long_bare_value_is_still_redacted() {
        let error = crate::errors::CliError::Api {
            message: "error: value super-secret-123 failed validation".to_string(),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        };
        let redacted = redact_submitted_value_in_error(error, "super-secret-123");
        match redacted {
            crate::errors::CliError::Api { ref message, .. } => {
                assert!(
                    !message.contains("super-secret-123"),
                    "long bare value must be redacted: {message}"
                );
                assert!(
                    message.contains("[REDACTED]"),
                    "placeholder must appear: {message}"
                );
            }
            other => panic!("expected CliError::Api, got: {other:?}"),
        }
    }
}
