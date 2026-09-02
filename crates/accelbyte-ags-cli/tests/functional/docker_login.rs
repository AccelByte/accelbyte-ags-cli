//! Functional tests for `ags extend docker-login`.
//!
//! Exercises the full CLI path: argv → clap parse → router classification →
//! workflow dispatch → rendered output. The `--print` tests use a wiremock
//! server for the EHS credential endpoint; the default path uses `--dry-run`
//! to avoid requiring a Docker binary.

use crate::common::cli_helpers::{ags_isolated, ags_with_base_url};
use crate::common::wiremock_helpers::mount_token_success;
use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── --help ──

/// `--help` renders usage text on stdout and exits cleanly.
#[test]
fn test_help_renders_usage() {
    ags_isolated()
        .args(["extend", "docker-login", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("docker-login"))
        .stdout(predicate::str::contains("--namespace"))
        .stdout(predicate::str::contains("--app"))
        .stdout(predicate::str::contains("--print"));
}

// ── Missing required flags ──

/// Without `--namespace` (and no AGS_NAMESPACE env, no profile config) the
/// command fails with a usage error.
#[test]
fn test_missing_namespace_fails() {
    let mut cmd = ags_isolated();
    // Clear AGS_NAMESPACE to guarantee the "all sources unset" path.
    cmd.env_remove("AGS_NAMESPACE");
    cmd.args(["extend", "docker-login", "--app", "myapp"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--namespace"));
}

/// Without `--app` the command fails with a usage error.
#[test]
fn test_missing_app_fails() {
    ags_isolated()
        .args(["extend", "docker-login", "--namespace", "ns"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--app"));
}

// ── --print-format without --print ──

/// `--print-format` without `--print` is rejected before any network call.
#[test]
fn test_format_without_print_is_rejected() {
    ags_isolated()
        .args([
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print-format",
            "json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--print-format requires --print"));
}

// ── --print --print-format json (wiremock) ──

/// `--print --print-format json` against a mocked EHS backend emits the
/// documented three-field JSON object on stdout.
#[tokio::test]
async fn test_print_format_json_emits_credential_json() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path("/ehs/v1/namespaces/ns/apps/myapp/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"repositoryBaseUrl":"https://registry.example.com","username":"user","token":"tok123"}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
            "--print-format",
            "json",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));

    assert_eq!(json["repositoryBaseUrl"], "https://registry.example.com");
    assert_eq!(json["username"], "user");
    assert_eq!(json["token"], "tok123");
}

// ── --print --print-format token (wiremock) ──

/// `--print --print-format token` emits only the raw token and nothing else.
#[tokio::test]
async fn test_print_format_token_emits_raw_token() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path("/ehs/v1/namespaces/ns/apps/myapp/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"repositoryBaseUrl":"https://registry.example.com","username":"user","token":"raw-tok-value"}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
            "--print-format",
            "token",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        "raw-tok-value",
        "token format must emit only the raw token"
    );
    assert!(
        !stdout.contains("registry.example.com"),
        "registry URL must not appear in token output"
    );
}

// ── --dry-run on default path ──

/// `--dry-run` produces a preview and does not attempt to invoke Docker.
#[test]
fn test_dry_run_default_path_produces_preview() {
    let assert = ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    let combined = format!("{stdout}{stderr}");

    // The dry-run preview must mention the EHS token endpoint path fragment.
    assert!(
        combined.contains("/ehs/") || combined.contains("Dry run") || combined.contains("dry_run"),
        "dry-run output must contain preview content:\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// `--dry-run --format json` on the default path produces a JSON envelope.
#[test]
fn test_dry_run_format_json_default_path_produces_envelope() {
    let assert = ags_isolated()
        .args([
            "--dry-run",
            "--format",
            "json",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
    assert_eq!(json["dry_run"], true, "envelope must have dry_run: true");
    assert_eq!(
        json["workflow"], "docker-login",
        "envelope must identify the workflow"
    );
}

// ── --dry-run --print ──

/// `--dry-run --print` must succeed and make zero HTTP calls. The mock
/// server records all requests; an empty list proves the dry-run guard
/// prevented any network call.
#[tokio::test]
async fn test_dry_run_print_makes_no_http_call() {
    let server = MockServer::start().await;
    // No mocks mounted — any request to the server is unexpected.

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--dry-run",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "dry-run --print must succeed without network:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Prove no HTTP call was made.
    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        0,
        "dry-run --print must not make any HTTP call, but {} requests were recorded",
        received.len()
    );
}

// ── --app "" ──

/// An empty `--app` value is rejected with a usage error mentioning --app.
#[test]
fn test_empty_app_fails() {
    let mut cmd = ags_isolated();
    cmd.env_remove("AGS_NAMESPACE");
    cmd.args(["extend", "docker-login", "--namespace", "ns", "--app", ""])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--app"));
}

// ── Compat-flag notice on stderr (F8) ──

/// When `--login` is explicitly supplied, a backward-compatibility notice
/// must appear on stderr.
#[test]
fn test_compat_notice_on_stderr_when_login_supplied() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
            "--login",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("backward compatibility"),
        "stderr must contain compat notice when --login is supplied:\n{stderr}"
    );
}

/// When `--verbosity <value>` is explicitly supplied, a backward-
/// compatibility notice must appear on stderr.
#[test]
fn test_compat_notice_on_stderr_when_verbosity_supplied() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
            "--verbosity",
            "debug",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("backward compatibility"),
        "stderr must contain compat notice when --verbosity is supplied:\n{stderr}"
    );
}

/// When neither `--login` nor `--verbosity` is explicitly passed, no
/// backward-compatibility notice must appear on stderr.
#[test]
fn test_no_compat_notice_when_no_compat_flags() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("backward compatibility"),
        "stderr must NOT contain compat notice when no compat flags are supplied:\n{stderr}"
    );
}

