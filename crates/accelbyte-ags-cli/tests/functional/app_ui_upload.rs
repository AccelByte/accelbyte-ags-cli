//! Functional tests for `ags extend app-ui upload`.
//!
//! Exercises the full CLI path: argv -> clap parse -> handler -> dispatch -> render.
//! Uses wiremock at the HTTP boundary; `--no-build` with a pre-populated build
//! directory so no package manager, cluster, or credentials are needed.

use crate::common::cli_helpers::{ags_isolated, ags_with_base_url};
use crate::common::wiremock_helpers::mount_token_success;
use predicates::prelude::*;
use wiremock::matchers::{header_regex, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Helpers ──

/// Create a project directory with a populated `dist/` build output.
/// Returns the project directory path.
fn create_project_with_build(parent: &std::path::Path) -> std::path::PathBuf {
    let project = parent.join("project");
    let dist = project.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), "<html><body>test</body></html>").unwrap();
    std::fs::write(dist.join("app.js"), "console.log('hello')").unwrap();
    project
}

/// Recursively find `.zip` files in a directory tree.
fn find_zip_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut zips = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                zips.extend(find_zip_files(&p));
            } else if p.extension().and_then(|e| e.to_str()) == Some("zip") {
                zips.push(p);
            }
        }
    }
    zips
}

// ── --help ──

/// `--help` renders usage text on stdout and exits cleanly.
#[test]
fn test_help_renders_usage() {
    ags_isolated()
        .args(["extend", "app-ui", "upload", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("upload"))
        .stdout(predicate::str::contains("--name"))
        .stdout(predicate::str::contains("--no-build"))
        .stdout(predicate::str::contains("--build-path"))
        .stdout(predicate::str::contains("--build-version"));
}

// ── Missing required flags ──

/// Without `--name` the command fails with a Clap error.
#[test]
fn test_missing_name_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    ags_isolated()
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "upload",
            "--no-build",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--name"));
}

/// Without `--namespace` (and no profile default) the command exits 1
/// with a usage error mentioning namespace.
#[test]
fn test_missing_namespace_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    ags_isolated()
        .args([
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--no-build",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("namespace"));
}

// ── Success ──

/// Happy-path: mock returns 200 with a CSM envelope. The command exits 0
/// and the temp archive is cleaned up afterwards.
#[tokio::test]
async fn test_success_exits_zero_and_cleans_up_archive() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(
            "/csm/v1/admin/namespaces/test-ns/app-ui/my-app/files/upload",
        ))
        .and(header_regex("content-type", "^multipart/form-data"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"ok":true}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());
    let archive_tmp = tempfile::TempDir::new().unwrap();

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        // Redirect temp files to a known directory so we can verify cleanup.
        .env("TMPDIR", archive_tmp.path())
        .env("TMP", archive_tmp.path())
        .env("TEMP", archive_tmp.path())
        .args([
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--build-version",
            "test-ver",
            "--no-build",
            "--namespace",
            "test-ns",
            "--project-path",
        ])
        .arg(project.to_str().unwrap());

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Cleanup invariant: no .zip files remain in the archive temp directory.
    let remaining_zips = find_zip_files(archive_tmp.path());
    assert!(
        remaining_zips.is_empty(),
        "temp archive must be cleaned up after success, but found: {remaining_zips:?}"
    );
}

// ── Upload failure (500) ──

/// Mock returns 500. The command fails and the archive is cleaned up.
/// This is the failure half of the cleanup invariant: even on upload
/// failure, the temp archive must be removed.
#[tokio::test]
async fn test_upload_failure_500_returns_error_and_cleans_up() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(
            "/csm/v1/admin/namespaces/test-ns/app-ui/my-app/files/upload",
        ))
        .respond_with(
            ResponseTemplate::new(500)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":"internal error"}"#),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());
    let archive_tmp = tempfile::TempDir::new().unwrap();

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        // Redirect temp files to a known directory so we can verify cleanup.
        .env("TMPDIR", archive_tmp.path())
        .env("TMP", archive_tmp.path())
        .env("TEMP", archive_tmp.path())
        .args([
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--build-version",
            "test-ver",
            "--no-build",
            "--namespace",
            "test-ns",
            "--project-path",
        ])
        .arg(project.to_str().unwrap());

    let output = cmd.output().unwrap();
    assert!(
        !output.status.success(),
        "expected failure on 500, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Cleanup invariant: no .zip files remain even on failure.
    let remaining_zips = find_zip_files(archive_tmp.path());
    assert!(
        remaining_zips.is_empty(),
        "temp archive must be cleaned up even on failure, but found: {remaining_zips:?}"
    );
}

// ── 413 Entity Too Large ──

/// Mock returns 413 with an AccelByte-format error body. The error
/// classification extracts `errorMessage` and surfaces it in the CLI
/// output. The test uses the AccelByte error envelope (`errorMessage`
/// field), which the classify system recognizes for catch-all HTTP
/// status codes.
#[tokio::test]
async fn test_413_entity_too_large_surfaces_server_message() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(
            "/csm/v1/admin/namespaces/test-ns/app-ui/my-app/files/upload",
        ))
        .respond_with(
            ResponseTemplate::new(413)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"errorMessage":"payload too large"}"#),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--build-version",
            "test-ver",
            "--no-build",
            "--namespace",
            "test-ns",
            "--project-path",
        ])
        .arg(project.to_str().unwrap());

    let output = cmd.output().unwrap();
    assert!(
        !output.status.success(),
        "expected failure on 413, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The classify system extracts `errorMessage` from the body and
    // surfaces it as the primary error message (the catch-all arm for
    // unrecognized status codes uses `clean_message.unwrap_or(HTTP N)`).
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("payload too large"),
        "error must surface the server's errorMessage verbatim: {stderr}"
    );
}

