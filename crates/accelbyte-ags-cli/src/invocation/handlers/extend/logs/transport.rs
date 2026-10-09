//! HTTP transport for `ags extend logs stream`: the stream client, the URL
//! builder, the token seam, and the production connection attempt that sends
//! one request (with one token refresh on `401`), feeds the response body
//! to the SSE reader, and reports how the attempt ended.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::redirect::Policy;

use ags_runtime::runtime::dispatch::http::{APPLICATION_USER_AGENT, CONNECT_TIMEOUT_SECS};

use crate::errors::CliError;

use super::sse::{SseError, SseEvent, SseReader};

/// Check whether a reqwest error is caused by a read timeout, walking the
/// error source chain. reqwest's `is_timeout()` is set on the outermost
/// error only when the timeout originates at the reqwest layer (e.g. the
/// total `.timeout()`). A `read_timeout` error surfaces as a hyper body
/// decode error wrapping an `io::TimedOut`; `is_timeout()` returns `false`
/// on the outer error even though the root cause is a timeout. This helper
/// checks both.
fn is_read_timeout(err: &reqwest::Error) -> bool {
    if err.is_timeout() {
        return true;
    }
    // Walk the source chain looking for io::ErrorKind::TimedOut.
    let mut source = std::error::Error::source(err);
    while let Some(inner) = source {
        if let Some(io_err) = inner.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::TimedOut {
                return true;
            }
        }
        source = inner.source();
    }
    false
}

/// Check whether a read timeout is actually the idle timer, not an earlier
/// timeout from a different source. A timeout that fires before the idle
/// limit has elapsed is not the idle timer: it may be a connect timeout,
/// a proxy timeout, or any other timeout in the chain. Only when the
/// silence duration meets or exceeds the idle limit do we report it as
/// `IdleTimeout`.
fn is_idle_timeout(timed_out: bool, silent_for: Duration, idle_limit: Duration) -> bool {
    timed_out && silent_for >= idle_limit
}

// ── Constants ──

/// Idle read timeout for the log stream. The server sends a keepalive every
/// 30 seconds, so 90 seconds of silence means the connection is dead and the
/// CLI should reconnect.
pub(crate) const STREAM_IDLE_READ_TIMEOUT: Duration = Duration::from_secs(90);

/// Maximum bytes to read from an error response body. The raw body is never
/// returned to the caller; only the CSM `errorCode`/`errorMessage` fields
/// extracted by `extract_csm_error_detail` are surfaced.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// Maximum time to spend reading an error response body. The body is read
/// only to extract a short CSM error message, so a stalled body must not
/// hold a reconnect. Independent of `--timeout` (which controls the header
/// wait).
const ERROR_BODY_READ_LIMIT: Duration = Duration::from_secs(5);

/// Format an error and its source chain into a single string.
///
/// Each `source()` in the chain is appended, separated by `: `. A source
/// whose text is already contained in the accumulated text so far is
/// skipped, so that chains like `"error sending request: refused"` wrapping
/// `io::Error("refused")` do not repeat `refused`.
///
/// Accepts `&dyn std::error::Error` so tests can build a chain without
/// requiring `reqwest::Error`.
fn describe_error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(inner) = source {
        let inner_text = inner.to_string();
        if !text.contains(&inner_text) {
            text.push_str(": ");
            text.push_str(&inner_text);
        }
        source = inner.source();
    }
    text
}

// ── Stream Client ──

/// Build an HTTP client for the log stream.
///
/// This client has no total timeout (`.timeout(...)` is deliberately omitted)
/// because the shared `build_http_client` sets a total timeout that also
/// covers reading the body, which would cut every `--follow` session after
/// one minute. The `read_timeout` resets after every successful chunk read,
/// so it acts as an idle timeout.
pub(crate) fn build_stream_client(
    idle_read_timeout: Duration,
) -> Result<reqwest::Client, CliError> {
    reqwest::Client::builder()
        .user_agent(APPLICATION_USER_AGENT)
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .read_timeout(idle_read_timeout)
        .redirect(Policy::none())
        .build()
        .map_err(|e| CliError::Network {
            message: format!("failed to build stream HTTP client: {e}"),
            metadata: None,
        })
}

// ── URL Builder ──

/// Query parameters for the log-streaming endpoint.
pub(crate) struct LogsQuery {
    pub(crate) follow: bool,
    pub(crate) tail_lines: u32,
    pub(crate) pods: u32,
    pub(crate) since_seconds: Option<u32>,
    pub(crate) previous: bool,
}

