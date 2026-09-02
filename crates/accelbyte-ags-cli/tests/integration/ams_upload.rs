//! End-to-end `ags ams upload` through the real binary against a mocked AMS.
//!
//! The functional tests cover the flag surface and pre-flight validation; this
//! covers the wiring the CLI adds on top — the auth prologue, the bespoke
//! route, and the rendered result.

use std::io::Write;
use std::path::Path;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::cli_helpers::ags_isolated;

/// Build a directory with an x86-64 ELF entrypoint and one data file.
fn build_directory() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let mut header = vec![0u8; 20];
    header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    header[4] = 2;
    header[5] = 1;
    header[6] = 1;
    header[18..20].copy_from_slice(&62u16.to_le_bytes());
    write(temp.path(), "server", &header);
    write(temp.path(), "assets/data.bin", b"asset bytes");
    temp
}

/// Write `contents` to `name` (creating parents) inside `directory`.
fn write(directory: &Path, name: &str, contents: &[u8]) {
    let path = directory.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(&path)
        .unwrap()
        .write_all(contents)
        .unwrap();
}

/// Mount a mock that plays the AGS platform, the AMS upload API, and the
/// storage service behind the pre-signed URL.
async fn start_ams_mock() -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(200).set_body_string(uri.clone()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/images"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": "img-e2e" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "url": format!("{uri}/storage") })),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"etag-1\""))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/v1/complete"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// Run `ags ams upload` against `server` with an injected access token.
fn run_upload(server: &MockServer, build: &Path, extra: &[&str]) -> std::process::Output {
    let mut command = ags_isolated();
    command
        .env("AGS_BASE_URL", server.uri())
        .env("AGS_ACCESS_TOKEN", "fake-token")
        .args([
            "ams",
            "upload",
            "--path",
            build.to_str().unwrap(),
            "--executable",
            "server",
            "--image-name",
            "my-image",
        ])
        .args(extra);
    command.output().unwrap()
}

#[tokio::test]
async fn test_upload_end_to_end_renders_the_created_image() {
    let server = start_ams_mock().await;
    let build = build_directory();
    let output = run_upload(&server, build.path(), &[]);

    assert!(
        output.status.success(),
        "upload failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("uploaded"), "{stdout}");
    assert!(stderr.contains("img-e2e"), "{stderr}");
    assert!(stderr.contains("linux-x86_64"), "{stderr}");

    let complete = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|request| request.url.path() == "/upload/v1/complete")
        .expect("the upload must be marked complete");
    let body: serde_json::Value = serde_json::from_slice(&complete.body).unwrap();
    assert_eq!(body["imageId"], "img-e2e");
    assert_eq!(body["command"], "./server");
    assert!(body["imageSizeBytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_upload_end_to_end_json_envelope() {
    let server = start_ams_mock().await;
    let build = build_directory();
    let output = run_upload(&server, build.path(), &["--format", "json"]);

    assert!(
        output.status.success(),
        "upload failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "uploaded");
    assert_eq!(json["image_id"], "img-e2e");
    assert_eq!(json["part_count"], 1);
    assert_eq!(json["file_count"], 2);
    assert_eq!(json["upload_base_url"], server.uri());
}

/// A platform that cannot answer host discovery must fail the upload, not
/// silently ship the build to production AMS.
#[tokio::test]
async fn test_upload_fails_when_host_discovery_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let build = build_directory();
    let output = run_upload(&server, build.path(), &[]);
    assert!(!output.status.success(), "discovery failure must be fatal");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not determine the AMS upload host"),
        "{stderr}"
    );
    assert!(stderr.contains("--upload-url"), "{stderr}");
}
