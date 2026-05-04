//! Embedded bundled OpenAPI specs and raw spec-loading helpers.

use std::io::Read;

use flate2::read::GzDecoder;

use super::openapi::SwaggerSpec;
use crate::protocol::error::RuntimeError;
use crate::runtime::config;

/// All bundled service specs (gzip-compressed). Order matches
/// `manifest::SERVICES`.
pub(crate) static BUNDLED_SPECS: &[(&str, &[u8])] = &[
    (
        "achievement",
        include_bytes!("../../specs/achievement.json.gz"),
    ),
    ("ams", include_bytes!("../../specs/ams.json.gz")),
    ("basic", include_bytes!("../../specs/basic.json.gz")),
    ("challenge", include_bytes!("../../specs/challenge.json.gz")),
    ("chat", include_bytes!("../../specs/chat.json.gz")),
    ("cloudsave", include_bytes!("../../specs/cloudsave.json.gz")),
    ("csm", include_bytes!("../../specs/csm.json.gz")),
    (
        "gametelemetry",
        include_bytes!("../../specs/gametelemetry.json.gz"),
    ),
    ("gdpr", include_bytes!("../../specs/gdpr.json.gz")),
    ("group", include_bytes!("../../specs/group.json.gz")),
    ("iam", include_bytes!("../../specs/iam.json.gz")),
    ("inventory", include_bytes!("../../specs/inventory.json.gz")),
    (
        "leaderboard",
        include_bytes!("../../specs/leaderboard.json.gz"),
    ),
    ("legal", include_bytes!("../../specs/legal.json.gz")),
    ("lobby", include_bytes!("../../specs/lobby.json.gz")),
    (
        "loginqueue",
        include_bytes!("../../specs/loginqueue.json.gz"),
    ),
    ("match2", include_bytes!("../../specs/match2.json.gz")),
    ("platform", include_bytes!("../../specs/platform.json.gz")),
    ("reporting", include_bytes!("../../specs/reporting.json.gz")),
    (
        "seasonpass",
        include_bytes!("../../specs/seasonpass.json.gz"),
    ),
    ("session", include_bytes!("../../specs/session.json.gz")),
    (
        "sessionhistory",
        include_bytes!("../../specs/sessionhistory.json.gz"),
    ),
    ("social", include_bytes!("../../specs/social.json.gz")),
    ("ugc", include_bytes!("../../specs/ugc.json.gz")),
];

/// Load a bundled spec by service name (raw SwaggerSpec, no caching).
pub(crate) fn load_bundled_spec(service: &str) -> Result<SwaggerSpec, RuntimeError> {
    let compressed = BUNDLED_SPECS
        .iter()
        .find(|(name, _)| *name == service)
        .map(|(_, data)| *data)
        .ok_or_else(|| {
            config::internal_error(format!("No bundled spec for service '{service}'"))
        })?;
    decompress_and_parse(compressed)
}

/// Decompress gzip data and parse it as a SwaggerSpec.
fn decompress_and_parse(data: &[u8]) -> Result<SwaggerSpec, RuntimeError> {
    let mut decoder = GzDecoder::new(data);
    let mut json_str = String::new();
    decoder
        .read_to_string(&mut json_str)
        .map_err(|e| config::internal_error(format!("Failed to decompress spec: {e}")))?;
    let spec: SwaggerSpec = serde_json::from_str(&json_str)
        .map_err(|e| config::internal_error(format!("Failed to parse spec JSON: {e}")))?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bundled IAM spec decompresses into a valid Swagger 2.0 spec with paths.
    #[test]
    fn test_load_bundled_iam() {
        let spec = load_bundled_spec("iam").expect("Failed to load bundled IAM spec");
        assert_eq!(spec.swagger, "2.0");
        assert!(!spec.paths.is_empty(), "Spec should have paths");
    }

    /// Every service in the bundled table decompresses to a valid Swagger 2.0 spec.
    #[test]
    fn test_all_bundled_specs_decompress() {
        for (service, _) in BUNDLED_SPECS {
            let spec = load_bundled_spec(service)
                .unwrap_or_else(|e| panic!("Failed to load bundled '{service}' spec: {e:?}"));
            assert_eq!(
                spec.swagger, "2.0",
                "service '{service}' wrong swagger version"
            );
            assert!(!spec.paths.is_empty(), "service '{service}' has no paths");
        }
    }

    /// Bundled table contains every active service (24 total) and stays in lockstep with the manifest.
    ///
    /// Both the spec table and the manifest must be updated when a service is
    /// added or removed; asserting they share the same length catches partial
    /// registrations that would otherwise pass the per-table count checks.
    #[test]
    fn test_bundled_specs_count() {
        assert_eq!(BUNDLED_SPECS.len(), 24);
        assert_eq!(
            BUNDLED_SPECS.len(),
            super::super::manifest::SERVICES.len(),
            "BUNDLED_SPECS and manifest::SERVICES must stay in lockstep",
        );
    }

    /// Requesting an unbundled service returns an error rather than panicking.
    #[test]
    fn test_load_bundled_unknown_service() {
        let result = load_bundled_spec("nonexistent");
        assert!(result.is_err());
    }
}