/// Build the URL for the CSM log-streaming endpoint.
///
/// Path: `/csm/v4/admin/namespaces/{namespace}/apps/{app}/logs`.
///
/// Query parameters are appended in the order the Spec defines: `follow`,
/// `tailLines`, `previous`, `pods`, then `sinceSeconds` (only when set).
///
/// With `reconnect == true`: `tailLines` is forced to `1` and `sinceSeconds`
/// is omitted. The other parameters are unchanged.
pub(crate) fn logs_url(
    base_url: &str,
    namespace: &str,
    app: &str,
    query: &LogsQuery,
    reconnect: bool,
) -> Result<reqwest::Url, CliError> {
    let mut url = reqwest::Url::parse(base_url).map_err(|e| CliError::Usage {
        message: format!("invalid base URL {base_url:?}: {e}"),
        metadata: None,
    })?;

    // Use `path_segments_mut` so that values cannot change the path structure
    // and a trailing `/` on the base does not double.
    url.path_segments_mut()
        .map_err(|()| CliError::Usage {
            message: format!("base URL {base_url:?} cannot have path segments"),
            metadata: None,
        })?
        .push("csm")
        .push("v4")
        .push("admin")
        .push("namespaces")
        .push(namespace)
        .push("apps")
        .push(app)
        .push("logs");

    let tail_lines = if reconnect { 1 } else { query.tail_lines };

    url.query_pairs_mut()
        .append_pair("follow", &query.follow.to_string())
        .append_pair("tailLines", &tail_lines.to_string())
        .append_pair("previous", &query.previous.to_string())
        .append_pair("pods", &query.pods.to_string());

    if !reconnect {
        if let Some(since) = query.since_seconds {
            url.query_pairs_mut()
                .append_pair("sinceSeconds", &since.to_string());
        }
    }

    Ok(url)
}

// ── Token Seam ──

/// Type alias for the async closure that resolves the current access token.
pub(crate) type ResolveTokenFn =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<String, CliError>> + Send>> + Send + Sync>;

/// Type alias for the async closure that refreshes the session and returns
/// the new token. Returns `Ok(None)` when no refresh was possible (no stored
/// refresh token, client mismatch, etc.).
pub(crate) type RefreshTokenFn = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<Option<String>, CliError>> + Send>>
        + Send
        + Sync,
>;

/// Build the production token closures backed by `resolve_access_token` and
/// `force_refresh_session`, as `dial_ws` does for the tunnel command.
pub(crate) fn production_token_fns(
    profile: String,
) -> Result<(ResolveTokenFn, RefreshTokenFn), CliError> {
    let http_client =
        ags_runtime::runtime::dispatch::http::build_http_client(None).map_err(CliError::from)?;

    let resolve_client = http_client.clone();
    let resolve_profile = profile.clone();
    let resolve: ResolveTokenFn = Box::new(move || {
        let client = resolve_client.clone();
        let prof = resolve_profile.clone();
        Box::pin(async move {
            let resolution =
                ags_runtime::runtime::auth::session::resolve_access_token(&client, &prof)
                    .await
                    .map_err(CliError::from)?;
            Ok(resolution.token)
        })
    });

    let refresh_client = http_client;
    let refresh_profile = profile;
    let refresh: RefreshTokenFn = Box::new(move || {
        let client = refresh_client.clone();
        let prof = refresh_profile.clone();
        Box::pin(async move {
            let refreshed =
                ags_runtime::runtime::auth::session::force_refresh_session(&client, &prof)
                    .await
                    .map_err(CliError::from)?;
            if !refreshed {
                return Ok(None);
            }
            let resolution =
                ags_runtime::runtime::auth::session::resolve_access_token(&client, &prof)
                    .await
                    .map_err(CliError::from)?;
            Ok(Some(resolution.token))
        })
    });

    Ok((resolve, refresh))
}

// ── Event Sink ──

/// Controls whether the stream attempt should continue reading events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SinkControl {
    /// Continue reading the next event.
    Continue,
    /// Stop reading; the consumer has seen enough.
    Stop,
}

/// Receives SSE events from a stream attempt. Implementations decide what to
/// do with each event (print, record, filter duplicates) and whether to stop
/// the stream.
pub(crate) trait EventSink {
    /// Called once when the server answers `200` and the response body begins.
    /// Return `SinkControl::Stop` to end the attempt without reading any
    /// events (e.g. when stdout is already closed).
    fn opened(&mut self) -> SinkControl;

    /// Called for every complete SSE event the reader returns, in order.
    /// Return `SinkControl::Stop` to end the attempt early.
    fn event(&mut self, event: SseEvent) -> SinkControl;
}

// ── Attempt Outcome ──

