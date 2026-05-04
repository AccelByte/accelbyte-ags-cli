//! Confirmation policy for potentially destructive dispatch operations.

use crate::protocol::catalogue::HttpMethod;

/// Decide whether a command needs an explicit confirmation prompt.
///
/// DELETE always confirms; POST/PUT/PATCH confirm only when the operation name
/// contains a "risky" keyword (e.g. "delete", "revoke", "reset"). GET never
/// confirms.
pub(crate) fn requires_confirmation(http_method: HttpMethod, operation_name: &str) -> bool {
    if matches!(http_method, HttpMethod::Delete) {
        return true;
    }
    if matches!(
        http_method,
        HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch
    ) {
        const RISKY_KEYWORDS: &[&str] = &[
            "delete", "ban", "revoke", "reset", "disable", "remove", "force",
        ];
        let lowered_name = operation_name.to_ascii_lowercase();
        return RISKY_KEYWORDS
            .iter()
            .any(|keyword| lowered_name.contains(keyword));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DELETE always requires confirmation regardless of the operation name
    #[test]
    fn test_delete_always_confirms() {
        assert!(requires_confirmation(HttpMethod::Delete, "list-items"));
        assert!(requires_confirmation(HttpMethod::Delete, "create-thing"));
    }

    /// GET never requires confirmation even when the name contains risky keywords
    #[test]
    fn test_get_never_confirms() {
        assert!(!requires_confirmation(HttpMethod::Get, "delete-user"));
        assert!(!requires_confirmation(HttpMethod::Get, "ban-user"));
    }

    /// POST requires confirmation when the operation name contains a risky keyword
    #[test]
    fn test_post_with_risky_keyword_confirms() {
        assert!(requires_confirmation(HttpMethod::Post, "ban-user"));
        assert!(requires_confirmation(HttpMethod::Post, "revoke-permission"));
        assert!(requires_confirmation(HttpMethod::Post, "reset-password"));
        assert!(requires_confirmation(HttpMethod::Post, "disable-user"));
        assert!(requires_confirmation(HttpMethod::Post, "remove-member"));
        assert!(requires_confirmation(HttpMethod::Post, "force-logout"));
        assert!(requires_confirmation(HttpMethod::Post, "delete-bulk"));
    }

    /// POST skips confirmation when the operation name has no risky keyword
    #[test]
    fn test_post_without_risky_keyword_skips() {
        assert!(!requires_confirmation(HttpMethod::Post, "create-role"));
        assert!(!requires_confirmation(HttpMethod::Post, "add-permission"));
        assert!(!requires_confirmation(HttpMethod::Post, "verify-user"));
        assert!(!requires_confirmation(HttpMethod::Post, "send-code"));
        assert!(!requires_confirmation(
            HttpMethod::Post,
            "register-resource"
        ));
    }

    /// PUT and PATCH apply the same risky-keyword rules as POST
    #[test]
    fn test_put_patch_same_rules_as_post() {
        assert!(requires_confirmation(HttpMethod::Put, "ban-user"));
        assert!(!requires_confirmation(HttpMethod::Put, "update-config"));
        assert!(requires_confirmation(HttpMethod::Patch, "revoke-session"));
        assert!(!requires_confirmation(HttpMethod::Patch, "update-profile"));
    }

    /// Risky keywords match anywhere in the operation name, not just at boundaries
    #[test]
    fn test_keyword_match_is_substring() {
        assert!(requires_confirmation(HttpMethod::Post, "force-delete-all"));
        assert!(requires_confirmation(HttpMethod::Post, "bulk-ban-users"));
        assert!(requires_confirmation(HttpMethod::Post, "admin-reset-mfa"));
    }

    /// Risky keyword matching ignores case so "Ban-User" still triggers
    #[test]
    fn test_keyword_match_is_case_insensitive() {
        assert!(requires_confirmation(HttpMethod::Post, "Ban-User"));
        assert!(requires_confirmation(HttpMethod::Post, "RESET-PASSWORD"));
    }
}
