//! Session state machine: one active WebSocket debug session, multiplexing
//! many logical streams plus a session-level RPC channel. Ported from Go's
//! `pkg/tunnel/session.go` (+ `rpc_typed.go`'s wrapper concept, in `rpc.rs`).
//!
//! Only the client role is a supported public entry point
//! ([`Session::send_session_to_server`]); the server role
//! ([`Session::wait_session_from_client`]) exists so this crate's own tests
//! can stand in for the sidecar counterpart without the real Go test-peer
//! (not yet wired up — see the crate-level doc comment).
//!
//! The transport's concrete stream type (`S`) and each logical stream's
//! local connection type (`C`) are both generic at the API boundary but
//! erased before being stored, so `Session` itself stays a single,
//! non-generic type usable with a real WebSocket in production and an
//! in-memory duplex pair in tests.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use futures_util::{FutureExt, SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::sync::CancellationToken;

use crate::protocol::{
    self, reason, CmdResponsePayload, Frame, FrameBody, NackPayload, OpenPayload,
    SessionInitPayload, MAX_FRAME_SIZE_BYTES, OUTBOUND_CHANNEL_CAPACITY, PROTOCOL_VERSION,
    SESSION_INIT_TIMEOUT,
};
use crate::session::idgen::{IdGenerator, IdParity};
use crate::session::stream::{BoxFuture, DuplexConn, Stream};

/// Key: target port, value: full local address (`host:port`) or a bare port
/// number (dialed on loopback). Mirrors `tunnel.InterceptTargetToLocalMapping`.
pub type InterceptTargetToLocalMapping = HashMap<i32, String>;

/// Callee-side handler for an RPC command: takes the raw JSON params and
/// returns a JSON-serializable response or an error message. Mirrors Go's
/// `CommandHandler` (`func(ctx, json.RawMessage) (any, error)`).
pub type CommandHandler = Arc<
    dyn Fn(Option<serde_json::Value>) -> BoxFuture<Result<Option<serde_json::Value>, String>>
        + Send
        + Sync,
>;

/// Dials a local connection for a given target host/port, returning it
/// type-erased. Mirrors Go's `DialFunc`.
pub type DialFn =
    Arc<dyn Fn(String, i32) -> BoxFuture<std::io::Result<Box<dyn DuplexConn>>> + Send + Sync>;

type BoxedWsSource = Pin<
    Box<
        dyn futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + Send,
    >,
>;

/// Which side of the handshake a `Session` plays. Mirrors Go's `SessionRole`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Client,
    Server,
}

/// Validates whether a target host:port combination is allowed. Implements
/// server-side access control to prevent SSRF attacks. Mirrors Go's
/// `TargetAllowlist`. Only exercised by this crate's server-role test
/// double; the client role never validates outbound targets itself.
#[derive(Debug, Clone)]
pub struct TargetAllowlist {
    allowed_targets: HashSet<String>,
}

impl TargetAllowlist {
    /// Creates an allowlist from a list of `"host:port"` strings.
    pub fn new(targets: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed_targets: targets.into_iter().collect(),
        }
    }

    /// Checks whether a host:port combination is in the allowlist.
    pub fn allows_target(&self, host: &str, port: i32) -> Result<(), String> {
        let target = join_host_port(host, port);
        if self.allowed_targets.contains(&target) {
            Ok(())
        } else {
            Err(format!("target {target:?} not in allowlist"))
        }
    }
}

/// Joins a host and port into a `"host:port"` string, bracketing the host
/// when it looks like a literal IPv6 address (contains `:`). Mirrors Go's
/// `net.JoinHostPort`.
fn join_host_port(host: &str, port: i32) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Failure establishing a session handshake. Mirrors the error paths of
/// `tunnel.SendSessionToServer`/`tunnel.WaitSessionFromClient`.
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("websocket transport error: {0}")]
    Transport(Box<tokio_tungstenite::tungstenite::Error>),
    #[error(transparent)]
    Decode(#[from] protocol::DecodeError),
    #[error(transparent)]
    Encode(#[from] protocol::EncodeError),
    #[error("invalid SESSION_INIT frame")]
    InvalidSessionInit,
    #[error("invalid SESSION_ACK frame")]
    InvalidSessionAck,
    #[error("version mismatch: server={server} client={client}")]
    VersionMismatch { server: i64, client: i64 },
    #[error("connection closed during handshake")]
    ConnectionClosed,
    #[error("handshake timed out")]
    Timeout,
}

// Manual `From` impl so that `?` on a bare `tungstenite::Error` still works
// after boxing the `Transport` variant. `#[from]` on `Box<E>` would generate
// `From<Box<E>>`, not `From<E>`, breaking every existing `?` site.
impl From<tokio_tungstenite::tungstenite::Error> for HandshakeError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        HandshakeError::Transport(Box::new(err))
    }
}

/// Failure calling or serving an RPC. Mirrors `Session.CallRPC`'s error paths
/// plus the (de)serialization failures `tunnel.CallRPCTyped` can add.
#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("rpc call timed out")]
    Timeout,
    #[error("session closed")]
    SessionClosed,
    #[error("{0}")]
    Callee(String),
    #[error("serialize rpc params: {0}")]
    Serialize(serde_json::Error),
    #[error("deserialize rpc response: {0}")]
    Deserialize(serde_json::Error),
}

/// Returned by [`Session::send_session_ack`] when called on a non-server-role session.
#[derive(Debug, thiserror::Error)]
pub enum SessionAckError {
    #[error("only the server can send SESSION_ACK to a SESSION_INIT request")]
    NotServer,
    #[error("session closed")]
    SessionClosed,
}

struct HeartbeatState {
    awaiting_pong: bool,
    pong_deadline: Instant,
}

struct RpcResult {
    response: Option<serde_json::Value>,
    error: Option<String>,
}

/// Manages one active WebSocket debug session. Mirrors Go's `Session`.
pub struct Session {
    id: String,
    role: SessionRole,
    outbound_tx: mpsc::Sender<Frame>,
    /// Taken exactly once, by `run`.
    pending_source: StdMutex<Option<BoxedWsSource>>,
    cancel: CancellationToken,

    streams: StdRwLock<HashMap<u64, Arc<Stream>>>,
    stream_id_gen: StdMutex<IdGenerator>,
    /// True if frames received from the peer are expected to carry odd stream IDs.
    expect_odd_from_peer: bool,

    idle_timeout: Duration,
    /// Millis since `UNIX_EPOCH`, updated on every frame received.
    last_activity: AtomicI64,

    heartbeat: StdMutex<HeartbeatState>,

    handlers: StdRwLock<HashMap<String, CommandHandler>>,
    in_flight_calls: StdMutex<HashMap<u64, oneshot::Sender<RpcResult>>>,
    next_cmd_id: AtomicU64,
    /// Limits concurrently-running CMD handlers, mirroring Go's `cmdSem` channel.
    cmd_semaphore: Arc<Semaphore>,

    dial: DialFn,
    #[allow(dead_code)] // retained for parity with Go's field; dial closures already capture it
    server_target_allowlist: Option<TargetAllowlist>,
}

