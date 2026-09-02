//! Pure merge-rule logic for `ags extend update-secret`'s upsert.
//!
//! Isolated from I/O so the merge rules (unset flags preserve existing
//! values; explicit flags override; secrets default to masked) can be
//! tested without a mock server.

use super::api::SecretRecord;

/// The `applyMask`/`description` fields to send in a create or update
/// request, after applying update-secret's merge rules.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EffectiveFields {
    pub(crate) apply_mask: bool,
    pub(crate) description: Option<String>,
}

/// Compute the effective fields for an **update** (existing record found).
///
/// `sensitive_override` is `Some(bool)` only when `--sensitive` was passed
/// on the command line (`None` if not supplied) — it carries the actual
/// `true`/`false` value the user gave, since `--sensitive false` must be
/// distinguishable from "not supplied". When not supplied, the existing
/// `applyMask` is preserved. `description_override` is the value of
/// `--description` if supplied (`None` if absent); when absent, the
/// existing description is preserved.
pub(crate) fn compute_effective_fields(
    existing: &SecretRecord,
    sensitive_override: Option<bool>,
    description_override: Option<String>,
) -> EffectiveFields {
    EffectiveFields {
        apply_mask: sensitive_override.unwrap_or(existing.apply_mask),
        description: description_override.or_else(|| existing.description.clone()),
    }
}

/// Compute the effective fields for a **create** (`--force`, no existing
/// record). There is no existing record to fall back to: an unsupplied
/// `--sensitive` defaults to `true` (secrets are masked by default — spec
/// acceptance criterion 2, unlike `update-var`'s `false` default for
/// variables), an unsupplied `--description` defaults to `None`.
pub(crate) fn compute_new_fields(
    sensitive_override: Option<bool>,
    description_override: Option<String>,
) -> EffectiveFields {
    EffectiveFields {
        apply_mask: sensitive_override.unwrap_or(true),
        description: description_override,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing(apply_mask: bool, description: Option<&str>) -> SecretRecord {
        SecretRecord {
            config_id: "id-1".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask,
            description: description.map(|d| d.to_string()),
        }
    }

    /// Mirror of T-UVAR-04 for secrets: unset `--sensitive` preserves the
    /// existing `applyMask`, not any flag-level default.
    #[test]
    fn update_unset_sensitive_preserves_existing_apply_mask_false() {
        let record = existing(false, None);
        let result = compute_effective_fields(&record, None, None);
        assert!(
            !result.apply_mask,
            "existing applyMask=false must be preserved, not defaulted to true"
        );
    }

    /// T-USEC-04: `--sensitive false` explicitly removes masking, even
    /// when the existing record was masked.
    #[test]
    fn update_explicit_sensitive_false_overrides_existing_true() {
        let record = existing(true, None);
        let result = compute_effective_fields(&record, Some(false), None);
        assert!(!result.apply_mask, "explicit --sensitive false must win");
    }

    #[test]
    fn update_explicit_sensitive_true_overrides_existing_false() {
        let record = existing(false, None);
        let result = compute_effective_fields(&record, Some(true), None);
        assert!(result.apply_mask);
    }

    #[test]
    fn update_unset_description_preserves_existing() {
        let record = existing(false, Some("keep me"));
        let result = compute_effective_fields(&record, None, None);
        assert_eq!(result.description.as_deref(), Some("keep me"));
    }

    #[test]
    fn update_explicit_description_overrides_existing() {
        let record = existing(false, Some("old"));
        let result = compute_effective_fields(&record, None, Some("new".to_string()));
        assert_eq!(result.description.as_deref(), Some("new"));
    }

    /// T-USEC-03: on create, an unsupplied --sensitive defaults to `true`
    /// for secrets — the opposite of `update-var`'s `false` default.
    #[test]
    fn create_unset_sensitive_defaults_to_true() {
        let result = compute_new_fields(None, None);
        assert!(
            result.apply_mask,
            "secrets must default to masked on create"
        );
        assert_eq!(result.description, None);
    }

    #[test]
    fn create_explicit_sensitive_false_overrides_default() {
        let result = compute_new_fields(Some(false), Some("desc".to_string()));
        assert!(!result.apply_mask);
        assert_eq!(result.description.as_deref(), Some("desc"));
    }
}
