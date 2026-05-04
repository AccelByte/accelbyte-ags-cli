//! Error types, metadata, and exit code mapping.

pub use crate::protocol::error::{ErrorMetadata, SuggestionKind};

/// Top-level error enum that maps each failure category to a distinct exit code
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Invalid input or misconfiguration (exit code 1)
    #[error("{message}")]
    Usage {
        message: String,
        metadata: Option<Box<ErrorMetadata>>,
    },
    /// Authentication or authorization failure (exit code 2)
    #[error("{message}")]
    Auth {
        message: String,
        metadata: Option<Box<ErrorMetadata>>,
    },
    /// AccelByte API returned an error response (exit code 3)
    #[error("{message}")]
    Api {
        message: String,
        metadata: Option<Box<ErrorMetadata>>,
    },
    /// Connection or transport-level failure (exit code 4)
    #[error("{message}")]
    Network {
        message: String,
        metadata: Option<Box<ErrorMetadata>>,
    },
    /// Unexpected internal error — a bug or unhandled condition (exit code 5).
    /// Uses `#[error("{0}")]` rather than `#[error(transparent)]` because
    /// `anyhow::Error` does not implement `std::error::Error`. The `From`
    /// impl below is also manual for the same reason — `#[from]` would
    /// auto-add `#[source]` which requires `Error` on the inner type.
    #[error("{0}")]
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for CliError {
    fn from(e: anyhow::Error) -> Self {
        CliError::Internal(e)
    }
}

/// Structured, frontend-agnostic view of a `CliError` for rendering.
///
/// Every `Frontend` implementation consumes this; `CliError` itself no longer
/// knows how to render. This keeps the error type decoupled from the
/// presentation layer.
#[derive(Debug, Clone)]
pub struct ErrorView {
    pub message: String,
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub suggestion: Option<String>,
    pub suggestion_kind: SuggestionKind,
    pub tip: Option<String>,
    pub exit_code: i32,
    pub trace: Option<Box<crate::protocol::output_views::ExecutionTrace>>,
}

impl CliError {
    /// Return the numeric exit code for this error category
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Usage { .. } => 1,
            CliError::Auth { .. } => 2,
            CliError::Api { .. } => 3,
            CliError::Network { .. } => 4,
            CliError::Internal(_) => 5,
        }
    }

    /// Project the error into a structured `ErrorView` for a frontend to render.
    pub fn view(&self) -> ErrorView {
        let exit_code = self.exit_code();
        match self {
            CliError::Usage { message, metadata }
            | CliError::Auth { message, metadata }
            | CliError::Api { message, metadata }
            | CliError::Network { message, metadata } => {
                let meta = metadata.as_deref();
                ErrorView {
                    message: message.clone(),
                    reason: meta.and_then(|m| m.reason.clone()),
                    detail: meta.and_then(|m| m.detail.clone()),
                    suggestion: meta.and_then(|m| m.suggestion.clone()),
                    suggestion_kind: meta.map_or(SuggestionKind::default(), |m| m.suggestion_kind),
                    tip: meta.and_then(|m| m.tip.clone()),
                    exit_code,
                    trace: meta.and_then(|m| m.trace.clone()),
                }
            }
            CliError::Internal(err) => ErrorView {
                message: format!("{err}"),
                reason: None,
                detail: None,
                suggestion: None,
                suggestion_kind: SuggestionKind::default(),
                tip: None,
                exit_code,
                trace: None,
            },
        }
    }
}