impl Session {
    /// This session's server-assigned ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// A clone of this session's cancellation token; cancelling it stops
    /// `run`'s reader/heartbeat/cleanup loops. The Rust equivalent of
    /// passing a cancellable `context.Context` into Go's `Run`.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Client-side handshake: sends SESSION_INIT, waits for SESSION_ACK, and
    /// spawns the writer task. Mirrors `tunnel.SendSessionToServer`. This is
    /// the only handshake entry point the production `client::Agent` uses.
    pub async fn send_session_to_server<S>(
        ws: WebSocketStream<S>,
        ports: InterceptTargetToLocalMapping,
        init_timeout: Duration,
    ) -> Result<Arc<Session>, HandshakeError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sink, source) = ws.split();
        let mut source = source.boxed();

        let intercept_ports: Vec<i32> = ports.keys().copied().collect();
        tracing::info!(?intercept_ports, "SESSION_INIT sent");
        let init_frame =
            Frame::new_session_init(i64::from(PROTOCOL_VERSION), Vec::new(), intercept_ports);
        send_message(&mut sink, &init_frame).await?;

        let ack_frame = tokio::time::timeout(init_timeout, recv_frame(&mut source))
            .await
            .map_err(|_| HandshakeError::Timeout)??;

        let (session_id, server_version, idle_timeout_sec) = match ack_frame.body {
            FrameBody::SessionAck(Some(payload)) => (
                payload.session_id,
                payload.server_version,
                payload.idle_timeout_sec,
            ),
            _ => return Err(HandshakeError::InvalidSessionAck),
        };
        tracing::info!(session_id = %session_id, server_version, idle_timeout_sec, "SESSION_ACK received");

        let dial: DialFn = Arc::new(move |_host: String, port: i32| {
            let ports = ports.clone();
            Box::pin(async move { dial_via_port_mapping(&ports, port).await }) as BoxFuture<_>
        });

        let cancel = CancellationToken::new();
        let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
        tokio::spawn(writer_task(sink, outbound_rx, cancel.clone()));

        let session = Arc::new(Session::new_internal(
            session_id,
            SessionRole::Client,
            outbound_tx,
            source,
            cancel,
            Duration::from_secs(idle_timeout_sec.max(0) as u64),
            dial,
            None,
        ));

        Ok(session)
    }

    /// Server-side handshake: waits for SESSION_INIT and spawns the writer
    /// task, returning the session (not yet ACKed — call
    /// [`Session::send_session_ack`] next) and the decoded init payload.
    /// Mirrors `tunnel.WaitSessionFromClient`. Server-role construction is a
    /// test-only stand-in for the sidecar (see the module doc comment).
    pub async fn wait_session_from_client<S>(
        ws: WebSocketStream<S>,
        idle_timeout: Duration,
        allowlist: Option<TargetAllowlist>,
    ) -> Result<(Arc<Session>, SessionInitPayload), HandshakeError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (sink, source) = ws.split();
        let mut source = source.boxed();

        // Mirrors Go's `SessionInitTimeout`, which `sidecar/server.go` now
        // wraps `r.Context()` with via `context.WithTimeout` before calling
        // `WaitSessionFromClient` — a slow or stalled client can't hang this
        // handshake (or the single session slot) forever on either side.
        let init_frame = tokio::time::timeout(SESSION_INIT_TIMEOUT, recv_frame(&mut source))
            .await
            .map_err(|_| HandshakeError::Timeout)??;

        let init_payload = match init_frame.body {
            FrameBody::SessionInit(Some(payload)) => payload,
            _ => return Err(HandshakeError::InvalidSessionInit),
        };
        if init_payload.client_version != i64::from(PROTOCOL_VERSION) {
            return Err(HandshakeError::VersionMismatch {
                server: i64::from(PROTOCOL_VERSION),
                client: init_payload.client_version,
            });
        }

        let session_id = generate_session_id();
        let dial: DialFn = {
            let allowlist = allowlist.clone();
            Arc::new(move |host: String, port: i32| {
                let allowlist = allowlist.clone();
                Box::pin(async move { dial_with_allowlist(allowlist.as_ref(), host, port).await })
                    as BoxFuture<_>
            })
        };

        let cancel = CancellationToken::new();
        let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
        tokio::spawn(writer_task(sink, outbound_rx, cancel.clone()));

        let session = Arc::new(Session::new_internal(
            session_id,
            SessionRole::Server,
            outbound_tx,
            source,
            cancel,
            idle_timeout,
            dial,
            allowlist,
        ));

        Ok((session, init_payload))
    }

    /// Constructs the shared internal state. Not part of the public API —
    /// callers always go through one of the two handshake functions above.
    #[allow(clippy::too_many_arguments)]
    fn new_internal(
        id: String,
        role: SessionRole,
        outbound_tx: mpsc::Sender<Frame>,
        pending_source: BoxedWsSource,
        cancel: CancellationToken,
        idle_timeout: Duration,
        dial: DialFn,
        server_target_allowlist: Option<TargetAllowlist>,
    ) -> Self {
        let (stream_id_gen, expect_odd_from_peer) = match role {
            SessionRole::Client => (IdGenerator::new(IdParity::Even), true),
            SessionRole::Server => (IdGenerator::new(IdParity::Odd), false),
        };
        Self {
            id,
            role,
            outbound_tx,
            pending_source: StdMutex::new(Some(pending_source)),
            cancel,
            streams: StdRwLock::new(HashMap::new()),
            stream_id_gen: StdMutex::new(stream_id_gen),
            expect_odd_from_peer,
            idle_timeout,
            last_activity: AtomicI64::new(now_millis()),
            heartbeat: StdMutex::new(HeartbeatState {
                awaiting_pong: false,
                pong_deadline: Instant::now(),
            }),
            handlers: StdRwLock::new(HashMap::new()),
            in_flight_calls: StdMutex::new(HashMap::new()),
            next_cmd_id: AtomicU64::new(1),
            cmd_semaphore: Arc::new(Semaphore::new(32)),
            dial,
            server_target_allowlist,
        }
    }

    /// Sends SESSION_ACK. Only valid for the server role. Mirrors `Session.SendSessionAck`.
    pub async fn send_session_ack(&self) -> Result<(), SessionAckError> {
        if self.role != SessionRole::Server {
            return Err(SessionAckError::NotServer);
        }
        let ack = Frame::new_session_ack(
            i64::from(PROTOCOL_VERSION),
            self.id.clone(),
            self.idle_timeout.as_secs() as i64,
        );
        self.write_frame(ack)
            .await
            .map_err(|_| SessionAckError::SessionClosed)
    }

    /// Registers a handler for the given command name on this session's
    /// callee side. Registering the same name twice overwrites the previous
    /// handler. Mirrors `Session.RegisterCommandHandler`.
    pub fn register_command_handler(&self, name: impl Into<String>, handler: CommandHandler) {
        self.handlers.write().unwrap().insert(name.into(), handler);
    }

    /// Sends a CMD frame to the remote peer and waits for a CMD_RESPONSE or
    /// `timeout` to elapse. Mirrors `Session.CallRPC` (using a `Duration`
    /// instead of a generic cancellable context — the only cancellation
    /// shape Go's own test suite exercises here).
    pub async fn call_rpc(
        &self,
        name: impl Into<String>,
        params: Option<serde_json::Value>,
        timeout: Duration,
    ) -> Result<Option<serde_json::Value>, RpcError> {
        // `fetch_add` returns the pre-increment value; `next_cmd_id` starts
        // at 1, so the `+ 1` yields the post-increment id — ids start at 2
        // and are never 1, matching Go's `CallRPC` id sequence.
        let cmd_id = self.next_cmd_id.fetch_add(1, Ordering::SeqCst) + 1;
        let frame = Frame::new_cmd(cmd_id, name, params);

        let (tx, rx) = oneshot::channel();
        // nosemgrep -- mutex lock: only fails on poisoning by a panicking thread; propagating is not actionable
        self.in_flight_calls.lock().unwrap().insert(cmd_id, tx);

        if self.write_frame(frame).await.is_err() {
            // nosemgrep -- mutex lock: only fails on poisoning by a panicking thread; propagating is not actionable
            self.in_flight_calls.lock().unwrap().remove(&cmd_id);
            return Err(RpcError::SessionClosed);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => match result.error {
                Some(msg) => Err(RpcError::Callee(msg)),
                None => Ok(result.response),
            },
            Ok(Err(_)) => Err(RpcError::SessionClosed),
            Err(_) => {
                // nosemgrep -- mutex lock: only fails on poisoning by a panicking thread; propagating is not actionable
                self.in_flight_calls.lock().unwrap().remove(&cmd_id);
                Err(RpcError::Timeout)
            }
        }
    }

    /// Sends OPEN for a new client-initiated stream over `conn` (typically a
    /// connection accepted by `forwarder::ServiceListener`). Mirrors
    /// `Session.OpenRemoteStream`.
    pub async fn open_remote_stream<C>(
        self: &Arc<Self>,
        conn: C,
        target_host: impl Into<String>,
        target_port: i32,
        remote_addr: impl Into<String>,
    ) where
        C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let target_host = target_host.into();
        let remote_addr = remote_addr.into();

        // Holds the streams write lock for the full allocate-and-insert
        // operation to prevent a TOCTOU race, mirroring the Go comment on
        // `OpenRemoteStream`.
        let (stream_id, stream) = {
            let mut streams = self.streams.write().unwrap();
            let active: HashSet<u64> = streams.keys().copied().collect();
            let stream_id = self.stream_id_gen.lock().unwrap().next(&active);
            let stream = Stream::new(stream_id, conn, self.outbound_tx.clone());
            streams.insert(stream_id, stream.clone());
            (stream_id, stream)
        };

        let open_frame = Frame::new_open(
            stream_id,
            target_host.clone(),
            target_port,
            remote_addr.clone(),
        );
        if self.write_frame(open_frame).await.is_err() {
            tracing::error!(session_id = %self.id, stream_id, "OPEN send failed");
            self.streams.write().unwrap().remove(&stream_id);
            stream.close().await;
            return;
        }
        tracing::info!(
            session_id = %self.id,
            stream_id,
            target_host = %target_host,
            target_port,
            remote_addr = %remote_addr,
            "OPEN sent"
        );
    }

    /// Session main loop: starts the heartbeat and stream-cleanup tasks,
    /// then processes frames until the WebSocket disconnects or
    /// [`Session::cancel_token`] is cancelled. Mirrors `Session.Run`.
    /// `interval`/`pong_timeout` of `Duration::ZERO` mean "disabled"
    /// (client role) or "use defaults" (server role), matching Go's `<= 0` checks.
    pub async fn run(self: Arc<Self>, ping_interval: Duration, pong_timeout: Duration) {
        let source = self.pending_source.lock().unwrap().take();
        let Some(source) = source else {
            return; // run() already called
        };

        let heartbeat = tokio::spawn(heartbeat_loop(self.clone(), ping_interval, pong_timeout));
        let cleanup = tokio::spawn(stream_cleanup_loop(self.clone(), Duration::from_secs(30)));

        reader_loop(self.clone(), source).await;

        self.rst_all_streams().await;
        // Proactively cancel rather than relying solely on an external
        // cancellation, unlike Go (whose heartbeat/cleanup goroutines only
        // exit via the caller's ctx and otherwise linger past session end) —
        // a deliberate cleanliness improvement, not a protocol behavior change.
        self.cancel.cancel();
        let _ = heartbeat.await;
        let _ = cleanup.await;
    }

    /// Snapshots and clears the streams map, then sends RST to each and
    /// closes it. Mirrors `Session.rstAllStreams`.
    async fn rst_all_streams(self: &Arc<Self>) {
        let streams: HashMap<u64, Arc<Stream>> = {
            let mut guard = self.streams.write().unwrap();
            std::mem::take(&mut *guard)
        };
        for (id, stream) in streams {
            let _ = self.write_frame(Frame::new_rst(id, "session closed")).await;
            stream.close().await;
        }
        tracing::info!(session_id = %self.id, "all streams cleared");
    }

    /// Dispatches one already-validated frame. Mirrors the big `switch` in
    /// Go's `readLoop`.
    async fn dispatch_frame(self: &Arc<Self>, frame: Frame) {
        match frame.body {
            FrameBody::Ping => {
                let _ = self.write_frame(Frame::new_pong()).await;
            }
            FrameBody::Pong => self.ack_pong(),
            FrameBody::OpenAck => self.handle_open_ack(frame.stream_id).await,
            FrameBody::Nack(Some(payload)) => self.handle_nack(frame.stream_id, payload).await,
            FrameBody::Open(Some(payload)) => {
                let session = self.clone();
                tokio::spawn(
                    async move { session.handle_open_stream(frame.stream_id, payload).await },
                );
            }
            FrameBody::Data(_) | FrameBody::Fin | FrameBody::Rst(_) => {
                let stream = self.streams.read().unwrap().get(&frame.stream_id).cloned();
                match stream {
                    Some(s) => {
                        if s.is_pending() {
                            tracing::debug!(
                                session_id = %self.id,
                                stream_id = frame.stream_id,
                                "frame received for OPENING stream, dispatching"
                            );
                        }
                        s.dispatch(frame).await;
                    }
                    None => {
                        tracing::warn!(session_id = %self.id, stream_id = frame.stream_id, "frame for unknown stream");
                        if matches!(frame.body, FrameBody::Data(_) | FrameBody::Fin) {
                            let _ = self
                                .write_frame(Frame::new_rst(
                                    frame.stream_id,
                                    reason::STREAM_UNKNOWN,
                                ))
                                .await;
                        }
                    }
                }
            }
            FrameBody::Cmd(Some(payload)) => {
                let session = self.clone();
                match self.cmd_semaphore.clone().try_acquire_owned() {
                    Ok(permit) => {
                        tokio::spawn(async move {
                            let _permit = permit;
                            session
                                .handle_cmd(payload.cmd_id, payload.name, payload.params)
                                .await;
                        });
                    }
                    Err(_) => {
                        tracing::warn!(
                            session_id = %self.id,
                            cmd_id = payload.cmd_id,
                            "CMD: too many concurrent handlers, dropping"
                        );
                        let _ = self
                            .write_frame(Frame::new_cmd_error_response(
                                payload.cmd_id,
                                "server busy",
                            ))
                            .await;
                    }
                }
            }
            FrameBody::CmdResponse(Some(payload)) => self.handle_cmd_response(payload),
            FrameBody::SessionInit(_) => {
                tracing::warn!(session_id = %self.id, "SESSION_INIT frame received after session established");
            }
            FrameBody::SessionAck(_) => {
                tracing::warn!(session_id = %self.id, "SESSION_ACK frame received after session established");
            }
            // Validated frames never reach the rest — Frame::validate rejects
            // a missing payload/unknown type before dispatch_frame is called.
            FrameBody::Nack(None)
            | FrameBody::Open(None)
            | FrameBody::Cmd(None)
            | FrameBody::CmdResponse(None)
            | FrameBody::Unknown(_) => {}
        }
    }

    /// Transitions a stream from OPENING to OPEN and starts its read pump.
    /// Mirrors `Session.handleOpenAck`.
    async fn handle_open_ack(self: &Arc<Self>, stream_id: u64) {
        let stream = self.streams.read().unwrap().get(&stream_id).cloned();
        let Some(stream) = stream else {
            tracing::warn!(session_id = %self.id, stream_id, "OPEN_ACK for unknown stream");
            return;
        };
        tracing::info!(session_id = %self.id, stream_id, "OPEN_ACK received");
        stream.set_pending(false);
        tokio::spawn(stream.read_pump());
    }

    /// Closes a pending stream and removes it from the streams map. Mirrors
    /// `Session.handleNack`.
    async fn handle_nack(self: &Arc<Self>, stream_id: u64, payload: NackPayload) {
        let stream = self.streams.write().unwrap().remove(&stream_id);
        match stream {
            Some(stream) => {
                tracing::warn!(
                    session_id = %self.id,
                    stream_id,
                    reason = %payload.reason_code,
                    "NACK received for pending stream"
                );
                stream.close().await;
            }
            None => {
                tracing::warn!(
                    session_id = %self.id,
                    stream_id,
                    reason = %payload.reason_code,
                    "NACK received for unknown stream"
                );
            }
        }
    }

    /// Handles an inbound, sidecar-initiated OPEN: validates the stream ID's
    /// expected parity, dials the mapped local target (retrying once after
    /// 200ms), and starts the new stream's read pump. Mirrors
    /// `Session.handleOpenStream` — the client-side counterpart to
    /// the sidecar-initiated / intercepted-traffic conformance case.
    ///
    /// This handler trusts `payload.target_host`/`target_port` implicitly —
    /// there is no client-side [`TargetAllowlist`] check (that type is
    /// exercised only by the server-role test double). The actual protection
    /// boundary is [`InterceptTargetToLocalMapping`]: `dial_via_port_mapping`
    /// ignores `target_host` entirely and looks up `target_port` in that
    /// fixed, caller-supplied map, failing the dial (and NACKing) if the
    /// port isn't present. This is safe only because the peer here is the
    /// already-authenticated sidecar this client paired with during the
    /// handshake; if the client role is ever extended to accept OPEN frames
    /// from a less-trusted peer, a real allowlist would need adding from
    /// scratch. Confirm `InterceptTargetToLocalMapping` itself is validated
    /// and complete wherever `client::Agent` is constructed.
    async fn handle_open_stream(self: Arc<Self>, stream_id: u64, payload: OpenPayload) {
        let expected = if self.expect_odd_from_peer {
            stream_id % 2 == 1
        } else {
            stream_id % 2 == 0
        };
        if !expected {
            tracing::warn!(stream_id, "OPEN: unexpected stream_id");
            let _ = self
                .write_frame(Frame::new_nack(
                    stream_id,
                    reason::INVALID_STREAM_ID,
                    Some(false),
                ))
                .await;
            return;
        }

        if self.streams.read().unwrap().contains_key(&stream_id) {
            tracing::warn!(stream_id, "OPEN: stream_id already active");
            let _ = self
                .write_frame(Frame::new_nack(
                    stream_id,
                    reason::STREAM_ID_CONFLICT,
                    Some(false),
                ))
                .await;
            return;
        }

        let mut conn = (self.dial)(payload.target_host.clone(), payload.target_port).await;
        if conn.is_err() {
            // Race the retry delay against session cancellation, mirroring
            // Go's `select { case <-time.After(200ms): case <-ctx.Done(): return }` —
            // otherwise a session torn down mid-retry (after `rst_all_streams`
            // already ran) could still register an orphaned stream once the
            // retried dial completes.
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_millis(200)) => {}
            }
            conn = (self.dial)(payload.target_host.clone(), payload.target_port).await;
        }
        let conn = match conn {
            Ok(c) => c,
            Err(error) => {
                tracing::warn!(
                    stream_id,
                    host = %payload.target_host,
                    port = payload.target_port,
                    %error,
                    "OPEN: dial failed after retry"
                );
                let _ = self
                    .write_frame(Frame::new_nack(stream_id, reason::NO_LOCAL_LISTENER, None))
                    .await;
                return;
            }
        };

        let stream = Stream::new(stream_id, conn, self.outbound_tx.clone());
        // Scoped so the write guard's live range never overlaps an `.await`
        // (a `RwLockWriteGuard` is `!Send`, which would make this whole
        // spawned future non-`Send`) — second duplicate check under the
        // write lock, handling the race window between the read-only check
        // above and the dial completing.
        let conflict = {
            let mut streams = self.streams.write().unwrap();
            match streams.entry(stream_id) {
                std::collections::hash_map::Entry::Occupied(_) => true,
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(stream.clone());
                    false
                }
            }
        };
        if conflict {
            tracing::warn!(stream_id, "OPEN: stream_id conflict after dial");
            stream.close().await;
            let _ = self
                .write_frame(Frame::new_nack(
                    stream_id,
                    reason::STREAM_ID_CONFLICT,
                    Some(false),
                ))
                .await;
            return;
        }

        if let Err(error) = self.write_frame(Frame::new_open_ack(stream_id)).await {
            tracing::warn!(session_id = %self.id, stream_id, %error, "OPEN_ACK send failed");
            self.streams.write().unwrap().remove(&stream_id);
            stream.close().await;
            return;
        }
        tracing::info!(session_id = %self.id, stream_id, "OPEN_ACK sent");
        stream.set_pending(false);
        tokio::spawn(stream.read_pump());
    }

    /// Callee-side CMD frame processor. Runs in its own spawned task so the
    /// reader loop is never blocked by a slow handler. Mirrors `Session.handleCmd`.
    async fn handle_cmd(
        self: Arc<Self>,
        cmd_id: u64,
        name: String,
        params: Option<serde_json::Value>,
    ) {
        let handler = self.handlers.read().unwrap().get(&name).cloned();
        let response_frame = match handler {
            // Recover from a panicking handler to keep the session alive and
            // still answer the caller, mirroring Go's `defer recover()` in
            // `handleCmd` — otherwise a bare `tokio::spawn` (below) silently
            // drops the panicked task's result and the caller only learns
            // about it via its own RPC timeout.
            Some(h) => match std::panic::AssertUnwindSafe(h(params)).catch_unwind().await {
                Ok(Ok(response)) => Frame::new_cmd_response(cmd_id, response),
                Ok(Err(err_msg)) => {
                    tracing::debug!(session_id = %self.id, cmd_id, %name, error = %err_msg, "CMD: handler returned error");
                    Frame::new_cmd_error_response(cmd_id, err_msg)
                }
                Err(panic) => {
                    let msg = panic_message(&panic);
                    tracing::warn!(session_id = %self.id, cmd_id, %name, panic = %msg, "CMD: handler panicked");
                    Frame::new_cmd_error_response(cmd_id, format!("handler panic: {msg}"))
                }
            },
            None => {
                tracing::warn!(session_id = %self.id, cmd_id, %name, "CMD: no handler for command");
                Frame::new_cmd_error_response(cmd_id, "command not found")
            }
        };
        if let Err(error) = self.write_frame(response_frame).await {
            tracing::warn!(session_id = %self.id, cmd_id, %error, "CMD: failed to send response");
        }
    }

    /// Resolves the in-flight call on the caller side. Mirrors
    /// `Session.handleCmdResponse`. A missing in-flight entry (late or
    /// duplicate response, or the caller already timed out) is silently
    /// ignored, matching Go.
    fn handle_cmd_response(&self, payload: CmdResponsePayload) {
        let sender = self.in_flight_calls.lock().unwrap().remove(&payload.cmd_id);
        let Some(tx) = sender else {
            tracing::warn!(
                session_id = %self.id,
                cmd_id = payload.cmd_id,
                "CMD_RESPONSE: no in-flight call (late or duplicate response)"
            );
            return;
        };
        let result = RpcResult {
            response: payload.response,
            error: if payload.error.is_empty() {
                None
            } else {
                Some(payload.error)
            },
        };
        let _ = tx.send(result); // caller may have timed out and dropped its receiver
    }

    /// Records a PONG for the heartbeat state machine. Only meaningful for
    /// the server role, which is the side that sends PING. Mirrors `Session.ackPong`.
    fn ack_pong(&self) {
        if self.role != SessionRole::Server {
            return;
        }
        let mut hb = self.heartbeat.lock().unwrap();
        if !hb.awaiting_pong {
            tracing::warn!(session_id = %self.id, "unexpected PONG received");
            return;
        }
        hb.awaiting_pong = false;
        tracing::debug!(session_id = %self.id, "heartbeat PONG received");
    }

    /// Enqueues a frame for the writer task. Mirrors `Session.writeFrame`
    /// (minus the mutex — ordering is already guaranteed by the channel).
    /// Bounded (see [`OUTBOUND_CHANNEL_CAPACITY`]), so this awaits when the
    /// writer is stalled — the backpressure point that keeps a slow/stalled
    /// peer from causing unbounded memory growth.
    async fn write_frame(&self, frame: Frame) -> Result<(), mpsc::error::SendError<Frame>> {
        self.outbound_tx.send(frame).await
    }
}

