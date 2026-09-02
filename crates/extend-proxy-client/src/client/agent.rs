//! Client connection lifecycle: dials the sidecar's WebSocket endpoint,
//! performs the session handshake, and runs the session until it ends.
//! Ported from Go's `pkg/client/client.go`. Reconnection is the caller's
//! responsibility, same as Go — this module only manages a single
//! connection attempt per `Agent::run` call.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_util::sync::CancellationToken;

use crate::protocol::MAX_FRAME_SIZE_BYTES;
use crate::session::{BoxFuture, HandshakeError, InterceptTargetToLocalMapping, Session};

use super::session_holder::SessionHolder;

/// Environment variable consulted for a bearer token when no
/// [`Config::token_provider`] is supplied. Mirrors Go's fallback to
/// `os.Getenv("AB_ACCESS_TOKEN")`.
const ACCESS_TOKEN_ENV_VAR: &str = "AB_ACCESS_TOKEN";

/// A local port the client is allowed to dial for a sidecar-initiated
/// stream. Mirrors Go's `PortMapping`.
#[derive(Debug, Clone)]
pub struct PortMapping {
    pub target_port: i32,
    pub local_addr: String,
}

/// Supplies a bearer token on each connection attempt, so a caller can
/// return a freshly-refreshed token on every reconnect. The provided
/// [`CancellationToken`] mirrors Go's `TokenProvider func(ctx context.Context) string` —
/// an implementation that fetches a token remotely can honor it to abort
/// promptly on session shutdown instead of blocking the connect path.
pub type TokenProvider = Arc<dyn Fn(CancellationToken) -> BoxFuture<String> + Send + Sync>;

/// Client session configuration. Mirrors Go's `client.Config`.
pub struct Config {
    // nosemgrep -- doc example only; see extend-proxy-client status in CONTRIBUTING.md for why production always dials ws:// on loopback
    /// WebSocket base URL, e.g. `"ws://localhost:15080/tunnel"`.
    pub sidecar_ws: String,
    /// Local ports the client is allowed to dial.
    pub allow_ports: Vec<PortMapping>,
    /// Timeout waiting for SESSION_ACK.
    pub session_init_timeout: Duration,
    /// Called on each connection attempt to supply a bearer token; falls
    /// back to [`ACCESS_TOKEN_ENV_VAR`] when `None`.
    pub token_provider: Option<TokenProvider>,
    /// Receives a session each time one is fully established (after
    /// SESSION_ACK), so the caller can consume it without blocking the
    /// agent. Mirrors Go's `SessionReadyChan`.
    pub session_ready_tx: Option<mpsc::Sender<Arc<Session>>>,
}

/// Failure connecting to or establishing a session with the sidecar.
/// Mirrors the error paths of Go's `Agent.connect`.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("invalid websocket request: {0}")]
    InvalidRequest(String),
    #[error("websocket dial failed: {0}")]
    Dial(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("websocket dial rejected: status={status} message={message:?}")]
    DialRejected {
        status: u16,
        message: Option<String>,
    },
    #[error("session handshake failed: {0}")]
    Handshake(#[from] HandshakeError),
    #[error("connection cancelled")]
    Cancelled,
}

/// Matches the sidecar's JSON error body shape (`errorCode`/`errorMessage`),
/// used to surface a human-readable message on a rejected dial (e.g. a 409
/// when a debug session is already active on the sidecar).
#[derive(Debug, Default, Deserialize)]
struct SidecarApiError {
    #[serde(rename = "errorMessage", default)]
    error_message: String,
}

/// Manages the client connection lifecycle, including exposing the current
/// session via a [`SessionHolder`]. Mirrors Go's `Agent`.
pub struct Agent {
    cfg: Config,
    sessions: Arc<SessionHolder>,
}

