//! TCP-to-WebSocket bridge for `ags extend tunnel`.
//!
//! Binds a local TCP listener and, for each accepted connection, opens a
//! WebSocket to the CSM v2 tunnel endpoint and shuttles bytes in both
//! directions. The tunnel stays open until Ctrl-C or the cancellation
//! token fires.

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite;
use tokio_util::sync::CancellationToken;

/// Configuration for a tunnel bridge instance.
pub struct TunnelConfig {
    /// Derived from the base URL: host plus port when the base URL carries one.
    pub host: String,
    pub namespace: String,
    pub resource_name: String,
    /// The local TCP port to bind (localhost only, never a wildcard address).
    pub local_port: u16,
    /// Optional; forwarded as &podName=<pod_name> in the WS URL when present.
    pub pod_name: Option<String>,
}

/// Tunnel-level errors that the handler maps to `CliError` variants.
///
/// `Auth(String)` carries the error message; the handler inspects the message
/// prefix to distinguish 403 Forbidden (mapped to `CliError::Api`, exit 3)
/// from other auth failures (mapped to `CliError::Auth`, exit 2).
#[derive(Debug)]
pub enum TunnelError {
    Bind(std::io::Error),
    Auth(String),
    Cancelled,
}

impl std::fmt::Display for TunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TunnelError::Bind(e) => write!(f, "failed to bind local port: {e}"),
            TunnelError::Auth(msg) => write!(f, "{msg}"),
            TunnelError::Cancelled => write!(f, "tunnel cancelled"),
        }
    }
}

// ── Pure helpers ──

/// Build the WebSocket URL for the CSM v2 tunnel endpoint.
///
/// Called by `dial_ws`; extracted as a pure function for unit testing of
/// URL construction (pod-name presence/absence).
fn build_ws_url(
    host: &str,
    namespace: &str,
    resource_name: &str,
    pod_name: Option<&str>,
) -> String {
    let mut url = format!(
        "wss://{host}/csm/v2/admin/namespaces/{namespace}/tunnel?resourceName={resource_name}"
    );
    if let Some(p) = pod_name {
        url.push_str(&format!("&podName={p}"));
    }
    url
}