/// Dials the local address mapped to `port` (client role). Mirrors the
/// inline `dialFunc` built in `tunnel.SendSessionToServer`.
async fn dial_via_port_mapping(
    ports: &InterceptTargetToLocalMapping,
    port: i32,
) -> std::io::Result<Box<dyn DuplexConn>> {
    let local_addr = ports.get(&port).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no local address mapping for target port {port}"),
        )
    })?;
    let addr = match local_addr.parse::<u16>() {
        Ok(port_num) => format!("127.0.0.1:{port_num}"),
        Err(_) => local_addr.clone(),
    };
    let conn = tokio::net::TcpStream::connect(addr).await?;
    Ok(Box::new(conn))
}

/// Dials a real TCP target after an allowlist check (server role). Mirrors
/// the inline `dialFunc` built in `tunnel.WaitSessionFromClient`.
async fn dial_with_allowlist(
    allowlist: Option<&TargetAllowlist>,
    host: String,
    port: i32,
) -> std::io::Result<Box<dyn DuplexConn>> {
    if let Some(allowlist) = allowlist {
        allowlist
            .allows_target(&host, port)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;
    }
    let conn = tokio::net::TcpStream::connect((host.as_str(), port as u16)).await?;
    Ok(Box::new(conn))
}

/// Encodes and sends one frame directly on the sink, bypassing the writer
/// task — used only during the handshake, before the writer task exists.
async fn send_message<Si>(sink: &mut Si, frame: &Frame) -> Result<(), HandshakeError>
where
    Si: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let bytes = protocol::encode(frame)?;
    sink.send(Message::Text(
        // nosemgrep -- `protocol::encode` emits JSON, which is UTF-8 by construction
        String::from_utf8(bytes).expect("json is always valid utf8"),
    ))
    .await?;
    Ok(())
}