/// `--verbosity` has a default value (`info`). When the user does NOT type
/// `--verbosity` on the command line, the default value must NOT trigger
/// the notice. A value-presence check instead of a flag-presence check
/// would fire the notice on every single run.
#[test]
fn test_no_compat_notice_from_default_verbosity_value() {
    // Invoke without --verbosity; clap injects "info" as the default.
    let output = ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("--verbosity"),
        "stderr must NOT mention --verbosity when it was not explicitly supplied:\n{stderr}"
    );
}

/// When `--quiet` is supplied alongside a compat flag, the notice must be
/// suppressed. The route gates emission on `!flags.verbosity.is_quiet()`.
#[test]
fn test_no_compat_notice_when_quiet() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "--quiet",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
            "--login",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("backward compatibility"),
        "stderr must NOT contain compat notice when --quiet is supplied:\n{stderr}"
    );
}

/// When `--format json` is in effect, the compat-flag notice must not appear
/// on stderr. The pre-surface backend selects StructuredJson which suppresses
/// render_warning output. This is the automation contract: human-only text
/// on stderr in JSON mode is a defect.
#[test]
fn test_no_compat_notice_in_json_format() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "--format",
            "json",
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--login",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("backward compatibility"),
        "stderr must NOT contain compat notice in --format json mode:\n{stderr}"
    );
}

// ── Compat-flag notice + --print --print-format json stdout separation (F9) ──

/// When a compat flag is supplied with `--print --print-format json`,
/// stdout alone must parse as valid JSON while the notice appears on stderr.
#[tokio::test]
async fn test_print_json_stdout_parseable_with_compat_flag_on_stderr() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("GET"))
        .and(path("/ehs/v1/namespaces/ns/apps/myapp/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"repositoryBaseUrl":"https://registry.example.com","username":"user","token":"tok123"}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "extend",
            "docker-login",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--print",
            "--print-format",
            "json",
            "--login",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // stdout alone must be valid JSON
    let stdout = String::from_utf8(output.stdout).unwrap();
    let _: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("stdout must be valid JSON with compat flag present ({e}):\n{stdout}")
    });

    // stderr must contain the compat notice
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("backward compatibility"),
        "stderr must contain the compat notice:\n{stderr}"
    );
}
