use crate::common::cli_helpers::ags_isolated;

// ── Channel routing ──

#[test]
fn test_auth_status_headline_on_stdout() {
    let output = ags_isolated().args(["auth", "status"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Not authenticated"),
        "Auth status headline should be on stdout, got: {stdout}"
    );
}

#[test]
fn test_auth_status_next_on_stderr() {
    let output = ags_isolated().args(["auth", "status"]).output().unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Next:"),
        "Next suggestion should be on stderr, got: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Next:"),
        "Next suggestion should not be on stdout, got: {stdout}"
    );
}

// ── JSON mode ──

#[test]
fn test_auth_status_json_valid() {
    let output = ags_isolated()
        .args(["auth", "status", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("Auth status JSON should be valid");
    assert!(json.is_object(), "JSON output should be an object");
}

#[test]
fn test_auth_status_json_has_status_field() {
    let output = ags_isolated()
        .args(["auth", "status", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        json.get("status").is_some(),
        "JSON must always contain 'status' field"
    );
}

#[test]
fn test_auth_status_json_not_authenticated() {
    let output = ags_isolated()
        .args(["auth", "status", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["status"], "not_authenticated");
    // Broken state should have no other fields
    let obj = json.as_object().unwrap();
    assert_eq!(
        obj.len(),
        1,
        "not_authenticated JSON should only have 'status' field, got: {obj:?}"
    );
}

#[test]
fn test_auth_status_json_no_guidance_fields() {
    let output = ags_isolated()
        .args(["auth", "status", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let obj = json.as_object().unwrap();
    for key in ["fix", "tip", "next"] {
        assert!(
            !obj.contains_key(key),
            "JSON must not contain human guidance field '{key}'"
        );
    }
}

#[test]
fn test_auth_status_json_field_names_contract() {
    let output = ags_isolated()
        .args(["auth", "status", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let obj = json.as_object().unwrap();

    let allowed_keys = [
        "status",
        "source",
        "base_url",
        "login_type",
        "client_id",
        "client_secret",
        "access_token",
        "token_expires_in",
        "refresh_token",
        "refresh_expires_in",
        "namespace",
    ];

    for key in obj.keys() {
        assert!(
            allowed_keys.contains(&key.as_str()),
            "Unexpected JSON field '{key}' — adding new fields to the output contract requires updating this test"
        );
    }
}

#[test]
fn test_auth_status_json_stderr_empty() {
    let output = ags_isolated()
        .args(["auth", "status", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "JSON mode should not produce stderr for status, got: {stderr}"
    );
}