/// Reads and decodes exactly one frame — used only during the handshake.
async fn recv_frame(source: &mut BoxedWsSource) -> Result<Frame, HandshakeError> {
    match source.next().await {
        Some(Ok(Message::Text(text))) => Ok(protocol::decode(text.as_bytes())?),
        Some(Ok(Message::Binary(bytes))) => Ok(protocol::decode(&bytes)?),
        Some(Ok(_other)) => Err(HandshakeError::ConnectionClosed),
        Some(Err(e)) => Err(HandshakeError::Transport(Box::new(e))),
        None => Err(HandshakeError::ConnectionClosed),
    }
}

/// Drains `rx`, encoding and writing each frame to the WebSocket in order —
/// the single-writer task that replaces Go's `writeMu`-guarded direct writes.
async fn writer_task<Si>(mut sink: Si, mut rx: mpsc::Receiver<Frame>, cancel: CancellationToken)
where
    Si: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    loop {
        let frame = tokio::select! {
            _ = cancel.cancelled() => break,
            frame = rx.recv() => frame,
        };
        let Some(frame) = frame else { break };
        if let Ok(bytes) = protocol::encode(&frame) {
            // Debug-only assert would be a no-op in release builds, leaving
            // no defense-in-depth if a future change to buffer sizing or a
            // new payload-bearing frame type ever violated the wire
            // contract; drop the offending frame in all build profiles
            // instead of forwarding a message the peer may reject anyway.
            if bytes.len() > MAX_FRAME_SIZE_BYTES {
                tracing::error!(
                    frame_bytes = bytes.len(),
                    max_bytes = MAX_FRAME_SIZE_BYTES,
                    "frame exceeds max wire size, dropping"
                );
                continue;
            }
            let text = String::from_utf8(bytes).expect("json is always valid utf8");
            if sink.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    }
    let _ = sink.close().await;
}

