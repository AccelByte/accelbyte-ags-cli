use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ags::runtime::auth::tokens as service;

const TEST_CLIENT_ID: &str = "test-client-id";
const TEST_CLIENT_SECRET: &str = "test-client-secret";

fn expected_basic_auth() -> String {
    use base64::Engine;
    let encoded =
        base64::engine::general_purpose::STANDARD.encode("test-client-id:test-client-secret");
    format!("Basic {encoded}")
}

fn token_response_json(access_token: &str, expires_in: u64) -> String {
    format!(
        r#"{{"access_token":"{access_token}","expires_in":{expires_in},"token_type":"Bearer"}}"#
    )
}

fn token_response_with_refresh(
    access_token: &str,
    expires_in: u64,
    refresh_token: &str,
    refresh_expires_in: u64,
) -> String {
    format!(
        r#"{{"access_token":"{access_token}","expires_in":{expires_in},"token_type":"Bearer","refresh_token":"{refresh_token}","refresh_expires_in":{refresh_expires_in}}}"#
    )
}

// ── Client credentials grant ──

/// Client credentials grant must send Basic auth and the correct grant_type so the IAM server can authenticate the client
#[tokio::test]
async fn test_client_credentials_sends_correct_request() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(header("Authorization", expected_basic_auth().as_str()))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(token_response_json("cc-token-123", 3600)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let result = service::fetch_client_credentials_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        TEST_CLIENT_SECRET,
    )
    .await
    .unwrap();

    assert_eq!(result.access_token, "cc-token-123");
    assert_eq!(result.expires_in, 3600);
}

/// Successful token responses must be deserialized into a usable TokenResult with correct field values
#[tokio::test]
async fn test_client_credentials_returns_token_result() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(token_response_json("my-token", 7200)),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let result = service::fetch_client_credentials_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        TEST_CLIENT_SECRET,
    )
    .await
    .unwrap();

    assert_eq!(result.access_token, "my-token");
    assert_eq!(result.expires_in, 7200);
}

/// Invalid credentials must return a clear error rather than panicking, so callers can prompt the user to fix their config
#[tokio::test]
async fn test_client_credentials_handles_401() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid_client"}"#))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let error = service::fetch_client_credentials_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        TEST_CLIENT_SECRET,
    )
    .await
    .unwrap_err();

    let error_message = error.to_string();
    assert!(
        error_message.contains("Token request failed"),
        "Expected 'Token request failed', got: {error_message}"
    );
}

/// Server errors during token fetch must surface as actionable errors rather than silent failures
#[tokio::test]
async fn test_client_credentials_handles_500() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let error = service::fetch_client_credentials_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        TEST_CLIENT_SECRET,
    )
    .await
    .unwrap_err();

    let error_message = error.to_string();
    assert!(
        error_message.contains("Token request failed"),
        "Expected 'Token request failed', got: {error_message}"
    );
}

/// Malformed token responses must produce a parse error so the user knows the server returned garbage, not a valid token
#[tokio::test]
async fn test_client_credentials_handles_malformed_json() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let error = service::fetch_client_credentials_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        TEST_CLIENT_SECRET,
    )
    .await
    .unwrap_err();

    let error_message = error.to_string();
    assert!(
        error_message.contains("Failed to parse token response"),
        "Expected parse error, got: {error_message}"
    );
}

// ── Authorization code exchange ──