// ── Request shape ──

/// The upload request carries `?version=<build-version>` and sends
/// `multipart/form-data`. The mock requires BOTH matchers; if the
/// version query param is absent, the mock does not match and the
/// command fails — proving the query param is emitted.
#[tokio::test]
async fn test_request_carries_version_query_and_multipart_content_type() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    // The mock matches ONLY when the version query param is present
    // AND the content type is multipart/form-data. An absent query
    // param causes a wiremock 404, which makes the command fail.
    Mock::given(method("POST"))
        .and(path(
            "/csm/v1/admin/namespaces/test-ns/app-ui/my-app/files/upload",
        ))
        .and(query_param("version", "exact-ver"))
        .and(header_regex("content-type", "^multipart/form-data"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"ok":true}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--build-version",
            "exact-ver",
            "--no-build",
            "--namespace",
            "test-ns",
            "--project-path",
        ])
        .arg(project.to_str().unwrap());

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success (mock matches only with version query + multipart), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The expect(1) on the mock is verified by wiremock when the
    // MockServer drops — a mismatch (0 hits) panics the test.
}

// ── --format json ──

/// `--format json` renders the CSM envelope through the frontend
/// renderer. stdout must parse as valid JSON with the expected fields.
#[tokio::test]
async fn test_format_json_renders_envelope() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(
            "/csm/v1/admin/namespaces/test-ns/app-ui/my-app/files/upload",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"someField":"someValue"}"#),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--format",
            "json",
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--build-version",
            "json-ver",
            "--no-build",
            "--namespace",
            "test-ns",
            "--project-path",
        ])
        .arg(project.to_str().unwrap());

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success with --format json, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));

    assert_eq!(json["name"], "my-app", "JSON envelope must include name");
    assert_eq!(
        json["version"], "json-ver",
        "JSON envelope must include version"
    );
    assert!(
        json["archive_bytes"].is_number(),
        "JSON envelope must include archive_bytes as number: {json}"
    );
    assert!(
        json["response"].is_object(),
        "JSON envelope must include response object: {json}"
    );
    assert_eq!(
        json["response"]["someField"], "someValue",
        "response must carry the CSM body verbatim"
    );
}

// ── --dry-run ──

/// `--dry-run` exits 0 without spawning a build subprocess or making
/// any HTTP call. No mock server is started — if the command attempted
/// a network call it would fail.
#[test]
fn test_dry_run_exits_zero_no_upload() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--build-version",
            "v1",
            "--no-build",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .assert()
        .success()
        .stderr(predicate::str::contains("Dry run"));
}

// ── Build failure: stderr captured in error ──

/// A build subprocess that writes to stderr and exits non-zero must
/// produce a `CliError` whose message CONTAINS the subprocess's stderr
/// text. On Unix a fake `npm` script on PATH exercises the full CLI
/// path; on Windows `CreateProcessW` cannot find extensionless scripts,
/// so the test skips — the unit test
/// `test_execute_build_captures_stderr_in_error` (in upload.rs) covers
/// the same code path on all platforms.
///
/// The `#[cfg(not(windows))]` gates the body so clippy does not warn
/// about unreachable code; the function itself is unconditional so it
/// appears in the test listing on every platform.
#[test]
fn test_build_failure_stderr_captured_in_error() {
    #[cfg(windows)]
    {
        eprintln!(
            "SKIP: fake npm requires an executable shim on Windows; \
             the unit test covers this code path"
        );
    }

    #[cfg(not(windows))]
    {
        let tmp = tempfile::TempDir::new().unwrap();
        let project = create_project_with_build(tmp.path());

        // Create a fake npm that writes a marker to stderr and exits 1.
        let fake_bin_dir = tmp.path().join("fake-bin");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();

        let script = fake_bin_dir.join("npm");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'FAKE_BUILD_ERROR_XYZ' >&2\nexit 1\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        // package.json so PM detection finds npm.
        std::fs::write(project.join("package.json"), "{}").unwrap();

        // Prepend our fake-bin dir to PATH so `npm` resolves to the stub.
        let existing_path = std::env::var("PATH").unwrap_or_default();
        let new_path = std::env::join_paths(
            std::iter::once(fake_bin_dir).chain(std::env::split_paths(&existing_path)),
        )
        .unwrap();

        let output = ags_isolated()
            .env("PATH", &new_path)
            .args([
                "--namespace",
                "test-ns",
                "extend",
                "app-ui",
                "upload",
                "--name",
                "my-app",
                "--build-version",
                "v1",
                "--project-path",
            ])
            .arg(project.to_str().unwrap())
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "command must fail when build exits non-zero"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        // The structured CliError message (rendered to stderr) must contain
        // the fake npm's stderr output. With Stdio::inherit() this was empty.
        assert!(
            stderr.contains("FAKE_BUILD_ERROR_XYZ"),
            "error message must contain the build tool's stderr output:\n{stderr}"
        );
    }
}