/// How a single connection attempt ended.
#[derive(Debug)]
pub(crate) enum AttemptOutcome {
    /// The event sink returned `SinkControl::Stop`.
    StoppedBySink,
    /// The server answered `200` and the body ended normally (EOF).
    Closed,
    /// Reading the body failed because the idle read timeout elapsed without
    /// any data arriving.
    IdleTimeout,
    /// Reading the body failed for a reason other than the idle timeout.
    TransportError(String),
    /// The server answered with a status other than `200` (after the single
    /// `401` retry). `detail` comes from `extract_csm_error_detail` only.
    HttpStatus { status: u16, detail: String },
    /// `401` again after the refresh, or the refresh itself failed.
    Unauthorized(String),
    /// No response at all: DNS failure, connection refused, TLS error, or
    /// the response headers did not arrive within the header timeout.
    NoResponse(String),
    /// The SSE reader encountered a single line exceeding its size limit.
    LineTooLong,
}

// ── Stream Attempt ──

/// A single connection attempt against the CSM log-streaming endpoint.
///
/// Resolves a fresh token, sends the request with one `401` retry, feeds the
/// response body to an `SseReader`, and passes each event to the sink.
pub(crate) struct HttpStreamAttempt {
    pub(crate) client: reqwest::Client,
    pub(crate) resolve_token: ResolveTokenFn,
    pub(crate) refresh_token: RefreshTokenFn,
    pub(crate) header_timeout: Duration,
    /// The idle timeout the stream client was built with. Used by
    /// `read_body` to distinguish a genuine idle timeout from an early
    /// timeout fired by a different source.
    pub(crate) idle_timeout: Duration,
    pub(crate) base_url: String,
    pub(crate) namespace: String,
    pub(crate) app: String,
    pub(crate) query: LogsQuery,
}

impl HttpStreamAttempt {
    /// Execute one connection attempt.
    ///
    /// A fresh token is resolved at the start. On a `401` response, the
    /// session is refreshed once and the same request is retried — at most
    /// two HTTP requests per call.
    pub(crate) async fn run(
        &self,
        url: reqwest::Url,
        last_event_id: Option<&str>,
        sink: &mut (impl EventSink + ?Sized),
    ) -> Result<AttemptOutcome, CliError> {
        let token = (self.resolve_token)().await?;

        let outcome = self
            .send_and_read(url.clone(), &token, last_event_id, sink)
            .await?;

        // On 401: refresh once and retry.
        if let AttemptOutcome::HttpStatus { status: 401, .. } = &outcome {
            match (self.refresh_token)().await? {
                Some(new_token) => {
                    return self
                        .send_and_read(url, &new_token, last_event_id, sink)
                        .await
                        .map(|o| match o {
                            AttemptOutcome::HttpStatus {
                                status: 401,
                                detail,
                            } => AttemptOutcome::Unauthorized(format!(
                                "authentication failed after token refresh{detail}"
                            )),
                            other => other,
                        });
                }
                None => {
                    return Ok(AttemptOutcome::Unauthorized(
                        "authentication failed and session could not be refreshed".to_string(),
                    ));
                }
            }
        }

        Ok(outcome)
    }

