//! Cross-language conformance tests against the real sidecar Docker image.
//! Unlike `go_testpeer.rs` (against `cmd/testpeer`, which has no
//! auth/allowlist/single-session enforcement by design), these tests
//! exercise the genuine `pkg/sidecar` behavior that deliberately skips:
//! IAM-gated auth, the `TargetAllowlist`/SSRF guard, single-session 409
//! enforcement, and real iptables `REDIRECT`-based interception.
//!
//! This is the conformance-suite counterpart to `ags-cli`'s future
//! `security` test category, once this crate folds into that workspace.
//!
//! Auth uses the sidecar's `--iam-dev-mode` flag rather than a real
//! JWKS-serving mock: this crate never validates JWTs itself, it only
//! presents a bearer token, so any non-`"unauthorized"`/`"forbidden"` token
//! authenticates successfully.
//!
//! Deliberately does **not** build or start the sidecar image itself — bring
//! up the harness once via the Go module's own Makefile and point these
//! tests at the result:
//!
//! ```text
//! cd modules/extend-proxy && make conformance-up
//! cargo test -p extend-proxy-client --test sidecar_conformance -- --ignored
//! cd modules/extend-proxy && make conformance-down
//! ```
//!
//! Gated behind `#[ignore]` since it requires that harness to be running.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use extend_proxy_client::client::{Agent, AgentError, Config, PortMapping, TokenProvider};
use extend_proxy_client::protocol::{self, reason, FrameBody};
use extend_proxy_client::session::Session;
use socket2::Socket;

use common::{free_port, init_tracing, run_echo_listener};

/// The sidecar allows exactly one active session at a time (that's what
/// `test_single_session_409_against_real_sidecar` exercises), so these tests
/// cannot run concurrently — but `cargo test`'s default multi-threaded
/// runner will otherwise happily start them all at once. Serialize via this
/// lock rather than relying on every caller remembering `--test-threads=1`.
static HARNESS_LOCK: Mutex<()> = Mutex::const_new(());

/// Attempts for [`connect_via_agent`]/[`connect_ws_with_retry`] to retry a
/// 409. Even with tests serialized (`HARNESS_LOCK`), the *previous* test's
/// disconnect isn't necessarily complete by the time it returns: cancelling
/// an `Agent` only signals `Session`'s cancellation token — the actual
/// WebSocket close happens in a detached `writer_task`
/// (`extend_proxy_client::session::core`) whose `JoinHandle` nothing joins,
/// so it can finish closing the socket strictly after `Agent::run` returns.
/// Retrying absorbs that gap instead of racing it.
const CONNECT_RETRY_ATTEMPTS: u32 = 30;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// A bearer token accepted by the sidecar's `--iam-dev-mode` `MockClient` —
/// any string except the literal `"unauthorized"`/`"forbidden"`.
const TEST_TOKEN: &str = "conformance-test-token";

/// `gameNamespace` is a required query parameter on `/tunnel`; its value
/// doesn't matter under `--iam-dev-mode` (the mock grants permission for any
/// namespace), so a fixed literal is fine here.
const TEST_NAMESPACE: &str = "conformance-test";

fn sidecar_ws_addr() -> String {
    std::env::var("SIDECAR_WS_ADDR").unwrap_or_else(|_| "127.0.0.1:18080".to_string())
}

fn sidecar_ws_url() -> String {
    format!(
        // nosemgrep -- test fixture: loopback to the local docker-compose sidecar
        "ws://{}/tunnel?gameNamespace={TEST_NAMESPACE}",
        sidecar_ws_addr()
    )
}

/// The Docker-published copy of the sidecar's `--exposed-ports` target — a
/// plain TCP connection here genuinely transits the container's network
/// stack and hits the real `PREROUTING REDIRECT` rule installed by
/// `sidecar-init`, unlike Layer 2's in-process stand-in.
fn sidecar_exposed_addr() -> String {
    std::env::var("SIDECAR_EXPOSED_ADDR").unwrap_or_else(|_| "127.0.0.1:18008".to_string())
}

