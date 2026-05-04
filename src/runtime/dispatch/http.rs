//! HTTP client types and reqwest-backed transport.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;

use crate::protocol::catalogue::HttpMethod;
use crate::protocol::error::RuntimeError;

// ── Transport Types ──

/// A single HTTP request the runtime wants to send.
#[derive(Debug, Clone)]
pub(crate) struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
}

/// Response body, tagged by media-type family so binary payloads cannot be
/// accidentally UTF-8-decoded. Produced by the `HttpClient::send`
/// implementation; consumed by the dispatch layer.
#[derive(Debug, Clone)]
pub(crate) enum HttpBody {
    /// Decoded UTF-8 text. Used for JSON, text/*, and all error responses.
    Text(String),
    /// Raw bytes with the declared content type. Used for binary-producing
    /// operations (images, archives, Excel, etc.). Produced by
    /// `read_response_body_tagged` when `Content-Type` is non-text and
    /// status is 2xx; consumed by the dispatch execution and pagination
    /// flows for binary-aware response handling.
    Binary {
        content_type: String,
        bytes: Vec<u8>,
    },
}

impl HttpBody {
    /// Expect a text body. Panics if binary — internal use only for call
    /// sites that provably never receive binary (e.g. error-body decode
    /// helpers in tests).
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn expect_text(&self) -> &str {
        match self {
            HttpBody::Text(s) => s,
            HttpBody::Binary { content_type, .. } => {
                panic!("expected text body, got binary ({content_type})")
            }
        }
    }
}

/// The response the HTTP client returns. Body is tagged text or binary;
/// status is surfaced separately so the caller can branch on success vs
/// error without re-parsing the body.
#[derive(Debug, Clone)]
pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: HttpBody,
}

/// Maximum response body size (10 MB). Prevents memory exhaustion from oversized responses.
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

// ── Client Construction And Body Decoding ──

/// Build the shared HTTP client.
pub(crate) fn build_http_client(timeout_secs: Option<u64>) -> Result<Client, RuntimeError> {
    let timeout = timeout_secs.unwrap_or(60);
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

/// Classify an HTTP `Content-Type` header value as text (UTF-8 decodable)
/// or binary. Case- and whitespace-insensitive on the media type; the
/// parameter portion (after `;`) is ignored.
///
/// Text: `application/json`, `application/<subtype>+json` (JSON-LD,
/// JSON-API, …), `text/*`. Everything else — including
/// `application/octet-stream` and `image/*` — is binary.
pub(crate) fn is_text_content_type(content_type: &str) -> bool {
    let trimmed = content_type.trim();
    let media = trimmed
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if media.is_empty() {
        return false;
    }
    if media == "application/json" || media.starts_with("text/") {
        return true;
    }
    if let Some(rest) = media.strip_prefix("application/") {
        // Require a non-empty subtype before `+json` so that the edge
        // case `application/+json` does not accidentally classify as text.
        if rest.len() > "+json".len() && rest.ends_with("+json") {
            return true;
        }
    }
    false
}

/// Shared helper: read the response bytes enforcing the 10 MB size cap.
/// Used by both `read_response_body` (text-only, for auth) and
/// `read_response_body_tagged` (content-type aware).
async fn read_response_bytes_capped(response: reqwest::Response) -> Result<Vec<u8>, RuntimeError> {
    use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

    let content_length = response.content_length().unwrap_or(0) as usize;
    if content_length > MAX_RESPONSE_BYTES {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::ResponseTooLarge,
            message: format!(
                "Response too large ({} bytes, limit is {} bytes).",
                content_length, MAX_RESPONSE_BYTES
            ),
            details: None,
            hint: Some(
                "Narrow the query (e.g. --page-limit or filters) to reduce the response size."
                    .to_string(),
            ),
            trace: None,
        });
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::ResponseTooLarge,
            message: format!(
                "Response too large ({} bytes, limit is {} bytes).",
                bytes.len(),
                MAX_RESPONSE_BYTES
            ),
            details: None,
            hint: Some(
                "Narrow the query (e.g. --page-limit or filters) to reduce the response size."
                    .to_string(),
            ),
            trace: None,
        });
    }
    Ok(bytes.to_vec())
}

