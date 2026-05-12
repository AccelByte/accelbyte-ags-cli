use crate::common::cli_helpers::{ags, ags_isolated};
use crate::common::env_guard::{now_secs, TempEnvGuard};
use ags::runtime::auth::store::{self, TokenData};
use ags::runtime::config::{GlobalConfig, ProfileConfig};
use predicates::prelude::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Set "default" as the active profile in the global config so the
/// subprocess's profile-name resolver finds it instead of erroring with
/// "No active profile".
fn activate_default_profile() {
    GlobalConfig {
        active_profile: Some("default".to_string()),
        ..Default::default()
    }
    .save()
    .unwrap();
}

/// Persist a stale stored token to the default profile. If `refresh_token` is
/// Some, the refresh token is set with a still-valid local expiry; if None,
/// no refresh token is stored.
fn write_stale_token(profile: &str, refresh_token: Option<&str>) {
    let now = now_secs();
    store::store_token_data(
        profile,
        &TokenData {
            access_token: "expired-access".to_string(),
            expires_at: now.saturating_sub(300),
            refresh_token: refresh_token.map(|s| s.to_string()),
            refresh_expires_at: refresh_token.map(|_| now + 86_400),
            grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
        },
    )
    .unwrap();
}

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

/// When the stored access token is still valid, login is a no-op and
/// no network call is made.
#[tokio::test]
#[serial_test::serial]
async fn test_login_no_op_when_access_token_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let profile = "default";
    ProfileConfig {
        base_url: Some("https://unused.invalid".to_string()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();

    let now = now_secs();
    store::store_token_data(
        profile,
        &TokenData {
            access_token: "fresh".to_string(),
            expires_at: now + 3600,
            refresh_token: Some("rt".to_string()),
            refresh_expires_at: Some(now + 7200),
            grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
        },
    )
    .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["auth", "login"])
        .output()
        .unwrap();

    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        combined.contains("Already authenticated"),
        "expected 'Already authenticated' headline, got: {combined}"
    );
}

/// When the access token is expired but the refresh token is valid,
/// login refreshes in place — no browser flow runs — and reports the
/// 'Session refreshed' outcome.
#[tokio::test]
#[serial_test::serial]
async fn test_login_refreshes_when_access_expired_and_refresh_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"refreshed-access","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let profile = "default";
    ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();
    write_stale_token(profile, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["auth", "login"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Session refreshed"),
        "expected 'Session refreshed' headline, got stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stored = store::get_token_data(profile).unwrap().unwrap();
    assert_eq!(stored.access_token, "refreshed-access");
}

/// When the access token is expired and the server rejects the refresh,
/// login emits the 'Existing session was invalid' message, clears the
/// stored token, and proceeds to print the OAuth URL.
#[tokio::test]
#[serial_test::serial]
async fn test_login_falls_through_to_fresh_flow_on_refresh_rejection() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(401).set_body_string(
            r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
        ))
        .mount(&server)
        .await;

    let profile = "default";
    ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();
    write_stale_token(profile, Some("dead-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env("AGS_AUTH_TIMEOUT", "1") // abort callback server fast
        .args(["auth", "login"])
        .output()
        .unwrap();

    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        combined.contains("Existing session was invalid"),
        "expected probe-failure message, got: {combined}"
    );
    let after = store::get_token_data(profile).unwrap();
    assert!(
        after.is_none(),
        "stale token should have been cleared after probe rejection, got: {after:?}"
    );
}

/// When the stored access token is expired and no refresh token is stored,
/// login does NOT call /oauth/token for a refresh — it proceeds directly
/// to the fresh OAuth prompt.
#[tokio::test]
#[serial_test::serial]
async fn test_login_skips_probe_when_no_refresh_token() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let profile = "default";
    ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();
    write_stale_token(profile, None);

    let _output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env("AGS_AUTH_TIMEOUT", "1")
        .args(["auth", "login"])
        .output()
        .unwrap();

    // No assertion on stdout — the .expect(0) on the mock is the assertion.
    drop(server);
}

