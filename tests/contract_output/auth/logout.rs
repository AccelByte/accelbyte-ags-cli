use crate::common::cli_helpers::ags_isolated;

// ── Channel routing ──

#[test]
fn test_auth_logout_headline_on_stdout() {
    let output = ags_isolated().args(["auth", "logout"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Credentials cleared"),
        "Logout headline should be on stdout, got: {stdout}"
    );
}

#[test]
fn test_auth_logout_details_on_stderr() {
    let output = ags_isolated().args(["auth", "logout"]).output().unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Client ID:"),
        "Detail fields should be on stderr, got: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Client ID:"),
        "Detail fields should not be on stdout, got: {stdout}"
    );
}

// ── JSON mode ──

#[test]
fn test_auth_logout_json_valid() {
    let output = ags_isolated()
        .args(["auth", "logout", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("Auth logout JSON should be valid");
    assert!(json.is_object());
}

#[test]
fn test_auth_logout_json_status_only() {
    let output = ags_isolated()
        .args(["auth", "logout", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["status"], "cleared");
    let obj = json.as_object().unwrap();
    assert_eq!(
        obj.len(),
        1,
        "Logout JSON should only contain 'status', got: {obj:?}"
    );
}

#[test]
fn test_auth_logout_json_no_guidance_fields() {
    let output = ags_isolated()
        .args(["auth", "logout", "--format", "json"])
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
fn test_auth_logout_json_stderr_empty() {
    let output = ags_isolated()
        .args(["auth", "logout", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "JSON mode should not produce stderr for logout, got: {stderr}"
    );
}
