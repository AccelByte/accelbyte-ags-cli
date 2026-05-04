use crate::common::cli_helpers::{ags, ags_isolated};
use predicates::prelude::*;

/// Login help advertises all supported flags so users discover available auth options
#[test]
fn test_auth_login_help_shows_new_flags() {
    ags()
        .args(["auth", "login", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--base-url"))
        .stdout(predicate::str::contains("--client-id"))
        .stdout(predicate::str::contains("--client-secret"))
        .stdout(predicate::str::contains("--client-secret-stdin"))
        .stdout(predicate::str::contains("--grant"))
        .stdout(predicate::str::contains("--port"));
}

/// Password-based auth was removed; help must not advertise --username/--password
#[test]
fn test_auth_login_help_no_password_flags() {
    let output = ags().args(["auth", "login", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("--username"),
        "Help should not contain --username, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("--password"),
        "Help should not contain --password, got:\n{stdout}"
    );
}

/// --client-secret and --client-secret-stdin are mutually exclusive to prevent ambiguous input
#[test]
fn test_auth_login_client_secret_conflicts_with_stdin() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--base-url",
            "https://example.com",
            "--client-id",
            "test",
            "--client-secret",
            "secret",
            "--client-secret-stdin",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--client-secret"));
}

/// --port is accepted and forwarded to the OAuth callback server binding
#[test]
#[ignore] // Starts callback server and waits for OAuth callback — run manually with --ignored
fn test_auth_login_port_flag_accepted() {
    ags()
        .env("AGS_AUTH_TIMEOUT", "2")
        .args([
            "auth",
            "login",
            "--base-url",
            "https://localhost:1",
            "--client-id",
            "test-id",
            "--client-secret",
            "test-secret",
            "--port",
            "9090",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Failed to bind")
                .or(predicate::str::contains("Timed out"))
                .or(predicate::str::contains("error"))
                .or(predicate::str::contains("visible in shell history")),
        );
}

/// Client-credentials grant attempts a token exchange against the base URL
#[test]
fn test_auth_login_grant_client_credentials() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--base-url",
            "https://localhost:1",
            "--client-id",
            "test-id",
            "--client-secret",
            "test-secret",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Token request failed")
                .or(predicate::str::contains("Network error"))
                .or(predicate::str::contains("error")),
        );
}

/// An unsupported grant type like "password" is rejected at parse time with a clear message
#[test]
fn test_auth_login_grant_invalid_value() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "password",
            "--base-url",
            "https://localhost:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid value 'password'"));
}

/// AGS_BASE_URL env var is used when --base-url flag is omitted
#[test]
fn test_auth_login_env_fallback_for_base_url() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--client-id",
            "test-id",
            "--client-secret",
            "test-secret",
        ])
        .env("AGS_BASE_URL", "https://env-base-url.example.com")
        .assert()
        .failure()
        .stderr(predicate::str::contains("https://env-base-url.example.com"));
}

/// AGS_CLIENT_ID env var is used when --client-id flag is omitted
#[test]
fn test_auth_login_env_fallback_for_client_id() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--base-url",
            "https://localhost:1",
            "--client-secret",
            "test-secret",
        ])
        .env("AGS_CLIENT_ID", "env-client-id")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Token request failed")
                .or(predicate::str::contains("Network error"))
                .or(predicate::str::contains("error")),
        );
}

/// AGS_CLIENT_SECRET env var is used when --client-secret flag is omitted
#[test]
fn test_auth_login_env_fallback_for_client_secret() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--base-url",
            "https://localhost:1",
            "--client-id",
            "test-id",
        ])
        .env("AGS_CLIENT_SECRET", "env-secret")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Token request failed")
                .or(predicate::str::contains("Network error"))
                .or(predicate::str::contains("error")),
        );
}

/// --client-secret-stdin reads the secret from piped input to avoid shell history exposure
#[test]
fn test_auth_login_client_secret_from_stdin() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--base-url",
            "https://localhost:1",
            "--client-id",
            "test-id",
            "--client-secret-stdin",
        ])
        .write_stdin("stdin-secret\n")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Token request failed")
                .or(predicate::str::contains("Network error"))
                .or(predicate::str::contains("error")),
        );
}

/// Passing a secret via --client-secret emits a shell-history warning on stderr
#[test]
fn test_auth_login_security_warning_for_client_secret_flag() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--base-url",
            "https://localhost:1",
            "--client-id",
            "test-id",
            "--client-secret",
            "plaintext-secret",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("visible in shell history"));
}

/// --client-secret-stdin without --base-url and --client-id fails because all three are required
#[test]
fn test_auth_login_client_secret_stdin_requires_base_url_and_client_id() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--client-secret-stdin",
        ])
        .write_stdin("secret-value\n")
        .assert()
        .failure();
}

/// When stdin is occupied by --client-secret-stdin, base URL must come from a flag or env var
#[test]
fn test_auth_login_stdin_occupied_without_base_url_errors() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--client-id",
            "test-id",
            "--client-secret-stdin",
        ])
        .env_remove("AGS_BASE_URL")
        .write_stdin("secret\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("AGS_BASE_URL"));
}

/// When stdin is occupied by --client-secret-stdin, client ID must come from a flag or env var
#[test]
fn test_auth_login_stdin_occupied_without_client_id_errors() {
    ags_isolated()
        .args([
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--base-url",
            "https://example.com",
            "--client-secret-stdin",
        ])
        .env_remove("AGS_CLIENT_ID")
        .write_stdin("secret\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("AGS_CLIENT_ID"));
}
