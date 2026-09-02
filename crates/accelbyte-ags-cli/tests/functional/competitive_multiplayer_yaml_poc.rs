//! End-to-end dry-run coverage for `competitive-multiplayer-yaml-poc` — the
//! bundled-YAML translation of the `competitive-multiplayer` builtin,
//! registered under a separate id alongside its Rust original. Mirrors
//! `competitive_multiplayer.rs`'s dry-run assertions to prove the YAML
//! version behaves identically. Offline: every case uses `--dry-run`.

use ags_protocol::workflow::WorkflowId;
use ags_runtime::runtime::workflows::registry;

use crate::common::cli_helpers::ags_isolated;
use crate::competitive_multiplayer::{build_args, build_directory};

#[test]
fn test_competitive_multiplayer_yaml_poc_dry_run_previews_all_seven_steps() {
    let build = build_directory();
    let assert = ags_isolated()
        .args([
            "--dry-run",
            "workflow",
            "run",
            "competitive-multiplayer-yaml-poc",
            "--namespace",
            "dev",
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .args(build_args(build.path()))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Each step's request path must appear in the dry-run preview — same
    // fragments as the Rust original's equivalent test.
    for fragment in [
        "/stats",
        "/rulesets",
        "/configuration",
        "/match-pools",
        "/fleets",
        "/configurations/",
        "ams/upload-image",
    ] {
        assert!(
            stdout.contains(fragment),
            "dry-run output missing '{fragment}':\n{stdout}"
        );
    }
}

#[test]
fn test_competitive_multiplayer_yaml_poc_json_dry_run_emits_envelope() {
    let build = build_directory();
    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "competitive-multiplayer-yaml-poc",
            "--namespace",
            "dev",
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .args(build_args(build.path()))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
    assert_eq!(json["workflow"], "competitive-multiplayer-yaml-poc");
    assert_eq!(json["dry_run"], true);
    let steps = json["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 7, "dry-run envelope must list all seven steps");

    // Same dsSource=AMS wiring check as the Rust original — proves the
    // nested-path bindings translated correctly, not just that something parsed.
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

/// Runs both workflows through the real CLI with `--dry-run --format json`
/// on identical inputs and diffs the emitted step previews directly — the
/// black-box counterpart to `test_competitive_multiplayer_yaml_poc_matches_rust_definition`
/// below, which only compares the internal `WorkflowDefinition` structs.
/// This proves the actual dispatched method/url/query/body the CLI reports
/// are identical, not just that the definitions that produce them are equal.
#[test]
fn test_competitive_multiplayer_yaml_poc_dry_run_matches_rust_cli_json() {
    // One build directory for both runs: the local step's preview reports the
    // archive size, so two directories would differ for a reason that says
    // nothing about the translation.
    let build = build_directory();
    let run_dry = |workflow_id: &str| -> serde_json::Value {
        let assert = ags_isolated()
            .args([
                "--format",
                "json",
                "--dry-run",
                "workflow",
                "run",
                workflow_id,
                "--namespace",
                "dev",
                "--fleet-instance-id",
                "inst-1",
                "--fleet-region",
                "us-east-1",
            ])
            .args(build_args(build.path()))
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"))
    };

    let rust_json = run_dry("competitive-multiplayer");
    let yaml_json = run_dry("competitive-multiplayer-yaml-poc");

    assert_eq!(rust_json["dry_run"], true);
    assert_eq!(yaml_json["dry_run"], true);

    let rust_steps = rust_json["steps"].as_array().expect("rust steps array");
    let yaml_steps = yaml_json["steps"].as_array().expect("yaml steps array");
    assert_eq!(
        rust_steps.len(),
        yaml_steps.len(),
        "both workflows must dry-run the same number of steps"
    );

    for (rust_step, yaml_step) in rust_steps.iter().zip(yaml_steps.iter()) {
        let step_id = rust_step["id"].clone();
        assert_eq!(
            step_id, yaml_step["id"],
            "step order/ids must match between the Rust and YAML workflows"
        );
        // `action` and `preview` are absent on an operation step and populated
        // on a local one, so this covers both kinds in the same comparison.
        for field in ["method", "url", "query", "body", "action", "preview"] {
            assert_eq!(
                rust_step[field], yaml_step[field],
                "step '{step_id}' field '{field}' differs between Rust and YAML dry-run output"
            );
        }
    }
}

/// Compares the full `WorkflowDefinition` — briefing, inputs, steps (with
/// their bindings), outputs, and completion — rather than the handful of
/// dry-run fragments checked above. This is "everything the UI system would
/// see" for the workflow: `id` is the only field expected to differ between
/// the hand-written Rust original and its YAML translation.
#[test]
fn test_competitive_multiplayer_yaml_poc_matches_rust_definition() {
    let registry = registry();
    let mut rust_def = registry
        .resolve(&WorkflowId::new("competitive-multiplayer"))
        .expect("rust builtin 'competitive-multiplayer' must be registered")
        .definition()
        .clone();
    let mut yaml_def = registry
        .resolve(&WorkflowId::new("competitive-multiplayer-yaml-poc"))
        .expect("yaml poc 'competitive-multiplayer-yaml-poc' must be registered")
        .definition()
        .clone();

    let placeholder_id = WorkflowId::new("competitive-multiplayer-comparison");
    rust_def.id = placeholder_id.clone();
    yaml_def.id = placeholder_id;

    assert_eq!(
        rust_def, yaml_def,
        "YAML translation must match the Rust original in everything the UI \
         shows (briefing, inputs, steps, bindings, outputs, completion) — only \
         'id' is allowed to differ"
    );
}
