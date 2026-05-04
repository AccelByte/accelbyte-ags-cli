use crate::common::cli_helpers::ags_isolated;

/// Root catalogue JSON structure is stable across releases
#[test]
fn test_snapshot_root_catalogue() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Snapshot the first service entry shape rather than all 24 services
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let first_child = &json["data"]["children"][0];
    insta::assert_json_snapshot!("root_catalogue_child", first_child);
}

/// Service catalogue JSON structure for IAM is stable
#[test]
fn test_snapshot_service_catalogue() {
    let output = ags_isolated().args(["describe", "iam"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // Snapshot the envelope metadata (not children, which change with spec updates)
    insta::assert_json_snapshot!("service_catalogue_envelope", {
        ".data.children" => "[children]",
        ".generated_by.version" => "[version]",
    }, &json);
}

/// Method-level describe exposes the full scope/version contract matrix.
/// `iam roles list` is chosen because it spans both `admin` (v3, v4) and `public` (v3)
/// scopes — exercising multi-version admin and single-version public branches.
#[test]
fn test_snapshot_method_matrix() {
    let output = ags_isolated()
        .args(["describe", "iam", "roles", "list"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Snapshot the matrix skeleton: scope keys, default versions, supported
    // versions, and per-contract HTTP method / x-operationId. Parameter and
    // body detail is intentionally redacted so a spec tweak doesn't churn the
    // snapshot — the dedicated functional tests cover those fields.
    insta::assert_json_snapshot!("method_matrix", {
        ".generated_by.version" => "[version]",
        ".data.scopes.*.contracts.*.parameters" => "[parameters]",
        ".data.scopes.*.contracts.*.request_body" => "[request_body]",
        ".data.scopes.*.contracts.*.response" => "[response]",
        ".data.scopes.*.contracts.*.permissions" => "[permissions]",
        ".data.scopes.*.contracts.*.path_template" => "[path_template]",
    }, &json);
}

/// Error envelope JSON structure is stable
#[test]
fn test_snapshot_error_envelope() {
    let output = ags_isolated().args(["describe", "iamx"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    insta::assert_json_snapshot!("error_envelope", {
        ".generated_by.version" => "[version]",
    }, &json);
}
