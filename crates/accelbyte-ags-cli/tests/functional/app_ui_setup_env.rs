//! Functional tests for `ags extend app-ui setup-env`.
//!
//! Offline-safe tests run against the local binary. The command requires
//! auth + network for its happy path (ListAppUI API call), so the CI-runnable
//! cases exercise the offline paths: dry-run, skip guard, force-no-skip,
//! and Clap-level validation.

use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

// ── --dry-run ──

/// `--dry-run` prints the preview and exits 0 without network or file write.
/// CI-safe: no auth, no network required.
#[test]
fn test_dry_run_prints_preview_no_write() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();

    let assert = ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "setup-env",
            "--name",
            "my-app",
            "--project-path",
            project.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The dry-run output includes the preview fields on stderr.
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("Dry run"),
        "expected 'Dry run' in stderr: {stderr}"
    );
    assert!(
        stderr.contains("my-app"),
        "expected app name in stderr: {stderr}"
    );
    assert!(
        stderr.contains("test-ns"),
        "expected namespace in stderr: {stderr}"
    );
    assert!(
        stderr.contains("VITE_AB_"),
        "expected managed key names in stderr: {stderr}"
    );

    // No .env.local created.
    assert!(
        !project.join(".env.local").exists(),
        ".env.local must not be created during --dry-run"
    );
}

// ── Skip guard ──

/// When `.env.local` already exists without `--force`, the command exits 0
/// with a warning that mentions `--force`. Only one warning line should be
/// emitted (the duplicate-warning bug was B2).
#[test]
fn test_skip_guard_fires_when_env_local_exists() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join(".env.local"), "EXISTING=keep\n").unwrap();

    let assert = ags_isolated()
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "setup-env",
            "--name",
            "my-app",
            "--project-path",
            project.to_str().unwrap(),
        ])
        .assert()
        .success();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();

    // Must mention --force so the user knows how to proceed.
    assert!(
        stderr.contains("--force"),
        "skip warning must mention --force: {stderr}"
    );

    // Must mention "already exists" to explain why it was skipped.
    assert!(
        stderr.contains("already exists"),
        "skip warning must mention 'already exists': {stderr}"
    );

    // Exactly one warning line (B2 fix: no duplicate).
    let warning_count = stderr
        .lines()
        .filter(|l| l.contains("already exists"))
        .count();
    assert_eq!(
        warning_count, 1,
        "expected exactly 1 warning line, got {warning_count}; stderr:\n{stderr}"
    );

    // The existing file must not be modified.
    assert_eq!(
        std::fs::read_to_string(project.join(".env.local")).unwrap(),
        "EXISTING=keep\n"
    );
}

// ── --force does not take the skip path ──

/// With `--force` and an existing `.env.local`, the command does NOT emit
/// the skip warning. It would proceed to auth (which fails offline), so we
/// check that the failure is auth-related rather than a skip exit-0.
#[test]
fn test_force_does_not_skip() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join(".env.local"), "EXISTING=content\n").unwrap();

    let assert = ags_isolated()
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "setup-env",
            "--name",
            "my-app",
            "--project-path",
            project.to_str().unwrap(),
            "--force",
        ])
        .assert()
        .failure(); // fails because no credentials — but NOT a skip

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();

    // The skip warning must NOT appear.
    assert!(
        !stderr.contains("already exists"),
        "--force must bypass the skip guard; stderr:\n{stderr}"
    );
}

// ── --format json ──

/// `--format json` on the skip path emits a JSON envelope with the
/// expected keys (`status`, `env_path`) on stdout.
#[test]
fn test_format_json_skip_envelope() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join(".env.local"), "EXISTING=keep\n").unwrap();

    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "setup-env",
            "--name",
            "my-app",
            "--project-path",
            project.to_str().unwrap(),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));

    assert_eq!(
        json["status"], "skipped",
        "expected status 'skipped' in JSON: {json}"
    );
    assert!(
        json["env_path"].is_string(),
        "expected env_path string in JSON: {json}"
    );
}

// ── --help ──

/// `--help` renders usage text on stdout and exits cleanly.
#[test]
fn test_help_renders_usage() {
    ags_isolated()
        .args(["extend", "app-ui", "setup-env", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("setup-env"))
        .stdout(predicate::str::contains("--name"));
}

// ── Missing required --name ──

/// Without `--name` the command fails with a Clap error.
#[test]
fn test_missing_name_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();

    ags_isolated()
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "setup-env",
            "--project-path",
            project.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--name"));
}

// ── Missing required --namespace ──

/// Without `--namespace` (and no profile default) the command exits 1
/// with a usage error mentioning namespace.
#[test]
fn test_missing_namespace_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();

    ags_isolated()
        .args([
            "extend",
            "app-ui",
            "setup-env",
            "--name",
            "my-app",
            "--project-path",
            project.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("namespace"));
}

// ── Invalid project path ──

/// A nonexistent project path yields a usage error.
#[test]
fn test_invalid_project_path_fails() {
    ags_isolated()
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "setup-env",
            "--name",
            "my-app",
            "--project-path",
            "/nonexistent/path/that/does/not/exist",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("was not found"));
}
