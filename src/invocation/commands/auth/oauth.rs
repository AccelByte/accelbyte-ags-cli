//! OAuth2 PKCE utilities and callback server for browser-based login.

use base64::Engine;
use rand::distr::{Alphanumeric, Distribution};
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use crate::protocol::error::RuntimeError;
use crate::runtime::auth::errors::AuthError;

/// Generate a PKCE code_verifier and code_challenge pair.
///
/// The verifier is a 128-character random alphanumeric string.
/// The challenge is the SHA-256 hash of the verifier, base64url-encoded.
pub fn generate_pkce_pair() -> (String, String) {
    let mut rng = rand::rng();
    let verifier: String = Alphanumeric
        .sample_iter(&mut rng)
        .take(128)
        .map(char::from)
        .collect();

    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

    (verifier, challenge)
}

/// Generate a random 32-character hex string for CSRF state parameter.
pub fn generate_state() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    hex_encode(&bytes)
}

/// Encode a byte slice as a lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push_str(&format!("{byte:02x}"));
    }
    result
}

/// Construct the OAuth2 authorization URL with properly encoded query parameters.
///
/// AccelByte sets `redirect_uri` and `scope` from the client configuration,
/// so we omit them here to avoid mismatches.
pub fn build_authorize_url(
    base_url: &str,
    client_id: &str,
    state: &str,
    code_challenge: &str,
) -> Result<String, AuthError> {
    let base = base_url.trim_end_matches('/');
    let mut url = reqwest::Url::parse(&format!("{base}/iam/v3/oauth/authorize"))
        .map_err(|_| AuthError::InvalidBaseUrl(base_url.to_string()))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url.to_string())
}

/// Start a localhost callback server and wait for the authorization code.
///
/// Returns `(code, state)` from the callback query parameters.
/// Times out after `timeout_secs` seconds.
pub async fn start_callback_server(
    listener: TcpListener,
    timeout_secs: u64,
) -> Result<(String, String), RuntimeError> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| RuntimeError::from(AuthError::CallbackAcceptFailed(e.to_string())))?;

        let mut buffer = vec![0u8; 4096];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .map_err(|e| RuntimeError::from(AuthError::CallbackReadFailed(e.to_string())))?;

        let request = String::from_utf8_lossy(&buffer[..bytes_read]);

        // Parse the GET request line to extract query parameters
        let (code, state) = parse_callback_request(&request)?;

        // Send success response
        let html = r#"<!DOCTYPE html>
<html><body>
<h2>Authentication successful</h2>
<p>You can close this window and return to the terminal.</p>
</body></html>"#;

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );

        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;

        Ok((code, state))
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_) => Err(AuthError::CallbackTimeout(timeout_secs).into()),
    }
}

/// Parse the callback GET request to extract `code` and `state` query parameters.
fn parse_callback_request(request: &str) -> Result<(String, String), RuntimeError> {
    // Extract the request line: "GET /callback?code=xxx&state=yyy HTTP/1.1"
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| RuntimeError::from(AuthError::CallbackEmpty))?;

    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| RuntimeError::from(AuthError::CallbackMalformed))?;

    // Check for error response from the authorization server
    if let Some(query) = path.split('?').nth(1) {
        if let Some(error) = extract_query_param(query, "error") {
            let description = extract_query_param(query, "error_description").unwrap_or_default();
            return Err(RuntimeError::from(AuthError::AuthorizationFailed {
                error,
                description,
            }));
        }
    }

    let query = path
        .split('?')
        .nth(1)
        .ok_or_else(|| RuntimeError::from(AuthError::CallbackMissingQuery))?;

    let code = extract_query_param(query, "code")
        .ok_or_else(|| RuntimeError::from(AuthError::CallbackMissingCode))?;

    let state = extract_query_param(query, "state")
        .ok_or_else(|| RuntimeError::from(AuthError::CallbackMissingState))?;

    Ok((code, state))
}

