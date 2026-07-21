//! End-to-end dry-run coverage for the `season-pass` builtin workflow.
//! Offline: every case uses `--dry-run`. Pickers are not exercised offline;
//! input values are supplied via flags.

use crate::common::cli_helpers::ags_isolated;

#[test]
fn test_season_pass_dry_run_previews_all_eighteen_steps() {
    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "season-pass",
            "--namespace",
            "dev",
            "--store-id",
            "store-1",
            "--currency-code",
            "GOLD",
            "--free-reward-item-id",
            "free-1",
            "--premium-reward-item-id",
            "premium-1",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
    assert_eq!(json["workflow"], "season-pass");
    assert_eq!(json["dry_run"], true);
    let steps = json["steps"].as_array().expect("steps array");
    assert_eq!(
        steps.len(),
        18,
        "dry-run envelope must list all eighteen steps"
    );
    assert!(steps[0]["method"].is_string());
    assert!(steps[0]["url"].is_string());
    // Covers categories, items, store publish, seasons, passes, rewards, tiers.
    for fragment in [
        "/categories",
        "/items",
        "/catalogChanges/publishAll",
        "/seasons",
        "/passes",
        "/rewards",
        "/tiers",
        "/publish",
    ] {
        assert!(
            stdout.contains(fragment),
            "dry-run output missing '{fragment}':\n{stdout}"
        );
    }
}

#[test]
fn test_season_pass_help_lists_input_flags() {
    let assert = ags_isolated()
        .args(["workflow", "run", "season-pass", "--help"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    for flag in [
        "--namespace",
        "--season-name",
        "--start",
        "--end",
        "--store-id",
        "--currency-code",
        "--free-reward-item-id",
        "--premium-reward-item-id",
    ] {
        assert!(stderr.contains(flag), "help missing '{flag}':\n{stderr}");
    }
}

#[test]
fn test_season_pass_no_input_missing_namespace_exits_1() {
    ags_isolated()
        .env_remove("AGS_NAMESPACE")
        .args(["--dry-run", "--no-input", "workflow", "run", "season-pass"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_season_pass_dry_run_includes_optional_publish_steps() {
    // Verify that --dry-run previews all 18 steps including the two optional
    // publish steps (publish-store and publish-season). Dry-run must never
    // skip optional steps — it previews the full workflow regardless.
    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "season-pass",
            "--namespace",
            "dev",
            "--store-id",
            "store-1",
            "--currency-code",
            "GOLD",
            "--free-reward-item-id",
            "free-1",
            "--premium-reward-item-id",
            "premium-1",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
    let steps = json["steps"].as_array().expect("steps array");
    assert_eq!(
        steps.len(),
        18,
        "dry-run must preview all 18 steps, including optional publish steps"
    );
    let ids: Vec<&str> = steps
        .iter()
        .map(|s| s["id"].as_str().unwrap_or(""))
        .collect();
    assert!(
        ids.contains(&"publish-store"),
        "dry-run must include optional publish-store: {ids:?}"
    );
    assert!(
        ids.contains(&"publish-season"),
        "dry-run must include optional publish-season: {ids:?}"
    );
}

/// The date-time widget stores and sends the exact ISO wire value: a
/// `--dry-run --format json` run still shows `…T…Z` for start/end (the widget
/// is a form-edit affordance only; previews show the real payload).
#[test]
fn test_season_pass_dry_run_start_end_are_iso_on_the_wire() {
    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "season-pass",
            "--namespace",
            "dev",
            "--store-id",
            "store-1",
            "--currency-code",
            "GOLD",
            "--free-reward-item-id",
            "free-1",
            "--premium-reward-item-id",
            "premium-1",
            "--start",
            "2026-08-01T00:00:00Z",
            "--end",
            "2026-11-01T00:00:00Z",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("2026-08-01T00:00:00Z"),
        "start renders as ISO on the wire:\n{stdout}"
    );
    assert!(
        stdout.contains("2026-11-01T00:00:00Z"),
        "end renders as ISO on the wire:\n{stdout}"
    );
}
