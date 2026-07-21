//! Deprecated command aliases for renamed operations.
//!
//! When an `x-operationId`'s method segment is corrected (e.g. a typo fix), the
//! derived CLI command name changes — a breaking change for anyone scripting the
//! old name. To keep those invocations working, the former name is kept as a
//! hidden Clap alias (so the old command still parses) and matched during method
//! lookup (so it still resolves to the corrected operation).
//!
//! This table is the single source of truth for those renames. An alias is a CLI
//! back-compat concern, not an API concept, so it lives here in the catalogue
//! layer rather than in the bundled OpenAPI specs — which carry the corrected
//! `x-operationId` and may be regenerated from upstream.
//!
//! Keyed by `(internal service id, resource, current method name)`.

/// Former CLI method names retained as hidden aliases for a current method.
/// Returns an empty slice when the method has never been renamed.
pub fn former_method_names(service: &str, resource: &str, method: &str) -> &'static [&'static str] {
    match (service, resource, method) {
        // platform: corrected `delete-publisheed` typo (stores).
        ("platform", "stores", "delete-published") => &["delete-publisheed"],
        // platform: corrected `…by-app-d` typo (entitlements).
        ("platform", "entitlements", "check-my-ownership-by-app-id") => {
            &["check-my-ownership-by-app-d"]
        }
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_former_names_for_renamed_platform_ops() {
        assert_eq!(
            former_method_names("platform", "stores", "delete-published"),
            ["delete-publisheed"]
        );
        assert_eq!(
            former_method_names("platform", "entitlements", "check-my-ownership-by-app-id"),
            ["check-my-ownership-by-app-d"]
        );
    }

    #[test]
    fn test_no_aliases_for_unrenamed_method() {
        assert!(former_method_names("platform", "stores", "get").is_empty());
        assert!(former_method_names("iam", "users", "get").is_empty());
        // Wrong service/resource for a real alias yields nothing.
        assert!(former_method_names("iam", "stores", "delete-published").is_empty());
    }
}