// ── Input validation wiring ──
//
// These tests prove that handle_app_ui_upload WIRES its calls to
// validate_safe_component. The unit tests in the #[cfg(test)] module
// of upload.rs exercise the validator in isolation; these functional
// tests exercise the full CLI path and would fail if the handler
// stopped calling the validator.

/// `--name ../evil` is rejected before any archive or HTTP request.
/// The error message names the `--name` flag and cites the traversal
/// sequence. No mock server is started — if the command reached the
/// network boundary it would fail differently (missing credentials),
/// proving the rejection is at the validation layer.
#[test]
fn test_name_with_path_traversal_rejected_before_upload() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());
    let archive_tmp = tempfile::TempDir::new().unwrap();

    let output = ags_isolated()
        .env("TMPDIR", archive_tmp.path())
        .env("TMP", archive_tmp.path())
        .env("TEMP", archive_tmp.path())
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "upload",
            "--name",
            "../evil",
            "--build-version",
            "v1",
            "--no-build",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "path-traversal name must be rejected"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--name"),
        "error must name the --name flag: {stderr}"
    );
    assert!(
        stderr.contains(".."),
        "error must cite the traversal sequence: {stderr}"
    );

    // No archive must have been created — validation fires before
    // archive creation (run_steps_1_to_3).
    let remaining_zips = find_zip_files(archive_tmp.path());
    assert!(
        remaining_zips.is_empty(),
        "no archive must be created when validation rejects the name: {remaining_zips:?}"
    );
}

/// `--build-version v1/evil` is rejected before any archive or HTTP
/// request. The error message names the `--build-version` flag. No
/// mock server is started — reaching the network boundary would
/// produce a different error (missing credentials).
#[test]
fn test_build_version_with_slash_rejected_before_upload() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());
    let archive_tmp = tempfile::TempDir::new().unwrap();

    let output = ags_isolated()
        .env("TMPDIR", archive_tmp.path())
        .env("TMP", archive_tmp.path())
        .env("TEMP", archive_tmp.path())
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "upload",
            "--name",
            "valid-app",
            "--build-version",
            "v1/evil",
            "--no-build",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "slash in build-version must be rejected"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--build-version"),
        "error must name the --build-version flag: {stderr}"
    );

    // No archive must have been created.
    let remaining_zips = find_zip_files(archive_tmp.path());
    assert!(
        remaining_zips.is_empty(),
        "no archive must be created when validation rejects the build-version: {remaining_zips:?}"
    );
}

/// Selectivity control: a valid `--name` and `--build-version` pair
/// passes validation and reaches the dry-run preview. If the
/// rejection tests above passed because ALL inputs were blanket-
/// rejected (e.g. a broken validator that rejects everything), this
/// test would fail — proving the rejection is selective.
#[test]
fn test_valid_name_and_version_pass_validation_in_dry_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "upload",
            "--name",
            "safe-app",
            "--build-version",
            "v2.0.1",
            "--no-build",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .assert()
        .success()
        .stderr(predicate::str::contains("Dry run"));
}

// ── Compat-flag notice ──

/// When `--verbosity debug` is explicitly supplied, a backward-
/// compatibility notice must appear on stderr.
#[test]
fn test_compat_verbosity_notice_emitted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    let output = ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--no-build",
            "--verbosity",
            "debug",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("backward compatibility"),
        "stderr must contain compat notice when --verbosity is supplied:\n{stderr}"
    );
}

/// When `--verbosity` is NOT explicitly supplied (clap injects the
/// default value `info`), no backward-compatibility notice must appear.
#[test]
fn test_compat_verbosity_notice_absent_when_not_supplied() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project = create_project_with_build(tmp.path());

    let output = ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            "app-ui",
            "upload",
            "--name",
            "my-app",
            "--no-build",
            "--project-path",
        ])
        .arg(project.to_str().unwrap())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("backward compatibility"),
        "stderr must NOT contain compat notice when no compat flags supplied:\n{stderr}"
    );
}
