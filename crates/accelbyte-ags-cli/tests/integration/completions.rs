//! End-to-end tests for `ags completions`.

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn test_explicit_zsh_prints_script_with_no_stderr_hint() {
    Command::cargo_bin("ags")
        .unwrap()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(contains("#compdef ags"))
        .stderr(predicates::str::is_empty());
}

#[test]
fn test_explicit_bash_prints_script() {
    Command::cargo_bin("ags")
        .unwrap()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(contains("complete -"));
}

#[test]
fn test_explicit_fish_prints_script() {
    Command::cargo_bin("ags")
        .unwrap()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(contains("complete -c ags"));
}

#[test]
fn test_explicit_powershell_prints_script() {
    Command::cargo_bin("ags")
        .unwrap()
        .args(["completions", "powershell"])
        .assert()
        .success()
        .stdout(contains("Register-ArgumentCompleter"));
}

#[test]
fn test_auto_detected_zsh_prints_hint_to_stderr() {
    Command::cargo_bin("ags")
        .unwrap()
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
    Command::cargo_bin("ags")
        .unwrap()
        .args(["completions"])
        .env_remove("SHELL")
        .assert()
        .failure()
        .stderr(contains("Could not detect shell"));
}

#[test]
fn test_explicit_bogus_shell_is_usage_error() {
    Command::cargo_bin("ags")
        .unwrap()
        .args(["completions", "tcsh"])
        .assert()
        .failure()
        .stderr(contains("possible values"));
}
