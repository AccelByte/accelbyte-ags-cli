//! Functional coverage for `ags auth token` through the compiled binary.
//!
//! The command exists so a script can borrow the CLI's session (`curl -H
//! "Authorization: Bearer $(ags auth token)"`), so every test here asserts on
//! the exact bytes of stdout rather than on a substring: a headline, a hint or
//! a stray newline landing there would break the caller silently.

use crate::common::cli_helpers::{ags, ags_isolated};
use crate::common::env_guard::{now_secs, TempEnvGuard};
use ags_runtime::runtime::auth::store::{self, TokenData};
use ags_runtime::runtime::config::{GlobalConfig, ProfileConfig};
use predicates::prelude::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Set "default" as the active profile in the global config so the subprocess's
/// profile-name resolver finds it instead of erroring with "No active profile".
fn activate_default_profile() {
    GlobalConfig {
        active_profile: Some("default".to_string()),
        ..Default::default()
    }
    .save()
    .unwrap();
}

/// Point the default profile at `server_uri` with an authorization-code grant.
fn setup_authorization_code_profile(server_uri: &str) {
    ProfileConfig {
        base_url: Some(server_uri.to_string()),
        client_id: Some("cid".to_string()),
        ..Default::default()
    }
    .save("default")
    .unwrap();
    activate_default_profile();
}

/// Persist a stored token for the default profile, expiring `expires_in_secs`
/// from now (negative values are already expired).
fn write_stored_token(access_token: &str, expires_in_secs: i64, refresh_token: Option<&str>) {
    let now = now_secs();
    let expires_at = if expires_in_secs.is_negative() {
        now.saturating_sub(expires_in_secs.unsigned_abs())
    } else {
        now.saturating_add(expires_in_secs.unsigned_abs())
    };
    store::store_token_data(
        "default",
        &TokenData {
            access_token: access_token.to_string(),
            expires_at,
            refresh_token: refresh_token.map(str::to_string),
            refresh_expires_at: refresh_token.map(|_| now + 86_400),
            grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
            client_id: None,
        },
    )
    .unwrap();
}

// ── Environment source ──

/// `AGS_ACCESS_TOKEN` is the first source the request path consults, so the
/// command prints it verbatim — and prints nothing else, which is the property
/// `$(ags auth token)` depends on.
#[test]
fn test_auth_token_prints_environment_token_alone() {
    let output = ags_isolated()
        .env("AGS_ACCESS_TOKEN", "env-token-value")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "env-token-value\n",
        "stdout must be the token and a newline, nothing else"
    );
}

/// The JSON form carries the provenance and expiry the plain form omits. An
/// `AGS_ACCESS_TOKEN` is opaque to the CLI, so its expiry is null rather than
/// guessed.
#[test]
fn test_auth_token_json_environment_source() {
    let output = ags_isolated()
        .env("AGS_ACCESS_TOKEN", "env-token-value")
        .args(["--format", "json", "auth", "token"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("expected JSON: {stdout} ({e})"));
    assert_eq!(json["access_token"], "env-token-value");
    assert_eq!(json["source"], "env");
    assert!(
        json["expires_at"].is_null(),
        "an externally supplied token has no expiry the CLI can state: {json}"
    );
}

// ── Stored source ──

/// A still-valid stored token is printed as-is, with `source: stored` and the
/// stored token's own `expires_at`.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_prints_valid_stored_token() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    setup_authorization_code_profile("https://example.invalid");
    write_stored_token("stored-access", 3600, Some("rt"));
    let expected_expiry_floor = now_secs() + 3500;

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["--format", "json", "auth", "token"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("expected JSON: {stdout} ({e})"));
    assert_eq!(json["access_token"], "stored-access");
    assert_eq!(json["source"], "stored");
    assert!(
        json["expires_at"].as_u64().unwrap() >= expected_expiry_floor,
        "expires_at must be the stored token's own expiry, got: {json}"
    );
}