fn test_token_provider() -> TokenProvider {
    Arc::new(|_cancel: CancellationToken| Box::pin(async { TEST_TOKEN.to_string() }))
}

/// Connects to the real sidecar via the crate's real public `Agent`/`Config`
/// entry point — not a hand-rolled WebSocket dial — since attaching the
/// `Authorization` header the way production code does is part of what's
/// under test. Retries on a 409 (see [`CONNECT_RETRY_ATTEMPTS`]); any other
/// failure, or exhausting retries, panics with the real `AgentError`.
async fn connect_via_agent(allow_ports: Vec<PortMapping>) -> (Arc<Session>, CancellationToken) {
    for attempt in 1..=CONNECT_RETRY_ATTEMPTS {
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(1);
        let cfg = Config {
            sidecar_ws: sidecar_ws_url(),
            allow_ports: allow_ports.clone(),
            session_init_timeout: Duration::from_secs(5),
            token_provider: Some(test_token_provider()),
            session_ready_tx: Some(tx),
        };
        let agent = Agent::new(cfg);
        let run_cancel = cancel.clone();
        let handle = tokio::spawn(async move { agent.run(run_cancel).await });

        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(session)) => return (session, cancel),
            Ok(None) => {
                let result = handle.await.expect("agent task panicked");
                match result {
                    Err(AgentError::DialRejected { status: 409, .. })
                        if attempt < CONNECT_RETRY_ATTEMPTS =>
                    {
                        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                    }
                    other => panic!(
                        "agent failed to establish a session against the real sidecar: {other:?}"
                    ),
                }
            }
            Err(_) => panic!("timed out waiting for the agent to establish a session"),
        }
    }
    unreachable!("loop always returns or panics")
}

/// Connects a raw WebSocket to the sidecar with the same 409 retry as
/// [`connect_via_agent`] — used by the NACK test, which needs to speak raw
/// frames rather than go through `Agent`/`Session`.
async fn connect_ws_with_retry() -> WsStream {
    for attempt in 1..=CONNECT_RETRY_ATTEMPTS {
        let mut request = sidecar_ws_url()
            .into_client_request()
            .expect("build websocket request");
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {TEST_TOKEN}")).unwrap(),
        );
        match tokio_tungstenite::connect_async(request).await {
            Ok((ws, _response)) => return ws,
            Err(tokio_tungstenite::tungstenite::Error::Http(response))
                if response.status().as_u16() == 409 && attempt < CONNECT_RETRY_ATTEMPTS =>
            {
                tokio::time::sleep(CONNECT_RETRY_DELAY).await;
            }
            Err(e) => panic!("connect to real sidecar failed: {e}"),
        }
    }
    unreachable!("loop always returns or panics")
}

/// Opens a fresh client-initiated stream to `echo-backend:9000` (in the
/// harness's `--outbound-allow-list`) and returns the connected local end —
/// shared setup for every test below that needs continued control over the
/// local connection after the initial open (heartbeat/half-close/RST), not
/// just a one-shot round trip.
async fn open_echo_stream(session: &Arc<Session>, remote_addr: impl Into<String>) -> TcpStream {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener_addr = listener.local_addr().unwrap();
    let local_client = TcpStream::connect(listener_addr)
        .await
        .expect("connect to local listener");
    let (accepted_conn, _) = listener.accept().await.expect("accept local connection");

    session
        .open_remote_stream(accepted_conn, "echo-backend", 9000, remote_addr)
        .await;

    local_client
}

/// Writes `payload` and asserts it echoes back unchanged within `timeout`.
async fn assert_echoes(stream: &mut TcpStream, payload: &[u8], timeout: Duration) {
    stream.write_all(payload).await.expect("write");
    let mut buf = vec![0u8; payload.len().max(64)];
    let n = tokio::time::timeout(timeout, stream.read(&mut buf))
        .await
        .expect("timed out waiting for echoed data")
        .expect("read error");
    assert_eq!(&buf[..n], payload);
}

