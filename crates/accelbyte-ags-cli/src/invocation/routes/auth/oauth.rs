//! OAuth2 PKCE utilities and callback server for browser-based login.

use base64::Engine;
use rand::distr::{Alphanumeric, Distribution};
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use ags_protocol::error::RuntimeError;
use ags_runtime::runtime::auth::errors::AuthError;
use ags_runtime::runtime::dispatch::http::build_http_client;

/// PKCE `code_verifier` length in characters (RFC 7636 allows 43–128; we use
/// the maximum for entropy).
const PKCE_VERIFIER_LENGTH: usize = 128;

/// Read buffer size for the OAuth callback request — large enough for the
/// redirect line and headers of a single GET.
const CALLBACK_READ_BUFFER_BYTES: usize = 4096;

/// Generate a PKCE code_verifier and code_challenge pair.
///
/// The verifier is a 128-character random alphanumeric string.
/// The challenge is the SHA-256 hash of the verifier, base64url-encoded.
pub fn generate_pkce_pair() -> (String, String) {
    let mut rng = rand::rng();
    let verifier: String = Alphanumeric
        .sample_iter(&mut rng)
        .take(PKCE_VERIFIER_LENGTH)
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

/// Construct the Admin Portal link that opens the IAM client form with the
/// `ags_cli` template (a public client redirecting to the default callback
/// port) preselected.
pub fn build_create_client_url(base_url: &str, namespace: &str) -> Result<String, AuthError> {
    let base = base_url.trim_end_matches('/');
    let mut url = reqwest::Url::parse(&format!("{base}/admin/namespaces"))
        .map_err(|_| AuthError::InvalidBaseUrl(base_url.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| AuthError::InvalidBaseUrl(base_url.to_string()))?
        .extend([namespace, "iam-clients", "create"]);
    url.query_pairs_mut().append_pair("template", "ags_cli");
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

        let mut buffer = vec![0u8; CALLBACK_READ_BUFFER_BYTES];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .map_err(|e| RuntimeError::from(AuthError::CallbackReadFailed(e.to_string())))?;

        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let (code, state) = parse_callback_request(&request)?;

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

// ── Client check ──

/// Result of checking a client ID against IAM's authorize endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientCheck {
    /// IAM does not know this client ID.
    NotFound,
    /// IAM rejected the client with an error other than "not found".
    Rejected {
        error: String,
        description: Option<String>,
    },
    /// IAM accepted the client and redirected to the login page.
    Accepted { redirect_uri: Option<String> },
    /// The response was not a redirect or had no Location header.
    Inconclusive,
    /// The request to IAM failed (connection, DNS, timeout).
    Unreachable { reason: String },
}

/// Classify IAM's response to an authorize request from its status code
/// and Location header. A relative Location is resolved against the
/// authorize URL. Query values are read percent-decoded.
pub fn classify_authorize_redirect(
    authorize_url: &str,
    status: u16,
    location: Option<&str>,
) -> ClientCheck {
    let loc = match location {
        Some(l) if (300..400).contains(&status) => l,
        _ => return ClientCheck::Inconclusive,
    };

    let resolved = match reqwest::Url::parse(loc) {
        Ok(url) => url,
        Err(_) => match reqwest::Url::parse(authorize_url).and_then(|base| base.join(loc)) {
            Ok(url) => url,
            Err(_) => return ClientCheck::Inconclusive,
        },
    };

    let pairs: Vec<(String, String)> = resolved
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let find = |name: &str| -> Option<String> {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };

    if let Some(error) = find("error") {
        let description = find("error_description");
        // Measured 2026-10-22 against production IAM 7.43.4: an unknown client_id
        // answers 302 with error=unauthorized_client&error_description=client+not+found.
        // Any other description falls back to Rejected, which prints IAM's own text.
        let is_not_found = error == "unauthorized_client"
            && description
                .as_deref()
                .map(|d| {
                    let normalized = d.trim().to_ascii_lowercase();
                    let normalized = normalized.strip_suffix('.').unwrap_or(&normalized);
                    normalized.contains("not found")
                })
                .unwrap_or(false);
        if is_not_found {
            ClientCheck::NotFound
        } else {
            ClientCheck::Rejected { error, description }
        }
    } else {
        ClientCheck::Accepted {
            redirect_uri: find("redirect_uri"),
        }
    }
}

/// Join an error and each of its `source()` errors with `: `, skipping
/// consecutive duplicates (some error chains repeat the same text at two
/// levels).
fn error_chain_text(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut current = error.source();
    while let Some(cause) = current {
        let text = cause.to_string();
        if parts.last() != Some(&text) {
            parts.push(text);
        }
        current = cause.source();
    }
    parts.join(": ")
}

/// Send one GET to the authorize URL and classify IAM's redirect response.
///
/// Creates its own HTTP client with `build_http_client(Some(10))` -- a 10-second
/// timeout and no redirect following -- so the caller doesn't need to manage one.
pub async fn check_client(authorize_url: &str) -> ClientCheck {
    let client = match build_http_client(Some(10)) {
        Ok(c) => c,
        Err(e) => {
            return ClientCheck::Unreachable {
                reason: e.to_string(),
            }
        }
    };
    let response = match client.get(authorize_url).send().await {
        Ok(r) => r,
        Err(e) => {
            return ClientCheck::Unreachable {
                reason: error_chain_text(&e.without_url()),
            }
        }
    };
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    classify_authorize_redirect(authorize_url, status, location.as_deref())
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

    #[test]
    fn test_build_create_client_url_selects_cli_template() {
        let url = build_create_client_url(
            "https://my-game.prod.gamingservices.accelbyte.io/",
            "my-game",
        )
        .unwrap();
        assert_eq!(
            url,
            "https://my-game.prod.gamingservices.accelbyte.io/admin/namespaces/my-game/iam-clients/create?template=ags_cli"
        );
    }

    /// Separators in the namespace are percent-encoded rather than adding path segments
    #[test]
    fn test_build_create_client_url_encodes_namespace() {
        let url =
            build_create_client_url("https://my-game.prod.gamingservices.accelbyte.io", "a/b?c")
                .unwrap();
        assert_eq!(
            url,
            "https://my-game.prod.gamingservices.accelbyte.io/admin/namespaces/a%2Fb%3Fc/iam-clients/create?template=ags_cli"
        );
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

    // ── classify_authorize_redirect tests ──

    #[test]
    fn test_classify_redirect_client_not_found() {
        let auth_url = "https://demo.accelbyte.io/iam/v3/oauth/authorize?response_type=code&client_id=bad&state=s&code_challenge=c&code_challenge_method=S256";
        let location = "https://demo.accelbyte.io/auth/?error=unauthorized_client&error_description=client+not+found&state=s";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(location)),
            ClientCheck::NotFound,
        );
        // Title-cased description (case-insensitive match).
        let title_case = "https://demo.accelbyte.io/auth/?error=unauthorized_client&error_description=Client+Not+Found&state=s";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(title_case)),
            ClientCheck::NotFound,
        );
        // Trailing full stop is tolerated.
        let with_dot = "https://demo.accelbyte.io/auth/?error=unauthorized_client&error_description=Client+not+found.&state=s";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(with_dot)),
            ClientCheck::NotFound,
        );
        // Extra wording around "not found" is tolerated.
        let verbose = "https://demo.accelbyte.io/auth/?error=unauthorized_client&error_description=client+id+not+found&state=s";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(verbose)),
            ClientCheck::NotFound,
        );
        // A different unauthorized_client description is Rejected, not NotFound.
        let other_desc = "https://demo.accelbyte.io/auth/?error=unauthorized_client&error_description=grant+type+not+allowed&state=s";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(other_desc)),
            ClientCheck::Rejected {
                error: "unauthorized_client".to_string(),
                description: Some("grant type not allowed".to_string()),
            },
        );
    }

    #[test]
    fn test_classify_redirect_other_error_is_rejected() {
        let auth_url = "https://demo.accelbyte.io/iam/v3/oauth/authorize?response_type=code&client_id=x&state=s&code_challenge=c&code_challenge_method=S256";
        let location = "https://demo.accelbyte.io/auth/?error=invalid_scope&error_description=Scope+not+allowed&state=s";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(location)),
            ClientCheck::Rejected {
                error: "invalid_scope".to_string(),
                description: Some("Scope not allowed".to_string()),
            },
        );
    }

    #[test]
    fn test_classify_redirect_reads_redirect_uri() {
        let auth_url = "https://demo.accelbyte.io/iam/v3/oauth/authorize?response_type=code&client_id=ok&state=s&code_challenge=c&code_challenge_method=S256";
        let location = "https://demo.accelbyte.io/auth/?client_id=ok&redirect_uri=http%3A%2F%2F127.0.0.1%3A8080&request_id=r";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(location)),
            ClientCheck::Accepted {
                redirect_uri: Some("http://127.0.0.1:8080".to_string()),
            },
        );
        // Redirect with no redirect_uri query parameter.
        let no_redir = "https://demo.accelbyte.io/auth/?client_id=ok&request_id=r";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(no_redir)),
            ClientCheck::Accepted { redirect_uri: None },
        );
    }

    #[test]
    fn test_classify_redirect_resolves_relative_location() {
        let auth_url = "https://demo.accelbyte.io/iam/v3/oauth/authorize?response_type=code&client_id=ok&state=s&code_challenge=c&code_challenge_method=S256";
        let location = "/auth/?client_id=ok&redirect_uri=http%3A%2F%2F127.0.0.1%3A8080";
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, Some(location)),
            ClientCheck::Accepted {
                redirect_uri: Some("http://127.0.0.1:8080".to_string()),
            },
        );
    }

    #[test]
    fn test_error_chain_text_includes_every_source() {
        #[derive(Debug)]
        struct Inner;
        impl std::fmt::Display for Inner {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "inner")
            }
        }
        impl std::error::Error for Inner {}

        #[derive(Debug)]
        struct Outer(Inner);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "outer")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        assert_eq!(error_chain_text(&Outer(Inner)), "outer: inner");
    }

    #[test]
    fn test_error_chain_text_skips_repeated_text() {
        // Three-level chain where the middle level repeats the top level's
        // text. The duplicate should be collapsed so the output is "same: root"
        // rather than "same: same: root".
        #[derive(Debug)]
        struct Root;
        impl std::fmt::Display for Root {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "root")
            }
        }
        impl std::error::Error for Root {}

        #[derive(Debug)]
        struct Middle(Root);
        impl std::fmt::Display for Middle {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "same")
            }
        }
        impl std::error::Error for Middle {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        #[derive(Debug)]
        struct Top(Middle);
        impl std::fmt::Display for Top {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "same")
            }
        }
        impl std::error::Error for Top {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        assert_eq!(error_chain_text(&Top(Middle(Root))), "same: root");
    }

    #[test]
    fn test_classify_non_redirect_is_inconclusive() {
        let auth_url = "https://demo.accelbyte.io/iam/v3/oauth/authorize?response_type=code&client_id=x&state=s&code_challenge=c&code_challenge_method=S256";
        assert_eq!(
            classify_authorize_redirect(auth_url, 200, None),
            ClientCheck::Inconclusive,
        );
        assert_eq!(
            classify_authorize_redirect(auth_url, 400, Some("https://example.com")),
            ClientCheck::Inconclusive,
        );
        assert_eq!(
            classify_authorize_redirect(auth_url, 302, None),
            ClientCheck::Inconclusive,
        );
    }
}