/// Extract and percent-decode a query parameter value by name.
///
/// Uses `url::form_urlencoded::parse` so both `%xx` percent escapes and
/// `+`-as-space are decoded — matching what an OAuth provider's
/// `error_description` parameter actually contains.
fn extract_query_param(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Bind the callback server port, returning the listener for reuse.
pub async fn bind_callback_port(port: u16) -> Result<TcpListener, RuntimeError> {
    TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .map_err(|_| RuntimeError::from(AuthError::PortInUse(port)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PKCE verifiers must be exactly 128 chars to provide sufficient entropy per RFC 7636
    #[test]
    fn test_pkce_verifier_length() {
        let (verifier, _) = generate_pkce_pair();
        assert_eq!(verifier.len(), 128);
    }

    /// PKCE verifiers must only contain unreserved URI characters (alphanumeric) per RFC 7636
    #[test]
    fn test_pkce_verifier_alphanumeric() {
        let (verifier, _) = generate_pkce_pair();
        assert!(verifier.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    /// Code challenges must use base64url encoding without padding per RFC 7636
    #[test]
    fn test_pkce_challenge_is_valid_base64url() {
        let (_, challenge) = generate_pkce_pair();
        assert!(!challenge.is_empty());
        // base64url characters: A-Z, a-z, 0-9, -, _
        assert!(challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    /// The challenge must be the SHA-256 hash of the verifier, base64url-encoded
    #[test]
    fn test_pkce_challenge_matches_verifier() {
        let (verifier, challenge) = generate_pkce_pair();
        let hash = Sha256::digest(verifier.as_bytes());
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
        assert_eq!(challenge, expected);
    }

    /// CSRF state must be 32 hex chars (128 bits) for sufficient randomness
    #[test]
    fn test_state_length() {
        let state = generate_state();
        assert_eq!(state.len(), 32);
    }

    /// State parameter must be valid hex to survive URL round-tripping
    #[test]
    fn test_state_is_hex() {
        let state = generate_state();
        assert!(state.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Each state value must be unique to prevent CSRF replay attacks
    #[test]
    fn test_state_uniqueness() {
        let state1 = generate_state();
        let state2 = generate_state();
        assert_ne!(state1, state2);
    }

    /// Authorize URL must include all required OAuth2 PKCE params and omit redirect_uri/scope
    #[test]
    fn test_build_authorize_url_contains_all_params() {
        let url = build_authorize_url(
            "https://demo.accelbyte.io",
            "my-client-id",
            "abc123",
            "challenge456",
        )
        .unwrap();
        assert!(url.contains("https://demo.accelbyte.io/iam/v3/oauth/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=my-client-id"));
        assert!(url.contains("state=abc123"));
        assert!(url.contains("code_challenge=challenge456"));
        assert!(url.contains("code_challenge_method=S256"));
        // redirect_uri and scope are omitted — AccelByte uses client defaults
        assert!(!url.contains("redirect_uri"));
        assert!(!url.contains("scope"));
    }

    /// Trailing slashes on base URLs must be stripped to avoid double-slash in the path
    #[test]
    fn test_build_authorize_url_strips_trailing_slash() {
        let url =
            build_authorize_url("https://demo.accelbyte.io/", "id", "state", "challenge").unwrap();
        assert!(url.starts_with("https://demo.accelbyte.io/iam/v3/oauth/authorize?"));
        assert!(!url.contains("//iam"));
    }

    /// Relative / non-URL base values must return InvalidBaseUrl, not panic.
    #[test]
    fn test_build_authorize_url_rejects_relative_url() {
        for bad in ["", "bogus", "not-a-url", "/iam"] {
            let err = build_authorize_url(bad, "id", "state", "challenge").unwrap_err();
            match err {
                AuthError::InvalidBaseUrl(value) => assert_eq!(value, bad),
                other => panic!("expected InvalidBaseUrl for '{bad}', got {other:?}"),
            }
        }
    }

    /// Callback parser must extract both code and state from a well-formed OAuth redirect
    #[test]
    fn test_parse_callback_request_extracts_code_and_state() {
        let request =
            "GET /callback?code=auth-code-123&state=state-abc HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (code, state) = parse_callback_request(request).unwrap();
        assert_eq!(code, "auth-code-123");
        assert_eq!(state, "state-abc");
    }

    /// Missing authorization code must produce a clear error, not a silent failure
    #[test]
    fn test_parse_callback_request_missing_code() {
        let request = "GET /callback?state=abc HTTP/1.1\r\n\r\n";
        let result = parse_callback_request(request);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("code"));
    }

    /// Missing state parameter must be rejected to guard against CSRF
    #[test]
    fn test_parse_callback_request_missing_state() {
        let request = "GET /callback?code=abc HTTP/1.1\r\n\r\n";
        let result = parse_callback_request(request);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("state"));
    }

    /// Authorization server errors (e.g. access_denied) must surface the error code to the user
    #[test]
    fn test_parse_callback_request_error_response() {
        let request =
            "GET /callback?error=access_denied&error_description=User+denied+access HTTP/1.1\r\n\r\n";
        let result = parse_callback_request(request);
        assert!(result.is_err());
        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("access_denied"),
            "message should include error code: {error_message}"
        );
        // description now lives in metadata.reason — not checked via Display
    }

    /// End-to-end: callback server must accept a connection and return the code and state
    #[tokio::test]
    async fn test_callback_server_receives_code() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_handle = tokio::spawn(async move { start_callback_server(listener, 120).await });

        // Give the server a moment to start accepting
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Send a mock callback request
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        stream
            .write_all(b"GET /callback?code=test-code&state=test-state HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();

        let (code, state) = server_handle.await.unwrap().unwrap();
        assert_eq!(code, "test-code");
        assert_eq!(state, "test-state");
    }

    /// Query parameter extraction must find existing keys and return None for missing ones
    #[test]
    fn test_extract_query_param() {
        assert_eq!(
            extract_query_param("code=abc&state=def", "code"),
            Some("abc".to_string())
        );
        assert_eq!(
            extract_query_param("code=abc&state=def", "state"),
            Some("def".to_string())
        );
        assert_eq!(extract_query_param("code=abc&state=def", "missing"), None);
    }

    /// Percent-encoded values must be decoded — error_description='hello%20world' surfaces as 'hello world'
    #[test]
    fn test_extract_query_param_percent_decodes() {
        assert_eq!(
            extract_query_param("error_description=hello%20world", "error_description"),
            Some("hello world".to_string())
        );
    }

    /// Plus signs decode to spaces, matching form-urlencoded semantics
    #[test]
    fn test_extract_query_param_plus_is_space() {
        assert_eq!(
            extract_query_param("description=foo+bar", "description"),
            Some("foo bar".to_string())
        );
    }

    // ── parse_callback_request edge cases ──

    /// Empty HTTP request must fail gracefully rather than panic
    #[test]
    fn test_parse_callback_empty_request() {
        let result = parse_callback_request("");
        assert!(result.is_err());
    }

    /// HTTP request line with no path component must fail gracefully
    #[test]
    fn test_parse_callback_no_path() {
        let result = parse_callback_request("GET\r\n\r\n");
        assert!(result.is_err());
    }

    /// Callback path without query string must be rejected since code and state are required
    #[test]
    fn test_parse_callback_path_without_query() {
        let result = parse_callback_request("GET /callback HTTP/1.1\r\n\r\n");
        assert!(result.is_err());
    }

    /// Binary garbage input must not cause a panic or undefined behaviour
    #[test]
    fn test_parse_callback_garbage_input() {
        let result = parse_callback_request("\x00\x01\x02 binary garbage");
        assert!(result.is_err());
    }

    /// Authorization errors with a description must surface the error code in the message
    #[test]
    fn test_parse_callback_error_with_description() {
        let request = "GET /callback?error=invalid_scope&error_description=Scope+not+allowed HTTP/1.1\r\n\r\n";
        let result = parse_callback_request(request);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("invalid_scope"));
    }

    /// Authorization errors without a description must still be detected and rejected
    #[test]
    fn test_parse_callback_error_without_description() {
        let request = "GET /callback?error=server_error HTTP/1.1\r\n\r\n";
        let result = parse_callback_request(request);
        assert!(result.is_err());
    }
}
