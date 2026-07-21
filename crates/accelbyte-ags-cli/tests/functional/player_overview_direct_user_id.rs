//! player-overview accepts a user id directly, with no search query.
//! Offline: every case uses `--dry-run`.

use crate::common::cli_helpers::ags_isolated;

#[test]
fn test_player_overview_no_input_dry_run_with_direct_user_id_only() {
    // --user-id alone, no --search-query, under --no-input must succeed: the
    // supplied id is bound and the (now optional) search inputs are not demanded.
    ags_isolated()
        .args([
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "player-overview",
            "--namespace",
            "dev",
            "--user-id",
            "abc123",
        ])
        .assert()
        .success();
}

#[test]
fn test_player_overview_json_dry_run_binds_direct_user_id() {
    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "player-overview",
            "--namespace",
            "dev",
            "--user-id",
            "abc123",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
    assert_eq!(json["workflow"], "player-overview");
    // The account step GETs the user by id, so the supplied id appears in a
    // step URL — proof the direct id was bound with no search.
    let steps = json["steps"].as_array().expect("steps array");
    let any_url_has_id = steps
        .iter()
        .filter_map(|s| s["url"].as_str())
        .any(|u| u.contains("abc123"));
    assert!(
        any_url_has_id,
        "supplied user id must appear in a step URL:\n{stdout}"
    );
}