/// Reads, decodes, validates, and dispatches frames until disconnect or
/// cancellation. Mirrors `Session.readLoop`.
async fn reader_loop(session: Arc<Session>, mut source: BoxedWsSource) {
    loop {
        let message = tokio::select! {
            _ = session.cancel.cancelled() => return,
            msg = source.next() => msg,
        };
        let message = match message {
            Some(Ok(m)) => m,
            Some(Err(error)) => {
                tracing::info!(session_id = %session.id, %error, "session disconnected");
                return;
            }
            None => {
                tracing::info!(session_id = %session.id, "session disconnected");
                return;
            }
        };
        let bytes = match message {
            Message::Text(t) => t.into_bytes(),
            Message::Binary(b) => b,
            Message::Close(_) => {
                tracing::info!(session_id = %session.id, "session disconnected");
                return;
            }
            _ => continue,
        };
        let frame = match protocol::decode(&bytes) {
            Ok(f) => f,
            Err(error) => {
                tracing::warn!(session_id = %session.id, %error, "discard malformed frame");
                continue;
            }
        };

        tracing::debug!(
            session_id = %session.id,
            frame_type = ?frame.frame_type(),
            stream_id = frame.stream_id,
            payload_bytes = bytes.len(),
            "frame received"
        );

        session.last_activity.store(now_millis(), Ordering::Relaxed);

        if let Err(error) = frame.validate() {
            tracing::warn!(
                session_id = %session.id,
                frame_type = ?frame.frame_type(),
                stream_id = frame.stream_id,
                %error,
                "discard invalid frame"
            );
            continue;
        }

        session.dispatch_frame(frame).await;
    }
}

