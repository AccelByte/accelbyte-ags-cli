//! Legal service error codes.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        40035 => Some(ErrorMapping {
            message: "Legal policy version not found.",
            reason: Some("The requested policy version does not exist."),
            suggestion: Some("Run 'ags legal policies list' to see available policies."),
            tip: None,
        }),
        _ => None,
    }
}