    /// Send the request and process the response. Returns the outcome without
    /// performing any retry logic.
    async fn send_and_read(
        &self,
        url: reqwest::Url,
        token: &str,
        last_event_id: Option<&str>,
        sink: &mut (impl EventSink + ?Sized),
    ) -> Result<AttemptOutcome, CliError> {
        let mut request = self
            .client
            .get(url)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header(reqwest::header::ACCEPT_ENCODING, "identity");

        // Send Last-Event-ID only when it is Some and non-empty, and when
        // the value is a valid header value. An invalid header value is
        // silently dropped — the server will replay from the beginning,
        // which is safe; dropping is better than failing the request.
        if let Some(id) = last_event_id {
            if !id.is_empty() {
                if let Ok(hv) = reqwest::header::HeaderValue::from_str(id) {
                    request = request.header("Last-Event-ID", hv);
                }
            }
        }

        // Bound the wait for response headers by the header timeout.
        let response = match tokio::time::timeout(self.header_timeout, request.send()).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                return Ok(AttemptOutcome::NoResponse(describe_error_chain(&e)));
            }
            Err(_elapsed) => {
                return Ok(AttemptOutcome::NoResponse(format!(
                    "timed out waiting for response headers ({}s)",
                    self.header_timeout.as_secs()
                )));
            }
        };

        let status = response.status().as_u16();

        if status != 200 {
            // Read at most MAX_ERROR_BODY_BYTES of the error body for CSM
            // error detail extraction. The raw body is never returned.
            //
            // The stream client has no total timeout, so an error response
            // whose body is large or never ends could grow memory and hold
            // the command indefinitely. We read in chunks, stop as soon as
            // enough bytes are held, and bound the whole read by
            // ERROR_BODY_READ_LIMIT (5 s) so a stalled body cannot hold a
            // reconnect.
            let body_bytes = self.read_error_body(response).await;
            let body_str = String::from_utf8_lossy(&body_bytes);
            let detail = super::super::csm_error::extract_csm_error_detail(&body_str);
            return Ok(AttemptOutcome::HttpStatus { status, detail });
        }

        // 200 OK: notify the sink and start reading chunks.
        if sink.opened() == SinkControl::Stop {
            return Ok(AttemptOutcome::StoppedBySink);
        }
        self.read_body(response, sink).await
    }

    /// Read at most [`MAX_ERROR_BODY_BYTES`] from an error response body,
    /// bounded by [`ERROR_BODY_READ_LIMIT`] (5 s). When either limit is
    /// reached, or a chunk read fails, the bytes held so far are returned.
    /// The response is dropped immediately after, closing the connection.
    async fn read_error_body(&self, mut response: reqwest::Response) -> Vec<u8> {
        let mut buf = Vec::new();
        let deadline = tokio::time::Instant::now() + ERROR_BODY_READ_LIMIT;

        loop {
            let chunk = tokio::time::timeout_at(deadline, response.chunk()).await;
            match chunk {
                Ok(Ok(Some(bytes))) => {
                    let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(buf.len());
                    if remaining == 0 {
                        break;
                    }
                    let take = bytes.len().min(remaining);
                    buf.extend_from_slice(&bytes[..take]);
                    if buf.len() >= MAX_ERROR_BODY_BYTES {
                        break;
                    }
                }
                // EOF, chunk error, or timeout — use what we have.
                _ => break,
            }
        }

        // Drop the response to close the connection immediately.
        drop(response);
        buf
    }

    /// Read the response body chunk by chunk, feeding each to an SSE reader
    /// and passing complete events to the sink.
    ///
    /// Uses `response.chunk()` rather than `bytes_stream()` because
    /// `chunk()` preserves the `is_timeout()` classification on read-timeout
    /// errors, while `bytes_stream()` wraps them in a body-decode error that
    /// loses the flag.
    async fn read_body(
        &self,
        mut response: reqwest::Response,
        sink: &mut (impl EventSink + ?Sized),
    ) -> Result<AttemptOutcome, CliError> {
        let mut reader = SseReader::new();
        let mut last_chunk_at = tokio::time::Instant::now();

        loop {
            let chunk = match response.chunk().await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    // Stream ended (EOF).
                    return Ok(AttemptOutcome::Closed);
                }
                Err(e) => {
                    if is_idle_timeout(
                        is_read_timeout(&e),
                        last_chunk_at.elapsed(),
                        self.idle_timeout,
                    ) {
                        return Ok(AttemptOutcome::IdleTimeout);
                    }
                    return Ok(AttemptOutcome::TransportError(describe_error_chain(&e)));
                }
            };

            last_chunk_at = tokio::time::Instant::now();

            let events = match reader.feed(&chunk) {
                Ok(events) => events,
                Err(SseError::LineOverLimit) => {
                    return Ok(AttemptOutcome::LineTooLong);
                }
            };

            for event in events {
                if sink.event(event) == SinkControl::Stop {
                    return Ok(AttemptOutcome::StoppedBySink);
                }
            }
        }
    }
}

// ── Stream Attempt Trait ──

/// One connection attempt against the log-streaming endpoint. The reconnect
/// loop is generic over this trait so tests can inject scripted sequences.
pub(crate) trait StreamAttempt {
    /// Run one attempt. `reconnect` is true on every attempt after the first.
    /// `last_event_id` is the resume point from the duplicate-suppression
    /// window (passed as `Last-Event-ID` by the HTTP implementation).
    fn run_attempt<'a>(
        &'a self,
        reconnect: bool,
        last_event_id: Option<&'a str>,
        sink: &'a mut dyn EventSink,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AttemptOutcome, CliError>> + 'a>>;
}

