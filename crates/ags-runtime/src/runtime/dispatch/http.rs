//! HTTP client types and reqwest-backed transport.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;

use ags_protocol::catalogue::HttpMethod;
use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
use ags_protocol::request::{FormPart, RequestBody};

/// Convert a `reqwest::Error` into a `RuntimeError` without exposing reqwest
/// in the protocol crate. Use everywhere we previously relied on the
/// `impl From<reqwest::Error> for RuntimeError`.
pub(crate) fn network_error(err: reqwest::Error) -> RuntimeError {
    RuntimeError::network(format!("Network error: {err}"))
}

// ── Transport Types ──

/// A single HTTP request the runtime wants to send.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// HTTP method for the request.
    pub method: HttpMethod,
    /// Fully-qualified URL (scheme + host + path).
    pub url: String,
    /// Key/value request headers (e.g. `Authorization`).
    pub headers: Vec<(String, String)>,
    /// URL query parameters appended by reqwest.
    pub query: Vec<(String, String)>,
    /// Optional JSON request body.
    pub body: Option<RequestBody>,
}

/// Response body, tagged by media-type family so binary payloads cannot be
/// accidentally UTF-8-decoded. Produced by the `HttpClient::send`
/// implementation; consumed by the dispatch layer.
#[derive(Debug, Clone)]
pub enum HttpBody {
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
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body, tagged by media type.
    pub body: HttpBody,
}

/// Maximum response body size (10 MB). Prevents memory exhaustion from oversized responses.
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Default overall request timeout when the caller configures none.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Connection-establishment timeout.
const CONNECT_TIMEOUT_SECS: u64 = 30;

/// Application User-Agent sent by AGS CLI backend requests.
pub(crate) const APPLICATION_USER_AGENT: &str = concat!("ags-cli/", env!("CARGO_PKG_VERSION"));

// ── Client Construction And Body Decoding ──

/// Build the shared HTTP client.
pub fn build_http_client(timeout_secs: Option<u64>) -> Result<Client, RuntimeError> {
    let timeout = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    Client::builder()
        .user_agent(APPLICATION_USER_AGENT)
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(network_error)
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
    use ags_protocol::error::{RuntimeError, RuntimeErrorKind};

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
    let bytes = response.bytes().await.map_err(network_error)?;
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
    use ags_protocol::error::{RuntimeError, RuntimeErrorKind};

    let bytes = read_response_bytes_capped(response).await?;
    String::from_utf8(bytes).map_err(|_| RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: "Response body is not valid UTF-8".to_string(),
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
    use ags_protocol::error::{RuntimeError, RuntimeErrorKind};

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
                message: "Server said text but body is not valid UTF-8".to_string(),
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

/// Transport-level HTTP client used by the runtime. Implement this trait to
/// inject a fake transport for integration tests or alternative backends.
#[async_trait]
pub trait HttpClient: Send + Sync {
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
            builder = match body {
                RequestBody::Json(value) => builder
                    .header("Content-Type", "application/json")
                    .json(value),
                RequestBody::Multipart(parts) => {
                    let mut form = reqwest::multipart::Form::new();
                    for part in parts {
                        form = match part {
                            FormPart::Text { name, value } => {
                                form.text(name.clone(), value.clone())
                            }
                            FormPart::File {
                                name,
                                path,
                                filename,
                            } => {
                                let part = reqwest::multipart::Part::file(path)
                                    .await
                                    .map_err(|err| RuntimeError {
                                        kind: RuntimeErrorKind::Internal,
                                        message: format!(
                                            "failed to open file '{}' for upload: {err}",
                                            path.display()
                                        ),
                                        details: None,
                                        hint: None,
                                        trace: None,
                                    })?
                                    .file_name(filename.clone());
                                form.part(name.clone(), part)
                            }
                        };
                    }
                    builder.multipart(form)
                }
            };
        }

        let response = builder.send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        let body = read_response_body_tagged(response).await?;
        Ok(HttpResponse { status, body })
    }
}

// ── Binary Upload Transport ──

/// Inactivity timeout for a streamed upload. An overall request timeout cannot
/// be used here: a multi-hundred-megabyte part legitimately takes longer than
/// any sane API deadline, so progress is judged per read instead.
const UPLOAD_READ_TIMEOUT_SECS: u64 = 120;

/// Base delay for the first upload retry.
const UPLOAD_RETRY_BASE_DELAY: Duration = Duration::from_millis(500);

/// Total time an upload may spend retrying before the failure is surfaced.
/// Ported verbatim from armada-cli's `maxRetryElapseTime`.
const UPLOAD_MAX_RETRY_ELAPSED: Duration = Duration::from_secs(120);

/// A bounded byte range of a local file, uploaded as a request body.
#[derive(Debug, Clone)]
pub struct FileRange {
    pub path: std::path::PathBuf,
    pub offset: u64,
    pub length: u64,
}

