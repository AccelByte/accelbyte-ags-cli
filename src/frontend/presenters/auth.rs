//! Shared auth presentation helpers.

use crate::protocol::output::{AuthSource, Presence, TokenState};

/// Human-readable auth source label when one should be rendered explicitly.
pub(crate) fn human_source_label(source: AuthSource) -> Option<&'static str> {
    match source {
        AuthSource::EnvironmentAccessToken => Some("AGS_ACCESS_TOKEN environment variable"),
        AuthSource::EnvironmentClientCredentials => {
            Some("environment variables (client credentials)")
        }
        AuthSource::Stored => None,
    }
}

/// JSON auth source label when one should be rendered explicitly.
pub(crate) fn json_source_label(source: AuthSource) -> Option<&'static str> {
    match source {
        AuthSource::EnvironmentAccessToken | AuthSource::EnvironmentClientCredentials => {
            Some("environment")
        }
        AuthSource::Stored => None,
    }
}

/// Whether a token source should suppress all other human status rows.
pub(crate) fn is_source_only_status(source: AuthSource) -> bool {
    matches!(source, AuthSource::EnvironmentAccessToken)
}

/// Plain text label for a credential presence state.
pub(crate) fn presence_label(presence: Presence) -> &'static str {
    match presence {
        Presence::Stored => "stored",
        Presence::Cleared => "cleared",
        Presence::Missing => "not found",
        Presence::Unknown => "unknown",
    }
}

/// Plain text label for a token state.
pub(crate) fn token_state_label(state: &TokenState) -> &'static str {
    match state {
        TokenState::Valid { .. } => "valid",
        TokenState::Expired => "expired",
        TokenState::Missing => "not found",
        TokenState::Present => "present",
        TokenState::Unknown => "unknown",
    }
}

/// Optional expiry metadata for valid token states.
pub(crate) fn token_expiry_secs(state: &TokenState) -> Option<u64> {
    match state {
        TokenState::Valid {
            expires_in_secs: Some(expires_in_secs),
        } => Some(*expires_in_secs),
        _ => None,
    }
}
