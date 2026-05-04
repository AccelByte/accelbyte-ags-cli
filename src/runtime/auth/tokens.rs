//! OAuth token wire types, normalization, and token endpoint HTTP calls.

use reqwest::Client;
use serde::Deserialize;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::protocol::error::RuntimeError;
use crate::support::strings::truncate_display_text;

use super::errors::AuthError;
use super::store;

/// Default token lifetime when the server omits `expires_in`.
const DEFAULT_EXPIRES_IN: u64 = 3600;

/// Build the OAuth token endpoint URL from a base URL.
///
/// Concatenates the path explicitly rather than using `Url::join`, which
/// follows RFC 3986 relative-reference rules and would silently strip a
/// trailing path segment on the base URL (e.g. a gateway prefix like
/// `https://proxy/gw` would lose `/gw`).
fn token_url(base_url: &str) -> Result<reqwest::Url, AuthError> {
    reqwest::Url::parse(base_url).map_err(|_| AuthError::InvalidBaseUrl(base_url.to_string()))?;
    let trimmed = base_url.trim_end_matches('/');
    reqwest::Url::parse(&format!("{trimmed}/iam/v3/oauth/token"))
        .map_err(|_| AuthError::InvalidBaseUrl(base_url.to_string()))
}

/// Cap on token-endpoint error body bytes propagated into user-facing error strings.
const TOKEN_ERROR_BODY_LIMIT: usize = 200;

/// Truncate a token-endpoint error body to a bounded display length so a
/// misbehaving proxy returning a multi-KB error page cannot blow up the UI.
fn truncate_token_error_body(body: &str) -> String {
    truncate_display_text(body, TOKEN_ERROR_BODY_LIMIT).to_string()
}

/// Result of normalising a server-supplied `expires_in` value, with an
/// optional warning text the caller can surface to the user.
#[derive(Debug, PartialEq)]
pub(crate) struct NormalizedExpiresIn {
    pub value: u64,
    pub warning: Option<String>,
}

/// Raw JSON response from the IAM OAuth2 token endpoint.
#[derive(Debug, Deserialize, Zeroize, ZeroizeOnDrop)]
struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    #[allow(dead_code)] // Present in OAuth2 wire format; not consumed after deserialization
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub refresh_expires_in: Option<u64>,
}

/// Normalised result of a successful token fetch.
#[derive(Debug, Zeroize, ZeroizeOnDrop)]
pub struct TokenResult {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    pub refresh_expires_in: Option<u64>,
    /// Set when the server omitted/zeroed `expires_in` and we substituted a
    /// default. Callers should surface this once via their progress sink.
    #[zeroize(skip)]
    pub expires_in_warning: Option<String>,
}

/// Normalize `expires_in` — if the server returned 0 (or omitted the field),
/// default to 3600s and produce a warning the caller can surface. Callers
/// that do not need the warning may drop it.
pub(crate) fn normalize_expires_in(expires_in: u64) -> NormalizedExpiresIn {
    if expires_in == 0 {
        NormalizedExpiresIn {
            value: DEFAULT_EXPIRES_IN,
            warning: Some(format!(
                "Server did not return an expiry — assuming {DEFAULT_EXPIRES_IN}s"
            )),
        }
    } else {
        NormalizedExpiresIn {
            value: expires_in,
            warning: None,
        }
    }
}

