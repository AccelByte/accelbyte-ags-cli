//! Stream-ownership invariants.
//!
//! End-to-end (subprocess) assertions that the post-Phase-0 stream
//! ownership holds: `CommandOutput` rendering is the only thing on
//! stdout; chrome, help, and prompts land on stderr.

use std::process::Command;

fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_ags")
}

/// `ags --version` is a static-read command that writes its result to
/// stdout. Nothing else should appear on either channel.
#[test]
fn test_version_writes_only_to_stdout() {
    let out = Command::new(binary_path())
        .arg("--version")
        .output()
        .expect("run --version");
    assert!(out.status.success(), "ags --version must succeed");
    let stdout = String::from_utf8(out.stdout).expect("stdout is utf8");
    let stderr = String::from_utf8(out.stderr).expect("stderr is utf8");
    assert!(stdout.contains("ags"), "version line on stdout: {stdout}");
    assert!(
        stderr.is_empty(),
        "no chrome should land on stderr for --version: {stderr}"
    );
}

/// `ags workflow --help` writes help text to stderr after Phase 0's
/// stream-ownership refactor (invariant 1: only
/// `CommandOutput` belongs on stdout; help is chrome). Stdout stays
/// clean.
#[test]
fn test_workflow_help_writes_to_stderr_not_stdout() {
    let out = Command::new(binary_path())
        .args(["workflow", "--help"])
        .output()
        .expect("run workflow --help");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("stdout is utf8");
    let stderr = String::from_utf8(out.stderr).expect("stderr is utf8");
    assert!(
        stdout.is_empty(),
        "workflow --help must keep stdout clean (chrome only): {stdout}"
    );
    assert!(
        stderr.contains("workflow"),
        "workflow --help text expected on stderr: {stderr}"
    );
}

/// `ags workflow run --help` (no workflow id) writes the usage hint on
/// stderr per the Phase 0 refactor. Same invariant as the parent
/// command's help.
#[test]
fn test_workflow_run_help_with_no_id_writes_to_stderr() {
    let out = Command::new(binary_path())
        .args(["workflow", "run", "--help"])
        .output()
        .expect("run workflow run --help");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("stdout is utf8");
    let stderr = String::from_utf8(out.stderr).expect("stderr is utf8");
    assert!(
        stdout.is_empty(),
        "workflow run --help must keep stdout clean: {stdout}"
    );
    assert!(
        stderr.contains("Usage:") || stderr.contains("workflow-id"),
        "usage hint expected on stderr: {stderr}"
    );
}