/// Auth code exchange must include the PKCE verifier and code so the server can complete the OAuth2 handshake
#[tokio::test]
async fn test_authorization_code_exchange_sends_correct_request() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(header("Authorization", expected_basic_auth().as_str()))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=auth-code-123"))
        .and(body_string_contains("code_verifier=test-verifier"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(token_response_with_refresh(
                "ac-token-456",
                3600,
                "refresh-token-789",
                86400,
            )),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let result = ags::runtime::auth::tokens::exchange_authorization_code(
        &client,
        &server.uri(),
        "test-client-id",
        Some("test-client-secret"),
        "auth-code-123",
        "test-verifier",
    )
    .await
    .unwrap();

    assert_eq!(result.access_token, "ac-token-456");
    assert_eq!(result.expires_in, 3600);
    assert_eq!(result.refresh_token.as_deref(), Some("refresh-token-789"));
    assert_eq!(result.refresh_expires_in, Some(86400));
}

/// Expired or invalid auth codes must produce a descriptive error so the user knows to re-authenticate
#[tokio::test]
async fn test_authorization_code_exchange_handles_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"error":"invalid_grant","error_description":"code expired"}"#),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let error = ags::runtime::auth::tokens::exchange_authorization_code(
        &client,
        &server.uri(),
        "test-client-id",
        Some("test-client-secret"),
        "expired-code",
        "test-verifier",
    )
    .await
    .unwrap_err();

    let error_message = error.to_string();
    assert!(
        error_message.contains("Token exchange failed"),
        "Expected 'Token exchange failed', got: {error_message}"
    );
}

// ── Refresh token ──

/// Refresh token request must include Basic auth and the refresh_token grant_type to silently extend sessions
#[tokio::test]
async fn test_refresh_token_sends_correct_request() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(header("Authorization", expected_basic_auth().as_str()))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=my-refresh-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(token_response_with_refresh(
                "new-access-token",
                3600,
                "new-refresh-token",
                86400,
            )),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let result = service::fetch_refresh_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        Some(TEST_CLIENT_SECRET),
        "my-refresh-token",
    )
    .await
    .unwrap();

    assert_eq!(result.access_token, "new-access-token");
    assert_eq!(result.expires_in, 3600);
    assert_eq!(result.refresh_token.as_deref(), Some("new-refresh-token"));
}

/// Expired refresh tokens must produce a clear error so the caller can fall back to a full re-login
#[tokio::test]
async fn test_refresh_token_handles_401() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string(
            r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
        ))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let error = service::fetch_refresh_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        Some(TEST_CLIENT_SECRET),
        "expired-refresh-token",
    )
    .await
    .unwrap_err();

    let error_message = error.to_string();
    assert!(
        error_message.contains("Token refresh failed"),
        "Expected 'Token refresh failed', got: {error_message}"
    );
}

// ── Base URL handling ──

/// Trailing slashes on base URLs must be stripped to avoid double-slash path issues in token requests
#[tokio::test]
async fn test_base_url_trailing_slash_stripped() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(token_response_json("token", 3600)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let base_url_with_slash = format!("{}/", server.uri());

    let result = service::fetch_client_credentials_token(
        &client,
        &base_url_with_slash,
        TEST_CLIENT_ID,
        TEST_CLIENT_SECRET,
    )
    .await
    .unwrap();

    assert_eq!(result.access_token, "token");
}

// ── Response edge cases ──

/// Missing expires_in must default to 3600s so token refresh scheduling works even with incomplete server responses
#[tokio::test]
async fn test_token_response_without_expires_in_defaults_to_3600() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"access_token":"tok","token_type":"Bearer"}"#),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    let result = service::fetch_client_credentials_token(
        &client,
        &server.uri(),
        TEST_CLIENT_ID,
        TEST_CLIENT_SECRET,
    )
    .await
    .unwrap();

    assert_eq!(result.access_token, "tok");
    assert_eq!(
        result.expires_in, 3600,
        "Missing expires_in should default to 3600s"
    );
}

/// Regression guard: a non-URL `--base-url` must produce a graceful error,
/// not a Rust panic (which the earlier `.expect()` at oauth.rs:63 caused).
#[test]
fn test_auth_login_rejects_invalid_base_url_without_panic() {
    use predicates::prelude::PredicateBooleanExt;
    use predicates::str::contains;

    for bad in ["bogus", "not-a-url", ""] {
        super::common::cli_helpers::ags_isolated()
            .env("AGS_AUTH_TIMEOUT", "1")
            .args(["auth", "login", "--base-url", bad, "--client-id", "foo"])
            .assert()
            .failure()
            .code(predicates::ord::ne(101))
            .stderr(contains("Invalid base URL").and(contains("panicked").not()));
    }
}
