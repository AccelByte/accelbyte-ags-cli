use crate::common::cli_helpers::ags_isolated;

// ── Get all JSON ──

/// Config get-all JSON response contains a config array and profile field
#[test]
fn test_config_get_all_json_has_config_array() {
    let output = ags_isolated()
        .args(["config", "get", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["config"].is_array(), "Must have config array");
    assert!(json.get("profile").is_some(), "Must have profile field");
}

/// Each config entry in get-all JSON has key, value, and source fields
#[test]
fn test_config_get_all_json_entries_have_key_value_source() {
    let output = ags_isolated()
        .args(["config", "get", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for entry in json["config"].as_array().unwrap() {
        assert!(entry.get("key").is_some(), "Entry must have key");
        assert!(entry.get("value").is_some(), "Entry must have value");
        assert!(entry.get("source").is_some(), "Entry must have source");
    }
}

// ── Get single JSON ──

/// Config get-single JSON response contains key, value, and source fields
#[test]
fn test_config_get_single_json_has_key_value_source() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "namespace", "testns"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get", "namespace", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["key"], "namespace");
    assert_eq!(json["value"], "testns");
    assert!(json.get("source").is_some());
}

// ── Set JSON ──

/// Config set JSON response contains status, key, and value fields
#[test]
fn test_config_set_json_has_status_key_value() {
    let output = ags_isolated()
        .args(["config", "set", "namespace", "testns", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "set");
    assert_eq!(json["key"], "namespace");
    assert_eq!(json["value"], "testns");
}

// ── Unset JSON ──

/// Config unset JSON response contains status and key fields
#[test]
fn test_config_unset_json_has_status_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "namespace", "testns"])
        .output()
        .unwrap();

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

// ── Human-readable output ──

/// Config get-all human output annotates values with their source (profile or global)
#[test]
fn test_config_get_all_human_has_source_annotations() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "base-url", "https://example.com"])
        .output()
        .unwrap();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "set", "timeout", "30"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["config", "get"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("(profile:default)"),
        "Profile values should show source: {stdout}"
    );
    assert!(
        stdout.contains("(global)"),
        "Global values should show source: {stdout}"
    );
}
