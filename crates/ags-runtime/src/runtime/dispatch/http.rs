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

/// Default request timeout in seconds when --timeout is not given.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Connection-establishment timeout.
pub const CONNECT_TIMEOUT_SECS: u64 = 30;

/// Application User-Agent sent by AGS CLI backend requests.
pub const APPLICATION_USER_AGENT: &str = concat!("ags-cli/", env!("CARGO_PKG_VERSION"));

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

/// How long an upload attempt may go without data moving before it is given up.
///
/// The upload client has no request-wide deadline, because a
/// multi-hundred-megabyte part legitimately takes longer than any sane API
/// deadline. Instead `send_watched` gives up on an attempt when the transport
/// has taken no request-body bytes for this long, or, once it has taken the
/// whole body, when no response has arrived for this long. Reading the body of
/// an error response is limited to this long as well.
const UPLOAD_STALL_LIMIT: Duration = Duration::from_secs(120);

// The stall clock starts before the connection is made (`last_taken` is set
// before the send), so the connect timeout must stay below the stall limit for
// the stall error to stay true: a connection that is never made has to end with
// the connect timeout's error, not with "no data moved".
const _: () = assert!(CONNECT_TIMEOUT_SECS < UPLOAD_STALL_LIMIT.as_secs());

/// Base delay for the first upload retry.
const UPLOAD_RETRY_BASE_DELAY: Duration = Duration::from_millis(500);

/// Total time an upload may spend retrying before the failure is surfaced,
/// counted from the first retryable failure of a `put_file_range` call. The
/// value is armada-cli's `maxRetryElapseTime`.
///
/// The budget is wall-clock time from that first failure and includes the time
/// a retried transfer itself takes, so a retried part whose own transfer
/// outlasts the budget is not retried again if it fails too. After a failure
/// late in a long transfer there is in practice one retry, and this is
/// intended.
const UPLOAD_MAX_RETRY_ELAPSED: Duration = Duration::from_secs(120);

/// The time limits one `put_file_range` call works within. Production uses
/// [`UploadLimits::PRODUCTION`]; tests inject shorter limits.
#[derive(Debug, Clone, Copy)]
struct UploadLimits {
    /// How long an attempt may go without data moving. See [`UPLOAD_STALL_LIMIT`].
    stall: Duration,
    /// How long retries may continue after the call's first retryable failure.
    retry_budget: Duration,
}

