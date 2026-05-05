//! Standard error codes (`200xx` range) shared across all AccelByte services.
//!
//! These codes carry the same meaning regardless of which service returned
//! them, so a single entry per code is sufficient.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        20000 => Some(ErrorMapping {
            message: "Internal server error.",
            reason: Some("An unexpected error occurred on the server."),
            suggestion: None,
            tip: Some("If this persists, contact AccelByte support."),
        }),
        20001 => Some(ErrorMapping {
            message: "Request was not authorized.",
            reason: Some("Your access token is invalid or expired."),
            suggestion: Some("Run 'ags auth login'."),
            tip: None,
        }),
        20002 => Some(ErrorMapping {
            message: "Invalid request — validation failed.",
            reason: Some("One or more parameter values are invalid."),
            suggestion: Some("Check the parameter values and retry."),
            tip: None,
        }),
        20003 => Some(ErrorMapping {
            message: "Forbidden.",
            reason: Some("Your account does not have the required access."),
            suggestion: Some("Check that your account has the required role and permissions."),
            tip: Some("This operation may require admin or publisher-level access."),
        }),
        20006 => Some(ErrorMapping {
            message: "Resource has changed since you fetched it.",
            reason: Some("Another process updated this resource concurrently."),
            suggestion: Some("Fetch the latest version and retry."),
            tip: None,
        }),
        20007 => Some(ErrorMapping {
            message: "Too many requests.",
            reason: Some("You have exceeded the rate limit."),
            suggestion: Some("Wait a moment and retry."),
            tip: None,
        }),
        20008 => Some(ErrorMapping {
            message: "User not found.",
            reason: Some("The user does not exist or is not accessible in this context."),
            suggestion: Some("Run 'ags iam users search' to see available users."),
            tip: None,
        }),
        20009 => Some(ErrorMapping {
            message: "Request conflict.",
            reason: Some("The resource was modified concurrently."),
            suggestion: Some("Fetch the latest version and retry."),
            tip: None,
        }),
        20013 => Some(ErrorMapping {
            message: "You do not have permission for this operation.",
            reason: Some("Your account does not have the required role."),
            suggestion: Some("Check that your account has the required role and permissions."),
            tip: Some("This operation may require admin or publisher-level access."),
        }),
        20017 => Some(ErrorMapping {
            message: "User account is not linked.",
            reason: Some("The user has no linked account for the requested platform or context."),
            suggestion: Some("Link the platform account first, then retry."),
            tip: None,
        }),
        20018 => Some(ErrorMapping {
            message: "Request was rejected.",
            reason: Some("The server reported that the request is invalid."),
            suggestion: Some("Check the request fields and retry."),
            tip: None,
        }),
        20019 => Some(ErrorMapping {
            message: "Request body could not be parsed.",
            reason: Some("The request body is malformed or does not match the expected format."),
            suggestion: Some("Check the JSON payload and retry."),
            tip: None,
        }),
        20021 => Some(ErrorMapping {
            message: "Invalid pagination parameters.",
            reason: Some("The pagination offset or limit is out of range."),
            suggestion: Some("Check the --offset and --limit values."),
            tip: None,
        }),
        20022 => Some(ErrorMapping {
            message: "Token is not a user token.",
            reason: Some("This operation requires a user token, not a client token."),
            suggestion: Some("Run 'ags auth login' (browser-based) instead of client credentials."),
            tip: None,
        }),
        20024 => Some(ErrorMapping {
            message: "Not implemented.",
            reason: Some("This operation is not yet supported by the API."),
            suggestion: None,
            tip: Some("This endpoint may be available in a future API version."),
        }),
        20025 => Some(ErrorMapping {
            message: "Not a publisher account.",
            reason: Some("This operation requires a publisher namespace."),
            suggestion: Some("Ensure you are using a publisher namespace, not a game namespace."),
            tip: Some("Justice platform operations are only available in publisher namespaces."),
        }),
        20026 => Some(ErrorMapping {
            message: "Publisher namespace not allowed for this operation.",
            reason: Some("This operation must be performed in a game namespace, not the publisher namespace."),
            suggestion: Some("Switch to a game namespace and retry."),
            tip: None,
        }),
        20029 => Some(ErrorMapping {
            message: "Resource not found.",
            reason: Some("The requested resource does not exist."),
            suggestion: Some("Check the identifier and retry."),
            tip: None,
        }),
        _ => None,
    }
}