/// Sends PING and watches for PONG timeout (server role only) and checks
/// idle timeout (both roles). Mirrors `Session.heartbeatLoop`.
async fn heartbeat_loop(session: Arc<Session>, interval: Duration, pong_timeout: Duration) {
    let tick_every = if interval.is_zero() {
        Duration::from_secs(1)
    } else {
        interval
    };
    let pong_timeout = if pong_timeout.is_zero() {
        Duration::from_secs(5)
    } else {
        pong_timeout
    };

    let mut ticker = tokio::time::interval(tick_every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // tokio's interval fires immediately; Go's Ticker does not — align cadence

    loop {
        tokio::select! {
            _ = session.cancel.cancelled() => return,
            _ = ticker.tick() => {}
        }

        if session.role == SessionRole::Server && interval > Duration::ZERO {
            let now = Instant::now();
            let (timed_out, waiting_pong) = {
                let hb = session.heartbeat.lock().unwrap();
                (
                    hb.awaiting_pong && now >= hb.pong_deadline,
                    hb.awaiting_pong,
                )
            };

            if timed_out {
                tracing::warn!(
                    session_id = %session.id,
                    pong_timeout = ?pong_timeout,
                    "heartbeat timeout: missing PONG"
                );
                session.cancel.cancel();
                return;
            }
            if waiting_pong {
                continue;
            }

            {
                let mut hb = session.heartbeat.lock().unwrap();
                hb.awaiting_pong = true;
                hb.pong_deadline = now + pong_timeout;
            }

            if let Err(error) = session.write_frame(Frame::new_ping()).await {
                tracing::warn!(session_id = %session.id, %error, "heartbeat PING failed");
                session.cancel.cancel();
                return;
            }
            tracing::debug!(session_id = %session.id, "heartbeat PING sent");
        }

        let idle_ms = now_millis().saturating_sub(session.last_activity.load(Ordering::Relaxed));
        if !session.idle_timeout.is_zero()
            && idle_ms as u64 > session.idle_timeout.as_millis() as u64
        {
            tracing::warn!(
                session_id = %session.id,
                idle_duration_ms = idle_ms,
                "session idle timeout"
            );
            session.cancel.cancel();
            return;
        }
    }
}

/// Periodically prunes closed streams from the streams map. Mirrors
/// `Session.streamCleanupLoop`.
async fn stream_cleanup_loop(session: Arc<Session>, interval: Duration) {
    let interval = if interval.is_zero() {
        Duration::from_secs(60)
    } else {
        interval
    };
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = session.cancel.cancelled() => return,
            _ = ticker.tick() => {}
        }
        let removed = {
            let mut streams = session.streams.write().unwrap();
            let before = streams.len();
            streams.retain(|_, s| !s.is_closed());
            before - streams.len()
        };
        tracing::debug!(
            session_id = %session.id,
            active_streams = session.streams.read().unwrap().len(),
            removed_streams = removed,
            "stream cleanup check"
        );
    }
}

/// Extracts a human-readable message from a caught panic payload, mirroring
/// Go's `fmt.Errorf("handler panic: %v", r)`.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(inner) = payload.downcast_ref::<Box<dyn std::any::Any + Send>>() {
        // Tokio's own per-poll `catch_unwind` (which wraps every task,
        // outside this one) can end up nesting a panic payload inside
        // another `Box<dyn Any + Send>` before it reaches here in some
        // executor/task configurations — unwrap one extra layer rather than
        // reporting a message-less "unknown panic" for what is otherwise a
        // perfectly good `&str`/`String` payload underneath.
        panic_message(inner.as_ref())
    } else {
        "unknown panic".to_string()
    }
}

