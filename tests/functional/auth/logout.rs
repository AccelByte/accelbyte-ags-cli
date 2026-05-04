use crate::common::cli_helpers::{ags, ags_isolated};
use predicates::prelude::*;

/// `auth logout --help` succeeds and mentions the logout subcommand
#[test]
fn test_auth_logout_help() {
    ags()
        .args(["auth", "logout", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("logout"));
}

/// Logout when already unauthenticated succeeds idempotently with a "cleared" message
#[test]
fn test_auth_logout_when_not_authenticated() {
    ags_isolated()
        .args(["auth", "logout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Credentials cleared"));
}

/// Logout prints per-credential status lines so the user knows what was actually removed
#[test]
fn test_auth_logout_shows_status_lines() {
    ags_isolated()
        .args(["auth", "logout"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Client ID:"))
        .stderr(predicate::str::contains("Client Secret:"))
        .stderr(predicate::str::contains("Access Token:"))
        .stderr(predicate::str::contains("Refresh Token:"));
}