impl UploadLimits {
    const PRODUCTION: Self = Self {
        stall: UPLOAD_STALL_LIMIT,
        retry_budget: UPLOAD_MAX_RETRY_ELAPSED,
    };
}

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
/// Deliberately separate from [`build_http_client`]: uploads never follow
/// redirects into a second signed URL, and they set no request-wide timeout.
/// reqwest starts its `timeout` and `read_timeout` clocks when the request
/// starts and does not reset them while the body is sent, so either one would
/// end a large upload that is still moving data. `put_file_range` limits each
/// wait on the network itself instead.
pub fn build_upload_client() -> Result<Client, RuntimeError> {
    Client::builder()
        .user_agent(APPLICATION_USER_AGENT)
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
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
    ///
    /// Both AMS upload paths (`upload_whole` and `upload_part` in
    /// `ams_upload::pipeline`) re-sign once after `Unauthorized` and send the
    /// bytes again, so an `Unauthorized` reaches the user only when the fresh
    /// URL is rejected too. Its hint says that rather than blaming an expired
    /// signature.
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
                hint: Some(
                    "The storage service also rejected a second pre-signed URL. Run the upload again."
                        .to_string(),
                ),
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
/// An attempt runs for as long as data keeps moving and is given up after
/// [`UPLOAD_STALL_LIMIT`] without it. Retries cover what armada-cli retried,
/// timeouts and 5xx responses, with a stall counted as a timeout, for
/// [`UPLOAD_MAX_RETRY_ELAPSED`] after the first retryable failure. A 401/403 is
/// surfaced separately as [`BinaryPutError::Unauthorized`] so the caller can
/// re-sign.
pub async fn put_file_range(
    client: &Client,
    url: &str,
    range: &FileRange,
) -> Result<BinaryPutOutcome, BinaryPutError> {
    put_file_range_with_limits(client, url, range, UploadLimits::PRODUCTION).await
}

/// [`put_file_range`] with its time limits passed in.
async fn put_file_range_with_limits(
    client: &Client,
    url: &str,
    range: &FileRange,
    limits: UploadLimits,
) -> Result<BinaryPutOutcome, BinaryPutError> {
    let mut first_failure: Option<tokio::time::Instant> = None;
    let mut attempt = 0u32;

    loop {
        let (progress, watcher) = tokio::sync::watch::channel(BodyProgress {
            last_taken: tokio::time::Instant::now(),
            finished: range.length == 0,
        });
        let body = upload_body(range, progress)
            .await
            .map_err(BinaryPutError::Failed)?;
        let request = client
            .put(url)
            .header(reqwest::header::CONTENT_LENGTH, range.length)
            .body(body);
        let response = send_watched(request, watcher, limits.stall).await;

        // Stalls, timeouts and 5xx only, matching armada-cli. A refused
        // connection or a bad scheme is permanent there and stays permanent
        // here, so a typo'd storage host fails immediately instead of retrying
        // for two minutes.
        let is_retryable = match &response {
            Err(AttemptFailure::Stalled) => true,
            Err(AttemptFailure::Transport(error)) => error.is_timeout(),
            Ok(response) => response.status().is_server_error(),
        };

        if is_retryable {
            // The budget starts at the first failure rather than the first
            // attempt, so a failure at the end of a long transfer is retried.
            let failed_at = *first_failure.get_or_insert_with(tokio::time::Instant::now);
            if let Some(delay) = next_retry_delay(attempt, failed_at.elapsed(), limits.retry_budget)
            {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }
        }

        let response = match response {
            Ok(response) => response,
            Err(AttemptFailure::Stalled) => {
                return Err(BinaryPutError::Failed(upload_stall_error(limits.stall)))
            }
            Err(AttemptFailure::Transport(error)) => {
                return Err(BinaryPutError::Failed(network_error(error)))
            }
        };
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(BinaryPutError::Unauthorized {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            // The body only adds detail to the status, so a server that stops
            // sending it costs at most the stall limit and the status is still
            // reported.
            let body = tokio::time::timeout(limits.stall, response.text())
                .await
                .ok()
                .and_then(Result::ok);
            return Err(BinaryPutError::Failed(upload_status_error(
                status.as_u16(),
                body,
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

/// How far the transport has got with one attempt's request body.
#[derive(Debug, Clone, Copy)]
struct BodyProgress {
    /// When the transport last took body bytes, or when the attempt started.
    last_taken: tokio::time::Instant,
    /// Whether the transport has taken the whole body.
    finished: bool,
}

/// Why an upload attempt ended without a response.
#[derive(Debug)]
enum AttemptFailure {
    /// No data moved for the stall limit.
    Stalled,
    /// The transport reported an error.
    Transport(reqwest::Error),
}

/// Send `request` and wait for the response head, giving up with
/// [`AttemptFailure::Stalled`] when the transport takes no body bytes for
/// `stall`, or when no response arrives within `stall` of the transport taking
/// the whole body. `progress` is fed by the [`ProgressReader`] inside the body.
async fn send_watched(
    request: reqwest::RequestBuilder,
    mut progress: tokio::sync::watch::Receiver<BodyProgress>,
    stall: Duration,
) -> Result<reqwest::Response, AttemptFailure> {
    let response = request.send();
    tokio::pin!(response);

    // While the body is being sent, the clock restarts every time the
    // transport takes bytes.
    loop {
        let BodyProgress {
            last_taken,
            finished,
        } = *progress.borrow_and_update();
        if finished {
            break;
        }
        tokio::select! {
            result = &mut response => return result.map_err(AttemptFailure::Transport),
            // Wakes when the whole body is taken, or when the transport drops
            // the body without taking all of it; the send then fails on its
            // own, within the response wait below.
            changed = progress.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            () = tokio::time::sleep_until(last_taken + stall) => {
                if progress.borrow().last_taken + stall <= tokio::time::Instant::now() {
                    return Err(AttemptFailure::Stalled);
                }
            }
        }
    }

    // Once the whole body is taken, the response must arrive within `stall`.
    // `finished` means the transport has taken the last bytes, not that they
    // have left the machine: bytes still in the HTTP client's write buffer and
    // the operating system's send buffer are sent inside this wait. On a link
    // slow enough that those bytes take longer than `stall` to send, the
    // attempt ends as stalled while data is still moving.
    match tokio::time::timeout(stall, response).await {
        Ok(result) => result.map_err(AttemptFailure::Transport),
        Err(_) => Err(AttemptFailure::Stalled),
    }
}

/// A request-body reader that records in `progress` each time the transport
/// takes bytes. `ReaderStream` reads from it only when the transport asks for
/// the next chunk, so a read is the moment the transport takes those bytes.
struct ProgressReader {
    inner: tokio::io::Take<tokio::fs::File>,
    progress: tokio::sync::watch::Sender<BodyProgress>,
}

impl tokio::io::AsyncRead for ProgressReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = std::pin::Pin::new(&mut this.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            let finished = this.inner.limit() == 0;
            // Only the end of the body wakes the watcher; every other read
            // just moves the time it checks when its own timer fires.
            this.progress.send_if_modified(|progress| {
                progress.last_taken = tokio::time::Instant::now();
                progress.finished = finished;
                finished
            });
        }
        result
    }
}

/// Open the file range as a streaming request body that reports to
/// `progress` each time the transport takes bytes.
async fn upload_body(
    range: &FileRange,
    progress: tokio::sync::watch::Sender<BodyProgress>,
) -> Result<reqwest::Body, RuntimeError> {
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
    let reader = ProgressReader {
        inner: file.take(range.length),
        progress,
    };
    Ok(reqwest::Body::wrap_stream(
        tokio_util::io::ReaderStream::new(reader),
    ))
}

/// How long to wait before retry `attempt`, or `None` once `elapsed` has
/// reached `budget`. Exponential with no jitter, matching armada-cli's backoff,
/// and never longer than what is left of `budget`.
fn next_retry_delay(attempt: u32, elapsed: Duration, budget: Duration) -> Option<Duration> {
    if elapsed >= budget {
        return None;
    }
    let delay = UPLOAD_RETRY_BASE_DELAY * 2u32.saturating_pow(attempt.min(6));
    let remaining = budget - elapsed;
    Some(delay.min(remaining))
}

/// Build the error for an upload attempt given up because no data moved for
/// `stall`.
pub(crate) fn upload_stall_error(stall: Duration) -> RuntimeError {
    let seconds = stall.as_secs();
    RuntimeError {
        kind: RuntimeErrorKind::Network,
        message: format!(
            "The upload stopped because no data moved for {seconds} {}.",
            crate::support::strings::pluralize("second", seconds as usize)
        ),
        details: None,
        hint: Some("Check your network connection, then run the upload again.".to_string()),
        trace: None,
    }
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::time::Instant;

    #[test]
    fn test_retry_delay_grows_until_budget_is_spent() {
        let first = next_retry_delay(0, Duration::ZERO, UPLOAD_MAX_RETRY_ELAPSED)
            .expect("first retry is allowed");
        let second = next_retry_delay(1, Duration::from_secs(1), UPLOAD_MAX_RETRY_ELAPSED)
            .expect("second retry is allowed");
        assert!(second > first, "backoff must grow: {first:?} -> {second:?}");
        assert!(
            next_retry_delay(3, UPLOAD_MAX_RETRY_ELAPSED, UPLOAD_MAX_RETRY_ELAPSED).is_none(),
            "no retry once the elapsed budget is spent"
        );
    }

    #[test]
    fn test_retry_delay_never_exceeds_remaining_budget() {
        let elapsed = UPLOAD_MAX_RETRY_ELAPSED - Duration::from_millis(10);
        let delay = next_retry_delay(6, elapsed, UPLOAD_MAX_RETRY_ELAPSED)
            .expect("still inside the budget");
        assert!(delay <= Duration::from_millis(10));
    }

    /// The production limits are fixed product values: an attempt is given up
    /// after 120 seconds without data moving, and a failed upload is retried
    /// for 120 seconds from its first retryable failure. A change to either is
    /// a product decision, so it has to change this test too.
    #[test]
    fn test_production_upload_limits_are_120_seconds() {
        assert_eq!(UploadLimits::PRODUCTION.stall, Duration::from_secs(120));
        assert_eq!(
            UploadLimits::PRODUCTION.retry_budget,
            Duration::from_secs(120)
        );
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

    // ── Stall and retry-budget tests against a local storage server ──
    //
    // These tests run in real time against a local TCP server, and are written
    // so that the machine's speed does not decide whether they pass. Their
    // elapsed-time checks (`started.elapsed() >= ...`) are lower bounds set by
    // the server's pacing, which a slow machine only makes easier to pass. A
    // steady server pauses at most MAX_SERVER_PAUSE between reads, as
    // `read_body` asserts, while every stall limit in these tests is at least
    // 1 s, so a transfer that is moving has a wide margin before it can look
    // stalled; keep new stall limits at 1 s or more. The one upper bound is
    // TEST_DEADLINE, which `put_within_deadline` puts on every upload so that a
    // wait with no limit fails the test instead of hanging the run; it is set
    // well above the longest paced transfer here.

    const MIB: u64 = 1024 * 1024;

    /// Upper bound on every transport test, so a wait with no limit fails the
    /// test instead of hanging the run.
    const TEST_DEADLINE: Duration = Duration::from_secs(30);

    /// The most the scripted server reads at once.
    const SERVER_READ_BYTES: usize = 64 * 1024;

    /// The longest pause a steady server may take between reads.
    const MAX_SERVER_PAUSE: Duration = Duration::from_millis(50);

    /// What the scripted storage server does with one request. Scripts are
    /// played in request order, across connections.
    #[derive(Debug, Clone, Copy)]
    enum Script {
        /// Read the whole body, no faster than `bytes_per_second` when given,
        /// then answer `status` with the ETag `"request-<n>"`, where n counts
        /// requests from 1.
        Answer {
            status: u16,
            bytes_per_second: Option<u64>,
        },
        /// Read the whole body, then keep the connection open without answering.
        ReadThenHang,
        /// Read this many body bytes, then stop reading and keep the connection
        /// open without answering.
        StopReadingAfter(u64),
        /// Read this many body bytes, answer `status`, then stop reading and
        /// keep the connection open.
        AnswerEarly { after: u64, status: u16 },
        /// Read this many body bytes, then close the connection.
        CloseAfter(u64),
        /// Read the whole body, answer `status` with a `Content-Length` of 64,
        /// then keep the connection open without sending that body.
        AnswerWithoutPromisedBody(u16),
    }

    /// What the scripted storage server has seen.
    #[derive(Default)]
    struct StorageCounts {
        connections: AtomicUsize,
        requests: AtomicUsize,
        bodies_read: AtomicUsize,
    }

    /// A local HTTP/1.1 storage server that plays one [`Script`] per request.
    struct ScriptedStorage {
        url: String,
        counts: Arc<StorageCounts>,
    }

    impl ScriptedStorage {
        async fn start(scripts: Vec<Script>) -> Self {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            // A small receive buffer, inherited by accepted sockets, keeps few
            // bytes in flight between client and server, so the client stops
            // taking body bytes soon after the server stops reading.
            socket
                .set_recv_buffer_size(SERVER_READ_BYTES as u32)
                .unwrap();
            socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let listener = socket.listen(16).unwrap();
            let url = format!("http://{}/storage", listener.local_addr().unwrap());
            let counts = Arc::new(StorageCounts::default());
            let scripts = Arc::new(scripts);
            let server_counts = Arc::clone(&counts);
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    server_counts.connections.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(serve_connection(
                        stream,
                        Arc::clone(&scripts),
                        Arc::clone(&server_counts),
                    ));
                }
            });
            Self { url, counts }
        }

        fn connections(&self) -> usize {
            self.counts.connections.load(Ordering::SeqCst)
        }

        fn requests(&self) -> usize {
            self.counts.requests.load(Ordering::SeqCst)
        }

        fn bodies_read(&self) -> usize {
            self.counts.bodies_read.load(Ordering::SeqCst)
        }
    }

    async fn serve_connection(
        mut stream: tokio::net::TcpStream,
        scripts: Arc<Vec<Script>>,
        counts: Arc<StorageCounts>,
    ) {
        use tokio::io::AsyncWriteExt;

        let mut buffer = vec![0u8; SERVER_READ_BYTES];
        while let Some((content_length, already)) =
            read_request_head(&mut stream, &mut buffer).await
        {
            let number = counts.requests.fetch_add(1, Ordering::SeqCst) + 1;
            let Some(script) = scripts.get(number - 1).copied() else {
                return;
            };
            match script {
                Script::Answer {
                    status,
                    bytes_per_second,
                } => {
                    let read = read_body(
                        &mut stream,
                        &mut buffer,
                        already,
                        content_length,
                        bytes_per_second,
                    );
                    if !read.await {
                        return;
                    }
                    counts.bodies_read.fetch_add(1, Ordering::SeqCst);
                    let response = format!(
                        "HTTP/1.1 {status} Scripted\r\nETag: \"request-{number}\"\r\nContent-Length: 0\r\n\r\n"
                    );
                    if stream.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                }
                Script::ReadThenHang => {
                    if read_body(&mut stream, &mut buffer, already, content_length, None).await {
                        counts.bodies_read.fetch_add(1, Ordering::SeqCst);
                    }
                    std::future::pending::<()>().await;
                }
                Script::StopReadingAfter(bytes) => {
                    let target = bytes.min(content_length);
                    read_body(&mut stream, &mut buffer, already, target, None).await;
                    std::future::pending::<()>().await;
                }
                Script::AnswerEarly { after, status } => {
                    let target = after.min(content_length);
                    if read_body(&mut stream, &mut buffer, already, target, None).await {
                        let response =
                            format!("HTTP/1.1 {status} Scripted\r\nContent-Length: 0\r\n\r\n");
                        let _ = stream.write_all(response.as_bytes()).await;
                    }
                    std::future::pending::<()>().await;
                }
                Script::CloseAfter(bytes) => {
                    let target = bytes.min(content_length);
                    read_body(&mut stream, &mut buffer, already, target, None).await;
                    return;
                }
                Script::AnswerWithoutPromisedBody(status) => {
                    if read_body(&mut stream, &mut buffer, already, content_length, None).await {
                        counts.bodies_read.fetch_add(1, Ordering::SeqCst);
                        let response =
                            format!("HTTP/1.1 {status} Scripted\r\nContent-Length: 64\r\n\r\n");
                        let _ = stream.write_all(response.as_bytes()).await;
                    }
                    std::future::pending::<()>().await;
                }
            }
        }
    }

    /// Read one request head. Returns its `Content-Length` and the number of
    /// body bytes that arrived with it, or `None` once the client closes the
    /// connection.
    async fn read_request_head(
        stream: &mut tokio::net::TcpStream,
        buffer: &mut [u8],
    ) -> Option<(u64, u64)> {
        use tokio::io::AsyncReadExt;

        let mut head = Vec::new();
        loop {
            if let Some(end) = head.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let text = String::from_utf8_lossy(&head[..end]).to_ascii_lowercase();
                let content_length = text
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(0);
                return Some((content_length, (head.len() - end - 4) as u64));
            }
            let read = stream.read(buffer).await.ok().filter(|&read| read > 0)?;
            head.extend_from_slice(&buffer[..read]);
        }
    }

    /// Read body bytes until `target` have arrived, counting `already` that
    /// came with the head. With `bytes_per_second`, pause after each read
    /// until the bytes read so far are due at that rate; such a pause is at
    /// most one read's worth of time. Returns false if the client closed the
    /// connection first.
    async fn read_body(
        stream: &mut tokio::net::TcpStream,
        buffer: &mut [u8],
        already: u64,
        target: u64,
        bytes_per_second: Option<u64>,
    ) -> bool {
        use tokio::io::AsyncReadExt;

        if let Some(rate) = bytes_per_second {
            let longest_pause = Duration::from_secs_f64(buffer.len() as f64 / rate as f64);
            assert!(
                longest_pause <= MAX_SERVER_PAUSE,
                "{rate} bytes/s pauses up to {longest_pause:?} between reads"
            );
        }
        let started = Instant::now();
        let mut paced = 0u64;
        let mut total = already;
        while total < target {
            let want = buffer
                .len()
                .min(usize::try_from(target - total).unwrap_or(usize::MAX));
            let read = match stream.read(&mut buffer[..want]).await {
                Ok(read) if read > 0 => read as u64,
                _ => return false,
            };
            total += read;
            if let Some(rate) = bytes_per_second {
                paced += read;
                let due = Duration::from_secs_f64(paced as f64 / rate as f64);
                tokio::time::sleep_until(started + due).await;
            }
        }
        true
    }

    /// A file of `length` zero bytes in `dir`, as one upload range.
    fn upload_file(dir: &tempfile::TempDir, length: u64) -> FileRange {
        let path = dir.path().join("archive.bin");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(length)
            .unwrap();
        FileRange::whole(path, length)
    }

    /// Upload `range` to `storage` with the production upload client, failing
    /// the test if the call has not returned within `TEST_DEADLINE`.
    async fn put_within_deadline(
        storage: &ScriptedStorage,
        range: &FileRange,
        limits: UploadLimits,
    ) -> Result<BinaryPutOutcome, BinaryPutError> {
        let client = build_upload_client().expect("the upload client builds");
        let put = put_file_range_with_limits(&client, &storage.url, range, limits);
        tokio::time::timeout(TEST_DEADLINE, put)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "put_file_range did not return within {TEST_DEADLINE:?}: a wait on the network has no limit"
                )
            })
    }

    /// Assert that `result` is the stall error for a one-second stall limit.
    fn assert_one_second_stall(result: Result<BinaryPutOutcome, BinaryPutError>) {
        let error = match result {
            Err(BinaryPutError::Failed(error)) => error,
            other => panic!("expected the stall error, got {other:?}"),
        };
        assert_eq!(error.kind, RuntimeErrorKind::Network);
        assert_eq!(
            error.message,
            "The upload stopped because no data moved for 1 second."
        );
    }

    /// A transfer that keeps moving is never cut off, however long it takes:
    /// the server reads steadily for more than three stall limits and the
    /// upload succeeds.
    #[tokio::test]
    async fn test_put_file_range_succeeds_when_transfer_outlasts_the_stall_limit() {
        let stall = Duration::from_secs(2);
        // 52 MiB read at 8 MiB/s takes at least 6.4 s, over three stall limits.
        let storage = ScriptedStorage::start(vec![Script::Answer {
            status: 200,
            bytes_per_second: Some(8 * MIB),
        }])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let range = upload_file(&dir, 52 * MIB);
        let limits = UploadLimits {
            stall,
            retry_budget: Duration::ZERO,
        };

        let started = Instant::now();
        let outcome = put_within_deadline(&storage, &range, limits)
            .await
            .expect("a transfer that keeps moving must succeed");

        assert!(
            started.elapsed() >= stall * 3,
            "the transfer took {:?}, less than three stall limits",
            started.elapsed()
        );
        assert_eq!(outcome.etag.as_deref(), Some("\"request-1\""));
        assert_eq!(storage.requests(), 1);
    }

    /// When the server stops reading, the client stops taking body bytes and
    /// the attempt fails with the stall error.
    #[tokio::test]
    async fn test_put_file_range_fails_when_no_body_bytes_move_for_the_stall_limit() {
        let storage = ScriptedStorage::start(vec![Script::StopReadingAfter(MIB)]).await;
        let dir = tempfile::tempdir().unwrap();
        // Far more than the socket buffers between client and server hold, so
        // the client cannot take the whole body while the server is not reading.
        let range = upload_file(&dir, 64 * MIB);
        let limits = UploadLimits {
            stall: Duration::from_secs(1),
            retry_budget: Duration::ZERO,
        };

        let result = put_within_deadline(&storage, &range, limits).await;

        assert_one_second_stall(result);
        assert_eq!(storage.requests(), 1);
        assert_eq!(storage.bodies_read(), 0);
    }

    /// When the server takes the whole body but never answers, the attempt
    /// fails with the stall error.
    #[tokio::test]
    async fn test_put_file_range_fails_when_the_response_never_arrives() {
        let storage = ScriptedStorage::start(vec![Script::ReadThenHang]).await;
        let dir = tempfile::tempdir().unwrap();
        let range = upload_file(&dir, MIB);
        let limits = UploadLimits {
            stall: Duration::from_secs(1),
            retry_budget: Duration::ZERO,
        };

        let result = put_within_deadline(&storage, &range, limits).await;

        assert_one_second_stall(result);
        assert_eq!(storage.requests(), 1);
        assert_eq!(storage.bodies_read(), 1);
    }

    /// A 5xx that arrives after a transfer longer than the whole retry budget
    /// is still retried, because the budget starts at the first failure.
    #[tokio::test]
    async fn test_put_file_range_retries_a_server_error_after_a_transfer_longer_than_the_retry_budget(
    ) {
        let retry_budget = Duration::from_secs(1);
        // 10 MiB read at 4 MiB/s takes at least 2.4 s, over twice the budget.
        let storage = ScriptedStorage::start(vec![
            Script::Answer {
                status: 503,
                bytes_per_second: Some(4 * MIB),
            },
            Script::Answer {
                status: 200,
                bytes_per_second: None,
            },
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let range = upload_file(&dir, 10 * MIB);
        let limits = UploadLimits {
            stall: Duration::from_secs(10),
            retry_budget,
        };

        let started = Instant::now();
        let outcome = put_within_deadline(&storage, &range, limits)
            .await
            .expect("the 503 must be retried");

        assert!(
            started.elapsed() >= retry_budget * 2,
            "the upload took {:?}, so the first transfer did not outlast the retry budget",
            started.elapsed()
        );
        assert_eq!(outcome.etag.as_deref(), Some("\"request-2\""));
        assert_eq!(storage.requests(), 2);
    }

    /// A stalled attempt is retried on a new connection, and the upload
    /// succeeds when that one moves.
    #[tokio::test]
    async fn test_put_file_range_retries_after_a_stall() {
        let storage = ScriptedStorage::start(vec![
            Script::StopReadingAfter(MIB),
            Script::Answer {
                status: 200,
                bytes_per_second: None,
            },
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let range = upload_file(&dir, 64 * MIB);
        let limits = UploadLimits {
            stall: Duration::from_secs(1),
            retry_budget: Duration::from_secs(10),
        };

        let outcome = put_within_deadline(&storage, &range, limits)
            .await
            .expect("the stalled attempt must be retried");

        assert_eq!(outcome.etag.as_deref(), Some("\"request-2\""));
        assert_eq!(storage.connections(), 2);
        assert_eq!(storage.requests(), 2);
    }

    /// An error response whose body never arrives still reports its status,
    /// after at most the stall limit, instead of waiting for the body forever.
    #[tokio::test]
    async fn test_put_file_range_reports_the_status_when_the_error_body_never_arrives() {
        let storage = ScriptedStorage::start(vec![Script::AnswerWithoutPromisedBody(400)]).await;
        let dir = tempfile::tempdir().unwrap();
        let range = upload_file(&dir, MIB);
        let limits = UploadLimits {
            stall: Duration::from_secs(1),
            retry_budget: Duration::ZERO,
        };

        let result = put_within_deadline(&storage, &range, limits).await;

        let error = match result {
            Err(BinaryPutError::Failed(error)) => error,
            other => panic!("expected the HTTP 400 error, got {other:?}"),
        };
        assert_eq!(
            error.kind,
            RuntimeErrorKind::Upstream {
                status: 400,
                code: None
            }
        );
        assert_eq!(
            error.message,
            "The storage service rejected the upload (HTTP 400)"
        );
    }

    /// A response that arrives while the body is still being sent ends the
    /// attempt: an early 403 reaches the caller as `Unauthorized`, so it can
    /// re-sign, instead of waiting out a stall.
    #[tokio::test]
    async fn test_put_file_range_returns_a_response_that_arrives_before_the_body_is_sent() {
        let storage = ScriptedStorage::start(vec![Script::AnswerEarly {
            after: MIB,
            status: 403,
        }])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let range = upload_file(&dir, 64 * MIB);
        let limits = UploadLimits {
            stall: Duration::from_secs(2),
            retry_budget: Duration::ZERO,
        };

        let result = put_within_deadline(&storage, &range, limits).await;

        assert!(
            matches!(result, Err(BinaryPutError::Unauthorized { status: 403 })),
            "expected Unauthorized {{ status: 403 }}, got {result:?}"
        );
        assert_eq!(storage.requests(), 1);
    }

    /// A connection the server closes part way through the body is a
    /// permanent failure, as a refused connection is, and is not retried.
    #[tokio::test]
    async fn test_put_file_range_does_not_retry_a_connection_closed_mid_body() {
        let storage = ScriptedStorage::start(vec![Script::CloseAfter(MIB)]).await;
        let dir = tempfile::tempdir().unwrap();
        let range = upload_file(&dir, 64 * MIB);
        let limits = UploadLimits {
            stall: Duration::from_secs(10),
            retry_budget: Duration::from_secs(10),
        };

        let result = put_within_deadline(&storage, &range, limits).await;

        let error = match result {
            Err(BinaryPutError::Failed(error)) => error,
            other => panic!("expected a network error, got {other:?}"),
        };
        assert_eq!(error.kind, RuntimeErrorKind::Network);
        assert!(
            error.message.starts_with("Network error: "),
            "expected the transport error, got {:?}",
            error.message
        );
        assert_eq!(
            storage.requests(),
            1,
            "a closed connection must not be retried"
        );
    }

    /// The stall error says what happened and what to do, in full.
    #[test]
    fn test_upload_stall_error_message_is_complete() {
        let error = upload_stall_error(UPLOAD_STALL_LIMIT);

        assert_eq!(error.kind, RuntimeErrorKind::Network);
        assert_eq!(
            error.message,
            "The upload stopped because no data moved for 120 seconds."
        );
        assert_eq!(
            error.hint.as_deref(),
            Some("Check your network connection, then run the upload again.")
        );
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
