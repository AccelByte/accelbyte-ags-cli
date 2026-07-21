use crate::common::cli_helpers::ags_with_base_url;
use crate::common::wiremock_helpers::mount_token_success;
use std::io::Write;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bulk_ban_body(comment: &str) -> serde_json::Value {
    serde_json::json!([
        {
            "ban": "CHAT_SEND",
            "comment": comment,
            "endDate": "2030-01-01T00:00:00Z",
            "reason": "testing",
            "skipNotif": false,
            "userId": "u1",
        }
    ])
}

/// `--json @file` should load the body from disk and forward it unchanged.
#[tokio::test]
async fn test_json_file_path_is_read_and_used() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;
    let expected_body = bulk_ban_body("from-file");
    Mock::given(method("POST"))
        .and(path("/iam/v3/admin/namespaces/test/bans/users"))
        .and(body_json(&expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "{}", serde_json::to_string(&expected_body).unwrap()).unwrap();

    let arg = format!("@{}", file.path().display());
    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "bans",
            "bulk-ban-users",
            "--namespace",
            "test",
            "--json",
            &arg,
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `--json @-` should load the body from stdin and forward it unchanged.
#[tokio::test]
async fn test_json_stdin_is_read_and_used() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;
    let expected_body = bulk_ban_body("from-stdin");
    Mock::given(method("POST"))
        .and(path("/iam/v3/admin/namespaces/test/bans/users"))
        .and(body_json(&expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "bans",
            "bulk-ban-users",
            "--namespace",
            "test",
            "--json",
            "@-",
            "--yes",
        ])
        .write_stdin(serde_json::to_string(&expected_body).unwrap());

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `--no-input --json @-` should fail fast with a clear error rather than blocking on stdin.
#[tokio::test]
async fn test_json_stdin_with_no_input_returns_usage_error() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "bans",
            "bulk-ban-users",
            "--namespace",
            "test",
            "--json",
            "@-",
            "--no-input",
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not available with --no-input"),
        "stderr did not mention --no-input rejection: {stderr}"
    );
}

/// `--json @empty.json` should produce a clear "file is empty" error rather than a JSON parse error.
#[tokio::test]
async fn test_json_empty_file_returns_usage_error() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    let file = tempfile::NamedTempFile::new().unwrap();
    let arg = format!("@{}", file.path().display());

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "bans",
            "bulk-ban-users",
            "--namespace",
            "test",
            "--json",
            &arg,
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is empty"),
        "stderr did not mention empty file: {stderr}"
    );
}

/// `--json @missing.json` should fail with a Usage error pointing at the file.
#[tokio::test]
async fn test_json_file_missing_returns_usage_error() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "iam",
            "bans",
            "bulk-ban-users",
            "--namespace",
            "test",
            "--json",
            "@/definitely/not/here.json",
            "--yes",
        ]);

    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Failed to read --json file"),
        "stderr did not mention file read failure: {stderr}"
    );
    assert!(
        stderr.contains("/definitely/not/here.json"),
        "stderr did not mention the missing path: {stderr}"
    );
}
