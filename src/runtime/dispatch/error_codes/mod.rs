//! Lookup table for AccelByte-specific error codes and their user-friendly messages.
//!
//! Codes are organised into per-service submodules. The top-level
//! [`lookup_error`] dispatches by trying each submodule in turn — the
//! first match wins. Code IDs are unique across services in the cases
//! that matter (the standard `200xx` codes share meaning across every
//! service, so a single entry in [`standard`] handles them all).

mod basic;
mod challenge;
mod cloudsave;
mod group;
mod iam;
mod leaderboard;
mod legal;
mod platform;
mod seasonpass;
mod social;
mod standard;

/// Curated error message mapping for known AccelByte error codes.
///
/// When an error code is found in this table, its fields override
/// the generic HTTP-status-based classification.
pub(crate) struct ErrorMapping {
    pub message: &'static str,
    pub reason: Option<&'static str>,
    pub suggestion: Option<&'static str>,
    pub tip: Option<&'static str>,
}

/// Look up a curated error mapping for a `(service, error_code)` pair.
///
/// Routes the lookup to the matching service module first, so service-specific
/// meanings of shared code IDs (e.g. `20024` means "not implemented" for IAM
/// but "insufficient inventory capacity" for platform) win over the generic
/// fallback in [`standard`].
pub(crate) fn lookup_error(service: &str, error_code: i64) -> Option<ErrorMapping> {
    let service_specific = match service {
        "iam" => iam::lookup(error_code),
        "basic" => basic::lookup(error_code),
        "social" => social::lookup(error_code),
        "cloudsave" => cloudsave::lookup(error_code),
        "leaderboard" => leaderboard::lookup(error_code),
        "group" => group::lookup(error_code),
        "platform" => platform::lookup(error_code).or_else(|| seasonpass::lookup(error_code)),
        "seasonpass" => seasonpass::lookup(error_code).or_else(|| platform::lookup(error_code)),
        "legal" => legal::lookup(error_code),
        "challenge" => challenge::lookup(error_code),
        _ => None,
    };
    service_specific.or_else(|| standard::lookup(error_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mapped error codes must include actionable suggestions to guide the user
    #[test]
    fn test_lookup_returns_suggestion_not_fix() {
        let m = lookup_error("iam", 10133).expect("10133 should be mapped");
        assert!(m.suggestion.is_some(), "should have suggestion");
        assert!(m.suggestion.unwrap().contains("email") || m.suggestion.unwrap().len() > 5);
    }

    /// A code that means different things in different services must dispatch
    /// to the service-specific entry.
    #[test]
    fn test_service_routing_disambiguates_shared_codes() {
        // 20024 is "not implemented" for IAM, "insufficient inventory" for platform.
        let iam = lookup_error("iam", 20024).expect("iam 20024 should map");
        assert!(iam.message.contains("Not implemented"));
    }

    /// Unknown services still get the standard fallback for shared 200xx codes.
    #[test]
    fn test_unknown_service_falls_through_to_standard() {
        let m = lookup_error("nonexistent", 20001).expect("20001 should map via standard");
        assert!(m.message.contains("not authorized"));
    }
}
