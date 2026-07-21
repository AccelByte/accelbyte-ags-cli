//! `--output` on text responses: writes the raw JSON body to the file,
//! or the JSON envelope when combined with `--format json`.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::cli_helpers::ags_isolated;

const JSON_BODY: &str = r#"{"data":[{"id":"abc"}],"paging":{}}"#;

async fn start_json_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/iam/v3/admin/namespaces/test-ns/users/platforms/justice",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(JSON_BODY),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn test_output_writes_raw_json_body() {
    let server = start_json_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("users.json");

    let mut cmd = ags_isolated();
    let output = cmd
        .env("AGS_BASE_URL", server.uri())
        .env("AGS_ACCESS_TOKEN", "fake-token")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test-ns",
            "--limit",
            "1",
            "--output",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written = std::fs::read_to_string(&out).unwrap();
    let trimmed = written.trim_end_matches('\n');
    assert_eq!(trimmed, JSON_BODY);
}

#[tokio::test]
async fn test_output_plus_format_json_writes_envelope() {
    let server = start_json_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("users.envelope.json");

    let mut cmd = ags_isolated();
    let output = cmd
        .env("AGS_BASE_URL", server.uri())
        .env("AGS_ACCESS_TOKEN", "fake-token")
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test-ns",
            "--limit",
            "1",
            "--format",
            "json",
            "--output",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written = std::fs::read_to_string(&out).unwrap();
    // Must be valid JSON.
    serde_json::from_str::<serde_json::Value>(&written).unwrap();
    // Basic sanity: the envelope includes the server's `data` key verbatim.
    assert!(
        written.contains(r#""data""#),
        "envelope should contain data key: {written}"
    );
}
