use crate::common::cli_helpers;

/// The removed `--ui=tui` alias is rejected as an unknown value at flag-parse
/// time (it was a deprecated alias for `--ui=inline`).
#[test]
fn test_ui_tui_alias_is_rejected() {
    let mut cmd = cli_helpers::ags();
    cmd.args(["--ui=tui", "iam", "users", "list"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown --ui value 'tui'"),
        "stderr did not contain the expected rejection: {stderr}"
    );
}

/// The removed `--format=tui` legacy alias is rejected as an unknown value.
#[test]
fn test_format_tui_alias_is_rejected() {
    let mut cmd = cli_helpers::ags();
    cmd.args(["--format=tui", "iam", "users", "list"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown --format value 'tui'"),
        "stderr did not contain the expected rejection: {stderr}"
    );
}

/// `--format=json` silently wins over `--ui`.
/// The combination is not a usage error; the resolver drops `--ui` and proceeds
/// on the JSON path. Any error that comes back here is the downstream command
/// (e.g. unknown subcommand) — it must NOT be the old
/// "cannot be combined with --format=json" usage error.
#[test]
fn test_ui_with_format_json_is_silently_ignored() {
    let mut cmd = cli_helpers::ags();
    cmd.args(["--format=json", "--ui=inline", "iam", "users", "list"]);
    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("cannot be combined with --format=json"),
        "the --ui + --format=json combination should be silently accepted, \
         but the old usage error was emitted: {stderr}"
    );
}
