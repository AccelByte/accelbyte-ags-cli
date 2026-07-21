use crate::common::cli_helpers::ags_isolated;
use serde_json::Value;

// ── Success cases ──

/// --skeleton outputs valid JSON for an operation with a request body
#[test]
fn test_skeleton_outputs_valid_json() {
    let output = ags_isolated()
        .args(["iam", "roles", "create", "--skeleton"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert!(json.is_object(), "skeleton must be a JSON object");
}

/// Skeleton includes fields from the body schema
#[test]
fn test_skeleton_includes_body_fields() {
    let output = ags_isolated()
        .args(["iam", "roles", "create", "--skeleton"])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    // RoleV4Request has roleName as a required field
    assert!(
        json.get("roleName").is_some(),
        "must include roleName field"
    );
}

/// Skeleton output goes to stdout only
#[test]
fn test_skeleton_output_on_stdout_only() {
    let output = ags_isolated()
        .args(["iam", "roles", "create", "--skeleton"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "skeleton must not produce stderr: {stderr}"
    );
}

/// Skeleton does not require authentication
#[test]
fn test_skeleton_works_without_auth() {
    // ags_isolated() has no credentials configured
    let output = ags_isolated()
        .args(["iam", "roles", "create", "--skeleton"])
        .output()
        .unwrap();
    assert!(output.status.success(), "skeleton must not require auth");
}

/// Skeleton flag position does not matter (global flag)
#[test]
fn test_skeleton_flag_position_flexible() {
    let output = ags_isolated()
        .args(["--skeleton", "iam", "roles", "create"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert!(json.is_object());
}

// ── Error cases ──

/// --skeleton on a GET operation (no request body) returns an error
#[test]
fn test_skeleton_error_when_no_body() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--skeleton",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no request body"),
        "error must explain no body: {stderr}"
    );
}
