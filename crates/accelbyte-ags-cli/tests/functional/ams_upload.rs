//! CLI-surface tests for `ags ams upload` — discoverability, flag contract,
//! and the pre-flight validation a user hits before any network call.

use std::io::Write;
use std::path::Path;

use crate::common::cli_helpers::ags_isolated;

/// Build a directory holding an x86-64 ELF entrypoint plus one data file and
/// one debug-symbol file.
fn build_directory() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "server", &elf_bytes(62));
    write(temp.path(), "assets/data.bin", b"asset bytes");
    write(temp.path(), "server.pdb", b"symbols");
    temp
}

/// Write `contents` to `name` (creating parents) inside `directory`.
fn write(directory: &Path, name: &str, contents: &[u8]) {
    let path = directory.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(&path)
        .unwrap()
        .write_all(contents)
        .unwrap();
}

/// A 20-byte little-endian 64-bit ELF header for the given `e_machine`.
fn elf_bytes(machine: u16) -> Vec<u8> {
    let mut header = vec![0u8; 20];
    header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    header[4] = 2;
    header[5] = 1;
    header[6] = 1;
    header[18..20].copy_from_slice(&machine.to_le_bytes());
    header
}

/// Run `ags ams upload` with the given args against `directory`, in dry-run.
fn dry_run(directory: &Path, extra: &[&str]) -> std::process::Output {
    let mut command = ags_isolated();
    command.args([
        "ams",
        "upload",
        "--path",
        directory.to_str().unwrap(),
        "--dry-run",
    ]);
    command.args(extra);
    command.output().unwrap()
}

