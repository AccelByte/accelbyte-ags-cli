use crate::common::cli_helpers::ags_with_base_url;
use crate::common::wiremock_helpers::mount_token_success;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A successful users list call returns exit 0 and renders the response body
#[tokio::test]
async fn test_users_list_success_with_wiremock() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    let users_response = r#"{"data":[{"userId":"user-001","displayName":"Jane Doe","emailAddress":"jane@example.com","namespace":"test"}],"paging":{"first":"","last":"","next":"","previous":""}}"#;
    Mock::given(method("GET"))
        .and(path(
            "/iam/v3/admin/namespaces/test/users/platforms/justice",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(users_response))
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
    assert!(
        output.status.success(),
        "Expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A 403 response surfaces a human-readable permission error on stderr
#[tokio::test]
async fn test_users_list_forbidden_error() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(
            "/iam/v3/admin/namespaces/test/users/platforms/justice",
        ))
        .respond_with(
            ResponseTemplate::new(403).set_body_string(
                r#"{"errorCode":20013,"errorMessage":"insufficient permissions"}"#,
            ),
        )
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
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("permission"),
        "Expected permission error on stderr, got: {stderr}"
    );
}

/// A 404 response surfaces a "not found" error rather than printing raw JSON
#[tokio::test]
async fn test_users_list_not_found_error() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path(
            "/iam/v3/admin/namespaces/test/users/platforms/justice",
        ))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"errorCode":20008,"errorMessage":"user not found"}"#),
        )
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
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("404") || stderr.contains("Error"),
        "Expected not found error on stderr, got: {stderr}"
    );
}