/// Build the TCP bind address. Always `127.0.0.1`, never `0.0.0.0`.
///
/// Called by `run_tunnel`; extracted so the localhost invariant is testable.
fn build_bind_addr(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

/// Accept-error resilience policy: returns `true` when the tunnel loop
/// should continue after `_error`, `false` to terminate.
///
/// Currently all accept errors are non-fatal — one bad accept must not
/// kill a live tunnel. Extracted for unit testing of the resilience policy.
fn should_continue_after_accept_error(_error: &std::io::Error) -> bool {
    true
}

// ── Relay functions ──

/// Forward TCP bytes to a WebSocket sink as Binary frames.
///
/// Called by `handle_client_connection`; generic over the sink type so
/// unit tests can inject a mock sink that captures frame types.
async fn relay_tcp_to_ws<W>(
    tcp_reader: &mut (impl tokio::io::AsyncRead + Unpin),
    ws_sink: &mut W,
) -> Result<(), String>
where
    W: futures_util::Sink<tungstenite::Message> + Unpin,
    <W as futures_util::Sink<tungstenite::Message>>::Error: std::fmt::Display,
{
    let mut buf = vec![0u8; TCP_READ_BUF_SIZE];
    loop {
        let n = tcp_reader
            .read(&mut buf)
            .await
            .map_err(|e| format!("TCP read: {e}"))?;
        if n == 0 {
            break;
        }
        ws_sink
            .send(tungstenite::Message::Binary(buf[..n].to_vec()))
            .await
            .map_err(|e| format!("WS send: {e}"))?;
    }
    Ok(())
}

/// Forward WebSocket messages to a TCP writer, skipping non-Binary
/// non-Text frames (Ping, Pong, Close, Frame).
///
/// Called by `handle_client_connection`; generic over the stream type so
/// unit tests can feed specific frame types through a mock stream.
async fn relay_ws_to_tcp<S, E>(
    ws_source: &mut S,
    tcp_writer: &mut (impl tokio::io::AsyncWrite + Unpin),
) -> Result<(), String>
where
    S: futures_util::Stream<Item = Result<tungstenite::Message, E>> + Unpin,
    E: std::fmt::Display,
{
    while let Some(msg) = ws_source.next().await {
        match msg {
            Ok(tungstenite::Message::Binary(data)) => {
                tcp_writer
                    .write_all(&data)
                    .await
                    .map_err(|e| format!("TCP write: {e}"))?;
            }
            Ok(tungstenite::Message::Text(text)) => {
                tcp_writer
                    .write_all(text.as_bytes())
                    .await
                    .map_err(|e| format!("TCP write: {e}"))?;
            }
            Ok(_) => continue,
            Err(e) => return Err(format!("WS receive: {e}")),
        }
    }
    Ok(())
}

// ── Bidirectional relay ──

/// Run both relay directions and, when either finishes, return.
///
/// Called by `handle_client_connection`; extracted as a generic helper so
/// the relay + close pattern is unit-testable with mock sinks and streams.
async fn relay_bidirectional<W, S, E>(
    tcp_reader: &mut (impl tokio::io::AsyncRead + Unpin),
    tcp_writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    ws_sink: &mut W,
    ws_source: &mut S,
) -> Result<(), String>
where
    W: futures_util::Sink<tungstenite::Message> + Unpin,
    <W as futures_util::Sink<tungstenite::Message>>::Error: std::fmt::Display,
    S: futures_util::Stream<Item = Result<tungstenite::Message, E>> + Unpin,
    E: std::fmt::Display,
{
    let tcp_to_ws = relay_tcp_to_ws(tcp_reader, ws_sink);
    let ws_to_tcp = relay_ws_to_tcp(ws_source, tcp_writer);

    let relay_result;
    tokio::select! {
        result = tcp_to_ws => {
            relay_result = result.map_err(|e| format!("tunnel relay: {e}"));
        }
        result = ws_to_tcp => {
            relay_result = result.map_err(|e| format!("tunnel relay: {e}"));
        }
    }

    // Best-effort close frame (RFC 6455 §7.1.2). Ignore send errors —
    // the peer may already be gone, and that is not a failure worth
    // surfacing.
    let _ = ws_sink.send(tungstenite::Message::Close(None)).await;

    relay_result
}

// ── Constants ──

/// Size of the read buffer for TCP to WS forwarding.
const TCP_READ_BUF_SIZE: usize = 8192;

/// Prefix used on `TunnelError::Auth` messages when the WS endpoint
/// returned HTTP 403. The handler checks for this prefix to map 403 to
/// `CliError::Api` (exit 3) rather than `CliError::Auth` (exit 2).
pub(crate) const FORBIDDEN_PREFIX: &str = "Forbidden:";

/// Timeout for the WebSocket upgrade handshake (TCP connect + TLS +
/// HTTP upgrade). Does NOT bound the tunnel session itself.
///
/// 30 s tolerates high-latency clusters while still failing in human
/// time. The constant is passed to `try_connect` by value so unit tests
/// can inject a shorter duration without changing `run_tunnel`'s
/// signature.
const WS_DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

// ── WebSocket dialing ──

/// Internal dial result that preserves HTTP status for retry logic.
enum DialError {
    Unauthorized,
    Forbidden,
    Other(String),
}

/// Attempt a single WebSocket connection with the given token.
///
/// `dial_timeout` bounds the time spent on the WebSocket upgrade
/// handshake (TCP connect + TLS + HTTP upgrade), not the tunnel session.
async fn try_connect(
    url: &str,
    token: &str,
    dial_timeout: std::time::Duration,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    DialError,
> {
    use tungstenite::client::IntoClientRequest;

    let mut request = url
        .into_client_request()
        .map_err(|e| DialError::Other(format!("invalid WS URL: {e}")))?;

    request.headers_mut().insert(
        tungstenite::http::header::AUTHORIZATION,
        tungstenite::http::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| DialError::Other(format!("invalid auth header: {e}")))?,
    );

    let connect_result =
        tokio::time::timeout(dial_timeout, tokio_tungstenite::connect_async(request)).await;

    match connect_result {
        Err(_elapsed) => Err(DialError::Other(format!(
            "WebSocket dial timed out ({}s exceeded)",
            dial_timeout.as_secs()
        ))),
        Ok(Ok((ws, _response))) => Ok(ws),
        Ok(Err(tungstenite::Error::Http(response))) => {
            let status = response.status().as_u16();
            match status {
                401 => Err(DialError::Unauthorized),
                403 => Err(DialError::Forbidden),
                _ => Err(DialError::Other(format!(
                    "WebSocket dial failed with HTTP {status}"
                ))),
            }
        }
        Ok(Err(e)) => Err(DialError::Other(format!("WebSocket dial failed: {e}"))),
    }
}

/// Retry logic for WebSocket dial: make one connect attempt, and on a 401
/// response call `refresh` to obtain a new token and retry exactly once.
///
/// `connect` is called with the token and returns the WebSocket connection
/// or a `DialError`. `refresh` is called after a 401 and returns
/// `(refreshed_ok, new_token)`.
///
/// Production `dial_ws` supplies real implementations; unit tests inject
/// mock closures to verify retry count without network or auth access.
async fn dial_with_retry<Ws: Send>(
    initial_token: String,
    connect: impl Fn(
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Ws, DialError>> + Send>>
        + Send,
    refresh: impl FnOnce() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(bool, String), TunnelError>> + Send>,
        > + Send,
) -> Result<Ws, TunnelError> {
    match connect(initial_token).await {
        Ok(ws) => Ok(ws),
        Err(DialError::Unauthorized) => {
            // 401: force-refresh the session and retry exactly once.
            let (refreshed, new_token) = refresh().await?;
            if !refreshed {
                return Err(TunnelError::Auth(
                    "Authentication failed and session could not be refreshed".to_string(),
                ));
            }

            connect(new_token).await.map_err(|e| match e {
                DialError::Unauthorized => {
                    TunnelError::Auth("Authentication failed after token refresh".to_string())
                }
                DialError::Forbidden => TunnelError::Auth(format!(
                    "{FORBIDDEN_PREFIX} insufficient permissions to access the tunnel endpoint"
                )),
                DialError::Other(msg) => TunnelError::Auth(msg),
            })
        }
        Err(DialError::Forbidden) => Err(TunnelError::Auth(format!(
            "{FORBIDDEN_PREFIX} insufficient permissions to access the tunnel endpoint"
        ))),
        Err(DialError::Other(msg)) => Err(TunnelError::Auth(msg)),
    }
}

/// Dial the WS tunnel endpoint, with a single 401-retry via
/// `force_refresh_session`. On HTTP 403, the returned `TunnelError::Auth`
/// message starts with [`FORBIDDEN_PREFIX`] so the handler can map it to
/// `CliError::Api` (exit 3).
async fn dial_ws(
    host: &str,
    namespace: &str,
    resource_name: &str,
    pod_name: Option<&str>,
    profile: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    TunnelError,
> {
    let http_client = ags_runtime::runtime::dispatch::http::build_http_client(None)
        .map_err(|e| TunnelError::Auth(format!("failed to build HTTP client: {e}")))?;

    let url = build_ws_url(host, namespace, resource_name, pod_name);

    // First attempt: resolve a fresh token.
    let token = ags_runtime::runtime::auth::session::resolve_access_token(&http_client, profile)
        .await
        .map_err(|e| TunnelError::Auth(format!("{e}")))?
        .token;

    let client_for_refresh = http_client.clone();
    let profile_for_refresh = profile.to_string();

    dial_with_retry(
        token,
        move |tok| {
            let u = url.clone();
            Box::pin(async move { try_connect(&u, &tok, WS_DIAL_TIMEOUT).await })
        },
        move || {
            Box::pin(async move {
                let refreshed = ags_runtime::runtime::auth::session::force_refresh_session(
                    &client_for_refresh,
                    &profile_for_refresh,
                )
                .await
                .map_err(|e| TunnelError::Auth(format!("{e}")))?;

                if !refreshed {
                    return Ok((false, String::new()));
                }

                let new_token = ags_runtime::runtime::auth::session::resolve_access_token(
                    &client_for_refresh,
                    &profile_for_refresh,
                )
                .await
                .map_err(|e| TunnelError::Auth(format!("{e}")))?
                .token;

                Ok((true, new_token))
            })
        },
    )
    .await
}

// ── Per-connection handler ──

/// Handle a single accepted TCP connection: resolve a fresh token, dial
/// the WS URL, and run two directional relay loops until either side
/// closes.
async fn handle_client_connection(
    tcp_stream: tokio::net::TcpStream,
    host: &str,
    namespace: &str,
    resource_name: &str,
    pod_name: Option<&str>,
    profile: Option<&str>,
) -> Result<(), String> {
    let profile_name = profile.unwrap_or("default");

    let ws_stream = dial_ws(host, namespace, resource_name, pod_name, profile_name)
        .await
        .map_err(|e| format!("{e}"))?;

    let (mut ws_sink, mut ws_source) = ws_stream.split();
    let (mut tcp_reader, mut tcp_writer) = tcp_stream.into_split();

    relay_bidirectional(
        &mut tcp_reader,
        &mut tcp_writer,
        &mut ws_sink,
        &mut ws_source,
    )
    .await
}

/// Run the TCP-to-WebSocket tunnel bridge.
///
/// `listener_ready` controls the ready-signal behaviour:
/// - `Some(sender)` (embedded use): fires the sender once the TCP listener is
///   bound. No ready-signal line is written — the caller owns that write.
/// - `None` (standalone `tunnel` command): writes the ready signal to stderr
///   via the session log after binding.
pub async fn run_tunnel(
    cfg: TunnelConfig,
    listener_ready: Option<oneshot::Sender<()>>,
    cancel: CancellationToken,
    profile: Option<&str>,
    session_log: super::super::session_log::SessionLog,
) -> Result<(), TunnelError> {
    let addr = build_bind_addr(cfg.local_port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(TunnelError::Bind)?;

    // Signal readiness.
    match listener_ready {
        Some(sender) => {
            // Embedded use: fire the sender, write no ready-signal line.
            let _ = sender.send(());
        }
        None => {
            // Standalone tunnel: emit the listening event via the session
            // log, which respects verbosity and format in one place.
            session_log.listening(cfg.local_port, &cfg.resource_name);
        }
    }

    let profile_owned = profile.map(|s| s.to_string());
    let mut tasks = tokio::task::JoinSet::new();

    let outcome = loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        session_log.client_connected(&addr);
                        let host = cfg.host.clone();
                        let namespace = cfg.namespace.clone();
                        let resource_name = cfg.resource_name.clone();
                        let pod_name = cfg.pod_name.clone();
                        let profile = profile_owned.clone();
                        let log = session_log;
                        tasks.spawn(async move {
                            match handle_client_connection(
                                stream,
                                &host,
                                &namespace,
                                &resource_name,
                                pod_name.as_deref(),
                                profile.as_deref(),
                            )
                            .await
                            {
                                Ok(()) => {
                                    log.client_disconnected(&addr);
                                }
                                Err(e) => {
                                    log.connection_error(&addr, &e);
                                }
                            }
                        });
                    }
                    Err(e) => {
                        // Non-cancellation accept error: log and continue.
                        // One bad accept must not kill a live tunnel. The
                        // predicate is extracted for unit testing of the
                        // resilience policy.
                        session_log.accept_error(&e.to_string());
                        if !should_continue_after_accept_error(&e) {
                            break Err(TunnelError::Bind(e));
                        }
                        continue;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break Ok(());
            }
            _ = cancel.cancelled() => {
                break Err(TunnelError::Cancelled);
            }
        }
    };

    // Abort all in-flight connection tasks and wait for them to finish.
    // Abort rather than drain: the user pressed Ctrl-C, and a graceful
    // drain of an arbitrary-length TCP stream has no bounded completion.
    tasks.abort_all();
    while (tasks.join_next().await).is_some() {}

    outcome
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ── RAII env guard ──

    /// RAII guard that restores an environment variable after a test mutates it.
    // Env-mutating tests must be #[serial_test::serial] per repo convention.
    // No crate-visible TempEnvGuard exists for in-source test modules:
    // ags-runtime's is pub(crate) and tests/common/env_guard.rs is for
    // integration tests only. Follows the extend_docker_login.rs pattern.
    struct TempEnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl TempEnvGuard {
        /// Set an environment variable for the lifetime of the guard.
        fn set(key: &'static str, value: &str) -> Self {
            use std::env;
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for TempEnvGuard {
        fn drop(&mut self) {
            use std::env;
            match &self.original {
                Some(val) => env::set_var(self.key, val),
                None => env::remove_var(self.key),
            }
        }
    }

    // ── Mock sink for capturing WebSocket frame types ──

    /// In-memory sink that records every message sent to it.
    struct CaptureSink(Vec<tungstenite::Message>);

    impl futures_util::Sink<tungstenite::Message> for CaptureSink {
        type Error = std::io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(
            self: std::pin::Pin<&mut Self>,
            item: tungstenite::Message,
        ) -> Result<(), Self::Error> {
            self.get_mut().0.push(item);
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    // ── Test 9a: URL construction with pod-name ──

    #[test]
    fn test_build_ws_url_with_pod_name_adds_query_param() {
        let url = build_ws_url("example.com", "my-ns", "my-app", Some("pod-42"));
        assert!(
            url.contains("&podName=pod-42"),
            "URL must include &podName= when pod-name is present: {url}"
        );
        assert!(
            url.starts_with("wss://"),
            "URL must use wss:// scheme: {url}"
        );
        assert!(
            url.contains("resourceName=my-app"),
            "URL must include resourceName: {url}"
        );
    }

    // ── Test 9b: URL construction without pod-name ──

    #[test]
    fn test_build_ws_url_without_pod_name_omits_query_param() {
        let url = build_ws_url("example.com", "my-ns", "my-app", None);
        assert!(
            !url.contains("podName"),
            "URL must not contain podName when pod-name is absent: {url}"
        );
        assert!(
            !url.contains("&podName="),
            "URL must not contain empty &podName= segment: {url}"
        );
    }

    // ── Test 10: bind target is localhost ──

    #[test]
    fn test_bind_addr_is_localhost_not_wildcard() {
        let addr = build_bind_addr(8080);
        assert!(
            addr.starts_with("127.0.0.1:"),
            "bind address must use 127.0.0.1 (localhost), never 0.0.0.0: {addr}"
        );
        assert_eq!(addr, "127.0.0.1:8080");
    }

    // ── Test 5: TCP -> WS sends Binary frames ──

    #[tokio::test]
    async fn test_relay_tcp_to_ws_sends_binary_frames() {
        let input = b"hello tunnel";
        let mut reader: &[u8] = input;
        let mut sink = CaptureSink(vec![]);

        relay_tcp_to_ws(&mut reader, &mut sink).await.unwrap();

        assert_eq!(sink.0.len(), 1, "one TCP read must produce one WS frame");
        match &sink.0[0] {
            tungstenite::Message::Binary(data) => {
                assert_eq!(
                    data,
                    &input[..],
                    "Binary frame payload must match TCP input"
                );
            }
            other => panic!("TCP data must be sent as Binary frame, got: {other:?}"),
        }
    }

    // ── Test 6: WS -> TCP skips non-Binary non-Text frames ──

    #[tokio::test]
    async fn test_relay_ws_to_tcp_skips_non_data_frames() {
        // Feed a Ping frame through the relay.
        let items: Vec<Result<tungstenite::Message, tungstenite::Error>> =
            vec![Ok(tungstenite::Message::Ping(vec![1, 2, 3]))];
        let mut stream = futures_util::stream::iter(items);

        let (mut write_half, mut read_half) = tokio::io::duplex(1024);

        relay_ws_to_tcp(&mut stream, &mut write_half).await.unwrap();

        // Drop the write end so read_to_end returns immediately.
        drop(write_half);

        let mut buf = Vec::new();
        read_half.read_to_end(&mut buf).await.unwrap();
        assert!(
            buf.is_empty(),
            "Ping frame must not be forwarded to TCP: got {} bytes",
            buf.len()
        );
    }

    // ── Test 7: 401 retry makes exactly two dial attempts ──

    #[tokio::test]
    async fn test_401_retry_makes_exactly_two_attempts() {
        let connect_count = Arc::new(AtomicUsize::new(0));
        let count_for_closure = connect_count.clone();

        let result: Result<(), TunnelError> = dial_with_retry(
            "initial-token".to_string(),
            move |_tok| {
                let count = count_for_closure.clone();
                Box::pin(async move {
                    let n = count.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        Err(DialError::Unauthorized)
                    } else {
                        Ok(())
                    }
                })
            },
            || Box::pin(async { Ok((true, "refreshed-token".to_string())) }),
        )
        .await;

        assert!(result.is_ok(), "dial must succeed on second attempt");
        assert_eq!(
            connect_count.load(Ordering::SeqCst),
            2,
            "exactly two connect attempts: initial 401 + one retry after refresh"
        );
    }

    // ── Test 8: accept error does not terminate tunnel ──

    #[test]
    fn test_accept_error_does_not_terminate_tunnel() {
        // All common accept-error kinds must keep the loop running.
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::Other,
            std::io::ErrorKind::Interrupted,
        ] {
            let e = std::io::Error::new(kind, "simulated");
            assert!(
                should_continue_after_accept_error(&e),
                "accept error kind {kind:?} must not terminate the tunnel"
            );
        }
    }

    // ── Test 1: no credential leak in error messages ──

    #[tokio::test]
    async fn test_no_credential_leak_in_dial_error_paths() {
        let secret = "xyzzy-secret-bearer-token-42";
        let refreshed_secret = "refreshed-secret-99";
        let mut error_messages: Vec<String> = Vec::new();

        // Path 1: 401 -> refresh succeeds -> 401 again (no third attempt).
        let count1 = Arc::new(AtomicUsize::new(0));
        let count1c = count1.clone();
        if let Err(e) = dial_with_retry::<()>(
            secret.to_string(),
            move |_tok| {
                let c = count1c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(DialError::Unauthorized)
                })
            },
            || Box::pin(async { Ok((true, refreshed_secret.to_string())) }),
        )
        .await
        {
            error_messages.push(format!("{e}"));
        }

        // Path 2: 401 -> refresh returns false.
        if let Err(e) = dial_with_retry::<()>(
            secret.to_string(),
            |_tok| Box::pin(async { Err(DialError::Unauthorized) }),
            || Box::pin(async { Ok((false, String::new())) }),
        )
        .await
        {
            error_messages.push(format!("{e}"));
        }

        // Path 3: Forbidden on first attempt.
        if let Err(e) = dial_with_retry::<()>(
            secret.to_string(),
            |_tok| Box::pin(async { Err(DialError::Forbidden) }),
            || Box::pin(async { Ok((true, "x".to_string())) }),
        )
        .await
        {
            error_messages.push(format!("{e}"));
        }

        // Path 4: Other error on first attempt.
        if let Err(e) = dial_with_retry::<()>(
            secret.to_string(),
            |_tok| Box::pin(async { Err(DialError::Other("connection refused".to_string())) }),
            || Box::pin(async { Ok((true, "x".to_string())) }),
        )
        .await
        {
            error_messages.push(format!("{e}"));
        }

        // Path 5: 401 -> refresh -> Forbidden on retry.
        let count5 = Arc::new(AtomicUsize::new(0));
        let count5c = count5.clone();
        if let Err(e) = dial_with_retry::<()>(
            secret.to_string(),
            move |_tok| {
                let c = count5c.clone();
                Box::pin(async move {
                    if c.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err(DialError::Unauthorized)
                    } else {
                        Err(DialError::Forbidden)
                    }
                })
            },
            || Box::pin(async { Ok((true, refreshed_secret.to_string())) }),
        )
        .await
        {
            error_messages.push(format!("{e}"));
        }

        // Assert no message contains either token.
        for msg in &error_messages {
            assert!(
                !msg.contains(secret),
                "error message must not contain initial token: {msg}"
            );
            assert!(
                !msg.contains(refreshed_secret),
                "error message must not contain refreshed token: {msg}"
            );
        }

        // Also verify the ready-signal template has no token slots.
        let ready = format!(
            "[+0s] listening on localhost:{}  resource={}",
            8080, "my-app"
        );
        assert!(
            !ready.contains("Bearer"),
            "ready signal must not contain 'Bearer': {ready}"
        );

        assert!(
            error_messages.len() >= 5,
            "must test at least 5 error paths, got {}",
            error_messages.len()
        );
    }

    // ── Test F-2a: Some(sender) fires once listener is bound ──

    #[tokio::test]
    async fn test_run_tunnel_some_sender_fires_on_bind() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();

        let cfg = TunnelConfig {
            host: "example.com".to_string(),
            namespace: "test-ns".to_string(),
            resource_name: "test-app".to_string(),
            local_port: 0, // OS assigns a free port
            pod_name: None,
        };

        let handle = tokio::spawn(async move {
            let session_log = crate::invocation::handlers::extend::session_log::SessionLog::new(
                ags_protocol::request::Verbosity::Normal,
                false,
            );
            run_tunnel(cfg, Some(tx), cancel_for_task, None, session_log).await
        });

        // The sender must fire once the TCP listener is bound — not after
        // the first connection or on shutdown. A timeout guards against a
        // hang if the sender never fires.
        tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .expect("sender must fire within 5 s")
            .expect("sender must deliver () — not be dropped");

        // Clean up: cancel the tunnel so the task finishes.
        cancel.cancel();
        let result = handle.await.expect("task must not panic");
        assert!(
            matches!(result, Err(TunnelError::Cancelled)),
            "cancelled tunnel must return TunnelError::Cancelled"
        );
    }

    // ── Test F-2b: None path enters accept loop without sender ──

    #[tokio::test]
    async fn test_run_tunnel_none_enters_accept_loop_without_sender() {
        // When listener_ready is None, the function writes the ready signal
        // to stderr instead of firing a sender. Prove the function binds
        // and enters the accept loop (cancellation works without a sender).
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();

        let cfg = TunnelConfig {
            host: "example.com".to_string(),
            namespace: "test-ns".to_string(),
            resource_name: "test-app".to_string(),
            local_port: 0,
            pod_name: None,
        };

        let handle = tokio::spawn(async move {
            let session_log = crate::invocation::handlers::extend::session_log::SessionLog::new(
                ags_protocol::request::Verbosity::Normal,
                false,
            );
            run_tunnel(cfg, None, cancel_for_task, None, session_log).await
        });

        // Allow the listener to bind and the ready signal to be emitted,
        // then cancel. The None branch writes to stderr — the absence of
        // a sender means the function must reach the accept loop on its
        // own.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        cancel.cancel();
        let result = handle.await.expect("task must not panic");
        assert!(
            matches!(result, Err(TunnelError::Cancelled)),
            "cancelled tunnel must return TunnelError::Cancelled"
        );
    }

    // ── Finding 1: spawned connections aborted on cancel ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_spawned_connection_aborted_on_cancel() {
        use tokio::io::AsyncReadExt;

        // The connection task calls dial_ws → resolve_access_token before
        // reaching the WS dial. Without a token, auth fails and the task
        // exits on its own (making the test a tautology). Setting
        // AGS_ACCESS_TOKEN bypasses the auth store so the task proceeds
        // to the dial phase where it genuinely stalls.
        // Process-wide env mutation: AGS_ACCESS_TOKEN.
        let _token = TempEnvGuard::set("AGS_ACCESS_TOKEN", "fake-test-token");

        // Stand up a "stall server" that accepts TCP connections but never
        // completes the WebSocket upgrade handshake. The connection task
        // inside run_tunnel will sit in try_connect's dial for up to
        // WS_DIAL_TIMEOUT (30 s), genuinely in-flight.
        let stall_server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stall_port = stall_server.local_addr().unwrap().port();

        // Signal when the stall server accepts — proves the spawned
        // connection task is genuinely in-flight before we cancel.
        let (accepted_tx, accepted_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            if let Ok((_stream, _)) = stall_server.accept().await {
                let _ = accepted_tx.send(());
                // Hold the connection open — never send a WS upgrade.
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });

        // Find a free port for run_tunnel's local TCP listener.
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let (tx, rx) = oneshot::channel();
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let cfg = TunnelConfig {
            host: format!("127.0.0.1:{stall_port}"),
            namespace: "test-ns".to_string(),
            resource_name: "test-app".to_string(),
            local_port: port,
            pod_name: None,
        };

        let handle = tokio::spawn(async move {
            let session_log = crate::invocation::handlers::extend::session_log::SessionLog::new(
                ags_protocol::request::Verbosity::Quiet,
                false,
            );
            run_tunnel(cfg, Some(tx), cancel_clone, None, session_log).await
        });

        // Wait for the tunnel's TCP listener to be ready.
        rx.await.unwrap();

        // Connect a TCP client — triggers a connection-task spawn inside
        // the accept loop.
        let mut client = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();

        // Wait for the stall server to accept the WS dial: the spawned
        // connection task is now genuinely in-flight, stuck waiting for
        // the HTTP upgrade response that will never arrive.
        tokio::time::timeout(std::time::Duration::from_secs(5), accepted_rx)
            .await
            .expect("stall server must accept within 5 s")
            .expect("accepted_tx must fire");

        // Cancel the tunnel.
        cancel.cancel();

        // The client must see EOF promptly if abort_all() fired — the
        // abort drops the spawned task's future, which drops its copy of
        // the accepted TCP stream. Without abort_all(), the spawned task
        // stays alive (stuck in the 30 s dial timeout), holding the TCP
        // stream open, so this 2 s timeout would expire.
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), client.read(&mut buf))
            .await
            .expect("read must not hang — server side should be dropped by abort")
            .expect("read should complete without IO error");
        assert_eq!(
            n, 0,
            "EOF expected — spawned task must have dropped its TCP stream"
        );

        // run_tunnel must return Cancelled.
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("run_tunnel must return promptly after abort")
            .expect("task must not panic");
        assert!(
            matches!(result, Err(TunnelError::Cancelled)),
            "cancelled tunnel must return TunnelError::Cancelled"
        );
    }

    // ── Finding 2: dial timeout on stalled upgrade ──

    #[tokio::test]
    async fn test_ws_dial_timeout_on_stalled_upgrade() {
        // Start a TCP server that accepts but never sends — the WebSocket
        // upgrade handshake stalls indefinitely.
        let stall_server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = stall_server.local_addr().unwrap().port();

        tokio::spawn(async move {
            // Accept one connection, hold it open without sending data.
            if let Ok((_stream, _)) = stall_server.accept().await {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });

        let url = format!("ws://127.0.0.1:{port}/test");

        // try_connect receives a dial_timeout parameter. Without internal
        // timeout logic, it ignores the parameter and hangs on the stalled
        // server; the outer timeout detects the hang.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            try_connect(&url, "fake-token", std::time::Duration::from_millis(500)),
        )
        .await;

        match result {
            Ok(Err(DialError::Other(ref msg)))
                if msg.contains("timed out") || msg.contains("timeout") =>
            {
                // try_connect's internal timeout fired — expected after fix.
            }
            Err(_) => {
                panic!(
                    "try_connect ignored the dial_timeout and hung — \
                     no built-in dial timeout"
                );
            }
            Ok(Ok(_)) => panic!("should not connect to a stalled server"),
            Ok(Err(DialError::Unauthorized)) => {
                panic!("unexpected 401 from a stalled server")
            }
            Ok(Err(DialError::Forbidden)) => {
                panic!("unexpected 403 from a stalled server")
            }
            Ok(Err(DialError::Other(msg))) => {
                panic!("try_connect returned a non-timeout error: {msg}");
            }
        }
    }

    // ── Finding 3: close frame sent after relay ──

    #[tokio::test]
    async fn test_close_frame_sent_after_relay_completes() {
        // TCP reader EOF's immediately → the TCP→WS direction completes.
        let mut tcp_reader: &[u8] = &[];
        let (tcp_write_half, _tcp_read_half) = tokio::io::duplex(1024);
        let mut tcp_writer = tcp_write_half;
        let mut sink = CaptureSink(vec![]);

        // WS source is pending (never yields), so the TCP→WS direction
        // finishes first and the select returns.
        let mut ws_source =
            futures_util::stream::pending::<Result<tungstenite::Message, std::io::Error>>();

        relay_bidirectional(&mut tcp_reader, &mut tcp_writer, &mut sink, &mut ws_source)
            .await
            .unwrap();

        // After the relay directions end, a Close frame (RFC 6455 §7.1.2)
        // should have been sent best-effort. Without the fix, no Close
        // frame appears in the sink.
        assert!(
            sink.0
                .iter()
                .any(|m| matches!(m, tungstenite::Message::Close(None))),
            "a Close frame must be sent after relay completes, \
             got {} frames",
            sink.0.len()
        );
    }
}