/// Generates an 8-byte random session ID, hex-encoded (server role only).
/// Mirrors `tunnel.generateSessionID`.
fn generate_session_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Milliseconds since `UNIX_EPOCH`, used for idle-timeout bookkeeping.
fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX_EPOCH")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::Role;

    /// In-process WebSocket pair over an in-memory duplex byte stream —
    /// skips the HTTP upgrade handshake Go's `websocketPair` performs via
    /// `httptest.NewServer`, but exercises the same real WS framing +
    /// `protocol` codec used in production. Stands in for Go's
    /// `websocketPair(t)` test helper.
    async fn websocket_pair() -> (
        WebSocketStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        (client, server)
    }

    /// Performs the SESSION_INIT/SESSION_ACK handshake and returns two fully
    /// initialized sessions, each already running. Stands in for Go's
    /// `sessionPair(t)` test helper (`rpc_test.go`).
    async fn session_pair() -> (Arc<Session>, Arc<Session>) {
        let (client_ws, server_ws) = websocket_pair().await;

        let server_task = tokio::spawn(async move {
            let (session, _init) =
                Session::wait_session_from_client(server_ws, Duration::from_secs(3600), None)
                    .await
                    .expect("server handshake");
            session.send_session_ack().await.expect("send SESSION_ACK");
            session
        });

        let client_session =
            Session::send_session_to_server(client_ws, HashMap::new(), Duration::from_secs(3))
                .await
                .expect("client handshake");
        let server_session = server_task.await.expect("server task panicked");

        tokio::spawn(client_session.clone().run(Duration::ZERO, Duration::ZERO));
        tokio::spawn(server_session.clone().run(Duration::ZERO, Duration::ZERO));

        (client_session, server_session)
    }

    /// A no-op dial closure for heartbeat-only tests that never open streams.
    fn no_op_dial() -> DialFn {
        Arc::new(|_host, _port| {
            Box::pin(async { Err(std::io::Error::other("no_op_dial: not implemented")) })
                as BoxFuture<_>
        })
    }

    /// Builds a session directly over one half of a websocket pair, bypassing
    /// the handshake — mirrors Go's tests calling the unexported `newSession`
    /// constructor directly.
    fn new_session_for_test(
        id: &str,
        ws: WebSocketStream<tokio::io::DuplexStream>,
        role: SessionRole,
        idle_timeout: Duration,
    ) -> Arc<Session> {
        let (sink, source) = ws.split();
        let source = source.boxed();
        let cancel = CancellationToken::new();
        let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
        tokio::spawn(writer_task(sink, outbound_rx, cancel.clone()));
        Arc::new(Session::new_internal(
            id.to_string(),
            role,
            outbound_tx,
            source,
            cancel,
            idle_timeout,
            no_op_dial(),
            None,
        ))
    }

    /// Translated from `session_test.go`'s `TestClientSessionDoesNotSendPing`.
    #[tokio::test]
    async fn test_client_session_does_not_send_ping() {
        let (client_ws, mut server_ws) = websocket_pair().await;
        let sess = new_session_for_test(
            "client-passive",
            client_ws,
            SessionRole::Client,
            Duration::from_secs(3600),
        );

        let run_handle = tokio::spawn(
            sess.clone()
                .run(Duration::from_millis(20), Duration::from_millis(20)),
        );

        let deadline = tokio::time::Instant::now() + Duration::from_millis(140);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, server_ws.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    let frame = protocol::decode(text.as_bytes()).expect("decode frame");
                    assert!(
                        !matches!(frame.body, FrameBody::Ping),
                        "client sent PING but should remain passive"
                    );
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }

        sess.cancel_token().cancel();
        tokio::time::timeout(Duration::from_millis(500), run_handle)
            .await
            .expect("client session did not stop after cancel")
            .expect("run task panicked");
    }

    /// Translated from `session_test.go`'s `TestServerSessionTimesOutWithoutPong`.
    #[tokio::test]
    async fn test_server_session_times_out_without_pong() {
        let (mut client_ws, server_ws) = websocket_pair().await;
        let sess = new_session_for_test(
            "server-timeout",
            server_ws,
            SessionRole::Server,
            Duration::from_secs(3600),
        );

        let run_handle = tokio::spawn(
            sess.clone()
                .run(Duration::from_millis(20), Duration::from_millis(40)),
        );

        let first = tokio::time::timeout(Duration::from_millis(300), client_ws.next())
            .await
            .expect("expected server PING")
            .expect("stream ended")
            .expect("read error");
        let Message::Text(text) = first else {
            panic!("expected text message");
        };
        let frame = protocol::decode(text.as_bytes()).expect("decode frame");
        assert!(
            matches!(frame.body, FrameBody::Ping),
            "expected first heartbeat frame to be PING"
        );

        let closed = tokio::time::timeout(Duration::from_millis(700), async {
            loop {
                match client_ws.next().await {
                    Some(Ok(_)) => continue,
                    _ => return,
                }
            }
        })
        .await;
        assert!(
            closed.is_ok(),
            "expected websocket close after missing PONG timeout"
        );

        sess.cancel_token().cancel();
        tokio::time::timeout(Duration::from_millis(500), run_handle)
            .await
            .expect("server session did not stop after cancel")
            .expect("run task panicked");
    }

    /// Translated from `session_test.go`'s `TestServerSessionStaysAliveWithPong`.
    #[tokio::test]
    async fn test_server_session_stays_alive_with_pong() {
        let (mut client_ws, server_ws) = websocket_pair().await;
        let sess = new_session_for_test(
            "server-alive",
            server_ws,
            SessionRole::Server,
            Duration::from_secs(3600),
        );

        let run_handle = tokio::spawn(
            sess.clone()
                .run(Duration::from_millis(20), Duration::from_millis(120)),
        );

        let responder = tokio::spawn(async move {
            loop {
                match client_ws.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(frame) = protocol::decode(text.as_bytes()) {
                            if matches!(frame.body, FrameBody::Ping) {
                                let pong_bytes = protocol::encode(&Frame::new_pong()).unwrap();
                                let text = String::from_utf8(pong_bytes).unwrap();
                                if client_ws.send(Message::Text(text)).await.is_err() {
                                    return client_ws;
                                }
                            }
                        }
                    }
                    Some(Ok(_)) => continue,
                    _ => return client_ws,
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(180)).await;
        assert!(
            !run_handle.is_finished(),
            "server session closed unexpectedly despite receiving PONG"
        );

        sess.cancel_token().cancel();
        tokio::time::timeout(Duration::from_millis(500), run_handle)
            .await
            .expect("server session did not stop after cancel")
            .expect("run task panicked");

        tokio::time::timeout(Duration::from_millis(300), responder)
            .await
            .expect("responder task did not stop")
            .expect("responder task panicked");
    }

    /// Translated from `session_test.go`'s `TestServerHeartbeatIgnoresNonPongFrames`.
    #[tokio::test]
    async fn test_server_heartbeat_ignores_non_pong_frames() {
        let (mut client_ws, server_ws) = websocket_pair().await;
        let sess = new_session_for_test(
            "server-nonpong",
            server_ws,
            SessionRole::Server,
            Duration::from_secs(3600),
        );

        let run_handle = tokio::spawn(
            sess.clone()
                .run(Duration::from_millis(20), Duration::from_millis(60)),
        );

        let first = tokio::time::timeout(Duration::from_secs(1), client_ws.next())
            .await
            .expect("read server ping")
            .expect("stream ended")
            .expect("read error");
        let Message::Text(text) = first else {
            panic!("expected text message");
        };
        let frame = protocol::decode(text.as_bytes()).expect("decode frame");
        assert!(matches!(frame.body, FrameBody::Ping));

        let data_bytes = protocol::encode(&Frame::new_data(1, b"not-a-pong".to_vec())).unwrap();
        client_ws
            .send(Message::Text(String::from_utf8(data_bytes).unwrap()))
            .await
            .expect("write data frame");

        let _ = tokio::time::timeout(Duration::from_millis(700), async {
            loop {
                match client_ws.next().await {
                    Some(Ok(_)) => continue,
                    _ => return,
                }
            }
        })
        .await;

        sess.cancel_token().cancel();
        tokio::time::timeout(Duration::from_millis(500), run_handle)
            .await
            .expect("server session did not stop after cancel")
            .expect("run task panicked");
    }

    /// Translated from `session_test.go`'s `TestClientSessionDetectsMissingHeartbeat`.
    #[tokio::test]
    async fn test_client_session_detects_missing_heartbeat() {
        let (client_ws, mut server_ws) = websocket_pair().await;
        let idle_timeout = Duration::from_millis(50);
        let sess = new_session_for_test(
            "client-detect-missing",
            client_ws,
            SessionRole::Client,
            idle_timeout,
        );

        let run_handle = tokio::spawn(
            sess.clone()
                .run(Duration::from_millis(20), Duration::from_millis(20)),
        );

        let got_close = tokio::time::timeout(Duration::from_millis(200), async {
            loop {
                match server_ws.next().await {
                    Some(Ok(_)) => continue,
                    _ => return,
                }
            }
        })
        .await;
        assert!(
            got_close.is_ok(),
            "expected client to close connection after missing heartbeat"
        );

        sess.cancel_token().cancel();
        tokio::time::timeout(Duration::from_millis(500), run_handle)
            .await
            .expect("client session did not stop after cancel")
            .expect("run task panicked");
    }

    /// Translated from `session_test.go`'s `TestClientSessionRetriesHeartbeatAfterMissingPing`.
    #[tokio::test]
    async fn test_client_session_retries_heartbeat_after_missing_ping() {
        let (client_ws, mut server_ws) = websocket_pair().await;
        let idle_timeout = Duration::from_millis(60);
        let sess = new_session_for_test(
            "client-retry-heartbeat",
            client_ws,
            SessionRole::Client,
            idle_timeout,
        );

        let run_handle = tokio::spawn(
            sess.clone()
                .run(Duration::from_millis(20), Duration::from_millis(20)),
        );

        let got_close = tokio::time::timeout(Duration::from_millis(300), async {
            loop {
                match server_ws.next().await {
                    Some(Ok(_)) => continue,
                    _ => return,
                }
            }
        })
        .await;
        assert!(
            got_close.is_ok(),
            "expected client to close connection after missing heartbeat retry"
        );

        sess.cancel_token().cancel();
        tokio::time::timeout(Duration::from_millis(500), run_handle)
            .await
            .expect("client session did not stop after cancel")
            .expect("run task panicked");
    }

    /// Translated from `rpc_test.go`'s `TestRPCSuccessfulRoundTrip`.
    #[tokio::test]
    async fn test_rpc_successful_round_trip() {
        let (caller, callee) = session_pair().await;

        callee.register_command_handler(
            "echo",
            Arc::new(|params: Option<serde_json::Value>| {
                Box::pin(async move {
                    let msg = params
                        .and_then(|v| v.get("Msg").cloned())
                        .unwrap_or_default();
                    Ok(Some(serde_json::json!({ "Echo": msg })))
                }) as BoxFuture<_>
            }),
        );

        let raw = caller
            .call_rpc(
                "echo",
                Some(serde_json::json!({"Msg": "hello"})),
                Duration::from_secs(3),
            )
            .await
            .expect("CallRPC");
        assert_eq!(raw.unwrap()["Echo"], "hello");
    }

    /// Translated from `rpc_test.go`'s `TestRPCUnknownCommand`.
    #[tokio::test]
    async fn test_rpc_unknown_command() {
        let (caller, _callee) = session_pair().await;
        let result = caller
            .call_rpc("no_such_command", None, Duration::from_secs(3))
            .await;
        assert!(result.is_err());
    }

    /// Translated from `rpc_test.go`'s `TestRPCCalleeErrorForwarded`.
    #[tokio::test]
    async fn test_rpc_callee_error_forwarded() {
        let (caller, callee) = session_pair().await;
        callee.register_command_handler(
            "fail",
            Arc::new(|_params| {
                Box::pin(async { Err("deadline exceeded".to_string()) }) as BoxFuture<_>
            }),
        );

        let result = caller.call_rpc("fail", None, Duration::from_secs(3)).await;
        assert!(matches!(result, Err(RpcError::Callee(_))));
    }

    /// Translated from `rpc_test.go`'s `TestRPCCallerTimeout`.
    #[tokio::test]
    async fn test_rpc_caller_timeout() {
        let (caller, callee) = session_pair().await;
        callee.register_command_handler(
            "slow",
            Arc::new(|_params| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Ok(Some(serde_json::json!("done")))
                }) as BoxFuture<_>
            }),
        );

        let result = caller
            .call_rpc("slow", None, Duration::from_millis(100))
            .await;
        assert!(matches!(result, Err(RpcError::Timeout)));
    }

    /// Translated from `rpc_test.go`'s `TestRPCConcurrentCalls`.
    #[tokio::test]
    async fn test_rpc_concurrent_calls() {
        let (caller, callee) = session_pair().await;
        callee.register_command_handler(
            "sleep_echo",
            Arc::new(|params: Option<serde_json::Value>| {
                Box::pin(async move {
                    let delay_ms = params
                        .and_then(|v| v.get("DelayMs").and_then(|d| d.as_i64()))
                        .unwrap_or(0);
                    tokio::time::sleep(Duration::from_millis(delay_ms as u64)).await;
                    Ok(Some(serde_json::json!(delay_ms)))
                }) as BoxFuture<_>
            }),
        );

        let caller = caller.clone();
        let mut handles = Vec::new();
        for delay in [60, 10, 30] {
            let caller = caller.clone();
            handles.push(tokio::spawn(async move {
                caller
                    .call_rpc(
                        "sleep_echo",
                        Some(serde_json::json!({"DelayMs": delay})),
                        Duration::from_secs(3),
                    )
                    .await
            }));
        }

        let mut results = Vec::new();
        for handle in handles {
            let result = tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect("concurrent RPC calls timed out")
                .expect("task panicked")
                .expect("CallRPC");
            results.push(result.unwrap().as_i64().unwrap());
        }

        assert_eq!(
            results,
            vec![60, 10, 30],
            "expected correct correlation, not FIFO"
        );
    }

    /// Translated from `rpc_test.go`'s `TestRPCLateResponseIgnored`.
    #[tokio::test]
    async fn test_rpc_late_response_ignored() {
        let (caller, callee) = session_pair().await;
        callee.register_command_handler(
            "delayed",
            Arc::new(|_params| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    Ok(Some(serde_json::json!("late")))
                }) as BoxFuture<_>
            }),
        );

        let result = caller
            .call_rpc("delayed", None, Duration::from_millis(50))
            .await;
        assert!(matches!(result, Err(RpcError::Timeout)));

        // Give the callee enough time to respond after the caller already
        // cleaned up — asserts no panic or deadlock.
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    /// Translated from `rpc_test.go`'s `TestRPCHandlerNilResponse`.
    #[tokio::test]
    async fn test_rpc_handler_nil_response() {
        let (caller, callee) = session_pair().await;
        callee.register_command_handler(
            "noop",
            Arc::new(|_params| Box::pin(async { Ok(None) }) as BoxFuture<_>),
        );

        let raw = caller
            .call_rpc("noop", None, Duration::from_secs(3))
            .await
            .expect("CallRPC");
        assert!(raw.is_none());
    }

    /// A panicking handler must still yield a fast `Callee` error to the
    /// caller (mirroring Go's `recover()`-and-respond), not a silent drop
    /// that only surfaces as a caller-side timeout.
    #[tokio::test]
    async fn test_rpc_handler_panic_yields_error_response() {
        let (caller, callee) = session_pair().await;
        async fn panicking_handler(
            _params: Option<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, String> {
            panic!("handler exploded")
        }
        callee.register_command_handler(
            "boom",
            Arc::new(|params| Box::pin(panicking_handler(params)) as BoxFuture<_>),
        );

        let result = caller.call_rpc("boom", None, Duration::from_secs(3)).await;
        match result {
            Err(RpcError::Callee(msg)) => assert!(
                msg.contains("handler exploded"),
                "expected panic message in error, got: {msg}"
            ),
            other => panic!("expected Err(RpcError::Callee(_)), got {other:?}"),
        }
    }
}