/// Client-initiated (even stream ID) stream conformance against the real
/// sidecar: opens a stream to `echo-backend:9000` and confirms data
/// round-trips through genuine `pkg/sidecar`/`pkg/tunnel` code (auth, dial,
/// relay) — not `cmd/testpeer`.
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_client_initiated_stream_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();
    let (session, cancel) = connect_via_agent(vec![]).await;

    let mut local_client = open_echo_stream(&session, "conformance-client").await;
    assert_echoes(
        &mut local_client,
        b"layer3 outbound round trip",
        Duration::from_secs(3),
    )
    .await;

    cancel.cancel();
}

/// Heartbeat conformance against the real sidecar: it actively PINGs
/// (`--ping-interval`/`--pong-timeout` shortened in the harness compose
/// file, see `docker-compose.conformance.yml`) and expects PONG back. The
/// Rust client is passive by design — it never initiates PING — so this
/// proves it keeps up with a real sidecar-issued heartbeat, not just
/// `cmd/testpeer`'s (Layer 2).
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_heartbeat_keeps_session_alive_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();
    let (session, cancel) = connect_via_agent(vec![]).await;

    // Several real ping/pong cycles at the harness's shortened interval; if
    // the client failed to PONG, the sidecar would have force-closed the
    // session well within this window.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !session.cancel_token().is_cancelled(),
        "session was closed — client failed to keep up with the real sidecar's PING/PONG heartbeat"
    );

    // Not just "not cancelled" — confirm the session is genuinely still
    // functional end-to-end (the real sidecar has no RPC handlers
    // registered, so use a stream round trip instead of Layer 2's
    // call_rpc-based health check).
    let mut local_client = open_echo_stream(&session, "conformance-heartbeat").await;
    assert_echoes(&mut local_client, b"still alive", Duration::from_secs(3)).await;

    cancel.cancel();
}

/// FIN / half-close conformance against the real sidecar: shutting down only
/// the write half of the local connection lets the in-flight echo reply
/// still arrive, then propagates as a real FIN through genuine
/// `pkg/sidecar`/`pkg/tunnel` code — not just `cmd/testpeer`'s (Layer 2).
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_half_close_fin_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();
    let (session, cancel) = connect_via_agent(vec![]).await;

    let mut local_client = open_echo_stream(&session, "conformance-fin").await;

    local_client
        .write_all(b"half-close me")
        .await
        .expect("write");
    // Half-close: no more data will be sent, but the echoed reply already
    // in flight must still arrive — this is what triggers a real FIN.
    local_client.shutdown().await.expect("shutdown write half");

    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), local_client.read(&mut buf))
        .await
        .expect("timed out waiting for echoed data after half-close")
        .expect("read error");
    assert_eq!(
        &buf[..n],
        b"half-close me",
        "echoed data must still arrive after a half-close"
    );

    // Once the sidecar's own dialed connection also reaches EOF (echo-backend
    // closes after observing our FIN), it sends its own FIN back and the
    // stream fully closes.
    let n = tokio::time::timeout(Duration::from_secs(3), local_client.read(&mut buf))
        .await
        .expect("timed out waiting for the stream to fully close")
        .expect("read error");
    assert_eq!(n, 0, "expected EOF once both sides have half-closed");

    cancel.cancel();
}