/// Fetch a token via the client credentials grant.
pub async fn fetch_client_credentials_token(
    client: &Client,
    base_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<TokenResult, RuntimeError> {
    let url = token_url(base_url).map_err(RuntimeError::from)?;

    let response = client
        .post(url)
        .basic_auth(client_id, Some(client_secret))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await
        .map_err(RuntimeError::from)?;

    let status = response.status();
    let body = crate::runtime::dispatch::http::read_response_body(response).await?;

    if !status.is_success() {
        return Err(RuntimeError::from(AuthError::TokenRequestFailed {
            status: status.as_u16(),
            body: truncate_token_error_body(&body),
        }));
    }

    parse_token_result(&body)
}

/// Fetch a token via refresh token grant.
///
/// For confidential clients, sends Basic auth with client_id:client_secret.
/// For public clients (no secret), sends client_id in the form body.
pub async fn fetch_refresh_token(
    client: &Client,
    base_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<TokenResult, RuntimeError> {
    let url = token_url(base_url).map_err(RuntimeError::from)?;

    let mut request = client.post(url);
    if let Some(secret) = client_secret {
        request = request.basic_auth(client_id, Some(secret));
    }

    let mut form_params = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    if client_secret.is_none() {
        form_params.push(("client_id", client_id));
    }

    let response = request
        .form(&form_params)
        .send()
        .await
        .map_err(RuntimeError::from)?;

    let status = response.status();
    let body = crate::runtime::dispatch::http::read_response_body(response).await?;

    if !status.is_success() {
        return Err(RuntimeError::from(AuthError::TokenRefreshFailed {
            status: status.as_u16(),
            body: truncate_token_error_body(&body),
        }));
    }

    parse_token_result(&body)
}

/// Exchange an authorization code for tokens.
///
/// `client_secret` is optional — public OAuth clients use PKCE alone.
/// Confidential clients provide a secret for Basic auth.
pub async fn exchange_authorization_code(
    client: &Client,
    base_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    code_verifier: &str,
) -> Result<TokenResult, RuntimeError> {
    let url = token_url(base_url).map_err(RuntimeError::from)?;

    let mut request = client.post(url);

    if let Some(secret) = client_secret {
        request = request.basic_auth(client_id, Some(secret));
    }

    let mut form_params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", code_verifier),
    ];
    if client_secret.is_none() {
        form_params.push(("client_id", client_id));
    }

    let response = request
        .form(&form_params)
        .send()
        .await
        .map_err(RuntimeError::from)?;

    let status = response.status();
    let body = crate::runtime::dispatch::http::read_response_body(response).await?;

    if !status.is_success() {
        return Err(RuntimeError::from(AuthError::TokenExchangeFailed {
            status: status.as_u16(),
            body: truncate_token_error_body(&body),
        }));
    }

    parse_token_result(&body)
}

/// Converts a borrowed [`TokenResult`] into [`store::TokenData`] for
/// persistence, leaving the original accessible to the caller.
pub(crate) fn token_result_to_token_data(
    result: &TokenResult,
    grant_type: crate::protocol::request::GrantType,
    now: u64,
) -> store::TokenData {
    store::TokenData {
        access_token: result.access_token.clone(),
        expires_at: now.saturating_add(result.expires_in),
        refresh_token: result.refresh_token.clone(),
        refresh_expires_at: result.refresh_expires_in.map(|exp| now.saturating_add(exp)),
        grant_type: Some(grant_type),
    }
}

/// Parse one successful OAuth token response body into the normalized token result shape.
fn parse_token_result(body: &str) -> Result<TokenResult, RuntimeError> {
    let token_response: TokenResponse = serde_json::from_str(body)
        .map_err(|e| RuntimeError::from(AuthError::TokenParseFailed(e.to_string())))?;

    let normalized = normalize_expires_in(token_response.expires_in);
    Ok(TokenResult {
        access_token: token_response.access_token.clone(),
        expires_in: normalized.value,
        refresh_token: token_response.refresh_token.clone(),
        refresh_expires_in: token_response.refresh_expires_in,
        expires_in_warning: normalized.warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All TokenResult fields must be correctly mapped into TokenData including computed expiry times.
    #[test]
    fn test_token_result_to_token_data_maps_fields() {
        let result = TokenResult {
            access_token: "tok".to_string(),
            expires_in: 3600,
            refresh_token: Some("refresh".to_string()),
            refresh_expires_in: Some(86400),
            expires_in_warning: None,
        };
        let now = 1_000_000u64;
        let data = token_result_to_token_data(
            &result,
            crate::protocol::request::GrantType::ClientCredentials,
            now,
        );

        assert_eq!(data.access_token, "tok");
        assert_eq!(data.expires_at, now + 3600);
        assert_eq!(data.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(data.refresh_expires_at, Some(now + 86400));
        assert_eq!(
            data.grant_type,
            Some(crate::protocol::request::GrantType::ClientCredentials)
        );
    }

    /// Absent refresh token fields must map to None rather than zero or empty values.
    #[test]
    fn test_token_result_to_token_data_no_refresh() {
        let result = TokenResult {
            access_token: "tok2".to_string(),
            expires_in: 1800,
            refresh_token: None,
            refresh_expires_in: None,
            expires_in_warning: None,
        };
        let data = token_result_to_token_data(
            &result,
            crate::protocol::request::GrantType::AuthorizationCode,
            500_000,
        );

        assert_eq!(data.refresh_token, None);
        assert_eq!(data.refresh_expires_at, None);
        assert_eq!(
            data.grant_type,
            Some(crate::protocol::request::GrantType::AuthorizationCode)
        );
    }

    /// The helper borrows TokenResult so callers can reuse it after conversion.
    #[test]
    fn test_token_result_to_token_data_does_not_consume_result() {
        let result = TokenResult {
            access_token: "tok3".to_string(),
            expires_in: 100,
            refresh_token: None,
            refresh_expires_in: None,
            expires_in_warning: None,
        };
        let _data = token_result_to_token_data(
            &result,
            crate::protocol::request::GrantType::ClientCredentials,
            0,
        );
        assert_eq!(result.access_token, "tok3");
    }

    /// Server-supplied 0 must produce DEFAULT_EXPIRES_IN with a warning text.
    #[test]
    fn test_normalize_expires_in_zero_produces_warning() {
        let result = normalize_expires_in(0);
        assert_eq!(result.value, 3600);
        assert!(result.warning.is_some());
        assert!(result.warning.as_deref().unwrap().contains("3600"));
    }

    /// Non-zero values pass through unchanged with no warning.
    #[test]
    fn test_normalize_expires_in_passthrough() {
        let result = normalize_expires_in(7200);
        assert_eq!(result.value, 7200);
        assert!(result.warning.is_none());
    }
}