/// Read a response body as text, enforcing a size limit. Used by auth
/// token-response readers where the media type is always JSON. Callers
/// that may receive binary should use `read_response_body_tagged`.
pub(crate) async fn read_response_body(
    response: reqwest::Response,
) -> Result<String, RuntimeError> {
    use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

    let bytes = read_response_bytes_capped(response).await?;
    String::from_utf8(bytes).map_err(|_| RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: "Response body is not valid UTF-8.".to_string(),
        details: None,
        hint: None,
        trace: None,
    })
}

/// Read a response body, returning `Binary` for non-text content types and
/// `Text` for JSON/text media types. Error responses (status >= 400) are
/// always returned as `Text` regardless of `Content-Type` because the
/// AccelByte error classifier requires a string body.
pub(crate) async fn read_response_body_tagged(
    response: reqwest::Response,
) -> Result<HttpBody, RuntimeError> {
    use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = read_response_bytes_capped(response).await?;

    // Empty bodies (204 No Content, or any 2xx with zero-length payload) have
    // no meaningful media type. Classifying them as Binary would route them
    // through the binary-output path and suppress the success summary and the
    // verbose request/response trace.
    let is_text_body = bytes.is_empty() || status >= 400 || is_text_content_type(&content_type);

    if is_text_body {
        return match String::from_utf8(bytes) {
            Ok(s) => Ok(HttpBody::Text(s)),
            Err(_) => Err(RuntimeError {
                kind: RuntimeErrorKind::Upstream { status, code: None },
                message: "Server said text but body is not valid UTF-8.".to_string(),
                details: None,
                hint: None,
                trace: None,
            }),
        };
    }

    Ok(HttpBody::Binary {
        content_type,
        bytes,
    })
}

/// Transport-level HTTP client used by the runtime.
#[async_trait]
pub(crate) trait HttpClient: Send + Sync {
    /// Issue a single HTTP request and return the captured response or a transport-level error.
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RuntimeError>;
}

/// Reqwest-backed default implementation of `HttpClient`.
pub(crate) struct ReqwestHttpClient {
    client: Client,
}

impl ReqwestHttpClient {
    /// Wrap a configured `reqwest::Client` as the runtime's transport.
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Delete => reqwest::Method::DELETE,
        };

        let mut builder = self
            .client
            .request(method, &request.url)
            .query(&request.query);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &request.body {
            builder = builder
                .header("Content-Type", "application/json")
                .json(body);
        }

        let response = builder.send().await?;
        let status = response.status().as_u16();
        let body = read_response_body_tagged(response).await?;
        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
mod build_http_client_tests {
    use super::*;

    /// `build_http_client(None)` succeeds using the built-in default timeout
    #[test]
    fn test_default_timeout_builds_without_error() {
        build_http_client(None).expect("client should build with default timeout");
    }

    /// `build_http_client(Some(_))` accepts a caller-supplied timeout in seconds
    #[test]
    fn test_custom_timeout_builds_without_error() {
        build_http_client(Some(120)).expect("client should build with custom timeout");
    }
}

#[cfg(test)]
mod classify_tests {
    use super::is_text_content_type;

    /// All JSON-family content types (plain, charset suffix, structured `+json`) classify as text
    #[test]
    fn test_json_variants_are_text() {
        assert!(is_text_content_type("application/json"));
        assert!(is_text_content_type("application/json; charset=utf-8"));
        assert!(is_text_content_type("application/ld+json"));
        assert!(is_text_content_type("application/vnd.api+json"));
    }

    /// `text/*` content types classify as text regardless of subtype or charset
    #[test]
    fn test_text_mime_types_are_text() {
        assert!(is_text_content_type("text/plain"));
        assert!(is_text_content_type("text/csv"));
        assert!(is_text_content_type("text/html; charset=utf-8"));
    }

    /// Image, archive, and Office binary content types do not classify as text
    #[test]
    fn test_binary_mime_types_are_not_text() {
        assert!(!is_text_content_type("image/png"));
        assert!(!is_text_content_type("application/zip"));
        assert!(!is_text_content_type("application/octet-stream"));
        assert!(!is_text_content_type(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        ));
    }

    /// An empty content type does not classify as text — caller must fall back to binary
    #[test]
    fn test_missing_or_empty_content_type_is_not_text() {
        assert!(!is_text_content_type(""));
    }

    /// Content-type matching trims surrounding whitespace and ignores case
    #[test]
    fn test_whitespace_and_case_tolerant() {
        assert!(is_text_content_type("  APPLICATION/JSON  "));
        assert!(is_text_content_type("Text/CSV; charset=utf-8"));
    }