impl From<crate::protocol::error::RuntimeError> for CliError {
    fn from(error: crate::protocol::error::RuntimeError) -> Self {
        use crate::protocol::error::RuntimeErrorKind;

        let suggestion_kind = error
            .details
            .as_ref()
            .and_then(|details| details.suggestion_kind)
            .unwrap_or_default();

        let detail = error
            .details
            .as_ref()
            .and_then(|details| details.detail.clone());

        let tip = error
            .details
            .as_ref()
            .and_then(|details| details.tip.clone());

        let reason = error
            .details
            .as_ref()
            .and_then(|details| details.reason.clone());

        let metadata = Some(Box::new(ErrorMetadata {
            reason,
            detail,
            suggestion: error.hint,
            suggestion_kind,
            tip,
            trace: error.trace,
        }));

        match error.kind {
            RuntimeErrorKind::Validation => CliError::Usage {
                message: error.message,
                metadata,
            },
            RuntimeErrorKind::Network => CliError::Network {
                message: error.message,
                metadata,
            },
            RuntimeErrorKind::ResponseTooLarge => CliError::Api {
                message: error.message,
                metadata,
            },
            RuntimeErrorKind::Internal => {
                // Internal invariants become boxed anyhow errors (exit code 5).
                // Metadata is dropped on this path because CliError::Internal(anyhow::Error)
                // doesn't carry structured metadata — the anyhow chain will show the message.
                let _ = metadata;
                CliError::Internal(anyhow::anyhow!("{}", error.message))
            }
            RuntimeErrorKind::NotAuthenticated => CliError::Auth {
                message: error.message,
                metadata,
            },
            RuntimeErrorKind::Rejected
            | RuntimeErrorKind::Forbidden
            | RuntimeErrorKind::NotFound
            | RuntimeErrorKind::Upstream { .. } => CliError::Api {
                message: error.message,
                metadata,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Usage errors return exit code 1 so callers can distinguish bad input from other failures
    #[test]
    fn test_exit_code_usage() {
        assert_eq!(
            CliError::Usage {
                message: "bad input".into(),
                metadata: None
            }
            .exit_code(),
            1
        );
    }

    /// Auth errors return exit code 2 so scripts can trigger re-authentication
    #[test]
    fn test_exit_code_auth() {
        assert_eq!(
            CliError::Auth {
                message: "no token".into(),
                metadata: None
            }
            .exit_code(),
            2
        );
    }

    /// Attaching metadata does not change the exit code — category alone determines it
    #[test]
    fn test_exit_code_auth_with_metadata() {
        let err = CliError::Auth {
            message: "msg".into(),
            metadata: Some(Box::new(ErrorMetadata::with_suggestion(
                "Run 'ags auth login'.",
            ))),
        };
        assert_eq!(err.exit_code(), 2);
    }

    /// API errors return exit code 3 so callers know the server rejected the request
    #[test]
    fn test_exit_code_api() {
        assert_eq!(
            CliError::Api {
                message: "forbidden".into(),
                metadata: None
            }
            .exit_code(),
            3
        );
    }

    /// Internal errors return exit code 5 to signal an unexpected bug rather than a user mistake
    #[test]
    fn test_exit_code_internal() {
        let err = CliError::Internal(anyhow::anyhow!("unexpected"));
        assert_eq!(err.exit_code(), 5);
    }

    /// The suggestion constructor sets only the suggestion field, leaving all others None
    #[test]
    fn test_error_metadata_suggestion_constructor() {
        let m = ErrorMetadata::with_suggestion("Run 'ags auth login'.");
        assert_eq!(m.suggestion.as_deref(), Some("Run 'ags auth login'."));
        assert!(m.reason.is_none());
        assert!(m.detail.is_none());
        assert!(m.tip.is_none());
    }

    /// All metadata fields can be set via struct literal when both constructors are too narrow
    #[test]
    fn test_error_metadata_full_construction() {
        let m = ErrorMetadata {
            reason: Some("r".into()),
            detail: Some("d".into()),
            suggestion: Some("s".into()),
            tip: Some("t".into()),
            ..Default::default()
        };
        assert_eq!(m.detail.as_deref(), Some("d"));
    }

    mod runtime_error_conversion {
        use super::*;
        use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

        /// Build a placeholder `RuntimeError` so each test only varies the kind it cares about.
        fn make(kind: RuntimeErrorKind) -> RuntimeError {
            RuntimeError {
                kind,
                message: "test message".to_string(),
                details: None,
                hint: None,
                trace: None,
            }
        }

        /// Validation errors must map to Usage (exit 1) — client caught bad local input before the API call
        #[test]
        fn test_validation_maps_to_usage_exit_1() {
            let err: CliError = make(RuntimeErrorKind::Validation).into();
            assert!(matches!(err, CliError::Usage { .. }));
            assert_eq!(err.exit_code(), 1);
        }

        /// Server-side rejections (400/422) must map to Api (exit 3) — the server rejected the request
        #[test]
        fn test_rejected_maps_to_api_exit_3() {
            let err: CliError = make(RuntimeErrorKind::Rejected).into();
            assert!(matches!(err, CliError::Api { .. }));
            assert_eq!(err.exit_code(), 3);
        }

        /// NotAuthenticated maps to Auth (exit 2) so scripts can trigger re-authentication
        #[test]
        fn test_not_authenticated_maps_to_auth_exit_2() {
            let err: CliError = make(RuntimeErrorKind::NotAuthenticated).into();
            assert!(matches!(err, CliError::Auth { .. }));
            assert_eq!(err.exit_code(), 2);
        }

        /// Forbidden must map to Api (exit 3) — the server rejected an authorized request
        #[test]
        fn test_forbidden_maps_to_api_exit_3() {
            let err: CliError = make(RuntimeErrorKind::Forbidden).into();
            assert!(matches!(err, CliError::Api { .. }));
            assert_eq!(err.exit_code(), 3);
        }

        /// NotFound must map to Api (exit 3) — distinct from Usage so scripts can retry safely
        #[test]
        fn test_not_found_maps_to_api_exit_3() {
            let err: CliError = make(RuntimeErrorKind::NotFound).into();
            assert!(matches!(err, CliError::Api { .. }));
            assert_eq!(err.exit_code(), 3);
        }

        /// Upstream failures must map to Api (exit 3) regardless of status/code payload
        #[test]
        fn test_upstream_maps_to_api_exit_3() {
            let err: CliError = make(RuntimeErrorKind::Upstream {
                status: 502,
                code: Some("SERVICE_UNAVAILABLE".to_string()),
            })
            .into();
            assert!(matches!(err, CliError::Api { .. }));
            assert_eq!(err.exit_code(), 3);
        }

        /// Network failures must map to Network (exit 4) so retries can be triggered automatically
        #[test]
        fn test_network_maps_to_network_exit_4() {
            let err: CliError = make(RuntimeErrorKind::Network).into();
            assert!(matches!(err, CliError::Network { .. }));
            assert_eq!(err.exit_code(), 4);
        }

        /// Internal invariants must map to Internal (exit 5) to signal a bug rather than user error
        #[test]
        fn test_internal_maps_to_internal_exit_5() {
            let err: CliError = make(RuntimeErrorKind::Internal).into();
            assert!(matches!(err, CliError::Internal(_)));
            assert_eq!(err.exit_code(), 5);
        }

        /// Verbose execution trace attached to a RuntimeError must propagate
        /// through the conversion to CliError and surface in the rendered
        /// ErrorView, so the human frontend can show the request/response
        /// diagnostic block on error paths under `--verbose`.
        #[test]
        fn test_runtime_error_trace_propagates_to_error_view() {
            use crate::protocol::output_views::{
                ExecutionTrace, RequestTrace, ResolutionTrace, ResponseTrace,
            };

            let trace = ExecutionTrace {
                resolution: Some(ResolutionTrace {
                    spec_source: "IAM loaded from cache".to_string(),
                    profile: ("dev-private".to_string(), "global config".to_string()),
                    base_url: ("https://example".to_string(), "profile".to_string()),
                    namespace: Some(("philtest".to_string(), "--namespace flag".to_string())),
                    token_source: "stored".to_string(),
                    token_expiry: Some("expires in 1h".to_string()),
                }),
                request: RequestTrace {
                    http_method: "GET".to_string(),
                    url: "https://example/path".to_string(),
                    query_params: vec![],
                    has_auth_header: true,
                    body_size: None,
                },
                response: Some(ResponseTrace {
                    status: 503,
                    reason: None,
                    body_size: Some(0),
                }),
            };

            let mut runtime_err = make(RuntimeErrorKind::Upstream {
                status: 503,
                code: None,
            });
            runtime_err.trace = Some(Box::new(trace.clone()));

            let cli_err: CliError = runtime_err.into();
            let view = cli_err.view();

            assert!(view.trace.is_some(), "trace should propagate to ErrorView");
            assert_eq!(
                view.trace.as_deref(),
                Some(&trace),
                "the propagated trace should match what was attached"
            );
        }
    }
}