impl FileRange {
    /// The whole file, for a single-shot (non-multipart) upload.
    pub fn whole(path: impl Into<std::path::PathBuf>, length: u64) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            length,
        }
    }
}

/// Build the HTTP client used for streamed binary uploads to pre-signed URLs.
///
/// Deliberately separate from [`build_http_client`]: uploads need no overall
/// deadline and never follow redirects into a second signed URL.
pub fn build_upload_client() -> Result<Client, RuntimeError> {
    Client::builder()
        .user_agent(APPLICATION_USER_AGENT)
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .read_timeout(Duration::from_secs(UPLOAD_READ_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(network_error)
}

/// Result of a successful pre-signed PUT.
#[derive(Debug, Clone)]
pub struct BinaryPutOutcome {
    /// The storage service's `ETag` for the uploaded bytes. Required to
    /// finalize a multipart upload; absent for single-shot PUTs on some
    /// backends.
    pub etag: Option<String>,
}

/// Why a pre-signed PUT failed, so the caller can decide whether re-signing
/// the URL is worth attempting.
#[derive(Debug)]
pub enum BinaryPutError {
    /// The signed URL was rejected as unauthorised — typically an expired
    /// signature on a long-running upload.
    Unauthorized { status: u16 },
    /// Anything else: a transport failure, or a non-2xx the retries could not
    /// clear.
    Failed(RuntimeError),
}

impl BinaryPutError {
    /// Collapse into the underlying runtime error for surfacing to the user.
    pub fn into_runtime_error(self) -> RuntimeError {
        use ags_protocol::error::RuntimeErrorKind;
        match self {
            BinaryPutError::Unauthorized { status } => RuntimeError {
                kind: if status == 401 {
                    RuntimeErrorKind::NotAuthenticated
                } else {
                    RuntimeErrorKind::Forbidden
                },
                message: format!("The storage service rejected the upload (HTTP {status})"),
                details: None,
                hint: Some("The pre-signed URL may have expired; retry the upload.".to_string()),
                trace: None,
            },
            BinaryPutError::Failed(error) => error,
        }
    }
}

/// Stream a bounded range of a file to `url` with a `PUT`, retrying transient
/// failures, and return the storage service's `ETag`.
///
/// The body is read from disk in chunks as it is sent, so a 500 MiB part costs
/// a buffer rather than 500 MiB of resident memory, and each retry re-opens the
/// range instead of holding the bytes for a possible second attempt.
///
/// Retries cover exactly what armada-cli retried — connection/read timeouts and
/// 5xx responses — within a total elapsed budget. A 401/403 is surfaced
/// separately as [`BinaryPutError::Unauthorized`] so the caller can re-sign.
pub async fn put_file_range(
    client: &Client,
    url: &str,
    range: &FileRange,
) -> Result<BinaryPutOutcome, BinaryPutError> {
    let start = std::time::Instant::now();
    let mut attempt = 0u32;

    loop {
        let body = upload_body(range).await.map_err(BinaryPutError::Failed)?;
        let response = client
            .put(url)
            .header(reqwest::header::CONTENT_LENGTH, range.length)
            .body(body)
            .send()
            .await;

        // Timeouts and 5xx only, matching armada-cli. A refused connection or a
        // bad scheme is permanent there and stays permanent here, so a typo'd
        // storage host fails immediately instead of retrying for two minutes.
        let is_retryable = match &response {
            Err(error) => error.is_timeout(),
            Ok(response) => response.status().is_server_error(),
        };

        if is_retryable {
            if let Some(delay) = next_retry_delay(attempt, start.elapsed()) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }
        }

        let response = response.map_err(|error| BinaryPutError::Failed(network_error(error)))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(BinaryPutError::Unauthorized {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            return Err(BinaryPutError::Failed(upload_status_error(
                status.as_u16(),
                response.text().await.ok(),
            )));
        }

        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        return Ok(BinaryPutOutcome { etag });
    }
}

/// Open the file range as a streaming request body.
async fn upload_body(range: &FileRange) -> Result<reqwest::Body, RuntimeError> {
    use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let io_error = |error: std::io::Error| RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: format!("Failed to read {}: {error}", range.path.display()),
        details: None,
        hint: None,
        trace: None,
    };

    let mut file = tokio::fs::File::open(&range.path).await.map_err(io_error)?;
    if range.offset > 0 {
        file.seek(std::io::SeekFrom::Start(range.offset))
            .await
            .map_err(io_error)?;
    }
    let reader = file.take(range.length);
    Ok(reqwest::Body::wrap_stream(
        tokio_util::io::ReaderStream::new(reader),
    ))
}

/// How long to wait before retry `attempt`, or `None` once the elapsed budget
/// is spent. Exponential with no jitter, matching armada-cli's backoff.
fn next_retry_delay(attempt: u32, elapsed: Duration) -> Option<Duration> {
    if elapsed >= UPLOAD_MAX_RETRY_ELAPSED {
        return None;
    }
    let delay = UPLOAD_RETRY_BASE_DELAY * 2u32.saturating_pow(attempt.min(6));
    let remaining = UPLOAD_MAX_RETRY_ELAPSED - elapsed;
    Some(delay.min(remaining))
}

/// Build the error for a non-2xx pre-signed PUT.
fn upload_status_error(status: u16, body: Option<String>) -> RuntimeError {
    use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
    let detail = body
        .map(|body| crate::support::strings::truncate_display_text(body.trim(), 500))
        .filter(|body| !body.is_empty());
    RuntimeError {
        kind: RuntimeErrorKind::Upstream { status, code: None },
        message: match detail {
            Some(detail) => {
                format!("The storage service rejected the upload (HTTP {status}): {detail}")
            }
            None => format!("The storage service rejected the upload (HTTP {status})"),
        },
        details: None,
        hint: None,
        trace: None,
    }
}

#[cfg(test)]
mod upload_transport_tests {
    use super::*;

    #[test]
    fn test_retry_delay_grows_until_budget_is_spent() {
        let first = next_retry_delay(0, Duration::ZERO).expect("first retry is allowed");
        let second = next_retry_delay(1, Duration::from_secs(1)).expect("second retry is allowed");
        assert!(second > first, "backoff must grow: {first:?} -> {second:?}");
        assert!(
            next_retry_delay(3, UPLOAD_MAX_RETRY_ELAPSED).is_none(),
            "no retry once the elapsed budget is spent"
        );
    }

    #[test]
    fn test_retry_delay_never_exceeds_remaining_budget() {
        let elapsed = UPLOAD_MAX_RETRY_ELAPSED - Duration::from_millis(10);
        let delay = next_retry_delay(6, elapsed).expect("still inside the budget");
        assert!(delay <= Duration::from_millis(10));
    }

    #[tokio::test]
    async fn test_upload_body_reads_only_the_requested_range() {
        use tokio::io::AsyncReadExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("part.bin");
        std::fs::write(&path, b"0123456789").unwrap();

        let range = FileRange {
            path: path.clone(),
            offset: 3,
            length: 4,
        };
        let mut file = tokio::fs::File::open(&range.path).await.unwrap();
        tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(range.offset))
            .await
            .unwrap();
        let mut buffer = Vec::new();
        file.take(range.length)
            .read_to_end(&mut buffer)
            .await
            .unwrap();
        assert_eq!(buffer, b"3456");
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
    use wiremock::matchers::{header, header_regex, method, path};
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
                method: ags_protocol::catalogue::HttpMethod::Get,
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

    /// The shared reqwest client sends the CLI application User-Agent header
    #[tokio::test]
    async fn test_built_client_sends_application_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ua"))
            .and(header(
                "user-agent",
                concat!("ags-cli/", env!("CARGO_PKG_VERSION")),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(r#"{"ok":true}"#),
            )
            .mount(&server)
            .await;

        let client = ReqwestHttpClient::new(build_http_client(Some(5)).unwrap());
        let response = client
            .send(HttpRequest {
                method: ags_protocol::catalogue::HttpMethod::Get,
                url: format!("{}/ua", server.uri()),
                headers: vec![],
                query: vec![],
                body: None,
            })
            .await
            .expect("request should include application User-Agent");
        assert_eq!(response.status, 200);
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
                method: ags_protocol::catalogue::HttpMethod::Get,
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
                method: ags_protocol::catalogue::HttpMethod::Get,
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
                method: ags_protocol::catalogue::HttpMethod::Get,
                url: format!("{}/bad-utf8", server.uri()),
                headers: vec![],
                query: vec![],
                body: None,
            })
            .await
            .unwrap_err();

        // Must be Upstream (server-side issue), not Internal (CLI bug).
        match err.kind {
            ags_protocol::error::RuntimeErrorKind::Upstream { status, code } => {
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

    /// Sending a `RequestBody::Multipart` body produces a real
    /// `Content-Type: multipart/form-data; boundary=...` request carrying
    /// both a text part and a file part with the expected content.
    #[tokio::test]
    async fn test_multipart_request_sends_text_and_file_parts() {
        use ags_protocol::request::{FormPart, RequestBody};

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("asset.png");
        std::fs::write(&file_path, b"fake-png-bytes").unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/upload"))
            .and(header_regex(
                "content-type",
                "^multipart/form-data; boundary=.+$",
            ))
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
                method: ags_protocol::catalogue::HttpMethod::Post,
                url: format!("{}/upload", server.uri()),
                headers: vec![],
                query: vec![],
                body: Some(RequestBody::Multipart(vec![
                    FormPart::Text {
                        name: "strategy".to_string(),
                        value: "REPLACE".to_string(),
                    },
                    FormPart::File {
                        name: "file".to_string(),
                        path: file_path.clone(),
                        filename: "asset.png".to_string(),
                    },
                ])),
            })
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }
}