    /// `application/+json` (no subtype before `+json`) should not match —
    /// real `+json` content types always have a non-empty subtype prefix.
    #[test]
    fn test_plus_json_requires_non_empty_subtype_prefix() {
        assert!(!is_text_content_type("application/+json"));
    }
}

#[cfg(test)]
mod send_classification_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A 200 JSON response surfaces as `HttpBody::Text` carrying the raw body string
    #[tokio::test]
    async fn test_json_response_is_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ok"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(r#"{"ok":true}"#),
            )
            .mount(&server)
            .await;

        let client = ReqwestHttpClient::new(build_http_client(Some(5)).unwrap());
        let resp = client
            .send(HttpRequest {
                method: crate::protocol::catalogue::HttpMethod::Get,
                url: format!("{}/ok", server.uri()),
                headers: vec![],
                query: vec![],
                body: None,
            })
            .await
            .unwrap();
        match resp.body {
            HttpBody::Text(s) => assert_eq!(s, r#"{"ok":true}"#),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    /// A PNG response surfaces as `HttpBody::Binary` preserving the original bytes
    #[tokio::test]
    async fn test_png_response_is_binary() {
        let server = MockServer::start().await;
        let png_magic = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        Mock::given(method("GET"))
            .and(path("/img"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(png_magic.to_vec()),
            )
            .mount(&server)
            .await;

        let client = ReqwestHttpClient::new(build_http_client(Some(5)).unwrap());
        let resp = client
            .send(HttpRequest {
                method: crate::protocol::catalogue::HttpMethod::Get,
                url: format!("{}/img", server.uri()),
                headers: vec![],
                query: vec![],
                body: None,
            })
            .await
            .unwrap();
        match resp.body {
            HttpBody::Binary {
                content_type,
                bytes,
            } => {
                assert!(content_type.starts_with("image/png"));
                assert_eq!(bytes, png_magic);
            }
            other => panic!("expected Binary, got {other:?}"),
        }
    }

    /// 4xx responses are always treated as text so JSON error bodies remain readable
    #[tokio::test]
    async fn test_error_response_is_always_text_even_with_binary_content_type() {
        // Endpoints that normally return binary still return JSON errors on 4xx.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/err"))
            .respond_with(
                ResponseTemplate::new(404)
                    .insert_header("content-type", "image/png")
                    .set_body_string(r#"{"errorCode":404}"#),
            )
            .mount(&server)
            .await;

        let client = ReqwestHttpClient::new(build_http_client(Some(5)).unwrap());
        let resp = client
            .send(HttpRequest {
                method: crate::protocol::catalogue::HttpMethod::Get,
                url: format!("{}/err", server.uri()),
                headers: vec![],
                query: vec![],
                body: None,
            })
            .await
            .unwrap();
        assert_eq!(resp.status, 404);
        assert!(matches!(resp.body, HttpBody::Text(_)));
    }

    /// A text content type with non-UTF-8 bytes surfaces an `Upstream` error rather than panicking
    #[tokio::test]
    async fn test_text_content_type_with_invalid_utf8_returns_upstream_error() {
        let server = MockServer::start().await;
        // Valid invalid-UTF-8 sequence: 0xFF is never legal UTF-8.
        let bad_bytes: Vec<u8> = vec![0xFF, 0xFE, 0xFD];
        Mock::given(method("GET"))
            .and(path("/bad-utf8"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_bytes(bad_bytes),
            )
            .mount(&server)
            .await;

        let client = ReqwestHttpClient::new(build_http_client(Some(5)).unwrap());
        let err = client
            .send(HttpRequest {
                method: crate::protocol::catalogue::HttpMethod::Get,
                url: format!("{}/bad-utf8", server.uri()),
                headers: vec![],
                query: vec![],
                body: None,
            })
            .await
            .unwrap_err();

        // Must be Upstream (server-side issue), not Internal (CLI bug).
        match err.kind {
            crate::protocol::error::RuntimeErrorKind::Upstream { status, code } => {
                assert_eq!(status, 200);
                assert!(code.is_none());
            }
            other => panic!("expected Upstream, got {other:?}"),
        }
        assert!(
            err.message.contains("not valid UTF-8"),
            "message should mention UTF-8: {}",
            err.message
        );
    }
}
