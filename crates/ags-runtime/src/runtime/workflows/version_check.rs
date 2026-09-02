//! Compares a workflow's declared `workflow_protocol_version` against the
//! workflow YAML protocol version this CLI build speaks
//! (`ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION`).

/// True when `declared` is present but does not parse as semver. Used by the
/// workflow route to distinguish "declared but garbled" from "declared and
/// parsable" — the former gets its own warning, the latter feeds
/// `is_mismatched` for a version-comparison check.
pub fn is_unparsable(declared: &str) -> bool {
    semver::Version::parse(declared).is_err()
}

/// True when `declared` parses as valid semver and is **not equal** to
/// `ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION` — in either direction
/// (older or newer). `None`, unparsable, or exactly-equal all resolve to
/// `false` — the safe default is silence, never a false-positive warning.
pub fn is_mismatched(declared: Option<&str>) -> bool {
    let Some(declared) = declared else {
        return false;
    };
    let Ok(declared_version) = semver::Version::parse(declared) else {
        return false;
    };
    let Ok(current_version) =
        semver::Version::parse(ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION)
    else {
        return false;
    };
    declared_version != current_version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_unparsable_false_for_valid_version() {
        assert!(!is_unparsable("1.0.0"));
    }

    #[test]
    fn test_is_unparsable_true_for_garbage_string() {
        assert!(is_unparsable("banana"));
    }

    #[test]
    fn test_is_unparsable_true_for_empty_string() {
        assert!(is_unparsable(""));
    }

    #[test]
    fn test_is_mismatched_false_when_absent() {
        assert!(!is_mismatched(None));
    }

    #[test]
    fn test_is_mismatched_false_when_unparsable() {
        assert!(!is_mismatched(Some("not-a-version")));
    }

    #[test]
    fn test_is_mismatched_true_when_older() {
        // "0.0.1" is guaranteed older than WORKFLOW_PROTOCOL_VERSION.
        assert!(is_mismatched(Some("0.0.1")));
    }

    #[test]
    fn test_is_mismatched_false_when_equal_to_current() {
        assert!(!is_mismatched(Some(
            ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION
        )));
    }

    #[test]
    fn test_is_mismatched_true_when_newer_than_current() {
        // "999.0.0" is guaranteed newer than WORKFLOW_PROTOCOL_VERSION. This
        // direction is the whole point of switching from an ordering check
        // to an equality check: under the old outdated-only semantics this
        // case resolved to "no warning"; here it must warn, same as the
        // older-version case above.
        assert!(is_mismatched(Some("999.0.0")));
    }
}
