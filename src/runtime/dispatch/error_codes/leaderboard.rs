//! Leaderboard service error codes.

use super::ErrorMapping;

/// Look up a curated error mapping for this service.
pub(super) fn lookup(error_code: i64) -> Option<ErrorMapping> {
    match error_code {
        71130 => Some(ErrorMapping {
            message: "Leaderboard configuration not found.",
            reason: Some("No leaderboard configuration matches the request."),
            suggestion: Some(
                "Run 'ags leaderboard configurations list' to see configured leaderboards.",
            ),
            tip: None,
        }),
        71132 => Some(ErrorMapping {
            message: "Leaderboard configuration already exists.",
            reason: Some("A leaderboard with this code is already configured."),
            suggestion: Some(
                "Use a different leaderboard code, or update the existing configuration.",
            ),
            tip: None,
        }),
        71133 => Some(ErrorMapping {
            message: "Leaderboard configuration has been deleted.",
            reason: Some("The configuration is in a deleted state and cannot be used."),
            suggestion: Some("Restore the configuration or create a new one."),
            tip: None,
        }),
        71230 => Some(ErrorMapping {
            message: "Leaderboard configuration not found.",
            reason: Some("The requested leaderboard configuration does not exist."),
            suggestion: Some(
                "Run 'ags leaderboard configurations list' to see configured leaderboards.",
            ),
            tip: None,
        }),
        71233 => Some(ErrorMapping {
            message: "User ranking data not found.",
            reason: Some("No ranking entry exists for this user on the requested leaderboard."),
            suggestion: Some("Confirm the user has submitted a score for this leaderboard."),
            tip: None,
        }),
        71235 => Some(ErrorMapping {
            message: "Leaderboard ranking not found.",
            reason: Some("No ranking data exists for this leaderboard."),
            suggestion: Some("Verify the leaderboard code and that scores have been submitted."),
            tip: None,
        }),
        71236 => Some(ErrorMapping {
            message: "Leaderboard ranking count failed.",
            reason: Some("The server failed to compute the ranking count."),
            suggestion: Some("Retry the command."),
            tip: Some("If this persists, contact AccelByte support."),
        }),
        71237 => Some(ErrorMapping {
            message: "Leaderboard is inactive — no ranking is generated.",
            reason: Some("Rankings are not produced for inactive leaderboards."),
            suggestion: Some("Activate the leaderboard before requesting rankings."),
            tip: None,
        }),
        71239 => Some(ErrorMapping {
            message: "Leaderboard is not archived.",
            reason: Some("The operation requires the leaderboard to be in an archived state."),
            suggestion: Some("Archive the leaderboard first."),
            tip: None,
        }),
        71241 => Some(ErrorMapping {
            message: "Operation forbidden in this environment.",
            reason: Some("This action is not allowed for the current environment."),
            suggestion: Some(
                "Run the command in the correct environment (e.g., dev/staging/live).",
            ),
            tip: None,
        }),
        71242 => Some(ErrorMapping {
            message: "Stat code not found in this namespace.",
            reason: Some("The leaderboard references a stat code that does not exist."),
            suggestion: Some("Run 'ags social stat-definitions list' to see configured stats."),
            tip: None,
        }),
        71243 => Some(ErrorMapping {
            message: "Cycle does not belong to this stat code.",
            reason: Some(
                "The supplied stat cycle is not associated with the leaderboard's stat code.",
            ),
            suggestion: Some("Use a cycle that matches the leaderboard's stat code."),
            tip: None,
        }),
        71244 => Some(ErrorMapping {
            message: "Stat cycle is already stopped.",
            reason: Some("This cycle has already been stopped and cannot be stopped again."),
            suggestion: None,
            tip: Some("No further action needed."),
        }),
        _ => None,
    }
}