#[test]
fn test_upload_is_listed_as_an_ams_resource() {
    let output = ags_isolated().args(["ams", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("upload"),
        "ags ams --help must list the upload resource:\n{stdout}"
    );
}

#[test]
fn test_upload_help_documents_the_new_flag_names() {
    let output = ags_isolated()
        .args(["ams", "upload", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--path",
        "--executable",
        "--image-name",
        "--target-arch",
        "--symbol-files",
        "--skip-script-validation",
    ] {
        assert!(
            stdout.contains(flag),
            "help must document {flag}:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("--namespace is accepted but ignored"),
        "help must explain that --namespace is a no-op:\n{stdout}"
    );
}

#[test]
fn test_dry_run_reports_the_plan_without_credentials() {
    let build = build_directory();
    let output = dry_run(
        build.path(),
        &["--executable", "server", "--image-name", "my-image"],
    );
    assert!(
        output.status.success(),
        "dry-run must work while logged out: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The plan is the command's result, so it is read from stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("linux-x86_64"), "{stdout}");
    assert!(stdout.contains("./server"), "{stdout}");
    assert!(
        stdout.contains("2 files"),
        "the .pdb must be excluded by default:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Would upload image"), "{stderr}");
    assert!(
        stderr.contains("Dry run: nothing was archived and no request was sent."),
        "{stderr}"
    );
}

#[test]
fn test_dry_run_json_envelope() {
    let build = build_directory();
    let output = dry_run(
        build.path(),
        &[
            "--executable",
            "server",
            "--image-name",
            "my-image",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "dry_run");
    assert_eq!(json["image_name"], "my-image");
    assert_eq!(json["command"], "./server");
    assert_eq!(json["target_architecture"], "linux-x86_64");
    assert_eq!(json["entrypoint_kind"], "elf_binary");
    assert_eq!(json["file_count"], 2);
    assert_eq!(json["excluded_symbol_file_count"], 1);
    assert!(json["upload_base_url"].is_null());
}

#[test]
fn test_dry_run_rejects_an_invalid_upload_url() {
    let build = build_directory();
    let output = dry_run(
        build.path(),
        &[
            "--executable",
            "server",
            "--image-name",
            "my-image",
            "--upload-url",
            "not-a-url",
        ],
    );
    assert!(
        !output.status.success(),
        "an invalid --upload-url must fail the dry run"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("AMS upload host 'not-a-url' is not an absolute http(s) URL"),
        "{stderr}"
    );
    assert!(
        stderr
            .contains("Pass --upload-url as an absolute URL, e.g. https://prod.ams.accelbyte.io."),
        "{stderr}"
    );
}

/// A symlinked directory is not archived, so the dry run has to name it — that
/// warning is the user's only chance to notice before the image ships short.
#[cfg(unix)]
#[test]
fn test_dry_run_names_skipped_directory_symlinks() {
    let build = build_directory();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "mod.pak", b"mod bytes");
    std::os::unix::fs::symlink(outside.path(), build.path().join("mods")).unwrap();

    let output = dry_run(
        build.path(),
        &[
            "--executable",
            "server",
            "--image-name",
            "my-image",
            "--format",
            "json",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["skipped_directory_symlinks"][0], "mods");
    assert_eq!(json["file_count"], 2, "the linked tree stays out");
}

#[test]
fn test_symbol_files_flag_includes_debug_symbols() {
    let build = build_directory();
    let output = dry_run(
        build.path(),
        &[
            "--executable",
            "server",
            "--image-name",
            "my-image",
            "--symbol-files",
            "--format",
            "json",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["file_count"], 3);
    assert_eq!(json["excluded_symbol_file_count"], 0);
}

#[test]
fn test_executable_and_image_name_are_required() {
    let build = build_directory();
    for args in [
        vec!["--executable", "server"],
        vec!["--image-name", "my-image"],
    ] {
        let output = dry_run(build.path(), &args);
        assert!(
            !output.status.success(),
            "{args:?} must be rejected as incomplete"
        );
    }
}

/// The old `ams` binary took `-c`/`-s`/`-H`. Dropping them is deliberate, so a
/// migrated pipeline must fail loudly rather than quietly ignore a secret.
#[test]
fn test_legacy_credential_flags_are_rejected() {
    let build = build_directory();
    let output = dry_run(
        build.path(),
        &[
            "--executable",
            "server",
            "--image-name",
            "my-image",
            "-c",
            "client-id",
            "-s",
            "client-secret",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("-c"), "{stderr}");
}

#[test]
fn test_shell_script_entrypoint_requires_target_arch() {
    let build = build_directory();
    write(build.path(), "start.sh", b"#!/bin/bash\nexec ./server\n");

    let without = dry_run(
        build.path(),
        &["--executable", "start.sh", "--image-name", "my-image"],
    );
    assert!(!without.status.success());
    assert!(String::from_utf8_lossy(&without.stderr).contains("--target-arch"));

    let with = dry_run(
        build.path(),
        &[
            "--executable",
            "start.sh",
            "--image-name",
            "my-image",
            "--target-arch",
            "linux-arm_64",
            "--format",
            "json",
        ],
    );
    assert!(with.status.success());
    let json: serde_json::Value = serde_json::from_slice(&with.stdout).unwrap();
    assert_eq!(json["entrypoint_kind"], "shell_script");
    assert_eq!(json["target_architecture"], "linux-arm_64");
}

#[test]
fn test_crlf_shell_script_is_rejected_but_can_be_skipped() {
    let build = build_directory();
    write(
        build.path(),
        "start.sh",
        b"#!/bin/bash\r\nexec ./server\r\n",
    );
    let args = [
        "--executable",
        "start.sh",
        "--image-name",
        "my-image",
        "--target-arch",
        "linux-x86_64",
    ];

    let rejected = dry_run(build.path(), &args);
    assert!(!rejected.status.success());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("CRLF"), "{stderr}");
    assert!(stderr.contains("--skip-script-validation"), "{stderr}");

    let mut skipped_args = args.to_vec();
    skipped_args.push("--skip-script-validation");
    assert!(dry_run(build.path(), &skipped_args).status.success());
}

#[test]
fn test_architecture_mismatch_is_rejected() {
    let build = build_directory();
    let output = dry_run(
        build.path(),
        &[
            "--executable",
            "server",
            "--image-name",
            "my-image",
            "--target-arch",
            "linux-arm_64",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not match the detected"), "{stderr}");
}

#[test]
fn test_non_elf_entrypoint_is_rejected() {
    let build = build_directory();
    write(build.path(), "windows.exe", b"MZ\x90\x00 a PE binary");
    let output = dry_run(
        build.path(),
        &["--executable", "windows.exe", "--image-name", "my-image"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ELF"), "{stderr}");
}

#[test]
fn test_image_name_length_is_validated() {
    let build = build_directory();
    let output = dry_run(
        build.path(),
        &["--executable", "server", "--image-name", "ab"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("at least 3 characters"));
}

/// The generated `ams` resources must be unaffected by the injected
/// hand-written one.
#[test]
fn test_generated_ams_resources_still_route() {
    let output = ags_isolated()
        .args([
            "ams",
            "images",
            "list",
            "--namespace",
            "test-ns",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ams images list --dry-run broke: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
