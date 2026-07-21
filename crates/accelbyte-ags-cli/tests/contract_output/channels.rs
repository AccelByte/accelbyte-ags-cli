use crate::common::cli_helpers::ags_with_base_url;
use crate::common::wiremock_helpers::mount_token_success;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const USERS_API_PATH: &str = "/iam/v3/admin/namespaces/test/users/platforms/justice";
const ROLES_API_PATH: &str = "/iam/v4/admin/roles";

// ── Success: read operation ──

/// Successful read operations emit response data on stdout so it can be piped to jq or scripts
#[tokio::test]
async fn test_read_success_data_on_stdout() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(USERS_API_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"data":[{"userId":"u1"}],"paging":{}}"#),
        )
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!stdout.is_empty(), "Read success must produce stdout");
    assert!(
        !stderr.contains("Error"),
        "Read success must not have errors on stderr: {stderr}"
    );
}

/// Read operations stay silent on stderr because a success banner would be noise for queries
#[tokio::test]
async fn test_read_success_no_success_banner_on_stderr() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(USERS_API_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"data":[{"userId":"u1"}],"paging":{}}"#),
        )
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains('\u{2714}'),
        "Read operations must not show success banner on stderr: {stderr}"
    );
}

// ── Success: mutation operation ──

/// Mutations show a success banner on stderr so users get confirmation without polluting stdout
#[tokio::test]
async fn test_mutation_success_banner_on_stderr() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(ROLES_API_PATH))
        .and(body_string_contains("roleName"))
        .respond_with(
            ResponseTemplate::new(201).set_body_string(r#"{"roleId":"r1","roleName":"Test"}"#),
        )
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args([
            "iam",
            "roles",
            "create",
            "--json",
            r#"{"roleName":"Test"}"#,
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains('\u{2714}'),
        "Mutation success must show success banner on stderr: {stderr}"
    );
}

/// Mutation response bodies go to stdout so scripts can capture the created/updated resource
#[tokio::test]
async fn test_mutation_response_data_on_stdout() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(ROLES_API_PATH))
        .and(body_string_contains("roleName"))
        .respond_with(
            ResponseTemplate::new(201).set_body_string(r#"{"roleId":"r1","roleName":"Test"}"#),
        )
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
        .args([
            "iam",
            "roles",
            "create",
            "--json",
            r#"{"roleName":"Test"}"#,
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.is_empty(),
        "Mutation response body should appear on stdout"
    );
}

// ── Error routing ──

/// API errors appear only on stderr, keeping stdout empty so piped consumers never parse error text as data
#[tokio::test]
async fn test_api_error_on_stderr_only() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(USERS_API_PATH))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string(r#"{"errorCode":20013,"errorMessage":"forbidden"}"#),
        )
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "cid")
        .env("AGS_CLIENT_SECRET", "csec")
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.is_empty(),
        "API error must not produce stdout: {stdout}"
    );
    assert!(
        stderr.contains('\u{2715}'),
        "API error must appear on stderr: {stderr}"
    );
}
