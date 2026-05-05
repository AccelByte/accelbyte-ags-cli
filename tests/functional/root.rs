use crate::common::cli_helpers::{ags, ags_isolated};
use predicates::prelude::*;

/// --version prints the semver string so scripts can check CLI compatibility
#[test]
fn test_version() {
    ags()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("ags "));
}

/// Root --help enumerates all registered services so users can discover available commands
#[test]
fn test_root_help_lists_services() {
    ags()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("AccelByte Gaming Services CLI"))
        .stdout(predicate::str::contains("iam"))
        .stdout(predicate::str::contains("platform"))
        .stdout(predicate::str::contains("achievement"))
        .stdout(predicate::str::contains("auth"));
}

/// Running with no arguments prints usage guidance rather than silently exiting
#[test]
fn test_no_args_shows_help() {
    let output = ags().output().unwrap();
    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        combined.contains("Usage") || combined.contains("ags"),
        "Expected help output, got: {combined}"
    );
}

/// An unrecognised service name produces a clear error with the valid service list
#[test]
fn test_unknown_service_error() {
    ags()
        .arg("badservice")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service: 'badservice'"))
        .stderr(predicate::str::contains("Valid services:"));
}

/// A token starting with '-' is a mistyped flag, not a service name. It must
/// be rejected with an "Unknown flag" error rather than routed through the
/// service lookup (which would produce a misleading "Unknown service" message
/// and a list of services as alternatives).
#[test]
fn test_unknown_flag_is_not_reported_as_unknown_service() {
    ags().arg("--bogus-flag").assert().failure().stderr(
        predicate::str::contains("Unknown flag").and(
            predicate::str::contains("--bogus-flag")
                .and(predicate::str::contains("Unknown service").not()),
        ),
    );
}

/// A close misspelling of a service name still shows the valid list so users can self-correct
#[test]
fn test_misspelled_service_shows_valid_services() {
    ags()
        .args(["iamm", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service: 'iamm'"))
        .stderr(predicate::str::contains("Valid services:"))
        .stderr(predicate::str::contains("iam"));
}

/// An empty string as the service argument is rejected rather than causing a panic
#[test]
fn test_empty_service_arg() {
    ags()
        .args([""])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service"));
}

/// Help includes worked examples so first-time users can copy-paste a real command
#[test]
fn test_root_help_shows_examples() {
    ags()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains(
            "ags iam users search --namespace my-game",
        ));
}

/// Help documents exit codes so CI pipelines can branch on specific failure modes
#[test]
fn test_root_help_shows_exit_codes() {
    ags()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Exit codes:"))
        .stdout(predicate::str::contains("0 = success"));
}

/// Global flags placed before the service name are recognised by the two-phase parser
#[test]
fn test_global_flags_accepted_before_service() {
    ags()
        .args(["--format", "json", "iam", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("users"));
}

/// Global flags interleaved mid-command still work because pre-scan extracts them first
#[test]
fn test_global_flags_accepted_mid_command() {
    ags()
        .args(["iam", "--verbose", "users", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"));
}

/// An API call without stored credentials fails with an auth error rather than a cryptic panic
#[test]
fn test_missing_auth_for_api_call() {
    ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test-ns",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Authentication error")
                .or(predicate::str::contains("No base_url configured"))
                .or(predicate::str::contains("auth")),
        );
}

// ── Version JSON ──

/// `version --format json` emits parseable JSON so automation can extract version metadata
#[test]
fn test_version_json_valid() {
    let output = ags()
        .args(["version", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("Version JSON should be valid");
    assert!(json.is_object());
}

/// The JSON version field matches CARGO_PKG_VERSION to prevent build/output drift
#[test]
fn test_version_json_has_version_field() {
    let output = ags()
        .args(["version", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let version = json["version"]
        .as_str()
        .expect("version should be a string");
    assert!(!version.is_empty(), "version field should not be empty");
    // Version should match the crate version
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
}

// ── Global flag validation ──

/// --namespace must reject empty/whitespace-only values rather than silently
/// falling back to the profile default.
#[test]
fn test_namespace_rejects_empty_value() {
    let output = ags_isolated()
        .args(["iam", "roles", "list", "--namespace", "", "--dry-run"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--namespace value cannot be empty"),
        "stderr: {stderr}"
    );
}

/// --timeout must reject non-numeric values rather than silently falling back
/// to the default.
#[test]
fn test_timeout_rejects_non_numeric() {
    let output = ags_isolated()
        .args([
            "--timeout",
            "abc",
            "iam",
            "roles",
            "list",
            "--namespace",
            "ns",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid --timeout value"),
        "stderr: {stderr}"
    );
}

/// --timeout 0 must be rejected — there's no meaningful zero-second request.
#[test]
fn test_timeout_rejects_zero() {
    let output = ags_isolated()
        .args([
            "--timeout",
            "0",
            "iam",
            "roles",
            "list",
            "--namespace",
            "ns",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--timeout must be at least 1 second"),
        "stderr: {stderr}"
    );
}
