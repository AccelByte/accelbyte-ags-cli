//! End-to-end coverage for `ags workflow remove`.

use crate::common::cli_helpers::ags_isolated;

fn valid_workflow_yaml() -> String {
    format!(
        r#"
id: functional-remove-probe
name: Functional remove probe
workflow_protocol_version: "{}"
steps:
  - id: only-step
    description: probe
    operation: {{service: iam, operation: iam/admin/users/v3/get}}
    inputs:
      - {{field: namespace, source: {{from: "workflow/namespace"}}}}
      - {{field: userId, source: {{from: "workflow/userId"}}}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {{type: string}}
  - name: userId
    description: probe
    required: true
    schema: {{type: string}}
"#,
        ags_protocol::workflow::WORKFLOW_PROTOCOL_VERSION
    )
}

#[test]
fn test_workflow_add_then_remove_disappears_from_list() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("probe.yaml");
    std::fs::write(&src, valid_workflow_yaml()).unwrap();

    ags_isolated()
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .success();

    ags_isolated()
        .args(["workflow", "remove", "functional-remove-probe"])
        .assert()
        .success();

    let list_output = ags_isolated()
        .args(["--format", "json", "workflow", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&list_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(
        !parsed
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "functional-remove-probe"),
        "removed workflow must not appear in `ags workflow list`: {stdout}"
    );
}

#[test]
fn test_workflow_remove_unknown_id_fails() {
    ags_isolated()
        .args(["workflow", "remove", "definitely-not-a-real-workflow"])
        .assert()
        .failure();
}

#[test]
fn test_workflow_remove_rejects_builtin_id() {
    let assert = ags_isolated()
        .args(["workflow", "remove", "competitive-multiplayer"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("built-in"), "stderr: {stderr}");
}

#[test]
fn test_workflow_remove_json_envelope() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("probe.yaml");
    std::fs::write(&src, valid_workflow_yaml()).unwrap();

    ags_isolated()
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .success();

    let output = ags_isolated()
        .args([
            "--format",
            "json",
            "workflow",
            "remove",
            "functional-remove-probe",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(parsed["id"], "functional-remove-probe");
    assert_eq!(parsed["builtin_still_registered"], false);
}
