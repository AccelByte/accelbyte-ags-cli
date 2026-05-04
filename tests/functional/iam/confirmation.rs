use crate::common::cli_helpers::ags_with_base_url;
use crate::common::wiremock_helpers::mount_token_success;
use wiremock::MockServer;

/// `--no-input` with a mutating operation should fail with a confirmation error
/// rather than hanging on stdin.
#[tokio::test]
async fn test_no_input_rejects_mutating_operation() {
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
            r#"{"ban":"CHAT_SEND","comment":"test","endDate":"2030-01-01T00:00:00Z","reason":"testing","skipNotif":false}"#,
            "--no-input",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        !output.status.success(),
        "Should fail when --no-input is set for mutating operation"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("confirmation") || stderr.contains("--yes"),
        "Should mention confirmation or --yes flag, got: {stderr}"
    );
}

/// `--yes` with a mutating dry-run should succeed without prompting.
/// (dry-run exits before the API call, but after flag parsing)
#[tokio::test]
async fn test_yes_flag_skips_confirmation_dry_run() {
    let server = MockServer::start().await;

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
            r#"{"ban":"CHAT_SEND","comment":"test","endDate":"2030-01-01T00:00:00Z","reason":"testing","skipNotif":false}"#,
            "--yes",
            "--dry-run",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Should succeed with --yes --dry-run, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
