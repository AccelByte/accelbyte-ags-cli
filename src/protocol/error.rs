//! Runtime error types — the errors a `Runtime` method can return.

use serde::{Deserialize, Serialize};

/// A structured error surfaced by the runtime for any failing command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    pub message: String,
    pub details: Option<Box<ErrorDetails>>,
    pub hint: Option<String>,
    /// Verbose execution trace (resolution, request, response status) populated
    /// when `--verbose` is set so the frontend can render the same diagnostic
    /// block on the error path that it does on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Box<crate::protocol::output_views::ExecutionTrace>>,
}

/// High-level category of a runtime error, for branching in the renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RuntimeErrorKind {
    /// No credentials, or credentials failed verification.
    NotAuthenticated,
    /// Authenticated but not permitted for this operation.
    Forbidden,
    /// The target entity or route does not exist.
    NotFound,
    /// Request failed client-side validation before reaching the API.
    Validation,
    /// Server returned 400 or 422 — the API rejected the request.
    Rejected,
    /// Upstream service returned an error with an HTTP status + optional code.
    Upstream { status: u16, code: Option<String> },
    /// Transport-level failure (DNS, TCP, TLS, timeout).
    Network,
    /// The server returned a response that exceeds the client-side size limit.
    ResponseTooLarge,
    /// Any internal-consistency failure in the runtime itself.
    Internal,
}

/// Structured supplementary fields from an upstream error payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorDetails {
    pub code: Option<String>,
    pub reason: Option<String>,
    /// Additional explanatory detail shown below the main error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Whether the suggestion should be rendered as a direct fix or a next step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion_kind: Option<SuggestionKind>,
    /// Optional aside shown after the suggestion line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<String>,
}

/// How the suggestion line should be labelled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SuggestionKind {
    /// "Fix:" — the suggestion directly resolves the error.
    #[default]
    Fix,
    /// "Next:" — a next-step hint rather than a direct fix.
    Next,
}

/// Structured metadata that any error can carry to enrich user-facing output
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMetadata {
    /// High-level reason for the failure (e.g. "Token is expired")
    pub reason: Option<String>,
    /// Additional context shown below the main error message
    pub detail: Option<String>,
    /// Actionable fix or next-step shown on the suggestion line
    pub suggestion: Option<String>,
    /// Controls whether the suggestion is labelled "Fix:" or "Next:"
    pub suggestion_kind: SuggestionKind,
    /// Optional aside shown after the suggestion line
    pub tip: Option<String>,
    /// Verbose execution trace, populated when `--verbose` is set so the
    /// frontend can show the request/response diagnostic block on error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Box<crate::protocol::output_views::ExecutionTrace>>,
}

impl ErrorMetadata {
    /// Create metadata containing only a suggestion with the default Fix label
    pub fn with_suggestion(suggestion: impl Into<String>) -> Self {
        Self {
            reason: None,
            detail: None,
            suggestion: Some(suggestion.into()),
            suggestion_kind: SuggestionKind::Fix,
            tip: None,
            trace: None,
        }
    }
}

impl From<reqwest::Error> for RuntimeError {
    fn from(error: reqwest::Error) -> Self {
        RuntimeError {
            kind: RuntimeErrorKind::Network,
            message: format!("Network error: {error}"),
            details: None,
            hint: Some("Check network connectivity and retry.".to_string()),
            trace: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise `value` to JSON, parse it back, and assert equality — the contract test for protocol types.
    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &parsed);
    }

    #[test]
    fn test_runtime_error_implements_std_error() {
        /// Compile-time assertion helper: only callable when `E: std::error::Error`.
        fn assert_is_error<E: std::error::Error>(_: &E) {}
        let err = RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: "boom".to_string(),
            details: None,
            hint: None,
            trace: None,
        };
        assert_is_error(&err);
        let boxed: Box<dyn std::error::Error> = Box::new(err);
        assert_eq!(boxed.to_string(), "boom");
    }

    #[test]
    fn test_runtime_error_minimal_round_trip() {
        round_trip(&RuntimeError {
            kind: RuntimeErrorKind::NotAuthenticated,
            message: "No credentials configured.".to_string(),
            details: None,
            hint: Some("Run `ags auth login`.".to_string()),
            trace: None,
        });
    }

    #[test]
    fn test_runtime_error_full_round_trip() {
        round_trip(&RuntimeError {
            kind: RuntimeErrorKind::Upstream {
                status: 409,
                code: Some("20010".to_string()),
            },
            message: "User already exists.".to_string(),
            details: Some(Box::new(ErrorDetails {
                code: Some("20010".to_string()),
                reason: Some("Email address is already in use.".to_string()),
                detail: None,
                suggestion_kind: None,
                tip: None,
            })),
            hint: None,
            trace: None,
        });
    }

    #[test]
    fn test_runtime_error_kind_simple_variants_round_trip() {
        round_trip(&RuntimeErrorKind::NotAuthenticated);
        round_trip(&RuntimeErrorKind::Forbidden);
        round_trip(&RuntimeErrorKind::NotFound);
        round_trip(&RuntimeErrorKind::Validation);
        round_trip(&RuntimeErrorKind::Rejected);
        round_trip(&RuntimeErrorKind::Network);
        round_trip(&RuntimeErrorKind::ResponseTooLarge);
        round_trip(&RuntimeErrorKind::Internal);
    }

    #[test]
    fn test_runtime_error_kind_upstream_with_code_round_trip() {
        round_trip(&RuntimeErrorKind::Upstream {
            status: 500,
            code: Some("10001".to_string()),
        });
    }

    #[test]
    fn test_runtime_error_kind_upstream_without_code_round_trip() {
        round_trip(&RuntimeErrorKind::Upstream {
            status: 502,
            code: None,
        });
    }

    #[test]
    fn test_error_details_empty_fields_round_trip() {
        round_trip(&ErrorDetails {
            code: None,
            reason: None,
            detail: None,
            suggestion_kind: None,
            tip: None,
        });
    }

    #[test]
    fn test_error_details_populated_round_trip() {
        round_trip(&ErrorDetails {
            code: Some("10001".to_string()),
            reason: Some("Validation failed.".to_string()),
            detail: Some("Error code 10001".to_string()),
            suggestion_kind: Some(SuggestionKind::Fix),
            tip: Some("Check the payload fields and retry.".to_string()),
        });
    }
}
