//! End-to-end coverage for `ags workflow add`/`ags workflow template`.

use crate::common::cli_helpers::ags_isolated;

// `iam/admin/users/v3/get` has two required path parameters (`namespace` and
// `userId` — see GET /iam/v3/admin/namespaces/{namespace}/users/{userId} in
// the bundled IAM spec), so the workflow must bind both or the run leaves
// `userId` unbound and auto-derived as a field to gather, which fails
// non-interactively.
fn valid_workflow_yaml() -> String {
    format!(
        r#"
id: functional-add-probe
name: Functional add probe
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
fn test_workflow_template_prints_skeleton_to_stdout() {
    let assert = ags_isolated()
        .args(["workflow", "template"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("id: my-workflow"));
}

#[test]
fn test_workflow_template_output_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("skeleton.yaml");
    ags_isolated()
        .args(["workflow", "template", "--output", path.to_str().unwrap()])
        .assert()
        .success();
    assert!(path.exists());
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("id: my-workflow"));
}

#[test]
fn test_workflow_add_then_list_then_run_dry_run() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("probe.yaml");
    std::fs::write(&src, valid_workflow_yaml()).unwrap();

    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_str().unwrap();

    // add
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .success();

    // list — the newly installed workflow must be visible
    let list_output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["--format", "json", "workflow", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&list_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(
        parsed
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "functional-add-probe"),
        "installed workflow must appear in `ags workflow list`: {stdout}"
    );

    // run --dry-run — the installed workflow must be runnable
    ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "--dry-run",
            "workflow",
            "run",
            "functional-add-probe",
            "--namespace",
            "dev",
            "--user-id",
            "probe-user",
        ])
        .assert()
        .success();
}

#[test]
fn test_workflow_add_validate_only_does_not_appear_in_list() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("probe.yaml");
    std::fs::write(&src, valid_workflow_yaml()).unwrap();

    ags_isolated()
        .args(["workflow", "add", src.to_str().unwrap(), "--validate-only"])
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
            .any(|e| e["id"] == "functional-add-probe"),
        "--validate-only must not install: {stdout}"
    );
}

#[test]
fn test_workflow_add_rejects_id_collision_with_builtin() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("collide.yaml");
    std::fs::write(
        &src,
        r#"
id: competitive-multiplayer
name: Colliding workflow
workflow_protocol_version: "0.1.0"
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
"#,
    )
    .unwrap();

    let assert = ags_isolated()
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("already registered"), "stderr: {stderr}");
}

#[test]
fn test_workflow_add_rejects_malformed_yaml() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("broken.yaml");
    std::fs::write(&src, "not: [valid").unwrap();

    ags_isolated()
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_workflow_run_warns_when_declared_protocol_version_is_older() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("old.yaml");
    std::fs::write(
        &src,
        r#"
id: old-version-probe
name: Old version probe
workflow_protocol_version: "0.0.1"
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
      - {field: userId, source: {from: "workflow/userId"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
  - name: userId
    description: probe
    required: true
    schema: {type: string}
"#,
    )
    .unwrap();

    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .success();

    let assert = ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "--dry-run",
            "--yes",
            "workflow",
            "run",
            "old-version-probe",
            "--namespace",
            "dev",
            "--user-id",
            "probe-user",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("targets protocol version 0.0.1"),
        "stderr: {stderr}"
    );
}

// Automation consumers (--format json) must not receive the protocol-version
// warning on stderr because it would pollute a machine-readable pipeline.
#[test]
fn test_workflow_run_suppresses_protocol_version_warning_under_json_format() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("old-json.yaml");
    std::fs::write(
        &src,
        r#"
id: old-version-json-probe
name: Old version JSON probe
workflow_protocol_version: "0.0.1"
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
      - {field: userId, source: {from: "workflow/userId"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
  - name: userId
    description: probe
    required: true
    schema: {type: string}
"#,
    )
    .unwrap();

    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .success();

    let assert = ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "--dry-run",
            "--yes",
            "--format",
            "json",
            "workflow",
            "run",
            "old-version-json-probe",
            "--namespace",
            "dev",
            "--user-id",
            "probe-user",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        !stderr.contains("targets protocol version"),
        "automation output must not contain the protocol-version warning: {stderr}"
    );
}

