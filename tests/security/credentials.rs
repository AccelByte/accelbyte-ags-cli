use crate::common::cli_helpers::{ags, ags_isolated, ags_with_base_url};
use crate::common::wiremock_helpers::{mount_api_error, mount_token_success};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_dry_run_shows_placeholder_not_real_token() {
    let output = ags()
        .args([
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("<token>") || stdout.contains("[REDACTED]"),
        "Dry-run should show placeholder, not a real token"
    );
    assert!(
        !stdout.contains("Bearer eyJ"),
        "Dry-run must not contain a real JWT token"
    );
}

#[test]
fn test_dry_run_stderr_does_not_leak_tokens() {
    let output = ags()
        .args([
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Bearer eyJ"),
        "Dry-run stderr must not contain a real JWT token"
    );
}

#[test]
fn test_verbose_dry_run_does_not_leak_tokens() {
    let output = ags()
        .args([
            "--verbose",
            "--dry-run",
            "--namespace",
            "test",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        !combined.contains("Bearer eyJ"),
        "Verbose dry-run must not contain a real JWT token"
    );
}

// ── Verbose mode with real API call redacts tokens ──

#[tokio::test]
async fn test_verbose_real_call_redacts_token_on_stderr() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(
            "/iam/v3/admin/namespaces/test/users/platforms/justice",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[],"paging":{}}"#))
        .mount(&server)
        .await;

    let output = ags_with_base_url(&server.uri())
        .env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args([
            "--verbose",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("test-access-token"),
        "Verbose stderr must not contain the actual access token: {stderr}"
    );
    assert!(
        stderr.contains("<token>") || stderr.contains("Bearer"),
        "Verbose stderr should show request line: {stderr}"
    );
}

// ── API error output doesn't leak authorization header ──

#[tokio::test]
async fn test_api_error_does_not_leak_token() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;
    mount_api_error(
        &server,
        "/iam/v3/admin/namespaces/test/users/platforms/justice",
        403,
        r#"{"errorCode":20013,"errorMessage":"forbidden"}"#,
    )
    .await;

    let output = ags_with_base_url(&server.uri())
        .env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains("test-access-token"),
        "Error output must not contain access token: {combined}"
    );
    assert!(
        !combined.contains("csec"),
        "Error output must not contain client secret: {combined}"
    );
}

// ── Auth status doesn't show raw token values ──

#[tokio::test]
async fn test_auth_status_does_not_show_raw_token() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    // First, authenticate to store a token
    let roles_path = "/iam/v4/admin/roles";
    Mock::given(method("POST"))
        .and(path(roles_path))
        .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"roleId":"r1"}"#))
        .mount(&server)
        .await;

    // Run a command to trigger token storage
    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args([
            "iam",
            "roles",
            "create",
            "--json",
            r#"{"roleName":"T"}"#,
            "--yes",
        ]);
    let _ = cmd.output().unwrap();

    // Now check auth status — it should not show the raw token
    let status_output = ags_isolated()
        .env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args(["auth", "status"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        !stdout.contains("test-access-token"),
        "Auth status must not show raw access token: {stdout}"
    );
}

// ── Client secret never appears in stdout or stderr ──

#[tokio::test]
async fn test_client_secret_never_in_output() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(
            "/iam/v3/admin/namespaces/test/users/platforms/justice",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[],"paging":{}}"#))
        .mount(&server)
        .await;

    let secret = "my-super-secret-value";
    let output = ags_with_base_url(&server.uri())
        .env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", secret)
        .args([
            "--verbose",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains(secret),
        "Client secret must never appear in output: {combined}"
    );
}
