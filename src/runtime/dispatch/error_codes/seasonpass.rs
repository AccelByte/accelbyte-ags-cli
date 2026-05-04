//! Season pass service error codes.
//!
//! Codes in the `30xxx`–`36xxx` range are shared with the platform service
//! and are defined in [`super::platform`]. This module owns the
//! season-pass-specific `49xxx` range plus a few configuration codes.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        // ── Configuration validation (49121–49124) ──
        49121 => Some(ErrorMapping {
            message: "Default language is required in localizations.",
            reason: Some("The default language is missing from the supplied localization map."),
            suggestion: Some("Include the namespace's default language in the localizations field."),
            tip: None,
        }),
        49122 => Some(ErrorMapping {
            message: "Invalid time range.",
            reason: Some("The supplied start/end time range is not valid."),
            suggestion: Some("Ensure the start time precedes the end time and both are in ISO-8601 format."),
            tip: None,
        }),
        49124 => Some(ErrorMapping {
            message: "Manual claim is not supported for this configuration.",
            reason: Some("The season pass is configured to use automatic claims."),
            suggestion: Some("Reconfigure the pass to support manual claims, or use the auto-claim flow."),
            tip: None,
        }),

        // ── Resource lookups (49141–49148) ──
        49141 => Some(ErrorMapping {
            message: "Tier item not found in the store.",
            reason: Some("The referenced tier item does not exist in this namespace's store."),
            suggestion: Some("Verify the tier item ID, or create the item in the store first."),
            tip: None,
        }),
        49142 => Some(ErrorMapping {
            message: "Pass item not found in the store.",
            reason: Some("The referenced pass item does not exist in this namespace's store."),
            suggestion: Some("Verify the pass item ID, or create the item in the store first."),
            tip: None,
        }),
        49143 => Some(ErrorMapping {
            message: "Season not found.",
            reason: Some("The specified season does not exist in this namespace."),
            suggestion: Some("Run 'ags seasonpass seasons list' to see configured seasons."),
            tip: None,
        }),
        49144 => Some(ErrorMapping {
            message: "Reward not found.",
            reason: Some("No reward exists with the supplied code."),
            suggestion: Some("Verify the reward code and retry."),
            tip: None,
        }),
        49145 => Some(ErrorMapping {
            message: "Pass not found.",
            reason: Some("No pass exists with the supplied code."),
            suggestion: Some("Verify the pass code and retry."),
            tip: None,
        }),
        49146 => Some(ErrorMapping {
            message: "Tier not found.",
            reason: Some("The specified tier does not exist in this season."),
            suggestion: Some("Verify the tier ID and retry."),
            tip: None,
        }),
        49147 => Some(ErrorMapping {
            message: "Published season not found.",
            reason: Some("No published season exists in this namespace."),
            suggestion: Some("Publish a season first, then retry."),
            tip: None,
        }),
        49148 => Some(ErrorMapping {
            message: "User season not found.",
            reason: Some("The user has no record for this season."),
            suggestion: Some("Confirm the user has been granted the pass or has earned tier progress."),
            tip: None,
        }),

        // ── Season state (49171–49189) ──
        49171 => Some(ErrorMapping {
            message: "Season is not updatable in its current status.",
            reason: Some("The season's lifecycle state does not permit updates."),
            suggestion: Some("Wait for the season to reach a draft or paused state before updating."),
            tip: None,
        }),
        49172 => Some(ErrorMapping {
            message: "Season has already ended.",
            reason: Some("The season is past its configured end time."),
            suggestion: Some("Use a different season, or extend this season's end time before retrying."),
            tip: None,
        }),
        49173 => Some(ErrorMapping {
            message: "Reward already exists in the season.",
            reason: Some("A reward with this code is already configured for the season."),
            suggestion: Some("Use a different code, or update the existing reward."),
            tip: None,
        }),
        49174 => Some(ErrorMapping {
            message: "Pass already exists in the season.",
            reason: Some("A pass with this code is already configured for the season."),
            suggestion: Some("Use a different code, or update the existing pass."),
            tip: None,
        }),
        49175 => Some(ErrorMapping {
            message: "Published season already exists.",
            reason: Some("A season is already published in this namespace."),
            suggestion: Some("Unpublish the existing season before publishing a new one."),
            tip: None,
        }),
        49176 => Some(ErrorMapping {
            message: "Season has no rewards configured.",
            reason: Some("Rewards must be defined before the season can be published."),
            suggestion: Some("Add at least one reward to the season."),
            tip: None,
        }),
        49177 => Some(ErrorMapping {
            message: "Season has no passes configured.",
            reason: Some("Passes must be defined before the season can be published."),
            suggestion: Some("Add at least one pass to the season."),
            tip: None,
        }),
        49178 => Some(ErrorMapping {
            message: "Season has no tiers configured.",
            reason: Some("Tiers must be defined before the season can be published."),
            suggestion: Some("Add at least one tier to the season."),
            tip: None,
        }),
        49179 => Some(ErrorMapping {
            message: "Reward is in use and cannot be removed.",
            reason: Some("The reward is referenced by one or more tiers."),
            suggestion: Some("Remove the reward from all tiers before deleting it."),
            tip: None,
        }),
        49180 => Some(ErrorMapping {
            message: "Season has already started.",
            reason: Some("The season is past its configured start time."),
            suggestion: Some("Some changes are not permitted once a season is live; consider creating a new season."),
            tip: None,
        }),
        49181 => Some(ErrorMapping {
            message: "Season has not ended.",
            reason: Some("This operation requires the season to be ended."),
            suggestion: Some("Wait for the season to end, or end it manually before retrying."),
            tip: None,
        }),
        49182 => Some(ErrorMapping {
            message: "Reward has already been claimed.",
            reason: Some("This reward is already in the user's inventory."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        49183 => Some(ErrorMapping {
            message: "Pass item does not match the published season pass.",
            reason: Some("The supplied pass item differs from the one published for this season."),
            suggestion: Some("Use the pass item that was configured at publish time."),
            tip: None,
        }),
        49184 => Some(ErrorMapping {
            message: "Tier item does not match the published season tier.",
            reason: Some("The supplied tier item differs from the one published for this season."),
            suggestion: Some("Use the tier item that was configured at publish time."),
            tip: None,
        }),
        49185 => Some(ErrorMapping {
            message: "Season has not started.",
            reason: Some("This operation requires the season to be active."),
            suggestion: Some("Wait until the season's start time, or adjust the start time."),
            tip: None,
        }),
        49186 => Some(ErrorMapping {
            message: "Pass is already owned.",
            reason: Some("The user already owns this pass for the current season."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        49187 => Some(ErrorMapping {
            message: "Maximum tier count exceeded.",
            reason: Some("Adding this tier would push the season past its tier limit."),
            suggestion: Some("Reduce the tier count, or contact AccelByte to raise the limit."),
            tip: None,
        }),
        49188 => Some(ErrorMapping {
            message: "Reward is currently being claimed.",
            reason: Some("Another claim operation for this reward is in progress."),
            suggestion: Some("Wait for the in-flight claim to complete, then retry."),
            tip: None,
        }),
        49189 => Some(ErrorMapping {
            message: "Duplicate season name in this namespace.",
            reason: Some("Another published season already uses this name."),
            suggestion: Some("Choose a unique season name."),
            tip: None,
        }),
        _ => None,
    }
}
