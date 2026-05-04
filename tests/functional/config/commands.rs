use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

// ── Help ──

/// Config help shows all available subcommands
#[test]
fn test_config_help() {
    ags_isolated()
        .args(["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("set"))
        .stdout(predicate::str::contains("unset"));
}

// ── Get (all) ──

/// Get all config displays every known config key
#[test]
fn test_config_get_all_shows_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get"])
        .assert()
        .success()
        .stdout(predicate::str::contains("base-url"))
        .stdout(predicate::str::contains("namespace"))
        .stdout(predicate::str::contains("format"))
        .stdout(predicate::str::contains("active-profile"));
}

/// Get all config includes previously set values with their profile source
#[test]
fn test_config_get_all_shows_set_values() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "base-url", "https://example.com"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get"])
        .assert()
        .success()
        .stdout(predicate::str::contains("https://example.com"))
        .stdout(predicate::str::contains("profile:default"));
}

/// Get all config in JSON format returns a config array and profile field
#[test]
fn test_config_get_all_json() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["config"].is_array());
    assert!(json.get("profile").is_some());
}

// ── Get (single key) ──

/// Get a single profile-scoped key returns its stored value
#[test]
fn test_config_get_single_profile_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "namespace", "myns"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "namespace"])
        .assert()
        .success()
        .stdout(predicate::str::contains("myns"));
}

/// Get a single global-scoped key returns its stored value
#[test]
fn test_config_get_single_global_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "format", "json"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "format"])
        .assert()
        .success()
        .stdout(predicate::str::contains("json"));
}

/// Get a key that has not been set shows "not set"
#[test]
fn test_config_get_unset_key() {
    ags_isolated()
        .args(["config", "get", "namespace"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not set"));
}

/// Get with an unrecognised key name fails with an error
#[test]
fn test_config_get_unknown_key_fails() {
    ags_isolated()
        .args(["config", "get", "unknown-key"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown config key"));
}

/// Get a single key in JSON format returns key, value, and source fields
#[test]
fn test_config_get_single_json() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "base-url", "https://example.com"])
        .assert()
        .success();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "base-url", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["key"], "base-url");
    assert_eq!(json["value"], "https://example.com");
    assert!(json.get("source").is_some());
}

// ── Set ──

/// Set a profile-scoped key persists and confirms the new value
#[test]
fn test_config_set_profile_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "base-url", "https://staging.example.com"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "base-url = https://staging.example.com",
        ));
}

/// Set a global-scoped key persists and confirms the new value
#[test]
fn test_config_set_global_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("format = json"));
}

/// Set a boolean config value stores and retrieves it correctly
#[test]
fn test_config_set_bool_value() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "no-color", "true"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("true"));
}

/// Set with an unrecognised key name fails with an error
#[test]
fn test_config_set_unknown_key_fails() {
    ags_isolated()
        .args(["config", "set", "bogus", "value"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown config key"));
}

/// Set a profile-scoped key with --global flag fails with a scope mismatch error
#[test]
fn test_config_set_profile_key_with_global_flag_fails() {
    ags_isolated()
        .args(["config", "set", "base-url", "x", "--global"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("profile-scoped key"));
}

/// Set a global-scoped key with --profile flag fails with a scope mismatch error
#[test]
fn test_config_set_global_key_with_profile_flag_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "format", "json", "--profile", "staging"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("global key"));
}

/// Set a value with --profile targets the specified profile instead of the active one
#[test]
fn test_config_set_with_specific_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "config",
            "set",
            "base-url",
            "https://staging.example.com",
            "--profile",
            "staging",
        ])
        .assert()
        .success();

    // Verify it landed in the right profile
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "base-url", "--profile", "staging"])
        .assert()
        .success()
        .stdout(predicate::str::contains("https://staging.example.com"));
}

/// Set active-profile switches the active profile in global config
#[test]
fn test_config_set_active_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "active-profile", "staging"])
        .assert()
        .success();

    // Verify it took effect
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("staging (active)"));
}

/// Set in JSON format returns status, key, and value fields
#[test]
fn test_config_set_json() {
    let output = ags_isolated()
        .args(["config", "set", "namespace", "myns", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "set");
    assert_eq!(json["key"], "namespace");
    assert_eq!(json["value"], "myns");
}

// ── Unset ──

/// Unset removes a previously set value and get shows "not set" afterwards
#[test]
fn test_config_unset_removes_value() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "namespace", "myns"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "unset", "namespace"])
        .assert()
        .success()
        .stdout(predicate::str::contains("namespace unset"));

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "namespace"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not set"));
}

/// Unset a global-scoped key clears the stored value
#[test]
fn test_config_unset_global_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "format", "json"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "unset", "format"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "format"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not set"));
}

/// Unset with an unrecognised key name fails with an error
#[test]
fn test_config_unset_unknown_key_fails() {
    ags_isolated()
        .args(["config", "unset", "bogus"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown config key"));
}

/// Unset in JSON format returns status and key fields
#[test]
fn test_config_unset_json() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "namespace", "myns"])
        .assert()
        .success();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "unset", "namespace", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "unset");
    assert_eq!(json["key"], "namespace");
}

// ── Namespace validation ──

#[test]
fn test_config_set_namespace_rejects_uppercase() {
    ags_isolated()
        .args(["config", "set", "namespace", "MyNS"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid namespace"));
}

#[test]
fn test_config_set_namespace_rejects_leading_hyphen() {
    ags_isolated()
        .args(["config", "set", "namespace", "foo-"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid namespace"));
}

#[test]
fn test_config_set_namespace_rejects_oversized() {
    let long_ns = "a".repeat(49);
    ags_isolated()
        .args(["config", "set", "namespace", &long_ns])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid namespace"));
}

#[test]
fn test_config_set_namespace_accepts_hyphens() {
    ags_isolated()
        .args(["config", "set", "namespace", "my-game-name"])
        .assert()
        .success();
}

// ── Base URL validation ──

#[test]
fn test_config_set_base_url_rejects_invalid_url() {
    ags_isolated()
        .args(["config", "set", "base-url", "not-a-url"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid URL"));
}

#[test]
fn test_config_set_base_url_rejects_ftp_scheme() {
    ags_isolated()
        .args(["config", "set", "base-url", "ftp://example.com"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid URL"));
}

#[test]
fn test_config_set_base_url_strips_trailing_slash() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "base-url", "https://demo.accelbyte.io/"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("https://demo.accelbyte.io")
                .and(predicate::str::contains("https://demo.accelbyte.io/").not()),
        );
}

// ── Client ID validation ──

#[test]
fn test_config_set_client_id_rejects_short_value() {
    ags_isolated()
        .args(["config", "set", "client-id", "tooshort"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid client-id"));
}

#[test]
fn test_config_set_client_id_accepts_valid_hex() {
    ags_isolated()
        .args([
            "config",
            "set",
            "client-id",
            "aabbccdd11223344aabbccdd11223344",
        ])
        .assert()
        .success();
}
