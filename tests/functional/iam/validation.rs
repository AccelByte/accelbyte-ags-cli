use crate::common::cli_helpers::ags;
use predicates::prelude::*;

/// A close misspelling of a resource triggers Clap's "did you mean" suggestion
#[test]
fn test_misspelled_resource_suggests_correction() {
    ags()
        .args(["iam", "userz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unrecognized subcommand 'userz'"))
        .stderr(predicate::str::contains("users"));
}

/// Appending --help to a misspelled resource still shows the correction suggestion
#[test]
fn test_misspelled_resource_with_help_suggests_correction() {
    ags()
        .args(["iam", "userz", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unrecognized subcommand 'userz'"))
        .stderr(predicate::str::contains("users"));
}

/// A truncated method name suggests the closest match so users can fix typos quickly
#[test]
fn test_misspelled_method_suggests_correction() {
    ags()
        .args(["iam", "bans", "list-reaso"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Unrecognized subcommand 'list-reaso'",
        ))
        .stderr(predicate::str::contains("list-reason"));
}

/// Appending --help to a misspelled method still shows the correction suggestion
#[test]
fn test_misspelled_method_with_help_suggests_correction() {
    ags()
        .args(["iam", "bans", "list-reaso", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Unrecognized subcommand 'list-reaso'",
        ))
        .stderr(predicate::str::contains("list-reason"));
}

/// An unrecognised flag on an operation is rejected so typos don't silently pass
#[test]
fn test_unknown_flag_on_operation() {
    ags()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--badopt",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--badopt"));
}

/// A flag that requires a value fails when invoked bare (e.g. `--limit` with no number)
#[test]
fn test_flag_missing_value() {
    ags()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--limit",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--limit"));
}

/// A completely unrelated resource name (no close match) is still rejected cleanly
#[test]
fn test_completely_unknown_resource() {
    ags()
        .args(["iam", "zzzzz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unrecognized subcommand 'zzzzz'"));
}

/// A completely unrelated method name (no close match) is still rejected cleanly
#[test]
fn test_completely_unknown_method() {
    ags()
        .args(["iam", "bans", "zzzzz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unrecognized subcommand 'zzzzz'"));
}

/// Trailing positional arguments after the method name are rejected to prevent silent mis-use
#[test]
fn test_extra_positional_after_method() {
    ags()
        .args(["iam", "bans", "list-reason", "extra-junk"])
        .assert()
        .failure();
}
