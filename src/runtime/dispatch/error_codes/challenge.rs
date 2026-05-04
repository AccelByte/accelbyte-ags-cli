//! Challenge service error codes.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        99002 => Some(ErrorMapping {
            message: "Duplicate key.",
            reason: Some("A challenge with the same identifier already exists."),
            suggestion: Some("Use a different code or update the existing challenge."),
            tip: None,
        }),
        99003 => Some(ErrorMapping {
            message: "Challenge validation failed.",
            reason: Some("The challenge configuration is invalid."),
            suggestion: Some("Check the challenge fields against the schema and retry."),
            tip: None,
        }),
        99004 => Some(ErrorMapping {
            message: "Challenge request could not be processed.",
            reason: Some(
                "The request is well-formed but cannot be applied in the current challenge state.",
            ),
            suggestion: Some("Check the challenge state and the request payload."),
            tip: None,
        }),
        _ => None,
    }
}
