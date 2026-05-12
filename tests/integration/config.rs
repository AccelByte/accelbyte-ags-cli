use ags::runtime::config::{self, ConfigScope, GlobalConfig, ProfileConfig};

use crate::common::env_guard::TempEnvGuard;

/// Profile config values round-trip through the type-owned accessors.
#[test]
#[serial_test::serial]
fn test_profile_value_round_trips_via_profile_config_accessors() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    config::ensure_profile_exists("default").unwrap();
    ProfileConfig::set_value("default", "base_url", "https://example.com").unwrap();

    let val = ProfileConfig::get_value("default", "base_url").unwrap();
    assert_eq!(val.as_deref(), Some("https://example.com"));
}

/// Global config values round-trip through the type-owned accessors.
#[test]
#[serial_test::serial]
fn test_global_value_round_trips_via_global_config_accessors() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    GlobalConfig::set_value("format", "json").unwrap();

    let val = GlobalConfig::get_value("format").unwrap();
    assert_eq!(val.as_deref(), Some("json"));
}

/// Unsetting a profile config value removes it from the stored JSON.
#[test]
#[serial_test::serial]
fn test_unset_value_removes_profile_field() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    config::ensure_profile_exists("default").unwrap();
    ProfileConfig::set_value("default", "namespace", "my-ns").unwrap();
    assert!(ProfileConfig::get_value("default", "namespace")
        .unwrap()
        .is_some());

    ProfileConfig::unset_value("default", "namespace").unwrap();
    assert!(ProfileConfig::get_value("default", "namespace")
        .unwrap()
        .is_none());
}

/// key registry correctly classifies all keys
#[test]
fn test_key_registry_scopes() {
    let global_keys = [
        "active-profile",
        "format",
        "no-color",
        "timeout",
        "page-limit",
    ];
    let profile_keys = ["base-url", "client-id", "namespace", "grant-type"];

    for key in global_keys {
        let def = config::find_key(key).unwrap();
        assert_eq!(def.scope, ConfigScope::Global, "{key} should be global");
    }
    for key in profile_keys {
        let def = config::find_key(key).unwrap();
        assert_eq!(def.scope, ConfigScope::Profile, "{key} should be profile");
    }
}

/// unknown keys return None from key lookup
#[test]
fn test_lookup_unknown_key_returns_none() {
    assert!(config::find_key("nonexistent").is_none());
}

/// resolved config entries include all known keys
#[test]
#[serial_test::serial]
fn test_resolved_config_entries_return_all_known_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    let entries = config::resolved_config_entries("default");
    let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();

    assert!(keys.contains(&"base-url"));
    assert!(keys.contains(&"format"));
    assert!(keys.contains(&"active-profile"));
    assert_eq!(entries.len(), config::KNOWN_KEYS.len());
}

/// bool values are stored as JSON booleans, not strings
#[test]
#[serial_test::serial]
fn test_bool_values_stored_as_json_bool() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    GlobalConfig::set_value("no_color", "true").unwrap();

    // Read raw JSON to verify it's a bool, not "true"
    let path = config::config_dir().unwrap().join("config.json");
    let data = std::fs::read_to_string(&path).unwrap();
    let obj: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert_eq!(obj["no_color"], serde_json::Value::Bool(true));
}

/// numeric values are stored as JSON numbers, not strings
#[test]
#[serial_test::serial]
fn test_numeric_values_stored_as_json_number() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    GlobalConfig::set_value("timeout", "30").unwrap();

    let path = config::config_dir().unwrap().join("config.json");
    let data = std::fs::read_to_string(&path).unwrap();
    let obj: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert_eq!(obj["timeout"], serde_json::json!(30));
}