/// JSON output for a refreshed session via the client-credentials login
/// path must carry the 'refreshed' status so downstream consumers can
/// distinguish 'refreshed in place' from a fresh OAuth flow (status
/// 'logged_in') and from a no-op (status 'already_authenticated').
#[tokio::test]
#[serial_test::serial]
async fn test_client_credentials_login_json_output_carries_refreshed_status() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"refreshed","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
        ))
        .mount(&server)
        .await;

    let profile = "default";
    ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();
    write_stale_token(profile, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args([
            "--format",
            "json",
            "auth",
            "login",
            "--grant",
            "client-credentials",
            "--client-secret",
            "unused",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The actual JSON envelope shape is what the JsonFrontend produces. We
    // search the entire JSON tree for a "status" field whose value is the
    // expected string. If this assertion fails, inspect stdout to see the
    // real shape.
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("expected JSON output\nstdout: {stdout}\nstderr: {stderr}\nerror: {e}")
    });

    let status = find_string_field(&parsed, "status")
        .unwrap_or_else(|| panic!("could not find a 'status' string field in JSON: {parsed}"));
    assert_eq!(
        status, "refreshed",
        "expected status='refreshed' in JSON, full output: {parsed}"
    );
}

/// JSON output for a refreshed session via the authorization-code login
/// path must also carry the 'refreshed' status. The probe runs for both
/// grants, so both must serialise the outcome identically.
#[tokio::test]
#[serial_test::serial]
async fn test_authorization_code_login_json_output_carries_refreshed_status() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"refreshed","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
        ))
        .mount(&server)
        .await;

    let profile = "default";
    ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();
    write_stale_token(profile, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args([
            "--format",
            "json",
            "auth",
            "login",
            "--grant",
            "authorization-code",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("expected JSON output\nstdout: {stdout}\nstderr: {stderr}\nerror: {e}")
    });

    let status = find_string_field(&parsed, "status")
        .unwrap_or_else(|| panic!("could not find a 'status' string field in JSON: {parsed}"));
    assert_eq!(
        status, "refreshed",
        "expected status='refreshed' in JSON for authorization-code probe, full output: {parsed}"
    );
}

