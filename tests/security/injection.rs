use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

// ── Path parameter injection ──

/// Path traversal sequences like "../../admin" are rejected to prevent accessing unintended API paths
#[test]
fn test_path_param_rejects_traversal() {
    ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "get",
            "--user-id",
            "../../admin",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("path traversal"));
}

/// Question marks in path params are rejected to prevent query string injection
#[test]
fn test_path_param_rejects_question_mark() {
    ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "get",
            "--user-id",
            "foo?admin=true",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("'?' is not allowed"));
}

/// Hash characters in path params are rejected to prevent fragment injection
#[test]
fn test_path_param_rejects_hash() {
    ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "get",
            "--user-id",
            "foo#fragment",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("'#' is not allowed"));
}

/// Standard UUIDs pass validation since they are the normal format for AccelByte resource IDs
#[test]
fn test_path_param_accepts_normal_uuid() {
    ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "get",
            "--user-id",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .assert()
        .success();
}

// ── Format flag validation ──

/// Unsupported format values like "yaml" are rejected early with a clear message
#[test]
fn test_format_rejects_unsupported_value() {
    ags_isolated()
        .args([
            "--format",
            "yaml",
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "get",
            "--user-id",
            "abc",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown --format value"));
}

/// The "json" format value is accepted as the only supported machine-readable output format
#[test]
fn test_format_accepts_json() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "--format",
            "json",
            "--namespace",
            "test",
            "iam",
            "users",
            "get",
            "--user-id",
            "abc",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unknown --format value"),
        "json format should be accepted: {stderr}"
    );
}
