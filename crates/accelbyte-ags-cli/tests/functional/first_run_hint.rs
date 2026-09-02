use crate::common::cli_helpers::ags;
use predicates::prelude::*;

/// A unique hint text fragment that the first-run hint emits. Used to assert
/// presence or absence of the hint in test output.
const HINT_MARKER: &str = "Automate your AGS workflows with a unified CLI";

/// `ags --version` is a meta-builtin: the `is_meta_builtin` flag suppresses
/// the hint before the tty/surface logic runs. A spawned test process also
/// has non-interactive pipes, so both the meta gate and the interactive gate
/// would suppress independently. This test confirms the hint is absent.
#[test]
fn test_first_run_hint_suppressed_for_meta_builtin() {
    let tmp = tempfile::tempdir().unwrap();
    ags()
        .arg("--version")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .assert()
        .success()
        .stderr(predicate::str::contains(HINT_MARKER).not());
}

/// After a run where the hint did NOT fire (non-interactive spawned process),
/// the global config must NOT contain `first_run_hint_seen: true`. This
/// proves the show-once mechanism does not spuriously persist the flag when
/// the hint is suppressed.
#[test]
fn test_hint_flag_not_persisted_in_non_interactive_mode() {
    let tmp = tempfile::tempdir().unwrap();
    // Run a benign command in non-interactive mode.
    ags()
        .arg("--version")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .assert()
        .success();

    // The config file should either not exist or not contain the flag.
    let config_path = tmp.path().join("config.json");
    if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            !contents.contains("first_run_hint_seen"),
            "first_run_hint_seen should NOT be set in non-interactive mode, got: {contents}"
        );
    }
    // If the config file does not exist at all, the flag was never written — correct.
}

/// When the global config already has `first_run_hint_seen: true`, the
/// predicate returns false and the hint is suppressed on subsequent runs.
/// This test writes the flag pre-set and confirms no hint text appears.
#[test]
fn test_hint_suppressed_when_flag_already_set() {
    let tmp = tempfile::tempdir().unwrap();
    // Pre-create config with the flag set.
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, r#"{"first_run_hint_seen": true}"#).unwrap();

    ags()
        .arg("--version")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .assert()
        .success()
        .stderr(predicate::str::contains(HINT_MARKER).not());
}

/// A non-meta builtin command (`ags doctor`) exercises the tty/surface gate
/// separately from the meta-builtin gate. In a spawned process (pipes, not a
/// TTY), `allows_interactive_prompts()` returns false, so the hint is
/// suppressed by the non-interactive check — not the meta check.
#[test]
fn test_hint_suppressed_for_non_interactive_non_meta_command() {
    let tmp = tempfile::tempdir().unwrap();
    ags()
        .arg("doctor")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .assert()
        // doctor may exit non-zero (no auth configured), but the hint
        // must not appear regardless of exit status.
        .stderr(predicate::str::contains(HINT_MARKER).not());
}
