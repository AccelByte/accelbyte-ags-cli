//! End-to-end dry-run coverage for the `competitive-multiplayer` builtin
//! workflow. Offline: every case uses `--dry-run`.
//!
//! The workflow's v3 simplified contract exposes four required inputs
//! (`namespace`, `fleetImageId`, `fleetInstanceId`, `fleetRegion`); the rest
//! carry defaults.

use crate::common::cli_helpers::ags_isolated;

#[test]
fn test_competitive_multiplayer_dry_run_previews_all_six_steps() {
    let assert = ags_isolated()
        .args([
            "--dry-run",
            "workflow",
            "run",
            "competitive-multiplayer",
            "--namespace",
            "dev",
            "--fleet-image-id",
            "img-1",
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Each step's request path must appear in the dry-run preview.
    for fragment in [
        "/stats",
        "/rulesets",
        "/configuration",
        "/match-pools",
        "/fleets",
        "/configurations/",
    ] {
        assert!(
            stdout.contains(fragment),
            "dry-run output missing '{fragment}':\n{stdout}"
        );
    }
}

#[test]
fn test_competitive_multiplayer_no_input_dry_run_succeeds_with_required_inputs() {
    ags_isolated()
        .args([
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "competitive-multiplayer",
            "--namespace",
            "dev",
            "--fleet-image-id",
            "img-1",
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .assert()
        .success();
}

#[test]
fn test_competitive_multiplayer_no_input_missing_required_input_exits_1() {
    // `ags_isolated()` gives a unique empty `AGS_HOME` (no config-file
    // namespace); `env_remove` drops any inherited `AGS_NAMESPACE`. With no
    // `--namespace` flag either — but the other required inputs supplied — the
    // required `namespace` input is genuinely absent and `--no-input` refuses
    // to gather.
    ags_isolated()
        .env_remove("AGS_NAMESPACE")
        .args([
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "competitive-multiplayer",
            "--fleet-image-id",
            "img-1",
            "--fleet-instance-id",
            "inst-1",
        ])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_competitive_multiplayer_help_lists_input_flags() {
    let assert = ags_isolated()
        .args(["workflow", "run", "competitive-multiplayer", "--help"])
        .assert()
        .success();
    // Help text is UI chrome — it renders on stderr so stdout
    // stays reserved for `CommandOutput`.
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    for flag in [
        "--namespace",
        "--fleet-image-id",
        "--fleet-instance-id",
        "--resource-prefix",
    ] {
        assert!(stderr.contains(flag), "help missing '{flag}':\n{stderr}");
    }
}

#[test]
fn test_competitive_multiplayer_json_dry_run_emits_envelope() {
    // --format json --dry-run runs offline (no auth) and needs no --yes even if
    // steps are confirm-gated (confirmation gates only live execution).
    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "competitive-multiplayer",
            "--namespace",
            "dev",
            "--fleet-image-id",
            "img-1",
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
    assert_eq!(json["workflow"], "competitive-multiplayer");
    assert_eq!(json["dry_run"], true);
    let steps = json["steps"].as_array().expect("steps array");
    assert!(
        !steps.is_empty(),
        "dry-run envelope must list steps:\n{stdout}"
    );
    assert!(steps[0]["method"].is_string(), "step has an HTTP method");
    assert!(steps[0]["url"].is_string(), "step has a URL");

    // A DS-type session template only claims the AMS fleet when dsSource is set.
    // Both session-template steps must carry dsSource=AMS in the request body, or
    // the session silently never claims a server (stays NEED_TO_REQUEST).
    for id in ["create-session-template", "update-session-template"] {
        let step = steps
            .iter()
            .find(|s| s["id"] == id)
            .unwrap_or_else(|| panic!("dry-run missing step '{id}':\n{stdout}"));
        assert_eq!(
            step["body"]["dsSource"], "AMS",
            "step '{id}' body must set dsSource=AMS:\n{stdout}"
        );
    }
}

#[test]
fn test_competitive_multiplayer_json_missing_input_emits_json_error() {
    // --format json implies non-interactive (no_input), so a missing required
    // input fails the precheck (exit 1) and the error is a JSON envelope on
    // stderr. --dry-run keeps it offline; env_remove drops any inherited
    // namespace so `namespace` is genuinely absent.
    let assert = ags_isolated()
        .env_remove("AGS_NAMESPACE")
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "competitive-multiplayer",
            "--fleet-image-id",
            "img-1",
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stderr)
        .unwrap_or_else(|e| panic!("stderr is not a JSON error envelope ({e}):\n{stderr}"));
    let error = json["error"]
        .as_str()
        .unwrap_or_else(|| panic!("`error` must be a string:\n{stderr}"));
    assert!(
        !error.is_empty(),
        "error message must be non-empty:\n{stderr}"
    );
    assert!(
        error.contains("namespace"),
        "error must name the missing 'namespace' input: {error}"
    );
}