/// `auth login --no-input` with a still-valid stored access token must
/// short-circuit via the probe and exit 0 with "Already authenticated"
/// instead of rejecting on non-interactive mode. The probe makes the
/// browser flow unnecessary, so the non-interactive guard should not fire.
#[tokio::test]
#[serial_test::serial]
async fn test_login_no_input_short_circuits_when_token_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let profile = "default";
    ProfileConfig {
        base_url: Some("https://unused.invalid".to_string()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();

    let now = now_secs();
    store::store_token_data(
        profile,
        &TokenData {
            access_token: "fresh".to_string(),
            expires_at: now + 3600,
            refresh_token: Some("rt".to_string()),
            refresh_expires_at: Some(now + 7200),
            grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
        },
    )
    .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["--no-input", "auth", "login"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        combined.contains("Already authenticated"),
        "expected 'Already authenticated' headline, got: {combined}"
    );
}

/// `auth login --format json` with a still-valid stored access token must
/// short-circuit via the probe and emit JSON `{"status":"already_authenticated"}`
/// instead of an error envelope.
#[tokio::test]
#[serial_test::serial]
async fn test_login_json_short_circuits_when_token_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let profile = "default";
    ProfileConfig {
        base_url: Some("https://unused.invalid".to_string()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();

    let now = now_secs();
    store::store_token_data(
        profile,
        &TokenData {
            access_token: "fresh".to_string(),
            expires_at: now + 3600,
            refresh_token: Some("rt".to_string()),
            refresh_expires_at: Some(now + 7200),
            grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
        },
    )
    .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["--format", "json", "auth", "login"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected JSON output\nstdout: {stdout}\nerror: {e}"));
    let status = find_string_field(&parsed, "status")
        .unwrap_or_else(|| panic!("could not find a 'status' string field in JSON: {parsed}"));
    assert_eq!(
        status, "already_authenticated",
        "expected status='already_authenticated' in JSON, full output: {parsed}"
    );
}

/// `auth login --no-input` with no stored session has nothing to probe and
/// can't drive the browser flow either, so it must reject with the
/// "requires browser interaction" guidance. Regression guard for the
/// non-interactive fallback once the probe is in place.
#[tokio::test]
#[serial_test::serial]
async fn test_login_no_input_rejects_when_no_existing_session() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let profile = "default";
    ProfileConfig {
        base_url: Some("https://unused.invalid".to_string()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();

    ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["--no-input", "auth", "login"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires browser interaction"));
}

/// `auth login --format json` with no stored session must emit a JSON error
/// envelope carrying the "requires browser interaction" message. Mirrors
/// the --no-input case but verifies the error renders as JSON.
#[tokio::test]
#[serial_test::serial]
async fn test_login_json_rejects_when_no_existing_session() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let profile = "default";
    ProfileConfig {
        base_url: Some("https://unused.invalid".to_string()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["--format", "json", "auth", "login"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected non-zero exit, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // JSON error envelopes are emitted on stderr (see `JsonFrontend::render_error`).
    let stderr = String::from_utf8_lossy(&output.stderr);
    let parsed: serde_json::Value = serde_json::from_str(stderr.trim())
        .unwrap_or_else(|e| panic!("expected JSON error envelope on stderr\nstderr: {stderr}\nerror: {e}"));
    let error = find_string_field(&parsed, "error")
        .unwrap_or_else(|| panic!("could not find an 'error' string field in JSON: {parsed}"));
    assert!(
        error.contains("requires browser interaction"),
        "expected JSON error to mention 'requires browser interaction', got: {error}"
    );
}

/// When `--base-url` differs from the value the stored session was minted
/// against, the probe must not short-circuit; non-interactive mode then
/// rejects with the browser-interaction guidance instead of falsely
/// reporting "Already authenticated".
#[tokio::test]
#[serial_test::serial]
async fn test_login_no_input_proceeds_when_base_url_differs() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let profile = "default";
    ProfileConfig {
        base_url: Some("https://dev.example.com".to_string()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();

    let now = now_secs();
    store::store_token_data(
        profile,
        &TokenData {
            access_token: "fresh".to_string(),
            expires_at: now + 3600,
            refresh_token: Some("rt".to_string()),
            refresh_expires_at: Some(now + 7200),
            grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
        },
    )
    .unwrap();

    ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args([
            "--no-input",
            "auth",
            "login",
            "--base-url",
            "https://demo.example.com",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires browser interaction"));
}

/// Same identity guard as above but exercised via a differing `--client-id`
/// and JSON output. Verifies the probe doesn't claim the stored session
/// belongs to a different OAuth client.
#[tokio::test]
#[serial_test::serial]
async fn test_login_json_proceeds_when_client_id_differs() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let profile = "default";
    ProfileConfig {
        base_url: Some("https://dev.example.com".to_string()),
        client_id: Some("original-cid".to_string()),
        ..Default::default()
    }
    .save(profile)
    .unwrap();
    activate_default_profile();

    let now = now_secs();
    store::store_token_data(
        profile,
        &TokenData {
            access_token: "fresh".to_string(),
            expires_at: now + 3600,
            refresh_token: Some("rt".to_string()),
            refresh_expires_at: Some(now + 7200),
            grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
        },
    )
    .unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args([
            "--format",
            "json",
            "auth",
            "login",
            "--client-id",
            "different-cid",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected non-zero exit (no short-circuit), got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    let json_start = combined
        .find('{')
        .unwrap_or_else(|| panic!("no JSON object in output: {combined}"));
    let parsed: serde_json::Value = serde_json::from_str(combined[json_start..].trim())
        .unwrap_or_else(|e| panic!("expected JSON\noutput: {combined}\nerror: {e}"));
    let error = find_string_field(&parsed, "error")
        .unwrap_or_else(|| panic!("could not find 'error' string in JSON: {parsed}"));
    assert!(
        error.contains("requires browser interaction"),
        "expected JSON error to mention 'requires browser interaction', got: {error}"
    );
}

/// Recursively walk a JSON value looking for the first occurrence of the
/// named field with a string value. Returns the string if found.
fn find_string_field<'a>(value: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(v) = map.get(field).and_then(|v| v.as_str()) {
                return Some(v);
            }
            for v in map.values() {
                if let Some(found) = find_string_field(v, field) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(found) = find_string_field(v, field) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}