/// `AGS_ACCESS_TOKEN` outranks a stored token, matching the request path's
/// resolution order — the command must not report a session the next API call
/// would not use.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_environment_outranks_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    setup_authorization_code_profile("https://example.invalid");
    write_stored_token("stored-access", 3600, Some("rt"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env("AGS_ACCESS_TOKEN", "env-wins")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert_eq!(String::from_utf8_lossy(&output.stdout), "env-wins\n");
}

// ── Expired, then refreshed ──

/// An expired stored token with a live refresh token is re-minted before it is
/// printed, so the caller never receives a token the API would reject.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_refreshes_expired_stored_token() {
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

    setup_authorization_code_profile(&server.uri());
    write_stored_token("expired-access", -300, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["--format", "json", "auth", "token"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("expected JSON: {stdout} ({e})"));
    assert_eq!(
        json["access_token"], "refreshed-access",
        "the expired token must not be printed: {json}"
    );
    assert_eq!(json["source"], "refreshed");
    assert!(
        json["expires_at"].as_u64().unwrap() > now_secs(),
        "a refreshed token must expire in the future: {json}"
    );
}

/// The refreshed token is the one the plain form prints too, on its own line.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_plain_output_is_the_refreshed_token_alone() {
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
        .mount(&server)
        .await;

    setup_authorization_code_profile(&server.uri());
    write_stored_token("expired-access", -300, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "refreshed-access\n"
    );
}

// ── Nothing to print ──

/// With no credentials at all the command exits 2 — the auth-error code the
/// siblings use — and points at `ags auth login` on stderr, leaving stdout
/// empty so a `$(...)` capture yields nothing rather than an error message.
#[test]
fn test_auth_token_no_credentials_exits_two_with_login_guidance() {
    let output = ags_isolated()
        .env_remove("AGS_ACCESS_TOKEN")
        .env_remove("AGS_CLIENT_ID")
        .env_remove("AGS_CLIENT_SECRET")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "auth errors exit 2");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "a failed resolution must put nothing on stdout"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ags auth login"),
        "expected login guidance on stderr, got: {stderr}"
    );
}

/// An expired stored token with no refresh token cannot be re-minted, so the
/// command fails rather than printing a token the API would reject.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_expired_without_refresh_token_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    setup_authorization_code_profile("https://example.invalid");
    write_stored_token("expired-access", -300, None);

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "the expired token must not reach stdout"
    );
}

// ── Dry run ──

/// `--dry-run` refuses to run: the command's only output would be a live
/// credential, so a dry run that printed it would break the dry-run contract.
/// An expired stored token with a refresh token is used to prove that no refresh
/// request is sent and the token store is not rewritten.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_dry_run_refuses_without_refreshing() {
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
        .expect(0)
        .mount(&server)
        .await;

    setup_authorization_code_profile(&server.uri());
    write_stored_token("expired-access", -300, Some("valid-refresh"));

    // Snapshot the token store before the run.
    let before = store::get_token_data("default").unwrap().unwrap();

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["--dry-run", "auth", "token"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "dry-run refusal is a usage error (exit 1)"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "stdout must be empty: no credential printed"
    );

    // No request was sent.
    let received = server.received_requests().await.unwrap();
    assert!(
        received.is_empty(),
        "no HTTP request must be sent under --dry-run"
    );

    // The token store is unchanged.
    let after = store::get_token_data("default").unwrap().unwrap();
    assert_eq!(after.access_token, before.access_token);
    assert_eq!(after.refresh_token, before.refresh_token);
    assert_eq!(after.expires_at, before.expires_at);

    // No credential value leaked to either stream.
    let stderr = String::from_utf8_lossy(&output.stderr);
    for secret in ["refreshed-access", "expired-access", "valid-refresh"] {
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains(secret),
            "{secret} must not appear on stdout"
        );
        assert!(
            !stderr.contains(secret),
            "{secret} must not appear on stderr"
        );
    }
}