impl StreamAttempt for HttpStreamAttempt {
    fn run_attempt<'a>(
        &'a self,
        reconnect: bool,
        last_event_id: Option<&'a str>,
        sink: &'a mut dyn EventSink,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AttemptOutcome, CliError>> + 'a>>
    {
        Box::pin(async move {
            let url = logs_url(
                &self.base_url,
                &self.namespace,
                &self.app,
                &self.query,
                reconnect,
            )?;
            self.run(url, last_event_id, sink).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ── Fake Token Functions ──

    /// Build fake token closures for tests: `resolve` returns a fixed token
    /// (or the refreshed token after a refresh), and `refresh` counts calls.
    fn fake_token_fns(
        initial_token: &str,
        refresh_result: Option<String>,
    ) -> (ResolveTokenFn, RefreshTokenFn, Arc<AtomicUsize>) {
        let refresh_count = Arc::new(AtomicUsize::new(0));

        let token = initial_token.to_string();
        let refreshed_for_resolve = refresh_result.clone();
        let resolve_count = refresh_count.clone();
        let resolve: ResolveTokenFn = Box::new(move || {
            let t = token.clone();
            let refreshed = refreshed_for_resolve.clone();
            let count = resolve_count.clone();
            Box::pin(async move {
                if count.load(Ordering::SeqCst) > 0 {
                    if let Some(ref rt) = refreshed {
                        return Ok(rt.clone());
                    }
                }
                Ok(t)
            })
        });

        let rc = refresh_count.clone();
        let rr = refresh_result;
        let refresh: RefreshTokenFn = Box::new(move || {
            let count = rc.clone();
            let result = rr.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(result)
            })
        });

        (resolve, refresh, refresh_count)
    }

    // ── Recording Sink ──

    struct RecordingSink {
        opened: bool,
        events: Vec<SseEvent>,
        stop_after: Option<usize>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                opened: false,
                events: Vec::new(),
                stop_after: None,
            }
        }
    }

    impl EventSink for RecordingSink {
        fn opened(&mut self) -> SinkControl {
            self.opened = true;
            SinkControl::Continue
        }

        fn event(&mut self, event: SseEvent) -> SinkControl {
            self.events.push(event);
            if let Some(limit) = self.stop_after {
                if self.events.len() >= limit {
                    return SinkControl::Stop;
                }
            }
            SinkControl::Continue
        }
    }

    // ── opened() Stop tests ──

    #[tokio::test]
    async fn opened_stop_ends_attempt_before_reading_body() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // The server responds 200 with a body containing a log event.
        // If the sink returns Stop from opened(), the event must never
        // reach the sink.
        let body = "id: pod|ts\ndata: should-not-arrive\n\n";
        Mock::given(method("GET"))
            .and(path("/logs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        struct StopOnOpenSink {
            events: Vec<SseEvent>,
        }
        impl EventSink for StopOnOpenSink {
            fn opened(&mut self) -> SinkControl {
                SinkControl::Stop
            }
            fn event(&mut self, event: SseEvent) -> SinkControl {
                self.events.push(event);
                SinkControl::Continue
            }
        }

        let client = build_stream_client(STREAM_IDLE_READ_TIMEOUT).unwrap();
        let (resolve, refresh, _) = fake_token_fns("test-token", Some("refreshed".into()));
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(5),
            idle_timeout: STREAM_IDLE_READ_TIMEOUT,
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        let url = reqwest::Url::parse(&format!("{}/logs", server.uri())).unwrap();
        let mut sink = StopOnOpenSink { events: Vec::new() };
        let outcome = attempt.run(url, None, &mut sink).await.unwrap();

        assert!(
            matches!(outcome, AttemptOutcome::StoppedBySink),
            "expected StoppedBySink, got: {outcome:?}"
        );
        assert!(
            sink.events.is_empty(),
            "no events should reach the sink when opened() returns Stop"
        );
    }

    // ── Error chain text tests ──

    #[test]
    fn error_text_includes_each_source_once() {
        // Build a chain: outer wraps an io::Error with ConnectionRefused.
        let io_err = std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "Connection refused (os error 111)",
        );
        // Wrap the io error in a custom error that includes it as a source.
        #[derive(Debug)]
        struct Outer {
            msg: String,
            source: std::io::Error,
        }
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.msg)
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.source)
            }
        }

        let outer = Outer {
            msg: "error sending request".to_string(),
            source: io_err,
        };

        let text = describe_error_chain(&outer);
        // Must contain both the outer message and the inner message.
        assert!(
            text.contains("error sending request"),
            "must contain outer message: {text}"
        );
        assert!(
            text.contains("Connection refused"),
            "must contain inner io error text: {text}"
        );

        // Now test deduplication: an error whose Display already contains
        // the source text should not repeat it.
        let io_err2 = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let outer2 = Outer {
            msg: "error sending request: refused".to_string(),
            source: io_err2,
        };
        let text2 = describe_error_chain(&outer2);
        // The word "refused" should appear exactly once (not repeated).
        let count = text2.matches("refused").count();
        assert_eq!(
            count, 1,
            "source text already in parent must not repeat: {text2}"
        );
    }

    // ── URL Tests ──

    #[test]
    fn logs_url_first_connection_carries_all_flags() {
        let query = LogsQuery {
            follow: true,
            tail_lines: 100,
            pods: 2,
            since_seconds: Some(300),
            previous: false,
        };
        let url = logs_url("https://example.com", "ns1", "myapp", &query, false).unwrap();

        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(
            pairs,
            vec![
                ("follow".to_string(), "true".to_string()),
                ("tailLines".to_string(), "100".to_string()),
                ("previous".to_string(), "false".to_string()),
                ("pods".to_string(), "2".to_string()),
                ("sinceSeconds".to_string(), "300".to_string()),
            ]
        );

        // sinceSeconds absent when not set.
        let query_no_since = LogsQuery {
            follow: false,
            tail_lines: 50,
            pods: 1,
            since_seconds: None,
            previous: true,
        };
        let url2 = logs_url(
            "https://example.com",
            "ns1",
            "myapp",
            &query_no_since,
            false,
        )
        .unwrap();
        let pairs2: Vec<(String, String)> = url2.query_pairs().into_owned().collect();
        assert!(!pairs2.iter().any(|(k, _)| k == "sinceSeconds"));
        // No container parameter ever.
        assert!(!pairs2.iter().any(|(k, _)| k == "container"));

        // Path is correct.
        assert_eq!(url.path(), "/csm/v4/admin/namespaces/ns1/apps/myapp/logs");
    }

    #[test]
    fn logs_url_reconnect_uses_tail_one_and_drops_since() {
        let query = LogsQuery {
            follow: true,
            tail_lines: 100,
            pods: 2,
            since_seconds: Some(300),
            previous: false,
        };
        let url = logs_url("https://example.com", "ns1", "myapp", &query, true).unwrap();

        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(
            pairs,
            vec![
                ("follow".to_string(), "true".to_string()),
                ("tailLines".to_string(), "1".to_string()),
                ("previous".to_string(), "false".to_string()),
                ("pods".to_string(), "2".to_string()),
            ]
        );
        // No sinceSeconds on reconnect.
        assert!(!pairs.iter().any(|(k, _)| k == "sinceSeconds"));
    }

    // ── Attempt Tests (against in-process servers) ──

    /// Spawn a TCP server task that accepts one connection, reads the
    /// request, writes the given response bytes, and optionally holds the
    /// connection open indefinitely.
    async fn spawn_server(response_bytes: Vec<u8>, hold_open: bool) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // Read the full HTTP request before replying, so the server
            // does not close the connection while the client is still
            // writing. We read until we see \r\n\r\n (end of headers).
            let mut buf = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;

            use tokio::io::AsyncWriteExt;
            stream.write_all(&response_bytes).await.unwrap();
            stream.flush().await.unwrap();
            if hold_open {
                // Hold the connection open until cancelled (test ends).
                // Using a large sleep keeps the socket alive without
                // consuming the request data that the read above captured.
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn stream_attempt_ends_with_idle_timeout_on_silent_connection() {
        // Server sends 200 headers and then goes silent (hold_open = true).
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";
        let addr = spawn_server(response.to_vec(), true).await;

        let client = build_stream_client(Duration::from_millis(200)).unwrap();
        let (resolve, refresh, _) = fake_token_fns("test-token", Some("refreshed".into()));
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_millis(200),
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        let url = reqwest::Url::parse(&format!("http://{addr}/logs")).unwrap();
        let mut sink = RecordingSink::new();
        let outcome = attempt.run(url, None, &mut sink).await.unwrap();

        assert!(sink.opened, "sink.opened must be called on 200");
        assert!(
            matches!(outcome, AttemptOutcome::IdleTimeout),
            "expected IdleTimeout, got: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn stream_attempt_sends_last_event_id_only_when_resuming() {
        use wiremock::matchers::{header_exists, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Mount a mock that expects Last-Event-ID — must receive exactly 1 hit.
        Mock::given(method("GET"))
            .and(path("/logs"))
            .and(header_exists("Last-Event-ID"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("event: stream-end\ndata: {\"reason\":\"completed\"}\n\n"),
            )
            .named("with-last-event-id")
            .expect(1)
            .mount(&server)
            .await;

        // Mount a fallback mock for requests without Last-Event-ID.
        Mock::given(method("GET"))
            .and(path("/logs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("event: stream-end\ndata: {\"reason\":\"completed\"}\n\n"),
            )
            .named("without-last-event-id")
            .expect(1)
            .mount(&server)
            .await;

        let client = build_stream_client(STREAM_IDLE_READ_TIMEOUT).unwrap();
        let (resolve, refresh, _) = fake_token_fns("test-token", Some("refreshed".into()));
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(5),
            idle_timeout: STREAM_IDLE_READ_TIMEOUT,
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        // First call: no last_event_id.
        let url = reqwest::Url::parse(&format!("{}/logs", server.uri())).unwrap();
        let mut sink1 = RecordingSink::new();
        let _ = attempt.run(url.clone(), None, &mut sink1).await.unwrap();

        // Second call: with last_event_id.
        let mut sink2 = RecordingSink::new();
        let _ = attempt
            .run(url, Some("pod-a|2026-09-28T09:14:02Z"), &mut sink2)
            .await
            .unwrap();

        // wiremock's expect(1) on each mock will panic on drop if not satisfied.
    }

    #[tokio::test]
    async fn stream_attempt_refreshes_once_on_401() {
        use wiremock::matchers::{bearer_token, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // First request with the initial token gets 401.
        Mock::given(method("GET"))
            .and(path("/logs"))
            .and(bearer_token("initial"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("{\"errorMessage\":\"unauthorized\"}"),
            )
            .named("401-initial")
            .expect(1)
            .mount(&server)
            .await;

        // Second request with the refreshed token gets 200.
        Mock::given(method("GET"))
            .and(path("/logs"))
            .and(bearer_token("initial-refreshed"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("event: stream-end\ndata: {\"reason\":\"completed\"}\n\n"),
            )
            .named("200-refreshed")
            .expect(1)
            .mount(&server)
            .await;

        let (resolve, refresh, refresh_count) =
            fake_token_fns("initial", Some("initial-refreshed".into()));
        let client = build_stream_client(STREAM_IDLE_READ_TIMEOUT).unwrap();
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(5),
            idle_timeout: STREAM_IDLE_READ_TIMEOUT,
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        let url = reqwest::Url::parse(&format!("{}/logs", server.uri())).unwrap();
        let mut sink = RecordingSink::new();
        let outcome = attempt.run(url, None, &mut sink).await.unwrap();

        assert!(
            matches!(outcome, AttemptOutcome::Closed),
            "expected Closed after 401-then-200, got: {outcome:?}"
        );
        assert_eq!(
            refresh_count.load(Ordering::SeqCst),
            1,
            "refresh called once"
        );
        assert!(sink.opened, "sink.opened called after the retry");
    }

    #[tokio::test]
    async fn stream_attempt_exits_auth_after_second_401() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Both requests get 401.
        Mock::given(method("GET"))
            .and(path("/logs"))
            .respond_with(ResponseTemplate::new(401).set_body_string("{\"errorMessage\":\"nope\"}"))
            .named("401-always")
            .expect(2)
            .mount(&server)
            .await;

        let (resolve, refresh, refresh_count) =
            fake_token_fns("token", Some("token-refreshed".into()));
        let client = build_stream_client(STREAM_IDLE_READ_TIMEOUT).unwrap();
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(5),
            idle_timeout: STREAM_IDLE_READ_TIMEOUT,
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        let url = reqwest::Url::parse(&format!("{}/logs", server.uri())).unwrap();
        let mut sink = RecordingSink::new();
        let outcome = attempt.run(url, None, &mut sink).await.unwrap();

        assert!(
            matches!(outcome, AttemptOutcome::Unauthorized(_)),
            "expected Unauthorized, got: {outcome:?}"
        );
        assert_eq!(
            refresh_count.load(Ordering::SeqCst),
            1,
            "refresh called exactly once"
        );
        assert!(
            !sink.opened,
            "sink.opened must not be called on auth failure"
        );
    }

    #[tokio::test]
    async fn stream_attempt_does_not_wait_the_header_timeout_for_a_stalled_error_body() {
        // Server sends 503 headers with Content-Length 4096 and a partial JSON
        // body, then holds the connection open. The attempt must return within
        // ERROR_BODY_READ_LIMIT (5 s), not the full header_timeout (30 s).
        let response =
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n{\"errorCode\":1,";
        let addr = spawn_server(response.to_vec(), true).await;

        let client = build_stream_client(Duration::from_secs(30)).unwrap();
        let (resolve, refresh, _) = fake_token_fns("test-token", Some("refreshed".into()));
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(30),
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        let url = reqwest::Url::parse(&format!("http://{addr}/logs")).unwrap();
        let mut sink = RecordingSink::new();

        let start = tokio::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            attempt.run(url, None, &mut sink).await
        })
        .await;

        let elapsed = start.elapsed();
        let outcome = result
            .expect("timed out at 60 s; the read was not bounded")
            .expect("attempt should not return Err");

        assert!(
            matches!(outcome, AttemptOutcome::HttpStatus { status: 503, .. }),
            "expected HttpStatus 503, got: {outcome:?}"
        );
        assert!(
            elapsed < Duration::from_secs(15),
            "error body read took {elapsed:?}, expected under 15 s"
        );
    }

    #[tokio::test]
    async fn stream_attempt_error_body_read_is_bounded() {
        // Server answers 500 with a CSM JSON error at the start, followed by
        // more than 128 KiB of padding, then holds the connection open. The
        // bounded read must stop after MAX_ERROR_BODY_BYTES and return the
        // detail without waiting for the body to end.
        let csm_json = r#"{"errorCode":20001,"errorMessage":"boom"}"#;
        let padding = vec![b' '; 128 * 1024 + 1];

        let mut response = Vec::new();
        // No Content-Length — the body never ends (hold_open = true).
        response.extend_from_slice(b"HTTP/1.1 500 Internal Server Error\r\n");
        response.extend_from_slice(b"Content-Type: application/json\r\n");
        response.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");

        // Write the CSM JSON as a chunk.
        let csm_hex = format!("{:x}\r\n", csm_json.len());
        response.extend_from_slice(csm_hex.as_bytes());
        response.extend_from_slice(csm_json.as_bytes());
        response.extend_from_slice(b"\r\n");

        // Write the padding as a chunk.
        let pad_hex = format!("{:x}\r\n", padding.len());
        response.extend_from_slice(pad_hex.as_bytes());
        response.extend_from_slice(&padding);
        response.extend_from_slice(b"\r\n");

        // Do NOT write the terminating 0-length chunk — the body never ends.
        let addr = spawn_server(response, true).await;

        let client = build_stream_client(Duration::from_secs(30)).unwrap();
        let (resolve, refresh, _) = fake_token_fns("test-token", Some("refreshed".into()));
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(30),
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        let url = reqwest::Url::parse(&format!("http://{addr}/logs")).unwrap();
        let mut sink = RecordingSink::new();

        // Wrap in a timeout so the test fails instead of hanging if the
        // bounded read does not work.
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            attempt.run(url, None, &mut sink).await
        })
        .await;

        let outcome = result
            .expect("timed out — error body read was not bounded")
            .expect("attempt should not return Err");

        match &outcome {
            AttemptOutcome::HttpStatus { status, detail } => {
                assert_eq!(*status, 500, "expected 500");
                assert!(
                    detail.contains("boom"),
                    "expected CSM detail to contain 'boom', got: {detail:?}"
                );
            }
            other => panic!("expected HttpStatus, got: {other:?}"),
        }
    }

    // ── is_idle_timeout unit tests ──

    #[test]
    fn idle_timeout_true_when_timed_out_and_silent_exceeds_limit() {
        assert!(is_idle_timeout(
            true,
            Duration::from_secs(100),
            Duration::from_secs(90)
        ));
    }

    #[test]
    fn idle_timeout_true_when_silent_equals_limit() {
        assert!(is_idle_timeout(
            true,
            Duration::from_secs(90),
            Duration::from_secs(90)
        ));
    }

    #[test]
    fn idle_timeout_false_when_not_timed_out() {
        assert!(!is_idle_timeout(
            false,
            Duration::from_secs(100),
            Duration::from_secs(90)
        ));
    }

    #[test]
    fn idle_timeout_false_when_silent_below_limit() {
        assert!(!is_idle_timeout(
            true,
            Duration::from_millis(50),
            Duration::from_secs(90)
        ));
    }

    #[test]
    fn timeout_after_30s_of_silence_is_a_transport_error() {
        assert!(!is_idle_timeout(
            true,
            Duration::from_secs(30),
            Duration::from_secs(90),
        ));
    }

    #[test]
    fn timeout_after_89s_of_silence_is_a_transport_error() {
        assert!(!is_idle_timeout(
            true,
            Duration::from_secs(89),
            Duration::from_secs(90),
        ));
    }

    // ── Early timeout → TransportError integration test ──

    #[tokio::test]
    async fn stream_attempt_reports_an_early_timeout_as_a_transport_error() {
        // The stream client's read_timeout is 200 ms, but idle_timeout is
        // set to 10 s. When the server goes silent after sending headers,
        // the timeout fires at 200 ms — well before the 10 s idle limit —
        // so `is_idle_timeout` returns false and the outcome must be
        // `TransportError`, not `IdleTimeout`.
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";
        let addr = spawn_server(response.to_vec(), true).await;

        let client = build_stream_client(Duration::from_millis(200)).unwrap();
        let (resolve, refresh, _) = fake_token_fns("test-token", Some("refreshed".into()));
        let attempt = HttpStreamAttempt {
            client,
            resolve_token: resolve,
            refresh_token: refresh,
            header_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(10),
            base_url: String::new(),
            namespace: String::new(),
            app: String::new(),
            query: LogsQuery {
                follow: false,
                tail_lines: 100,
                pods: 1,
                since_seconds: None,
                previous: false,
            },
        };

        let url = reqwest::Url::parse(&format!("http://{addr}/logs")).unwrap();
        let mut sink = RecordingSink::new();
        let outcome = attempt.run(url, None, &mut sink).await.unwrap();

        assert!(sink.opened, "sink.opened must be called on 200");
        assert!(
            matches!(outcome, AttemptOutcome::TransportError(_)),
            "expected TransportError for early timeout, got: {outcome:?}"
        );
    }
}