/// RST conformance against the real sidecar: forcibly aborting the local
/// connection with `SO_LINGER(0)` (a real TCP RST instead of a clean FIN)
/// must tear down only that one stream, not the whole session, through
/// genuine `pkg/sidecar`/`pkg/tunnel` code — not just `cmd/testpeer`'s
/// (Layer 2).
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_rst_on_local_reset_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();
    let (session, cancel) = connect_via_agent(vec![]).await;

    let mut local_client = open_echo_stream(&session, "conformance-rst").await;

    // Establish the stream is genuinely working before aborting it.
    assert_echoes(&mut local_client, b"before reset", Duration::from_secs(3)).await;

    // Abort, not close: SO_LINGER(0) makes the OS send a real TCP RST when
    // the socket is dropped, instead of the usual clean FIN.
    let std_stream = local_client.into_std().expect("into_std");
    let socket = Socket::from(std_stream);
    socket
        .set_linger(Some(Duration::ZERO))
        .expect("set SO_LINGER(0)");
    drop(socket);

    // Give the real sidecar time to observe the reset and tear down its
    // side of just this one stream.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The session itself — not just this one aborted stream — must still be
    // healthy: a fresh stream round trip proves the RST didn't corrupt
    // shared state.
    assert!(
        !session.cancel_token().is_cancelled(),
        "an aborted stream must not take down the whole session"
    );
    let mut follow_up = open_echo_stream(&session, "conformance-rst-followup").await;
    assert_echoes(&mut follow_up, b"still healthy", Duration::from_secs(3)).await;

    cancel.cancel();
}

/// Connects fresh, opens a stream to `(target_host, target_port)`, and
/// returns the NACK payload the real sidecar sends back. Bypasses
/// `Session`/`Agent` for the OPEN itself and speaks raw frames instead,
/// because the reason code isn't surfaced through the crate's public API
/// (`Session::handle_nack` only logs it). Shared by both NACK conformance
/// tests below (allowlist violation vs. allowed-but-unreachable).
async fn open_and_expect_nack(target_host: &str, target_port: i32) -> protocol::NackPayload {
    let mut ws = connect_ws_with_retry().await;

    let init = protocol::Frame::new_session_init(
        i64::from(protocol::PROTOCOL_VERSION),
        Vec::new(),
        Vec::new(),
    );
    ws.send(Message::Text(
        String::from_utf8(protocol::encode(&init).unwrap()).unwrap(),
    ))
    .await
    .expect("send SESSION_INIT");
    let ack = next_frame(&mut ws).await;
    assert!(
        matches!(ack.body, FrameBody::SessionAck(_)),
        "expected SESSION_ACK, got {ack:?}"
    );

    // Stream ID 2 is the first valid client-initiated (even) ID on a fresh session.
    let open = protocol::Frame::new_open(2, target_host, target_port, "conformance-nack-test");
    ws.send(Message::Text(
        String::from_utf8(protocol::encode(&open).unwrap()).unwrap(),
    ))
    .await
    .expect("send OPEN");

    // The sidecar retries its dial once after 200ms before NACKing.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let frame = next_frame(&mut ws).await;
            if let FrameBody::Nack(Some(payload)) = frame.body {
                return payload;
            }
        }
    })
    .await
    .expect("timed out waiting for NACK from the real sidecar")
}

/// `TargetAllowlist`/SSRF-guard conformance: an OPEN to a target *not* in
/// `--outbound-allow-list` (only `echo-backend:9000` and `:9001` are
/// allowed) must be rejected with a real NACK. Per `pkg/tunnel/session.go`'s
/// `handleOpenStream`, a failed dial always NACKs with `no_local_listener` —
/// the `port_not_allowed` constant exists but its code path is dead and
/// never actually emitted, despite what the name suggests.
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_nack_on_disallowed_target_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();

    // Not on --outbound-allow-list at all.
    let nack = open_and_expect_nack("echo-backend", 9999).await;

    assert_eq!(
        nack.reason_code,
        reason::NO_LOCAL_LISTENER,
        "expected the real sidecar's actual NACK reason (no_local_listener), not the \
         port_not_allowed constant's name — that code path is dead in pkg/tunnel/session.go"
    );
}

