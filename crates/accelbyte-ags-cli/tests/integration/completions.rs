//! End-to-end tests for `ags completions`.

use crate::common::cli_helpers;
use predicates::str::contains;

#[test]
fn test_explicit_zsh_prints_script_with_no_stderr_hint() {
    cli_helpers::ags()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(contains("#compdef ags"))
        .stderr(predicates::str::is_empty());
}

#[test]
fn test_explicit_bash_prints_script() {
    cli_helpers::ags()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(contains("complete -"));
}

#[test]
fn test_explicit_fish_prints_script() {
    cli_helpers::ags()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(contains("complete -c ags"));
}

#[test]
fn test_explicit_powershell_prints_script() {
    cli_helpers::ags()
        .args(["completions", "powershell"])
        .assert()
        .success()
        .stdout(contains("Register-ArgumentCompleter"));
}

#[test]
fn test_auto_detected_zsh_prints_hint_to_stderr() {
    cli_helpers::ags()
        .args(["completions"])
        .env("SHELL", "/bin/zsh")
        .assert()
        .success()
        .stdout(contains("#compdef ags"))
        .stderr(contains("Detected zsh"));
}

#[test]
fn test_auto_detect_fails_when_shell_unset_on_non_windows() {
    if cfg!(windows) {
        return;
    }
    cli_helpers::ags()
        .args(["completions"])
        .env_remove("SHELL")
        .assert()
        .failure()
        .stderr(contains("Could not detect shell"));
}

#[test]
fn test_explicit_bogus_shell_is_usage_error() {
    cli_helpers::ags()
        .args(["completions", "tcsh"])
        .assert()
        .failure()
        .stderr(contains("possible values"));
}
