//! End-to-end dry-run coverage for the `competitive-multiplayer` builtin
//! workflow. Offline: every case uses `--dry-run`.
//!
//! The workflow's required inputs are `namespace`, `buildPath`,
//! `buildExecutable`, `fleetInstanceId`, and `fleetRegion`; the rest carry
//! defaults.

use std::path::Path;

use crate::common::cli_helpers::ags_isolated;

/// A build directory the local upload step can validate: one 64-bit
/// little-endian x86-64 ELF entrypoint. The step's dry run checks the
/// entrypoint on disk, so a path that does not exist fails the run.
pub(crate) fn build_directory() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let mut header = vec![0u8; 20];
    header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    header[4] = 2;
    header[5] = 1;
    header[6] = 1;
    header[18..20].copy_from_slice(&62u16.to_le_bytes());
    std::fs::write(temp.path().join("server"), &header).unwrap();
    temp
}

/// The build inputs pointing at `directory`, in flag form.
pub(crate) fn build_args(directory: &Path) -> [&str; 4] {
    [
        "--build-path",
        directory.to_str().unwrap(),
        "--build-executable",
        "server",
    ]
}

#[test]
fn test_competitive_multiplayer_dry_run_previews_all_seven_steps() {
    let build = build_directory();
    let assert = ags_isolated()
        .args([
            "--dry-run",
            "workflow",
            "run",
            "competitive-multiplayer",
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
    // Each operation step's request path must appear in the dry-run preview,
    // and the local upload step must appear as itself.
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
fn test_competitive_multiplayer_no_input_dry_run_succeeds_with_required_inputs() {
    let build = build_directory();
    ags_isolated()
        .args([
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "competitive-multiplayer",
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
}

#[test]
fn test_competitive_multiplayer_no_input_missing_required_input_exits_1() {
    // `ags_isolated()` gives a unique empty `AGS_HOME` (no config-file
    // namespace); `env_remove` drops any inherited `AGS_NAMESPACE`. With no
    // `--namespace` flag either — but the other required inputs supplied — the
    // required `namespace` input is genuinely absent and `--no-input` refuses
    // to gather.
    let build = build_directory();
    ags_isolated()
        .env_remove("AGS_NAMESPACE")
        .args([
            "--dry-run",
            "--no-input",
            "workflow",
            "run",
            "competitive-multiplayer",
            "--fleet-instance-id",
            "inst-1",
        ])
        .args(build_args(build.path()))
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
        "--build-path",
        "--build-executable",
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
    let build = build_directory();
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
    assert_eq!(json["workflow"], "competitive-multiplayer");
    assert_eq!(json["dry_run"], true);
    let steps = json["steps"].as_array().expect("steps array");
    assert!(
        !steps.is_empty(),
        "dry-run envelope must list steps:\n{stdout}"
    );
    assert!(steps[0]["method"].is_string(), "step has an HTTP method");
    assert!(steps[0]["url"].is_string(), "step has a URL");

    // The local upload step is reported too, in its own shape.
    let upload = steps
        .iter()
        .find(|s| s["id"] == "upload-image")
        .unwrap_or_else(|| panic!("dry-run missing the local upload step:\n{stdout}"));
    assert_eq!(upload["action"], "ams/upload-image");

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

/// A symlinked directory is left out of the archive, so the workflow's preview
/// must name it — otherwise the only warning about a truncated image lives on
/// the live path, and a dry run reads as if nothing were missing.
#[cfg(unix)]
#[test]
fn test_competitive_multiplayer_dry_run_names_skipped_directory_symlinks() {
    let build = build_directory();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("texture.bin"), b"asset").unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), build.path().join("assets")).unwrap();

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
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .args(build_args(build.path()))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let upload = json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "upload-image")
        .unwrap_or_else(|| panic!("dry-run missing the upload step:\n{stdout}"));
    assert_eq!(
        upload["preview"]["skipped_directory_symlinks"],
        serde_json::json!(["assets"]),
        "the skipped symlink must be named in the preview:\n{stdout}"
    );
}

#[test]
fn test_competitive_multiplayer_json_missing_input_emits_json_error() {
    // --format json implies non-interactive (no_input), so a missing required
    // input fails the precheck (exit 1) and the error is a JSON envelope on
    // stderr. --dry-run keeps it offline; env_remove drops any inherited
    // namespace so `namespace` is genuinely absent.
    let build = build_directory();
    let assert = ags_isolated()
        .env_remove("AGS_NAMESPACE")
        .args([
            "--format",
            "json",
            "--dry-run",
            "workflow",
            "run",
            "competitive-multiplayer",
            "--fleet-instance-id",
            "inst-1",
            "--fleet-region",
            "us-east-1",
        ])
        .args(build_args(build.path()))
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
