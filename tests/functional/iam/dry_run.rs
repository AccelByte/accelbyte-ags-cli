use crate::common::cli_helpers::ags;
use predicates::prelude::*;

/// --dry-run prints the HTTP method, path, and auth header without making a real request
#[test]
fn test_dry_run_shows_request() {
    ags()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("GET"))
        .stdout(predicate::str::contains("test-ns"))
        .stdout(predicate::str::contains("Authorization: Bearer <token>"));
}

/// --dry-run produces no stderr output so stdout can be cleanly piped for inspection
#[test]
fn test_dry_run_no_status_on_stderr() {
    ags()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}
