use crate::common::cli_helpers::{ags_isolated, ags_with_base_url};
use crate::common::wiremock_helpers::{mount_token_error_401, mount_token_success};
use predicates::prelude::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const USERS_API_PATH: &str = "/iam/v3/admin/namespaces/test/users/platforms/justice";
const ROLES_API_PATH: &str = "/iam/v4/admin/roles";

fn users_json_response() -> &'static str {
    r#"{"data":[{"userId":"u1","displayName":"Alice","emailAddress":"alice@test.com","namespace":"test"}],"paging":{}}"#
}

// ── --format json ──

/// `--format json` writes machine-parseable JSON to stdout for pipeline consumption
#[tokio::test]
async fn test_format_json_produces_valid_json_on_stdout() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(USERS_API_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string(users_json_response()))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
            "--format",
            "json",
        ]);

    let output = cmd.output().unwrap();
    assert!(output.status.success(), "Expected success");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");
    assert!(parsed["data"].is_array(), "Should contain data array");
}

// ── --format unsupported ──

/// Unsupported format values (e.g. "table") fail early with a descriptive error
#[test]
fn test_format_unsupported_value_errors() {
    ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--dry-run",
            "--format",
            "table",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown --format value"));
}

// ── --verbose ──

/// --verbose logs the HTTP method, path, and response status to stderr for debugging
#[tokio::test]
async fn test_verbose_prints_request_details_to_stderr() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(USERS_API_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string(users_json_response()))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
            "--verbose",
        ]);

    let output = cmd.output().unwrap();
    assert!(output.status.success(), "Expected success");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("GET") && stderr.contains(USERS_API_PATH),
        "Verbose should show request method and path on stderr: {stderr}"
    );
    assert!(
        stderr.contains("200"),
        "Verbose should show response status on stderr: {stderr}"
    );
}

// ── --no-input blocks mutations ──

/// --no-input without --yes rejects destructive operations to prevent non-interactive scripts from modifying data
#[tokio::test]
async fn test_no_input_blocks_mutation_without_yes() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "roles",
            "delete",
            "--role-id",
            "fake-role-id",
            "--no-input",
        ]);

    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("confirmation") || stderr.contains("--yes"),
        "Should mention confirmation or --yes: {stderr}"
    );
}

// ── Token auth failure (401) ──

/// A 401 from the token endpoint surfaces an auth-related error rather than a generic failure
#[tokio::test]
async fn test_token_auth_failure_shows_auth_error() {
    let server = MockServer::start().await;
    mount_token_error_401(&server).await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "bad-client-id")
        .env("AGS_CLIENT_SECRET", "bad-secret")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.to_lowercase().contains("auth")
            || stderr.contains("401")
            || stderr.contains("token")
            || stderr.contains("Token"),
        "Should indicate auth failure: {stderr}"
    );
}

// ── Malformed (non-JSON) API response ──

/// A non-JSON 200 response is passed through to stdout rather than erroring on parse failure
#[tokio::test]
async fn test_malformed_api_response_still_shown() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(USERS_API_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string("plain text response"))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(output.status.success(), "Non-JSON 200 should still succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("plain text response"),
        "Should pass through non-JSON response: {stdout}"
    );
}

// ── --json body passthrough ──

/// --json sends the provided body verbatim as the POST payload for mutation operations
#[tokio::test]
async fn test_json_body_passthrough_for_mutation() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(ROLES_API_PATH))
        .and(body_string_contains("roleName"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_string(r#"{"roleId":"role-001","roleName":"test-role"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "roles",
            "create",
            "--json",
            r#"{"roleName":"test-role","adminRole":false}"#,
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