/// Distinct from the allowlist-violation case above: `echo-backend:9001` *is*
/// on `--outbound-allow-list`, but nothing listens there, so the sidecar's
/// dial genuinely fails on connect. Confirms both failure modes — rejected by
/// policy vs. rejected by the OS — route through the same real NACK path
/// with the same reason code, not a distinct one.
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_nack_on_dial_failure_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();

    let nack = open_and_expect_nack("echo-backend", 9001).await;

    assert_eq!(
        nack.reason_code,
        reason::NO_LOCAL_LISTENER,
        "expected the same NACK reason as a disallowed target — the sidecar's \
         dial-failure path doesn't distinguish allowlist-rejection from connection-refused"
    );
}

/// Single-session enforcement: the sidecar allows exactly one active session
/// at a time. A second connection attempt while the first is still open must
/// be rejected before the WebSocket upgrade even completes (HTTP 409), which
/// `Agent` must classify as `AgentError::DialRejected { status: 409, .. }` —
/// the caller-visible signal to treat this as a permanent error, not retry
/// forever.
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_single_session_409_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();
    let (_first_session, first_cancel) = connect_via_agent(vec![]).await;

    let second_cfg = Config {
        sidecar_ws: sidecar_ws_url(),
        allow_ports: vec![],
        session_init_timeout: Duration::from_secs(5),
        token_provider: Some(test_token_provider()),
        session_ready_tx: None,
    };
    let result = Agent::new(second_cfg).run(CancellationToken::new()).await;

    match result {
        Err(AgentError::DialRejected { status: 409, .. }) => {}
        other => panic!("expected AgentError::DialRejected {{ status: 409, .. }}, got {other:?}"),
    }

    first_cancel.cancel();
}

/// Layer 4, the required scenario: real iptables `REDIRECT` rules (installed
/// by `sidecar-init` via `--mode init`) intercept traffic arriving at the
/// sidecar's exposed port and tunnel it up as an unsolicited OPEN, which the
/// client must accept and relay to the locally-mapped backend. The published
/// `SIDECAR_EXPOSED_ADDR` port genuinely transits the container's network
/// stack (Docker's port publishing DNATs onto the container's real interface,
/// landing on `PREROUTING` exactly like real inbound pod traffic) — this is
/// not an in-process stand-in the way Layer 2's `--listen-and-forward` is.
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_sidecar_initiated_stream_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();
    let local_echo_port = free_port().await;
    tokio::spawn(run_echo_listener(local_echo_port));

    // Must match the harness's --exposed-ports (docker-compose.conformance.yml).
    const EXPOSED_TARGET_PORT: i32 = 8008;
    let (session, cancel) = connect_via_agent(vec![PortMapping {
        target_port: EXPOSED_TARGET_PORT,
        local_addr: format!("127.0.0.1:{local_echo_port}"),
    }])
    .await;

    let exposed_addr: SocketAddr = sidecar_exposed_addr()
        .parse()
        .expect("parse SIDECAR_EXPOSED_ADDR");
    // The REDIRECT rules were installed by sidecar-init before this test
    // started, but a fresh connection may still race container/network
    // readiness briefly — retry rather than assuming it's instantaneous.
    let mut external_client = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match TcpStream::connect(exposed_addr).await {
                Ok(stream) => return stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    })
    .await
    .expect("timed out connecting to the sidecar's intercepted port");

    external_client
        .write_all(b"layer4 iptables round trip")
        .await
        .expect("write to intercepted port");
    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), external_client.read(&mut buf))
        .await
        .expect("timed out waiting for echoed data via real iptables interception")
        .expect("read error");
    assert_eq!(&buf[..n], b"layer4 iptables round trip");

    drop(session);
    cancel.cancel();
}

