//! End-to-end coverage for terminal control sequence sanitization in the
//! security-assessment shape overrides. Verifies that untrusted strings
//! from an Extend app's scanned endpoints cannot inject escape sequences
//! into the rendered table output.

use crate::common::cli_helpers::ags_with_base_url;
use crate::common::wiremock_helpers::mount_token_success;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The `permission.resource` field from an Extend app's scanned OpenAPI spec
/// can carry terminal escape sequences. The CLI must strip them before
/// rendering the table cell. CLICOLOR_FORCE=1 defeats anstream's automatic
/// stripping so the test observes what the shaping layer actually emits.
#[tokio::test]
async fn list_endpoints_strips_control_sequences_from_permission() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    let body = serde_json::json!({
        "endpoints": [{
            "method": "GET",
            "path": "/test",
            "operationId": "op1",
            "requireAuthentication": true,
            "permission": {
                "resource": "\u{1b}]0;PWNED\u{7}NAMESPACE:test-ns:APP",
                "action": "\u{1b}[31mREAD\u{1b}[0m"
            }
        }],
        "hasAPISpec": true,
        "hasGRPCReflection": false,
        "isAppRunning": true,
        "maximumSelectableEndpoints": 20
    });

    Mock::given(method("GET"))
        .and(path(
            "/csm/v1/admin/namespaces/test-ns/pentestings/apps/my-app/endpoints",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .args([
            "--namespace",
            "test-ns",
            "--ui",
            "plain",
            "extend",
            "security-assessment",
            "list-endpoints",
            "--app",
            "my-app",
        ]);

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("NAMESPACE:test-ns:APP"),
        "stdout must contain the clean permission text; got:\n{stdout:?}"
    );
    assert!(
        !stdout.contains("\x1b]0;"),
        "stdout must not contain the OSC escape sequence; got:\n{stdout:?}"
    );
    assert!(
        !stdout.contains("PWNED"),
        "stdout must not contain the injected title; got:\n{stdout:?}"
    );
}

/// The `createdAt` field from the security-assessment list response can
/// carry terminal escape sequences when the timestamp is not valid RFC 3339
/// (the formatter falls back to the raw string). The CLI must strip them.
#[tokio::test]
async fn list_strips_control_sequences_from_requested_at() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    let body = serde_json::json!({
        "pentestings": [{
            "engagementId": 1,
            "targetApp": "svc",
            "createdAt": "\u{1b}]0;PWNED\u{7}not-a-date",
            "endpoints": [],
            "targetAppVersion": "v1",
            "status": "OK"
        }]
    });

    Mock::given(method("GET"))
        .and(path("/csm/v1/admin/namespaces/test-ns/pentestings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .args([
            "--namespace",
            "test-ns",
            "--ui",
            "plain",
            "extend",
            "security-assessment",
            "list",
        ]);

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("not-a-date"),
        "stdout must contain the clean fallback text; got:\n{stdout:?}"
    );
    assert!(
        !stdout.contains("\x1b]0;"),
        "stdout must not contain the OSC escape sequence; got:\n{stdout:?}"
    );
    assert!(
        !stdout.contains("PWNED"),
        "stdout must not contain the injected title; got:\n{stdout:?}"
    );
}
