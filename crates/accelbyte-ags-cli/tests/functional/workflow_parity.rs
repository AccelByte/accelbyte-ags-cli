//! Parity coverage for the workflow-executor migration (Plan D).
//!
//! Every service command now routes through `Executor::execute` as a
//! synthesised 1-step workflow. These tests are fully offline: the
//! live-shaped cases use `--dry-run` so no HTTP is performed.

use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

/// A representative read operation used across the parity cases. `namespace`
/// is its single required input.
const SERVICE: &str = "iam";
const RESOURCE: &str = "users";
const METHOD: &str = "list-users-with-accelbyte-account";

/// `--dry-run` on the synthesised path produces a `DryRun` envelope: the
/// request method, path, and auth header land on stdout, exit 0.
#[test]
fn test_dry_run_produces_dry_run_envelope() {
    ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            SERVICE,
            RESOURCE,
            METHOD,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("GET"))
        .stdout(predicate::str::contains("test-ns"));
}

/// `--dry-run --format=json` renders the `DryRun` envelope as JSON on stdout,
/// exit 0.
#[test]
fn test_dry_run_json_renders_valid_json() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "--format",
            "json",
            "--namespace",
            "test-ns",
            SERVICE,
            RESOURCE,
            METHOD,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .expect("dry-run --format=json stdout must be valid JSON");
}

/// `--format=json` with a missing required input is rejected up front with
/// the structured wording (error / context / suggested next step),
/// exit 1. `--dry-run` keeps the case offline; the JSON-mode strictness
/// check in `handle_service` runs regardless of dry-run.
#[test]
fn test_json_missing_required_arg_errors() {
    let output = ags_isolated()
        .args(["--format", "json", "--dry-run", SERVICE, RESOURCE, METHOD])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "expected exit code 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Missing required input"),
        "stderr must carry the structured missing-input wording: {stderr}"
    );
    assert!(
        stderr.contains("Pass --"),
        "stderr must include the suggested-next-step line: {stderr}"
    );
}

/// A non-interactive run (here `--no-input`, but equally a piped/non-TTY
/// terminal) with a missing required input is rejected by the route-level
/// `!allows_input()` precheck before any workflow lifecycle event fires (no
/// "Running workflow" chrome), exit 1. The wording is the structured
/// missing-input error, shared with the `--format=json` case above. `--dry-run`
/// keeps it offline.
#[test]
fn test_no_input_missing_input_errors_before_lifecycle() {
    let output = ags_isolated()
        .args(["--no-input", "--dry-run", SERVICE, RESOURCE, METHOD])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "expected exit code 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Running workflow"),
        "no workflow lifecycle chrome must appear before the precheck error: {stderr}"
    );
    assert!(
        stderr.contains("Missing required input"),
        "stderr must carry the structured missing-input wording: {stderr}"
    );
    assert!(
        stderr.contains("Pass --"),
        "stderr must include the suggested-next-step line: {stderr}"
    );
}

/// A plain human run (no `--no-input`, no `--format=json`) with a missing
/// required input but a non-promptable terminal (the test harness pipes
/// stdin/stderr) must NOT fall through to an interactive prompt it can never
/// answer. The route's `!allows_input()` gate rejects it up front with the
/// structured missing-input error, exit 1. `--dry-run` keeps it offline.
#[test]
fn test_plain_non_tty_missing_input_errors_instead_of_prompting() {
    let output = ags_isolated()
        .args(["--dry-run", SERVICE, RESOURCE, METHOD])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "expected exit code 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Missing required input"),
        "a non-promptable terminal must get the structured error, not a prompt: {stderr}"
    );
}

/// `--no-input --dry-run` with every required input supplied runs to
/// completion, exit 0 (the confirmation precheck is suppressed by dry-run).
#[test]
fn test_no_input_dry_run_all_inputs_supplied_succeeds() {
    ags_isolated()
        .args([
            "--no-input",
            "--dry-run",
            "--namespace",
            "test-ns",
            SERVICE,
            RESOURCE,
            METHOD,
        ])
        .assert()
        .success();
}