impl Agent {
    /// Creates a new client `Agent`.
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            sessions: Arc::new(SessionHolder::new()),
        }
    }

    /// The `SessionHolder` tracking the current active session.
    pub fn sessions(&self) -> Arc<SessionHolder> {
        self.sessions.clone()
    }

    /// Connects to the sidecar for a single session attempt and returns when
    /// the session ends or `cancel` fires. Reconnection is the caller's
    /// responsibility. Mirrors Go's `Agent.Run`.
    pub async fn run(&self, cancel: CancellationToken) -> Result<(), AgentError> {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let result = self.connect(cancel).await;
        if let Err(error) = &result {
            tracing::warn!(%error, "session ended");
        }
        result
    }

    /// Dials the sidecar, performs the session handshake, and runs the
    /// session (passively — the client only responds to server PING with
    /// PONG) until it ends or `cancel` fires. Mirrors Go's `Agent.connect`.
    async fn connect(&self, cancel: CancellationToken) -> Result<(), AgentError> {
        let url = &self.cfg.sidecar_ws;
        tracing::debug!(%url, "connecting to sidecar");

        let token = match &self.cfg.token_provider {
            Some(provider) => provider(cancel.clone()).await,
            None => std::env::var(ACCESS_TOKEN_ENV_VAR).unwrap_or_default(),
        };

        let mut request = self
            .cfg
            .sidecar_ws
            .as_str()
            .into_client_request()
            .map_err(|e| AgentError::InvalidRequest(e.to_string()))?;
        if !token.is_empty() {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| AgentError::InvalidRequest(e.to_string()))?;
            request.headers_mut().insert(AUTHORIZATION, value);
        }

        // Bounds incoming message/frame size the same way Go's
        // `wsConn.SetReadLimit(maxWsMessageBytes)` does — without this,
        // tungstenite's defaults (64 MiB message / 16 MiB frame) apply, far
        // beyond the protocol's own 64 KiB `MAX_FRAME_SIZE_BYTES` limit.
        let ws_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
            max_message_size: Some(MAX_FRAME_SIZE_BYTES),
            max_frame_size: Some(MAX_FRAME_SIZE_BYTES),
            ..Default::default()
        };

        // Race the dial against `cancel`, mirroring Go's `websocket.Dial(ctx, ...)`
        // aborting promptly when the caller's context is cancelled mid-dial.
        let dial_result = tokio::select! {
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            result = tokio_tungstenite::connect_async_with_config(request, Some(ws_config), false) => result,
        };
        let (ws_stream, _response) = match dial_result {
            Ok(pair) => pair,
            Err(e) => {
                let error = classify_dial_error(e);
                match &error {
                    AgentError::DialRejected {
                        status: 409,
                        message,
                    } => {
                        tracing::warn!(
                            status = 409,
                            ?message,
                            "WebSocket upgrade rejected: session already active on sidecar"
                        );
                    }
                    AgentError::DialRejected { status, message } => {
                        tracing::warn!(status, ?message, "WebSocket dial failed");
                    }
                    AgentError::Dial(dial_error) => {
                        tracing::warn!(error = %dial_error, "WebSocket dial failed");
                    }
                    _ => {}
                }
                return Err(error);
            }
        };

        let ports: InterceptTargetToLocalMapping = self
            .cfg
            .allow_ports
            .iter()
            .map(|p| (p.target_port, p.local_addr.clone()))
            .collect();

        // Race the handshake against `cancel` too, mirroring Go's `createCtx`
        // (derived from the caller's ctx) racing `tunnel.SendSessionToServer`.
        let handshake_result = tokio::select! {
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            result = Session::send_session_to_server(ws_stream, ports, self.cfg.session_init_timeout) => result,
        };
        let session = match handshake_result {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(%error, "session initialization failed");
                return Err(error.into());
            }
        };
        self.sessions.set(session.clone());

        if let Some(tx) = &self.cfg.session_ready_tx {
            let tx = tx.clone();
            let ready_session = session.clone();
            let notify_cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = tx.send(ready_session) => {}
                    _ = notify_cancel.cancelled() => {}
                }
            });
        }

        tokio::select! {
            _ = cancel.cancelled() => session.cancel_token().cancel(),
            _ = session.clone().run(Duration::ZERO, Duration::ZERO) => {}
        }

        self.sessions.clear();
        Ok(())
    }
}

