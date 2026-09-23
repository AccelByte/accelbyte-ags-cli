//! Error types, metadata, and exit code mapping.

pub use ags_protocol::error::{ErrorMetadata, SuggestionKind};

/// Sub-classification of `CliError::Api`, preserved from the originating
/// `RuntimeErrorKind` at conversion time. `CliError::Api` has exactly one real
/// constructor (`From<RuntimeError>` below), so this field is always populated
/// correctly there; nothing else constructs `CliError::Api` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorCategory {
    /// 403 — authenticated but not permitted.
    Permission,
    /// 404 — target entity or route does not exist.
    NotFound,
    /// 400/422 — the server rejected the request shape.
    Rejected,
    /// A client-side `--wait` limit elapsed before the operation reached a
    /// terminal state. Distinct from `Upstream` so a CI caller can tell "the
    /// rollout failed, do not retry" (a failed terminal state → `Upstream`,
    /// exit 3) apart from "the wait timed out, the operation may still land"
    /// (this → its own exit code). The server returned no error here; the CLI
    /// simply stopped waiting.
    Timeout,
    /// Any other upstream HTTP status (5xx, or an unmapped 4xx).
    Upstream,
}

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
        category: ApiErrorCategory,
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
    pub trace: Option<Box<ags_protocol::output_views::ExecutionTrace>>,
}

