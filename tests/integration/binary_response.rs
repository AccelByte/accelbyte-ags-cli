//! End-to-end tests for binary response handling — covers the four
//! destinations (file, `--output -`, piped stdout, TTY refusal) using a
//! wiremock backend that returns real PNG bytes.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::cli_helpers::ags_isolated;

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The exact path that `ags platform payment-station get-qr-code
/// --namespace test-ns --code testcode --dry-run` reports.
const EXPECTED_PATH: &str = "/platform/public/namespaces/test-ns/payment/qrcode";

async fn start_png_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(EXPECTED_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(PNG_MAGIC.to_vec()),
        )
        .mount(&server)
        .await;
    server
}

/// Common CLI setup: isolate config, point at wiremock, inject a fake
/// access token so no real auth is attempted.
fn cli_for_qr(server: &MockServer, home: &std::path::Path) -> assert_cmd::Command {
    let mut cmd = ags_isolated();
    cmd.env("AGS_BASE_URL", server.uri())
        .env("AGS_ACCESS_TOKEN", "fake-token")
        .env("AGS_HOME", home);
    cmd
}

/// Required args for get-qr-code: --namespace and --code.
fn base_args() -> Vec<&'static str> {
    vec![
        "platform",
        "payment-station",
        "get-qr-code",
        "--namespace",
        "test-ns",
        "--code",
        "testcode",
    ]
}

#[tokio::test]
async fn test_binary_response_to_file_flag() {
    let server = start_png_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("qr.png");

    let mut args = base_args();
    args.extend(["--output", out.to_str().unwrap()]);

    let mut cmd = cli_for_qr(&server, tmp.path());
    let output = cmd.args(args).output().unwrap();

    assert!(
        output.status.success(),
        "CLI exited non-zero: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let written = std::fs::read(&out).unwrap();
    assert_eq!(written, PNG_MAGIC);
}

#[tokio::test]
async fn test_binary_response_to_stdout_via_dash() {
    let server = start_png_server().await;
    let tmp = tempfile::tempdir().unwrap();

    let mut args = base_args();
    args.extend(["--output", "-"]);

    let mut cmd = cli_for_qr(&server, tmp.path());
    let output = cmd.args(args).output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, PNG_MAGIC);
}

#[tokio::test]
async fn test_binary_response_no_flag_piped_stdout_writes_bytes() {
    // When stdout is not a TTY (assert_cmd always pipes it), the CLI must
    // write binary bytes directly rather than refusing.
    let server = start_png_server().await;
    let tmp = tempfile::tempdir().unwrap();

    let mut cmd = cli_for_qr(&server, tmp.path());
    let output = cmd.args(base_args()).output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, PNG_MAGIC);
}

#[tokio::test]
async fn test_binary_response_file_overwrites_silently() {
    let server = start_png_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("qr.png");
    std::fs::write(&out, b"pre-existing").unwrap();

    let mut args = base_args();
    args.extend(["--output", out.to_str().unwrap()]);

    let mut cmd = cli_for_qr(&server, tmp.path());
    let output = cmd.args(args).output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&out).unwrap(), PNG_MAGIC);
}

#[tokio::test]
async fn test_binary_response_missing_dir_reports_os_error() {
    let server = start_png_server().await;
    let tmp = tempfile::tempdir().unwrap();

    let mut args = base_args();
    args.extend(["--output", "/definitely/does/not/exist/qr.png"]);

    let mut cmd = cli_for_qr(&server, tmp.path());
    let output = cmd.args(args).output().unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot write"),
        "expected OS-error message on stderr, got: {stderr}"
    );
}