/// Classifies a failed WebSocket dial, extracting an error message (if any)
/// from a non-101 HTTP response. Mirrors the response-body parsing in Go's
/// `Agent.connect`. The IAM auth filter's 4xx responses are JSON
/// (`SidecarApiError`), but the sidecar's single-session 409 is a plain-text
/// body (`net/http`'s `http.Error`) — fall back to it verbatim so that case
/// still surfaces a message instead of silently discarding it.
fn classify_dial_error(err: tokio_tungstenite::tungstenite::Error) -> AgentError {
    if let tokio_tungstenite::tungstenite::Error::Http(response) = &err {
        let status = response.status().as_u16();
        let body = response.body().as_deref().unwrap_or(&[]);
        let message = serde_json::from_slice::<SidecarApiError>(body)
            .ok()
            .map(|e| e.error_message)
            .filter(|m| !m.is_empty())
            .or_else(|| {
                (!body.is_empty()).then(|| String::from_utf8_lossy(body).trim().to_string())
            })
            .filter(|m| !m.is_empty());
        return AgentError::DialRejected { status, message };
    }
    AgentError::Dial(Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::http::Response;

    fn test_config(sidecar_ws: String) -> Config {
        Config {
            sidecar_ws,
            allow_ports: Vec::new(),
            session_init_timeout: Duration::from_secs(5),
            token_provider: None,
            session_ready_tx: None,
        }
    }

    /// A pre-cancelled token must short-circuit `run` with `Ok(())` rather
    /// than attempting a connection — asserted explicitly so a future
    /// refactor of this early-return can't silently turn it into a hang.
    #[tokio::test]
    async fn test_run_returns_ok_immediately_when_already_cancelled() {
        // nosemgrep -- unit-test fixture: loopback 127.0.0.1, never leaves the machine
        let agent = Agent::new(test_config("ws://127.0.0.1:1/tunnel".to_string()));
        let cancel = CancellationToken::new();
        cancel.cancel();

        let result = tokio::time::timeout(Duration::from_millis(200), agent.run(cancel))
            .await
            .expect("run did not return promptly for an already-cancelled token");
        assert!(matches!(result, Ok(())));
    }

    /// Cancelling mid-dial (the `tokio::select!` racing the WebSocket dial
    /// against `cancel` in `connect`) must surface `AgentError::Cancelled`
    /// promptly rather than hanging until the dial itself times out or
    /// errors. Dials a listener that accepts the TCP connection but never
    /// completes the WebSocket upgrade, so only cancellation can end the race.
    #[tokio::test]
    async fn test_connect_cancelled_mid_dial_returns_cancelled_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stalling listener");
        let addr = listener.local_addr().expect("local_addr");
        // Held for the test's duration without ever calling `accept()`, so
        // the TCP handshake completes but the WebSocket upgrade never does.
        let _listener = listener;

        // nosemgrep -- test fixture: loopback listener that never completes the upgrade; the production socket URL is caller-supplied
        let agent = Agent::new(test_config(format!("ws://{addr}/tunnel")));
        let cancel = CancellationToken::new();
        let run_cancel = cancel.clone();

        let run_handle = tokio::spawn(async move { agent.run(run_cancel).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();

        let result = tokio::time::timeout(Duration::from_millis(500), run_handle)
            .await
            .expect("run did not return promptly after cancellation")
            .expect("run task panicked");
        assert!(
            matches!(result, Err(AgentError::Cancelled)),
            "expected Err(AgentError::Cancelled), got {result:?}"
        );
    }

    /// A 409 (session already active on the sidecar) with a structured JSON
    /// body surfaces the sidecar's `errorMessage` field.
    #[test]
    fn test_classify_dial_error_extracts_sidecar_error_message() {
        let body =
            br#"{"errorCode":10142,"errorMessage":"a debug session is already running"}"#.to_vec();
        let response = Response::builder().status(409).body(Some(body)).unwrap();
        let err = classify_dial_error(tokio_tungstenite::tungstenite::Error::Http(response));

        match err {
            AgentError::DialRejected { status, message } => {
                assert_eq!(status, 409);
                assert_eq!(
                    message.as_deref(),
                    Some("a debug session is already running")
                );
            }
            other => panic!("expected DialRejected, got {other:?}"),
        }
    }

    /// The real sidecar's single-session 409 is plain text
    /// (`http.Error(w, wsSessionConflictMsg, http.StatusConflict)`), not
    /// JSON — it must still surface as `message`, not `None`.
    #[test]
    fn test_classify_dial_error_falls_back_to_plain_text_body() {
        let body =
            b"a debug session is already running; disconnect the existing client first".to_vec();
        let response = Response::builder().status(409).body(Some(body)).unwrap();
        let err = classify_dial_error(tokio_tungstenite::tungstenite::Error::Http(response));

        match err {
            AgentError::DialRejected { status, message } => {
                assert_eq!(status, 409);
                assert_eq!(
                    message.as_deref(),
                    Some(
                        "a debug session is already running; disconnect the existing client first"
                    )
                );
            }
            other => panic!("expected DialRejected, got {other:?}"),
        }
    }

    /// A non-JSON or empty body yields `None` rather than an error.
    #[test]
    fn test_classify_dial_error_handles_missing_body() {
        let response = Response::builder().status(500).body(None).unwrap();
        let err = classify_dial_error(tokio_tungstenite::tungstenite::Error::Http(response));

        match err {
            AgentError::DialRejected { status, message } => {
                assert_eq!(status, 500);
                assert!(message.is_none());
            }
            other => panic!("expected DialRejected, got {other:?}"),
        }
    }

    /// A non-HTTP transport error (e.g. connection refused) is not misclassified.
    #[test]
    fn test_classify_dial_error_passes_through_non_http_errors() {
        let err = classify_dial_error(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
        assert!(matches!(err, AgentError::Dial(_)));
    }
}
