use crate::common::cli_helpers::{ags, ags_isolated};
use predicates::prelude::*;

/// With no stored credentials, `auth status` reports "Not authenticated" for scripted checks
#[test]
fn test_auth_status_no_credentials() {
    ags_isolated()
        .args(["auth", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Not authenticated"));
}

/// `auth --help` lists all auth subcommands so users can discover login/logout/status
#[test]
fn test_auth_help() {
    ags()
        .args(["auth", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("logout"))
        .stdout(predicate::str::contains("status"));
}

/// An unknown auth subcommand fails with Clap's built-in "unrecognized subcommand" message
#[test]
fn test_auth_unknown_command() {
    ags()
        .args(["auth", "badcmd"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unrecognized subcommand 'badcmd'"));
}