impl CliError {
    /// Return the numeric exit code for this error category.
    ///
    /// Each `CliError` variant maps to a distinct code; the one place a
    /// sub-category refines the code is `ApiErrorCategory::Timeout`, which
    /// exits 6 rather than the generic Api 3. A `--wait` that timed out is not
    /// the same as a rollout the server actively failed — a CI caller keys
    /// "may still land, safe to re-check" (6) apart from "failed, do not blindly
    /// retry" (3) on the exit code alone, without string-matching messages.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Usage { .. } => 1,
            CliError::Auth { .. } => 2,
            CliError::Api {
                category: ApiErrorCategory::Timeout,
                ..
            } => 6,
            CliError::Api { .. } => 3,
            CliError::Network { .. } => 4,
            CliError::Internal(_) => 5,
        }
    }

    /// Stable, coarse error taxonomy for telemetry — never the raw message
    /// (which may embed user input). Matches the dashboard's friction-tile
    /// taxonomy (see `ags-telemetry-metrics-dashboard-design.md` §5/§9.4).
    pub fn telemetry_class(&self) -> &'static str {
        match self {
            CliError::Usage { .. } => "usage",
            CliError::Auth { .. } => "auth",
            CliError::Api { category, .. } => match category {
                ApiErrorCategory::Permission => "permission",
                ApiErrorCategory::NotFound => "not_found",
                ApiErrorCategory::Rejected => "rejected",
                ApiErrorCategory::Timeout => "timeout",
                ApiErrorCategory::Upstream => "upstream",
            },
            CliError::Network { .. } => "network",
            CliError::Internal(_) => "internal",
        }
    }

    /// Borrow the error's structured metadata, when it has any. `None` for
    /// `CliError::Internal`, which carries no structured metadata at all.
    pub fn metadata(&self) -> Option<&ErrorMetadata> {
        match self {
            CliError::Usage { metadata, .. }
            | CliError::Auth { metadata, .. }
            | CliError::Api { metadata, .. }
            | CliError::Network { metadata, .. } => metadata.as_deref(),
            CliError::Internal(_) => None,
        }
    }

    /// Project the error into a structured `ErrorView` for a frontend to render.
    pub fn view(&self) -> ErrorView {
        let exit_code = self.exit_code();
        match self {
            CliError::Usage { message, metadata }
            | CliError::Auth { message, metadata }
            | CliError::Api {
                message, metadata, ..
            }
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

impl From<ags_protocol::error::RuntimeError> for CliError {
    fn from(error: ags_protocol::error::RuntimeError) -> Self {
        use ags_protocol::error::RuntimeErrorKind;

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

        // Telemetry-only facts: the kind is authoritative for an upstream
        // status/code pair; otherwise fall back to a client-side code recorded
        // in `details.code` (e.g. a `--no-input` rejection). Never rendered.
        let (http_status, kind_code) = match &error.kind {
            RuntimeErrorKind::Upstream { status, code } => (Some(*status), code.clone()),
            _ => (None, None),
        };
        let code = kind_code.or_else(|| {
            error
                .details
                .as_ref()
                .and_then(|details| details.code.clone())
        });

        let metadata = Some(Box::new(ErrorMetadata {
            reason,
            detail,
            suggestion: error.hint,
            suggestion_kind,
            tip,
            code,
            http_status,
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
            // A client-side size guard, but bucketed as `upstream`
            // telemetry-wise since none of the other categories fit better
            // and it still represents an API response the CLI couldn't
            // fully process.
            RuntimeErrorKind::ResponseTooLarge => CliError::Api {
                message: error.message,
                metadata,
                category: ApiErrorCategory::Upstream,
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
            RuntimeErrorKind::Forbidden => CliError::Api {
                message: error.message,
                metadata,
                category: ApiErrorCategory::Permission,
            },
            RuntimeErrorKind::NotFound => CliError::Api {
                message: error.message,
                metadata,
                category: ApiErrorCategory::NotFound,
            },
            RuntimeErrorKind::Rejected => CliError::Api {
                message: error.message,
                metadata,
                category: ApiErrorCategory::Rejected,
            },
            RuntimeErrorKind::Upstream { .. } => CliError::Api {
                message: error.message,
                metadata,
                category: ApiErrorCategory::Upstream,
            },
        }
    }
}

/// Read a single line from stdin via the shared sanitizing reader and map
/// errors to [`CliError::Usage`].
///
/// Three callers need this conversion: `update-secret`, `update-var`, and
/// `auth login --client-secret-stdin`. The IO call is one line; the real
/// value is the shared error mapping — keeping the messages identical across
/// all call sites so a wording change never needs three lockstep edits.
///
/// Lives in `errors.rs` (alongside the existing `From<RuntimeError>` and
/// `From<anyhow::Error>` conversions) because its purpose is bridging
/// a runtime error type to a CLI error type. The one-line IO call is a
/// convenience that keeps every call site to a single function call.
pub(crate) fn read_stdin_line() -> Result<String, CliError> {
    ags_runtime::support::strings::read_stdin_line().map_err(map_stdin_line_error)
}

/// Map a [`StdinLineError`](ags_runtime::support::strings::StdinLineError)
/// to [`CliError::Usage`].
fn map_stdin_line_error(e: ags_runtime::support::strings::StdinLineError) -> CliError {
    match e {
        ags_runtime::support::strings::StdinLineError::Io(io_err) => CliError::Usage {
            message: format!("Failed to read from stdin: {io_err}"),
            metadata: None,
        },
        ags_runtime::support::strings::StdinLineError::Empty => CliError::Usage {
            message: "Expected a value from stdin but got empty input".to_string(),
            metadata: None,
        },
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
                metadata: None,
                category: ApiErrorCategory::Permission,
            }
            .exit_code(),
            3
        );
    }

    /// A `--wait` timeout returns exit code 6 — distinct from a server-failed
    /// rollout (Api/Upstream, exit 3) so a CI caller can tell "timed out, may
    /// still land" from "failed, do not retry" on the exit code alone.
    #[test]
    fn test_exit_code_api_timeout_is_distinct_from_upstream() {
        let timeout = CliError::Api {
            message: "timeout waiting for deployment".into(),
            metadata: None,
            category: ApiErrorCategory::Timeout,
        };
        let upstream = CliError::Api {
            message: "deployment failed".into(),
            metadata: None,
            category: ApiErrorCategory::Upstream,
        };
        assert_eq!(timeout.exit_code(), 6);
        assert_eq!(upstream.exit_code(), 3);
        assert_ne!(timeout.exit_code(), upstream.exit_code());
    }

    /// The timeout category has its own telemetry class, so a timed-out wait is
    /// not collapsed into the generic `upstream` bucket.
    #[test]
    fn test_timeout_telemetry_class_is_timeout() {
        let err = CliError::Api {
            message: "timeout".into(),
            metadata: None,
            category: ApiErrorCategory::Timeout,
        };
        assert_eq!(err.telemetry_class(), "timeout");
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
        use ags_protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};

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
            use ags_protocol::output_views::{
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

        /// Forbidden must classify as the `permission` telemetry class, distinct from a
        /// generic API error — the dashboard's friction panel needs auth vs. permission
        /// vs. not-found broken out, not collapsed into one `api` bucket.
        #[test]
        fn test_forbidden_telemetry_class_is_permission() {
            let err: CliError = make(RuntimeErrorKind::Forbidden).into();
            assert_eq!(err.telemetry_class(), "permission");
        }

        #[test]
        fn test_not_found_telemetry_class_is_not_found() {
            let err: CliError = make(RuntimeErrorKind::NotFound).into();
            assert_eq!(err.telemetry_class(), "not_found");
        }

        #[test]
        fn test_rejected_telemetry_class_is_rejected() {
            let err: CliError = make(RuntimeErrorKind::Rejected).into();
            assert_eq!(err.telemetry_class(), "rejected");
        }

        #[test]
        fn test_upstream_telemetry_class_is_upstream() {
            let err: CliError = make(RuntimeErrorKind::Upstream {
                status: 502,
                code: None,
            })
            .into();
            assert_eq!(err.telemetry_class(), "upstream");
        }

        /// `ResponseTooLarge` is a client-side size guard, not a server
        /// failure, but it still bucketed as `upstream` telemetry-wise since
        /// none of the other categories fit better and it represents an API
        /// response the CLI couldn't fully process. See the doc comment on
        /// the `RuntimeErrorKind::ResponseTooLarge` match arm in the
        /// `From<RuntimeError> for CliError` impl above for the full
        /// rationale.
        #[test]
        fn test_response_too_large_telemetry_class_is_upstream() {
            let err: CliError = make(RuntimeErrorKind::ResponseTooLarge).into();
            assert_eq!(err.telemetry_class(), "upstream");
        }

        #[test]
        fn test_validation_telemetry_class_is_usage() {
            let err: CliError = make(RuntimeErrorKind::Validation).into();
            assert_eq!(err.telemetry_class(), "usage");
        }

        #[test]
        fn test_not_authenticated_telemetry_class_is_auth() {
            let err: CliError = make(RuntimeErrorKind::NotAuthenticated).into();
            assert_eq!(err.telemetry_class(), "auth");
        }

        #[test]
        fn test_network_telemetry_class_is_network() {
            let err: CliError = make(RuntimeErrorKind::Network).into();
            assert_eq!(err.telemetry_class(), "network");
        }

        #[test]
        fn test_internal_telemetry_class_is_internal() {
            let err = CliError::Internal(anyhow::anyhow!("boom"));
            assert_eq!(err.telemetry_class(), "internal");
        }

        /// `RuntimeErrorKind::telemetry_class` and `CliError::telemetry_class`
        /// must never drift apart — they are two independent classifications
        /// of the same failure and telemetry consumers assume they agree.
        #[test]
        fn test_cli_error_class_matches_runtime_kind_class() {
            for kind in [
                RuntimeErrorKind::NotAuthenticated,
                RuntimeErrorKind::Forbidden,
                RuntimeErrorKind::NotFound,
                RuntimeErrorKind::Validation,
                RuntimeErrorKind::Rejected,
                RuntimeErrorKind::Network,
                RuntimeErrorKind::Internal,
            ] {
                let expected = kind.telemetry_class();
                let err: CliError = make(kind).into();
                assert_eq!(err.telemetry_class(), expected);
            }
        }

        /// The HTTP status and error code from an `Upstream` kind must reach
        /// `ErrorMetadata` verbatim so telemetry can report them without
        /// re-parsing the rendered message.
        #[test]
        fn test_upstream_error_carries_status_and_code_into_metadata() {
            let err: CliError = RuntimeError {
                kind: RuntimeErrorKind::Upstream {
                    status: 409,
                    code: Some("20013".to_string()),
                },
                message: "conflict".to_string(),
                details: None,
                hint: None,
                trace: None,
            }
            .into();
            let CliError::Api { metadata, .. } = &err else {
                panic!("expected CliError::Api, got {err:?}");
            };
            let metadata = metadata.as_deref().expect("metadata must be present");
            assert_eq!(metadata.http_status, Some(409));
            assert_eq!(metadata.code, Some("20013".to_string()));
        }

        /// When the kind carries no status/code (e.g. a client-side
        /// `Validation` rejection), a code recorded in `details.code` must
        /// still reach `ErrorMetadata` so telemetry can see it.
        #[test]
        fn test_details_code_reaches_metadata_when_kind_carries_none() {
            let err: CliError = RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: "cannot run non-interactively".to_string(),
                details: Some(Box::new(ErrorDetails {
                    code: Some("no_input.missing_input".to_string()),
                    reason: None,
                    detail: None,
                    suggestion_kind: None,
                    tip: None,
                })),
                hint: None,
                trace: None,
            }
            .into();
            let CliError::Usage { metadata, .. } = &err else {
                panic!("expected CliError::Usage, got {err:?}");
            };
            let metadata = metadata.as_deref().expect("metadata must be present");
            assert_eq!(metadata.code, Some("no_input.missing_input".to_string()));
            assert_eq!(metadata.http_status, None);
        }
    }

    // ── map_stdin_line_error ──

    mod stdin_line_error_mapping {
        use super::*;

        #[test]
        fn io_error_maps_to_usage_with_message() {
            let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe");
            let cli_err =
                map_stdin_line_error(ags_runtime::support::strings::StdinLineError::Io(io_err));
            match cli_err {
                CliError::Usage { ref message, .. } => {
                    assert!(
                        message.starts_with("Failed to read from stdin:"),
                        "message must describe the IO failure: {message}"
                    );
                    assert!(
                        message.contains("broken pipe"),
                        "message must include the inner error: {message}"
                    );
                }
                other => panic!("expected CliError::Usage, got: {other:?}"),
            }
        }

        #[test]
        fn empty_input_maps_to_usage_with_exact_message() {
            let cli_err =
                map_stdin_line_error(ags_runtime::support::strings::StdinLineError::Empty);
            match cli_err {
                CliError::Usage { ref message, .. } => {
                    assert_eq!(
                        message, "Expected a value from stdin but got empty input",
                        "empty-input message must match the exact wording all call sites expect"
                    );
                }
                other => panic!("expected CliError::Usage, got: {other:?}"),
            }
        }
    }
}
