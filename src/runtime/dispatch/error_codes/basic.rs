//! Basic service error codes.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        11121 => Some(ErrorMapping {
            message: "Category is not valid.",
            reason: Some("The supplied file category is not recognised."),
            suggestion: Some("Use one of the supported file categories."),
            tip: None,
        }),
        11131 => Some(ErrorMapping {
            message: "File type is not supported.",
            reason: Some("The uploaded file type is not allowed."),
            suggestion: Some("Use a supported file type."),
            tip: None,
        }),
        11132 => Some(ErrorMapping {
            message: "File storage limit exceeded.",
            reason: Some("The user has exceeded the namespace's file storage quota."),
            suggestion: Some(
                "Delete unused files or contact an administrator to increase the quota.",
            ),
            tip: None,
        }),
        11233 => Some(ErrorMapping {
            message: "Country group not found.",
            reason: Some("No country group exists with the supplied code."),
            suggestion: Some("Run 'ags basic country-groups list' to see configured groups."),
            tip: None,
        }),
        11234 => Some(ErrorMapping {
            message: "Country is already in another country group.",
            reason: Some("A country cannot belong to more than one country group at a time."),
            suggestion: Some(
                "Remove the country from its current group before adding it to a new one.",
            ),
            tip: None,
        }),
        11235 => Some(ErrorMapping {
            message: "Country group already exists.",
            reason: Some("A country group with the supplied code is already configured."),
            suggestion: Some("Use a different code or update the existing group."),
            tip: None,
        }),
        11336 => Some(ErrorMapping {
            message: "Namespace already exists.",
            reason: Some("A namespace with this identifier is already configured."),
            suggestion: Some("Use a different namespace name."),
            tip: None,
        }),
        11337 => Some(ErrorMapping {
            message: "Namespace not found.",
            reason: Some("The specified namespace does not exist."),
            suggestion: Some("Run 'ags basic namespaces list' to see available namespaces."),
            tip: None,
        }),
        11338 => Some(ErrorMapping {
            message: "Namespace contains invalid characters.",
            reason: Some("Namespace names allow only a restricted character set."),
            suggestion: Some("Use only alphanumeric characters and hyphens."),
            tip: None,
        }),
        11339 => Some(ErrorMapping {
            message: "Display name contains invalid characters.",
            reason: Some("The display name uses characters that are not allowed."),
            suggestion: Some("Remove special characters and retry."),
            tip: None,
        }),
        11340 => Some(ErrorMapping {
            message: "Game namespace limit reached for this studio.",
            reason: Some("The studio has hit its maximum number of game namespaces."),
            suggestion: Some(
                "Delete an unused game namespace, or contact AccelByte to raise the limit.",
            ),
            tip: None,
        }),
        11440 => Some(ErrorMapping {
            message: "User profile not found in this namespace.",
            reason: Some("The user has no profile in the requested namespace."),
            suggestion: Some("Create the profile first, or check the userId and namespace."),
            tip: None,
        }),
        11441 => Some(ErrorMapping {
            message: "User profile already exists.",
            reason: Some("A profile already exists for this user in this namespace."),
            suggestion: Some("Update the existing profile instead of creating a new one."),
            tip: None,
        }),
        11469 => Some(ErrorMapping {
            message: "User profile not found by public ID.",
            reason: Some("No profile matches the supplied publicId in this namespace."),
            suggestion: Some("Verify the publicId and namespace, then retry."),
            tip: None,
        }),
        11741 => Some(ErrorMapping {
            message: "Config not found.",
            reason: Some("The requested namespace config entry does not exist."),
            suggestion: Some("Run 'ags basic config list' to see configured entries."),
            tip: None,
        }),
        11771 => Some(ErrorMapping {
            message: "Config already exists.",
            reason: Some("A config entry with this key is already set."),
            suggestion: Some("Update the existing config or use a different key."),
            tip: None,
        }),
        _ => None,
    }
}
