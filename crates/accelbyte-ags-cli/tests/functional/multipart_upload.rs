//! `csm app-ui upload-assets` succeeds end-to-end with a real multipart
//! request, replacing the old `multipart_guard.rs` coverage of it being
//! hard-rejected.

use crate::common::cli_helpers::{ags_isolated, ags_with_base_url};
use crate::common::wiremock_helpers::mount_token_success;
use predicates::prelude::*;
use wiremock::matchers::{header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A real file-upload command succeeds against a mock server, sending an
/// actual multipart/form-data request, in normal mode.
#[tokio::test]
async fn test_file_upload_command_succeeds_with_real_multipart_request() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;
    Mock::given(method("POST"))
        .and(path(
            "/csm/v1/admin/namespaces/dev/app-ui/some-app/files/upload",
        ))
        .and(header_regex(
            "content-type",
            "^multipart/form-data; boundary=.+$",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"ok":true}"#),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("asset.png");
    std::fs::write(&file_path, b"fake-png-bytes").unwrap();

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "csm",
            "app-ui",
            "upload-assets",
            "--namespace",
            "dev",
            "--app-ui-name",
            "some-app",
            "--file",
        ])
        .arg(&file_path);

    cmd.assert().success();
}

/// `--dry-run` for the same command validates the file path and previews
/// the request without uploading — no network call is made (no mock server
/// is stood up in this test at all; if the CLI tried to reach out over the
/// network there would be nothing listening, so success here is itself
/// evidence no request was sent).
#[test]
fn test_file_upload_command_dry_run_previews_without_uploading() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("asset.png");
    std::fs::write(&file_path, b"fake-png-bytes").unwrap();

    ags_isolated()
        .args([
            "csm",
            "app-ui",
            "upload-assets",
            "--namespace",
            "dev",
            "--app-ui-name",
            "some-app",
            "--file",
        ])
        .arg(&file_path)
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("asset.png"));
}

/// A missing file path is a clean validation error, not a panic, and no
/// network call happens. This is checked under `--dry-run`: in normal mode
/// `ExecutionContext::resolve()` resolves the access token *before* the
/// workflow's gather/resolve step (where the form-data file is validated),
/// so without a mocked auth server a plain non-dry-run invocation would fail
/// on "not authenticated" rather than exercising the file-path validation
/// this test targets. `--dry-run` reaches the file-validation code path
/// directly without requiring any base URL or credentials.
#[test]
fn test_file_upload_command_missing_file_path_errors_cleanly() {
    ags_isolated()
        .args([
            "csm",
            "app-ui",
            "upload-assets",
            "--namespace",
            "dev",
            "--app-ui-name",
            "some-app",
            "--file",
            "/definitely/does/not/exist.png",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"))
        .stderr(predicate::str::contains("panic").not());
}