/// Both directions live on the same real-sidecar session at once, and each
/// direction opens *multiple* independent, repeated streams — not just one —
/// driven concurrently via `futures_util::future::join_all` within each side
/// and `tokio::join!` across both, so `2 * STREAMS_PER_SIDE` streams of
/// opposite parity (even client-initiated, odd sidecar-initiated) are alive
/// at once against genuine auth/allowlist/iptables infra, not just
/// `cmd/testpeer` (Layer 2). Every payload is tagged with its own stream
/// index and round number, so cross-mixed demultiplexing is caught
/// immediately instead of silently passing. Smaller scale than Layer 2's
/// equivalent (10 vs. 100 streams/side) since each stream here is a real
/// Docker-networked connection, not an in-process one.
#[tokio::test]
#[ignore = "requires the conformance harness; run `make conformance-up` in modules/extend-proxy first"]
async fn test_simultaneous_client_and_sidecar_initiated_streams_against_real_sidecar() {
    let _guard = HARNESS_LOCK.lock().await;
    init_tracing();

    const STREAMS_PER_SIDE: usize = 50;
    const ROUNDS_PER_STREAM: u16 = 10;

    let local_echo_port = free_port().await;
    tokio::spawn(run_echo_listener(local_echo_port));

    // Must match the harness's --exposed-ports (docker-compose.conformance.yml).
    const EXPOSED_TARGET_PORT: i32 = 8008;
    let (session, cancel) = connect_via_agent(vec![PortMapping {
        target_port: EXPOSED_TARGET_PORT,
        local_addr: format!("127.0.0.1:{local_echo_port}"),
    }])
    .await;
    let exposed_addr: SocketAddr = sidecar_exposed_addr()
        .parse()
        .expect("parse SIDECAR_EXPOSED_ADDR");

    // Client-initiated (even stream IDs): STREAMS_PER_SIDE independent
    // streams to echo-backend, all concurrently.
    let client_streams = futures_util::future::join_all((0..STREAMS_PER_SIDE).map(|i| {
        let session = session.clone();
        async move {
            let mut local_client =
                open_echo_stream(&session, format!("conformance-client-{i}")).await;
            for round in 0..ROUNDS_PER_STREAM {
                let payload = format!("client-stream-{i}-round-{round}");
                assert_echoes(
                    &mut local_client,
                    payload.as_bytes(),
                    Duration::from_secs(3),
                )
                .await;
            }
        }
    }));

    // Sidecar-initiated (odd stream IDs): STREAMS_PER_SIDE independent
    // external TCP connections to the real intercepted port, all concurrently.
    let sidecar_streams =
        futures_util::future::join_all((0..STREAMS_PER_SIDE).map(|i| async move {
            let mut external_client = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    match TcpStream::connect(exposed_addr).await {
                        Ok(stream) => return stream,
                        Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                    }
                }
            })
            .await
            .expect("timed out connecting to the sidecar's intercepted port");

            for round in 0..ROUNDS_PER_STREAM {
                let payload = format!("sidecar-stream-{i}-round-{round}");
                assert_echoes(
                    &mut external_client,
                    payload.as_bytes(),
                    Duration::from_secs(3),
                )
                .await;
            }
        }));

    // Run every client-initiated and sidecar-initiated stream concurrently —
    // 2 * STREAMS_PER_SIDE independent streams alive on the same real
    // session at once, not one after another.
    tokio::join!(client_streams, sidecar_streams);

    cancel.cancel();
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

/// Reads and decodes the next frame, panicking (failing the test) on
/// connection close or transport error — there's no legitimate reason for
/// either mid-handshake in these tests.
async fn next_frame(ws: &mut WsStream) -> protocol::Frame {
    match ws.next().await {
        Some(Ok(Message::Text(text))) => protocol::decode(text.as_bytes()).expect("decode frame"),
        Some(Ok(Message::Binary(bytes))) => protocol::decode(&bytes).expect("decode frame"),
        Some(Ok(other)) => panic!("unexpected websocket message: {other:?}"),
        Some(Err(e)) => panic!("websocket error waiting for frame: {e}"),
        None => panic!("websocket closed before the expected frame arrived"),
    }
}
