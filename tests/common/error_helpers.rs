//! Test-only helpers that extract fields from a classified `RuntimeError`.
//!
//! These helpers keep error-focused assertions readable while
//! `reason`/`detail`/`tip` remain nested under `RuntimeError.details`.

use ags::errors::SuggestionKind;
use ags::protocol::error::RuntimeError;

/// Extract the reason string from a classified `RuntimeError`.
pub fn error_reason(error: &RuntimeError) -> Option<&str> {
    error.details.as_ref().and_then(|d| d.reason.as_deref())
}

/// Extract the detail string from a classified `RuntimeError`.
pub fn error_detail(error: &RuntimeError) -> Option<&str> {
    error.details.as_ref().and_then(|d| d.detail.as_deref())
}

/// Extract the tip string from a classified `RuntimeError`.
pub fn error_tip(error: &RuntimeError) -> Option<&str> {
    error.details.as_ref().and_then(|d| d.tip.as_deref())
}

/// Read the suggestion kind from a classified `RuntimeError`.
pub fn error_suggestion_kind(error: &RuntimeError) -> SuggestionKind {
    error
        .details
        .as_ref()
        .and_then(|d| d.suggestion_kind)
        .unwrap_or_default()
}