/// A valid stored token is still refused under `--dry-run`, and
/// `--format json` does not change the outcome.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_dry_run_refuses_with_a_valid_stored_token() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    setup_authorization_code_profile("https://example.invalid");
    write_stored_token("live-stored", 3600, None);

    // Plain form.
    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["--dry-run", "auth", "token"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("live-stored"),
        "the stored token must not appear on stderr"
    );

    // JSON form.
    let output_json = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["auth", "token", "--dry-run", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output_json.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output_json.stdout), "");
    let stderr_json = String::from_utf8_lossy(&output_json.stderr);
    assert!(
        !stderr_json.contains("live-stored"),
        "the stored token must not appear on stderr in JSON mode"
    );
}

/// `AGS_ACCESS_TOKEN` set does not change the dry-run outcome: the command
/// still refuses, because printing any credential violates the contract.
#[test]
fn test_auth_token_dry_run_refuses_an_environment_token() {
    let output = ags_isolated()
        .env("AGS_ACCESS_TOKEN", "env-live")
        .args(["--dry-run", "auth", "token"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("env-live"),
        "the env token must not appear on stderr"
    );
}

// ── Exit codes for failures ──

/// When the identity service is unreachable the command exits 4 (network
/// error), not 2 (auth error), so a caller can distinguish "try again later"
/// from "you need to log in".
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_identity_service_unreachable_exits_four() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    // Bind a port and immediately drop the listener so nothing is listening.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };

    setup_authorization_code_profile(&format!("http://127.0.0.1:{port}"));
    write_stored_token("expired-access", -300, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "an unreachable identity service is exit 4 (network error)"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "stdout must be empty on failure"
    );
}

/// When the identity service rejects the refresh (400 invalid_grant) the
/// command exits 2 (auth error) with login guidance.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_refresh_rejected_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"error":"invalid_grant"}"#))
        .mount(&server)
        .await;

    setup_authorization_code_profile(&server.uri());
    write_stored_token("expired-access", -300, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["auth", "token"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "a rejected refresh is exit 2 (auth error)"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "stdout must be empty on failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ags auth login"),
        "expected login guidance on stderr, got: {stderr}"
    );
}

// ── Verbose and client-credentials paths ──

/// Under `--verbose` the refreshed token must still stay off stderr, even
/// though verbose mode adds extra output there.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_verbose_refresh_keeps_tokens_off_stderr() {
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

    setup_authorization_code_profile(&server.uri());
    write_stored_token("expired-access", -300, Some("valid-refresh"));

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["--verbose", "auth", "token"])
        .output()
        .unwrap();

    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "refreshed-access\n",
        "stdout must be the refreshed token alone"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for secret in [
        "refreshed-access",
        "rotated",
        "valid-refresh",
        "expired-access",
    ] {
        assert!(
            !stderr.contains(secret),
            "{secret} must not appear on stderr even under --verbose, got: {stderr}"
        );
    }
}

/// A client-credentials profile with a stored secret and no stored token
/// obtains a fresh token via the client-credentials grant.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_token_client_credentials_source() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"cc-new","expires_in":3600,"token_type":"Bearer"}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    // Set up a client-credentials profile with a stored secret.
    ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("cid".to_string()),
        grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
        ..Default::default()
    }
    .save("default")
    .unwrap();
    store::store_secret("default", "sekret").unwrap();
    activate_default_profile();

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .env_remove("AGS_ACCESS_TOKEN")
        .args(["--format", "json", "auth", "token"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("expected JSON: {stdout} ({e})"));
    assert_eq!(json["access_token"], "cc-new");
    assert_eq!(json["source"], "client_credentials");
    assert!(
        json["expires_at"].as_u64().unwrap() > now_secs(),
        "a client-credentials token must expire in the future: {json}"
    );
}

// ── Help ──

/// The help text must warn that the command prints a secret, and show the
/// substitution it exists for.
#[test]
fn test_auth_token_help_documents_the_secret_and_its_use() {
    ags()
        .args(["auth", "token", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Prints a secret"))
        .stdout(predicate::str::contains("$(ags auth token)"));
}
