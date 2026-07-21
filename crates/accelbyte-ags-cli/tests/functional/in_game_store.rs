//! End-to-end dry-run coverage for the `in-game-store` builtin workflow.
//! Offline: every case uses `--dry-run`.
//!
//! Only `namespace` is required; `currencyCode` carries a default, so a dry run
//! needs just `--namespace`.

use crate::common::cli_helpers::ags_isolated;

#[test]
fn test_in_game_store_dry_run_previews_all_eight_steps() {
    let assert = ags_isolated()
        .args([
            "--dry-run",
            "workflow",
            "run",
            "in-game-store",
            "--namespace",
            "dev",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Each step's request path appears in the dry-run preview: store, currency,
    // categories (x3), items (x2), publish (catalog changes).
    for fragment in [
        "/stores",
        "/currencies",
        "/categories",
        "/items",
        "/catalogChanges/publishAll",
    ] {
        assert!(
            stdout.contains(fragment),
            "dry-run output missing '{fragment}':\n{stdout}"
        );
    }
}

#[test]
fn test_in_game_store_no_input_dry_run_succeeds_with_namespace() {
    ags_isolated()
        .args([
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "in-game-store",
            "--namespace",
            "dev",
        ])
        .assert()
        .success();
}

#[test]
fn test_in_game_store_no_input_missing_namespace_exits_1() {
    ags_isolated()
        .env_remove("AGS_NAMESPACE")
        .args([
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "in-game-store",
        ])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_in_game_store_help_lists_input_flags() {
    let assert = ags_isolated()
        .args(["workflow", "run", "in-game-store", "--help"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    for flag in ["--namespace", "--currency-code"] {
        assert!(stderr.contains(flag), "help missing '{flag}':\n{stderr}");
    }
    // resourcePrefix / itemPrice are gone.
    for gone in ["--resource-prefix", "--item-price"] {
        assert!(
            !stderr.contains(gone),
            "help still lists removed flag '{gone}':\n{stderr}"
        );
    }
}

#[test]
fn test_in_game_store_json_dry_run_emits_envelope() {
    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "in-game-store",
            "--namespace",
            "dev",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
    assert_eq!(json["workflow"], "in-game-store");
    assert_eq!(json["dry_run"], true);
    let steps = json["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 8, "dry-run envelope must list all eight steps");
    assert!(steps[0]["method"].is_string(), "step has an HTTP method");
    assert!(steps[0]["url"].is_string(), "step has a URL");
}
