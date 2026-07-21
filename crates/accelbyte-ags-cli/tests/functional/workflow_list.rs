//! Functional coverage for `ags workflow list` and workflow discoverability.

use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

/// `ags workflow list` names the built-in `competitive-multiplayer` workflow.
#[test]
fn test_workflow_list_shows_builtin() {
    ags_isolated()
        .args(["workflow", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("competitive-multiplayer"));
}

/// `ags workflow list --format json` emits a valid JSON array of entries.
#[test]
fn test_workflow_list_json_is_valid() {
    let output = ags_isolated()
        .args(["--format", "json", "workflow", "list"])
        .output()
        .unwrap();
    assert!(output.status.success(), "expected success");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be valid JSON");
    assert!(parsed.is_array(), "expected a JSON array");
    assert!(
        parsed
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["id"] == "competitive-multiplayer" && entry["name"].is_string() }),
        "expected an entry for competitive-multiplayer with a name: {stdout}"
    );
}

/// `ags workflow --help` lists the subcommands but not the registered-workflow
/// list, which belongs to `ags workflow list`. Help is UI chrome on stderr, so
/// stdout stays reserved for `CommandOutput`.
#[test]
fn test_workflow_help_omits_registered_workflows() {
    ags_isolated()
        .args(["workflow", "--help"])
        .assert()
        .success()
        .stderr(predicate::str::contains("run"))
        .stderr(predicate::str::contains("list"))
        .stderr(predicate::str::contains("Registered workflows:").not());
}
