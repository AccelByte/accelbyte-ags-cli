use crate::common::cli_helpers::{ags, ags_isolated};
use predicates::prelude::*;
use std::fs;

#[test]
fn test_doctor_help_succeeds() {
    ags().args(["doctor", "--help"]).assert().success();
}

#[test]
fn test_doctor_offline_skips_network() {
    ags_isolated()
        .args(["doctor", "--offline"])
        .assert()
        .stdout(predicate::str::contains("skipped"));
}

#[test]
fn test_doctor_offline_json_has_skipped_status() {
    let output = ags_isolated()
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let profiles = json["profiles"].as_array().expect("profiles array");
    let checks = profiles[0]["checks"].as_array().expect("checks array");
    let skipped: Vec<_> = checks.iter().filter(|c| c["status"] == "skipped").collect();
    assert!(
        skipped.len() >= 2,
        "Network tier checks should be skipped in offline mode"
    );
}

#[test]
fn test_doctor_json_format_is_valid() {
    let output = ags_isolated()
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("must produce valid JSON");
    assert!(json["status"].is_string(), "must have status field");
    assert!(json["profiles"].is_array(), "must have profiles array");
}

#[test]
fn test_doctor_all_exits_nonzero_when_any_profile_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    // Create two profiles — "good" has valid config, "bad" has invalid JSON
    let good_dir = home.join("profiles").join("good");
    fs::create_dir_all(&good_dir).unwrap();
    fs::write(
        good_dir.join("config.json"),
        r#"{"base_url":"https://example.com","client_id":"aaaabbbbccccddddeeeeffffaaaabbbb"}"#,
    )
    .unwrap();

    let bad_dir = home.join("profiles").join("bad");
    fs::create_dir_all(&bad_dir).unwrap();
    fs::write(bad_dir.join("config.json"), "not valid json").unwrap();

    // Set "good" as active so resolve_profile_name works
    let global_path = home.join("config.json");
    fs::write(&global_path, r#"{"active_profile":"good"}"#).unwrap();

    ags()
        .env("AGS_HOME", home.to_str().unwrap())
        .env("AGS_NO_KEYCHAIN", "1")
        .args(["doctor", "--all", "--offline"])
        .assert()
        .failure();
}

#[test]
fn test_doctor_all_json_has_multiple_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    // Create two profiles
    for name in &["alpha", "beta"] {
        let dir = home.join("profiles").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{}").unwrap();
    }
    let global_path = home.join("config.json");
    fs::write(&global_path, r#"{"active_profile":"alpha"}"#).unwrap();

    let output = ags()
        .env("AGS_HOME", home.to_str().unwrap())
        .env("AGS_NO_KEYCHAIN", "1")
        .args(["doctor", "--all", "--offline", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(json["status"].is_string(), "must have top-level status");
    let profiles = json["profiles"].as_array().expect("profiles array");
    assert_eq!(profiles.len(), 2, "must have two profile entries");

    for profile in profiles {
        assert!(
            profile["profile"].is_string(),
            "each entry must have profile name"
        );
        assert!(
            profile["checks"].is_array(),
            "each entry must have checks array"
        );
    }
}

/// Regression: on platforms without a keychain backend (e.g. WSL2 without D-Bus,
/// or a container where the keyutils syscall is absent/blocked), a missing stored
/// token must produce a Warning, not a Fail, when client credentials are present.
/// Before the fix, Entry::new returning NoStorageAccess/PlatformFailure propagated
/// as a RuntimeError, causing the Auth tier to Fail and the Network tier to be
/// skipped with "earlier tier failed" even though auth was possible.
#[test]
fn test_doctor_missing_token_with_client_creds_is_warning_not_fail() {
    let tmp = tempfile::tempdir().unwrap();

    let output = ags()
        .env("AGS_HOME", tmp.path())
        .env("AGS_BASE_URL", "https://example.accelbyte.io/")
        .env("AGS_CLIENT_ID", "aaaabbbbccccddddeeeeffffaaaabbbb")
        .env("AGS_CLIENT_SECRET", "some-secret")
        // Intentionally no AGS_NO_KEYCHAIN — exercises the real keychain init path.
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON output");
    let checks = json["profiles"][0]["checks"]
        .as_array()
        .expect("checks array");

    let cred_check = checks
        .iter()
        .find(|c| c["name"] == "credential-state")
        .expect("credential-state check must be present");
    assert_eq!(
        cred_check["status"], "pass",
        "credential-state must Pass when AGS_CLIENT_SECRET is present"
    );

    let token_check = checks
        .iter()
        .find(|c| c["name"] == "token-state")
        .expect("token-state check must be present");
    assert_ne!(
        token_check["status"], "fail",
        "token-state must not Fail when no token is stored but client credentials are present"
    );

    // Network tier is skipped via --offline. Its skip reason must reflect offline
    // mode, not an Auth tier failure — confirming the Auth tier did not Fail.
    for check in checks.iter().filter(|c| c["tier"] == "network") {
        assert_ne!(
            check["message"].as_str().unwrap_or(""),
            "skipped (earlier tier failed)",
            "Network tier must not report 'earlier tier failed' when client credentials are present"
        );
    }
}
