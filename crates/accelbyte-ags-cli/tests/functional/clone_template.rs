//! Functional tests for `ags extend clone-template`.
//!
//! Offline-safe tests run against the local binary with no network or git
//! dependency. Tests that require a real `git clone` (and therefore network
//! access) are marked `#[ignore]` following the convention used by sibling
//! functional test modules.

use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

// ── JSON envelope shape (requires git + network) ──

/// The JSON output envelope contains the expected keys when a clone succeeds.
/// Requires `git` on PATH and network access to the template repository.
#[test]
#[ignore]
fn test_json_envelope_has_expected_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("clone-target");

    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "extend",
            "clone-template",
            "--template",
            "Extend Override :: Lootbox Roll :: Go",
            "--destination",
            dest.to_str().unwrap(),
            "--yes",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));

    assert!(json["template_name"].is_string(), "missing template_name");
    assert!(json["destination"].is_string(), "missing destination");
    // source_path is present (may be null when the template has no sub-path).
    assert!(json.get("source_path").is_some(), "missing source_path key");
}

// ── Non-interactive rejection ──

/// Without `--template` and with piped (empty) stdin the command cannot
/// prompt for interactive selection and must exit non-zero. The stderr
/// output must contain the actual rejection reason so the test cannot
/// pass for the wrong failure mode.
#[test]
fn test_no_template_no_stdin_exits_nonzero() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("no-tpl");

    ags_isolated()
        .args([
            "extend",
            "clone-template",
            "--destination",
            dest.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid selection"));
}

// ── --yes skips confirmation (requires git + network) ──

/// With `--yes`, the confirmation prompt is bypassed and the clone succeeds
/// even though stdin provides no interactive input. Requires `git` and
/// network access to the template repository.
#[test]
#[ignore]
fn test_yes_skips_confirmation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("yes-clone");

    ags_isolated()
        .args([
            "extend",
            "clone-template",
            "--template",
            "Extend Override :: Lootbox Roll :: Go",
            "--destination",
            dest.to_str().unwrap(),
            "--yes",
        ])
        .assert()
        .success();
}

// ── --no-input without --yes ──

/// `--no-input` without `--yes` must reject the destructive operation
/// because the confirmation prompt cannot be shown.
#[test]
fn test_no_input_without_yes_is_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("no-input");

    ags_isolated()
        .args([
            "--no-input",
            "extend",
            "clone-template",
            "--template",
            "Extend Override :: Lootbox Roll :: Go",
            "--destination",
            dest.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires confirmation"));
}

// ── Clap error formatting ──

/// An invalid `--depth` value triggers a Clap validation error with the
/// expected format (error on stderr, non-zero exit).
#[test]
fn test_invalid_depth_shows_clap_error() {
    ags_isolated()
        .args(["extend", "clone-template", "--depth", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid value 'abc'"));
}

// ── --help ──

/// `--help` renders usage text on stdout and exits cleanly.
#[test]
fn test_help_renders_usage() {
    ags_isolated()
        .args(["extend", "clone-template", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("clone-template"))
        .stdout(predicate::str::contains("--template"));
}

// ── --dry-run ──

/// `--dry-run` must preview the operation without creating the destination
/// directory or spawning git. CI-safe: no git binary or network required.
#[test]
fn test_dry_run_does_not_create_destination() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("dry-run-dest");

    ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "clone-template",
            "--template",
            "Extend Override :: Lootbox Roll :: Go",
            "--destination",
            dest.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("Dry run"))
        .stderr(predicate::str::contains("Lootbox Roll"))
        .stderr(predicate::str::contains("git clone"));

    assert!(
        !dest.exists(),
        "destination must not be created during --dry-run"
    );
}

// ── csm clone-template no longer resolves ──

/// `ags csm clone-template` must NOT resolve to the clone-template command.
/// CSM is a spec-driven service; `clone-template` is not one of its
/// resources, so the service parser rejects it.
#[test]
fn test_csm_clone_template_does_not_resolve() {
    ags_isolated()
        .args(["csm", "clone-template"])
        .assert()
        .failure();
}
