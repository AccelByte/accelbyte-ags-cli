//! Functional coverage for `ags auth refresh` through the compiled binary.
//! The no-credentials and env-token paths are offline; the success paths use a
//! wiremock IdP and exercise argv → route_auth → handle_auth_refresh →
//! Runtime::auth_refresh → human/JSON rendering end to end.

use crate::common::cli_helpers::ags_isolated;
use crate::common::env_guard::TempEnvGuard;
use ags_runtime::runtime::auth::store;
use ags_runtime::runtime::config::{GlobalConfig, ProfileConfig};
use predicates::prelude::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_auth_refresh_no_credentials_errors_with_login_guidance() {
    ags_isolated()
        .env_remove("AGS_ACCESS_TOKEN")
        .env_remove("AGS_CLIENT_ID")
        .env_remove("AGS_CLIENT_SECRET")
        .args(["auth", "refresh"])
        .assert()
        .failure()
        // Exit 2: "nothing to refresh" is a NotAuthenticated error, which maps
        // to CliError::Auth (exit 2) so scripts can trigger re-authentication.
        .code(2)
        .stderr(predicate::str::contains("ags auth login"));
}

#[test]
fn test_auth_refresh_env_access_token_is_rejected() {
    ags_isolated()
        .env("AGS_ACCESS_TOKEN", "external-token")
        .args(["auth", "refresh"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("AGS_ACCESS_TOKEN"));
}

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

/// Recursively find the first string value for `field` anywhere in `value`.
fn find_string_field<'a>(value: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(v) = map.get(field).and_then(|v| v.as_str()) {
                return Some(v);
            }
            map.values().find_map(|v| find_string_field(v, field))
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_string_field(v, field)),
        _ => None,
    }
}

/// A wiremock IdP that answers the client-credentials token exchange.
async fn client_credentials_mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"cc-new","expires_in":3600,"token_type":"Bearer"}"#,
        ))
        .mount(&server)
        .await;
    server
}

/// A client-credentials profile pointing at `server_uri`, with a stored secret,
/// activated as the default profile.
fn setup_client_credentials_profile(server_uri: &str) {
    ProfileConfig {
        base_url: Some(server_uri.to_string()),
        client_id: Some("cid".to_string()),
        grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
        ..Default::default()
    }
    .save("default")
    .unwrap();
    store::store_secret("default", "sekret").unwrap();
    activate_default_profile();
}

/// Client-credentials refresh with `--format json` emits a machine-readable
/// envelope carrying status="refreshed" plus the resolved base URL and client
/// id — exercising the full CLI seam, not just `refresh_profile` in isolation.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_refresh_client_credentials_json_success() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = client_credentials_mock().await;
    setup_client_credentials_profile(&server.uri());

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["--format", "json", "auth", "refresh"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("expected JSON output\nstdout: {stdout}\nstderr: {stderr}\nerror: {e}")
    });

    assert_eq!(
        find_string_field(&parsed, "status"),
        Some("refreshed"),
        "status must be 'refreshed': {parsed}"
    );
    assert_eq!(
        find_string_field(&parsed, "client_id"),
        Some("cid"),
        "refresh JSON must carry the client id: {parsed}"
    );
    let base = server.uri();
    assert_eq!(
        find_string_field(&parsed, "base_url"),
        Some(base.as_str()),
        "refresh JSON must carry the base URL: {parsed}"
    );
}

/// Client-credentials refresh in human mode prints the "Token refreshed"
/// headline through the compiled binary.
#[tokio::test]
#[serial_test::serial]
async fn test_auth_refresh_client_credentials_human_headline() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

    let server = client_credentials_mock().await;
    setup_client_credentials_profile(&server.uri());

    let output = ags_isolated()
        .env("AGS_HOME", tmp.path())
        .args(["auth", "refresh"])
        .output()
        .unwrap();

    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        combined.contains("Token refreshed"),
        "expected the 'Token refreshed' headline, got: {combined}"
    );
}
