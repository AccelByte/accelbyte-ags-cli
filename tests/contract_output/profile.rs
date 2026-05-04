use crate::common::cli_helpers::ags_isolated;

// ── List ──

/// Profile list JSON response contains a profiles array and active_profile field
#[test]
fn test_profile_list_json_has_profiles_array() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "list", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Profile list JSON should be valid");
    assert!(json["profiles"].is_array(), "Must have profiles array");
    assert!(
        json.get("active_profile").is_some(),
        "Must have active_profile field"
    );
}

/// Profile list JSON marks the active profile with an active: true field
#[test]
fn test_profile_list_json_active_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .output()
        .unwrap();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "use", "staging"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "list", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let profiles = json["profiles"].as_array().unwrap();
    let staging = profiles.iter().find(|p| p["name"] == "staging").unwrap();
    assert_eq!(staging["active"], true);
}

// ── Create ──

/// Profile create JSON response contains status and profile name fields
#[test]
fn test_profile_create_json_has_status_and_name() {
    let output = ags_isolated()
        .args(["profile", "create", "staging", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "created");
    assert_eq!(json["profile"], "staging");
}

// ── Use ──

/// Profile use JSON response contains switched status and profile name
#[test]
fn test_profile_use_json_has_status_and_name() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "use", "staging", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "switched");
    assert_eq!(json["profile"], "staging");
}

// ── Show ──

/// Profile show JSON response contains all expected fields (active, base_url, client_id, etc.)
#[test]
fn test_profile_show_json_has_all_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show", "staging", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["profile"], "staging");
    assert!(json.get("active").is_some(), "Must have active field");
    assert!(json.get("base_url").is_some(), "Must have base_url field");
    assert!(json.get("client_id").is_some(), "Must have client_id field");
    assert!(json.get("namespace").is_some(), "Must have namespace field");
    assert!(
        json.get("grant_type").is_some(),
        "Must have grant_type field"
    );
    assert!(
        json.get("has_secret").is_some(),
        "Must have has_secret field"
    );
    assert!(json.get("has_token").is_some(), "Must have has_token field");
}

/// Profile show human output includes all expected field labels
#[test]
fn test_profile_show_human_has_labels() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show", "staging"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    for label in [
        "Base URL",
        "Client ID",
        "Namespace",
        "Grant type",
        "Secret",
        "Token",
    ] {
        assert!(
            stdout.contains(label),
            "Show output must contain '{label}', got: {stdout}"
        );
    }
}

// ── Delete ──

/// Profile delete JSON response contains deleted status and profile name
#[test]
fn test_profile_delete_json_has_status_and_name() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "delete", "staging", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "deleted");
    assert_eq!(json["profile"], "staging");
}

// ── Rename ──

/// Profile rename JSON response contains renamed status with old and new name fields
#[test]
fn test_profile_rename_json_has_old_and_new() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .output()
        .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "rename", "staging", "dev", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "renamed");
    assert_eq!(json["old"], "staging");
    assert_eq!(json["new"], "dev");
}