#[test]
fn test_workflow_run_warns_when_declared_protocol_version_is_newer() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("new.yaml");
    std::fs::write(
        &src,
        r#"
id: new-version-probe
name: New version probe
workflow_protocol_version: "999.0.0"
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
      - {field: userId, source: {from: "workflow/userId"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
  - name: userId
    description: probe
    required: true
    schema: {type: string}
"#,
    )
    .unwrap();

    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .success();

    let assert = ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "--dry-run",
            "--yes",
            "workflow",
            "run",
            "new-version-probe",
            "--namespace",
            "dev",
            "--user-id",
            "probe-user",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("targets protocol version 999.0.0"),
        "stderr: {stderr}"
    );
}

#[test]
fn test_workflow_run_warns_and_registers_when_declared_protocol_version_is_missing() {
    // Simulates a workflow installed before `ags workflow add` required
    // `workflow_protocol_version` — written directly into `workflows_dir()`
    // since `add` itself now rejects a file missing the field.
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_str().unwrap();
    let workflows_dir = home_dir.path().join("workflows");
    std::fs::create_dir_all(&workflows_dir).unwrap();
    std::fs::write(
        workflows_dir.join("legacy-probe.yaml"),
        r#"
id: legacy-probe
name: Legacy probe
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
      - {field: userId, source: {from: "workflow/userId"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
  - name: userId
    description: probe
    required: true
    schema: {type: string}
"#,
    )
    .unwrap();

    let assert = ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "--dry-run",
            "--yes",
            "workflow",
            "run",
            "legacy-probe",
            "--namespace",
            "dev",
            "--user-id",
            "probe-user",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("does not declare a protocol version"),
        "a legacy (versionless) workflow must register and warn, not disappear: {stderr}"
    );
}

#[test]
fn test_workflow_run_warns_when_declared_protocol_version_is_unparsable() {
    // Simulates a workflow whose `workflow_protocol_version` is not a valid
    // semver string — written directly into `workflows_dir()` since `add`
    // itself rejects unparsable values.
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_str().unwrap();
    let workflows_dir = home_dir.path().join("workflows");
    std::fs::create_dir_all(&workflows_dir).unwrap();
    std::fs::write(
        workflows_dir.join("banana-probe.yaml"),
        r#"
id: banana-probe
name: Banana probe
workflow_protocol_version: "banana"
steps:
  - id: only-step
    description: probe
    operation: {service: iam, operation: iam/admin/users/v3/get}
    inputs:
      - {field: namespace, source: {from: "workflow/namespace"}}
      - {field: userId, source: {from: "workflow/userId"}}
inputs:
  - name: namespace
    description: probe
    required: true
    schema: {type: string}
  - name: userId
    description: probe
    required: true
    schema: {type: string}
"#,
    )
    .unwrap();

    let assert = ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "--dry-run",
            "--yes",
            "workflow",
            "run",
            "banana-probe",
            "--namespace",
            "dev",
            "--user-id",
            "probe-user",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("not a readable version number"),
        "an unparsable protocol version must warn: {stderr}"
    );
}

#[test]
fn test_workflow_run_silent_when_declared_protocol_version_is_current() {
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("current.yaml");
    std::fs::write(&src, valid_workflow_yaml()).unwrap();

    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["workflow", "add", src.to_str().unwrap()])
        .assert()
        .success();

    let assert = ags_isolated()
        .env("AGS_HOME", home)
        .args([
            "--dry-run",
            "--yes",
            "workflow",
            "run",
            "functional-add-probe",
            "--namespace",
            "dev",
            "--user-id",
            "probe-user",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        !stderr.contains("targets protocol version"),
        "a current-protocol-version workflow must not warn: {stderr}"
    );
}
