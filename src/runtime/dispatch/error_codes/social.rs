//! Social service error codes (game profile, slots, stats).

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        // ── Game profile (12021–12041) ──
        12021 => Some(ErrorMapping {
            message: "Too many users in a single bulk request.",
            reason: Some("The request exceeds the per-call user limit."),
            suggestion: Some("Reduce the number of users per request and retry."),
            tip: None,
        }),
        12022 => Some(ErrorMapping {
            message: "Game profile attribute name mismatch.",
            reason: Some("The attribute name in the URL does not match the body."),
            suggestion: Some("Use the same attribute name in the URL and the request body."),
            tip: None,
        }),
        12041 => Some(ErrorMapping {
            message: "Game profile not found.",
            reason: Some("No game profile exists with the supplied ID."),
            suggestion: Some("Verify the profile ID and retry."),
            tip: None,
        }),

        // ── Slots (12121–12171) ──
        12121 => Some(ErrorMapping {
            message: "Slot file checksum mismatch.",
            reason: Some("The uploaded file did not match the expected checksum."),
            suggestion: Some("Re-upload the file to ensure data integrity."),
            tip: None,
        }),
        12122 => Some(ErrorMapping {
            message: "Slot file exceeds the upload size limit.",
            reason: Some("The file is larger than the configured slot limit."),
            suggestion: Some("Reduce the file size, or contact an administrator to raise the limit."),
            tip: None,
        }),
        12141 => Some(ErrorMapping {
            message: "Slot not found.",
            reason: Some("The specified slot does not exist in this namespace."),
            suggestion: Some("Verify the slot ID and namespace, then retry."),
            tip: None,
        }),
        12171 => Some(ErrorMapping {
            message: "User slot count limit exceeded.",
            reason: Some("The user has reached the maximum number of slots allowed."),
            suggestion: Some("Delete an unused slot, or raise the per-user slot limit in the namespace config."),
            tip: None,
        }),

        // ── Stats (12221–12279) ──
        12221 => Some(ErrorMapping {
            message: "Invalid stat operator.",
            reason: Some("The stat operator is not valid for this stat configuration."),
            suggestion: Some("Use an operator that matches the stat's configured update strategy."),
            tip: None,
        }),
        12222 => Some(ErrorMapping {
            message: "Invalid stats data for namespace.",
            reason: Some("The supplied stats data does not match the namespace configuration."),
            suggestion: Some("Check the stat codes and value types against the namespace configuration."),
            tip: None,
        }),
        12223 => Some(ErrorMapping {
            message: "Invalid stat codes for namespace.",
            reason: Some("One or more stat codes are not configured in this namespace."),
            suggestion: Some(
                "Run 'ags social stat-definitions list' to see configured stat codes.",
            ),
            tip: None,
        }),
        12225 => Some(ErrorMapping {
            message: "Invalid time range.",
            reason: Some("The supplied time range is invalid."),
            suggestion: Some("Ensure the start time precedes the end time and both are in ISO-8601 format."),
            tip: None,
        }),
        12226 => Some(ErrorMapping {
            message: "Invalid date for the specified month.",
            reason: Some("The day-of-month is out of range for the supplied month."),
            suggestion: Some("Provide a valid calendar date and retry."),
            tip: None,
        }),
        12241 => Some(ErrorMapping {
            message: "Stat not found in this namespace.",
            reason: Some("The specified stat is not configured."),
            suggestion: Some("Run 'ags social stat-definitions list' to see configured stats."),
            tip: None,
        }),
        12242 => Some(ErrorMapping {
            message: "Stat item not found for this user.",
            reason: Some("The user has no recorded value for the requested stat."),
            suggestion: Some("Submit an initial value for the stat first."),
            tip: None,
        }),
        12243 => Some(ErrorMapping {
            message: "Stats not found in this namespace.",
            reason: Some("No stats are configured in this namespace."),
            suggestion: Some("Configure stats for this namespace before querying them."),
            tip: None,
        }),
        12244 => Some(ErrorMapping {
            message: "Global stat item not found.",
            reason: Some("The requested global stat item does not exist in this namespace."),
            suggestion: Some("Verify the stat code and namespace, then retry."),
            tip: None,
        }),
        12245 => Some(ErrorMapping {
            message: "Stat cycle not found.",
            reason: Some("The specified stat cycle does not exist."),
            suggestion: Some("Run 'ags social stat-cycles list' to see configured cycles."),
            tip: None,
        }),
        12271 => Some(ErrorMapping {
            message: "Stat template already exists.",
            reason: Some("A stat template with this code is already configured."),
            suggestion: Some("Use a different code, or update the existing template."),
            tip: None,
        }),
        12273 => Some(ErrorMapping {
            message: "Stat is not decreasable.",
            reason: Some("This stat is configured as monotonically increasing."),
            suggestion: Some("Use an operator that does not decrease the value, or reconfigure the stat."),
            tip: None,
        }),
        12274 => Some(ErrorMapping {
            message: "Stat item already exists for this user.",
            reason: Some("The user already has a recorded value for this stat."),
            suggestion: Some("Update the existing stat item instead of creating a new one."),
            tip: None,
        }),
        12275 => Some(ErrorMapping {
            message: "Stat value is out of allowed range.",
            reason: Some("The supplied value is below the minimum or above the maximum configured for this stat."),
            suggestion: Some("Submit a value within the configured min/max bounds."),
            tip: None,
        }),
        12276 => Some(ErrorMapping {
            message: "Stat cannot be deleted while in INIT status.",
            reason: Some("Stats in the INIT lifecycle state cannot be deleted."),
            suggestion: Some("Advance the stat past INIT before attempting to delete."),
            tip: None,
        }),
        12277 => Some(ErrorMapping {
            message: "Stat cycle cannot be updated in its current status.",
            reason: Some("The cycle's lifecycle state does not permit updates."),
            suggestion: Some("Wait for the cycle to reach an updatable state, or use a different cycle."),
            tip: None,
        }),
        12279 => Some(ErrorMapping {
            message: "Invalid stat cycle status for this operation.",
            reason: Some("The cycle is not in a state that permits this operation."),
            suggestion: Some("Check the cycle's current status before retrying."),
            tip: None,
        }),
        _ => None,
    }
}
