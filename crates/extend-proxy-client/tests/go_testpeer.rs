//! Cross-language conformance tests against the real Go server-role code.
//!
//! Spawns `modules/extend-proxy/cmd/testpeer` (a test-peer binary built from
//! the actual `pkg/tunnel` server-role logic, not a hand-written stand-in)
//! and drives this crate's real client over a genuine TCP
//! WebSocket connection — proving the Rust client and the Go server
//! actually interoperate, not just that the Rust side is internally
//! consistent (which the in-process unit tests in `src/session/core.rs`
//! already cover).
//!
//! Deliberately does **not** build the Go binary itself — Rust has no
//! `go build` invocation and no dependency on locating Go source. Build it
//! once via the Go module's own Makefile and point these tests at the
//! result with `TESTPEER_BIN`:
//!
//! ```text
//! cd modules/extend-proxy && make build-testpeer
//! TESTPEER_BIN=/path/to/modules/extend-proxy/bin/testpeer \
//!   cargo test --test go_testpeer -- --ignored
//! ```
//!
//! Gated behind `#[ignore]` since it requires that binary to exist.

mod common;

use std::collections::HashMap;
use std::io::BufRead;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use extend_proxy_client::session::{InterceptTargetToLocalMapping, RpcError, Session};

use common::{free_port, init_tracing, run_echo_listener};

/// Path to the pre-built `testpeer` binary, supplied by the caller via
/// `TESTPEER_BIN` — Rust never builds Go itself. Panics with a build
/// command to run if the variable is unset or doesn't point to a real
/// file; that's the intended failure mode for an `--ignored` test.
fn testpeer_binary() -> PathBuf {
    let raw = std::env::var("TESTPEER_BIN").unwrap_or_else(|_| {
        panic!(
            "TESTPEER_BIN is not set.\n\n\
             This test drives a real Go binary; Rust does not build it for you. \
             Build it once via the Go module's Makefile:\n\n  \
             cd modules/extend-proxy && make build-testpeer\n\n\
             then re-run pointing at the printed path, e.g.:\n\n  \
             TESTPEER_BIN=/path/to/modules/extend-proxy/bin/testpeer \\\n    \
             cargo test --test go_testpeer -- --ignored\n"
        )
    });
    let path = PathBuf::from(raw);
    assert!(
        path.is_file(),
        "TESTPEER_BIN={} does not point to an existing file — rebuild with \
         `make build-testpeer` in modules/extend-proxy",
        path.display()
    );
    path
}

/// A running `testpeer` subprocess, killed on drop.
struct TestPeer {
    child: std::process::Child,
    addr: SocketAddr,
}

impl TestPeer {
    /// Spawns `testpeer` with the given extra CLI args and blocks briefly
    /// (this is process setup, not test logic) until it prints its bound
    /// `TESTPEER_LISTEN <addr>` line.
    // `Drop for TestPeer` kills and waits on the child on every path out of
    // this type, including a panic during `spawn` unwinding; clippy can't
    // see across that RAII boundary.
    #[allow(clippy::zombie_processes)]
    fn spawn(extra_args: &[&str]) -> Self {
        let mut cmd = std::process::Command::new(testpeer_binary());
        cmd.args(["--ws-listen", "127.0.0.1:0", "--log-level", "warn"]);
        cmd.args(extra_args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().expect("failed to spawn testpeer");

        let stdout = child.stdout.take().expect("piped stdout");
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).expect("read testpeer stdout");
            assert!(n > 0, "testpeer exited before printing its listen address");
            if let Some(addr) = line.trim().strip_prefix("TESTPEER_LISTEN ") {
                let addr: SocketAddr = addr.parse().expect("parse testpeer listen address");
                return Self { child, addr };
            }
        }
    }

    fn ws_url(&self, path: &str) -> String {
        // nosemgrep -- test fixture: loopback to the local Go testpeer process
        format!("ws://{}{}", self.addr, path)
    }
}

impl Drop for TestPeer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn connect_client(
    peer: &TestPeer,
    ports: InterceptTargetToLocalMapping,
) -> std::sync::Arc<Session> {
    let (ws_stream, _response) = tokio_tungstenite::connect_async(peer.ws_url("/tunnel"))
        .await
        .expect("connect_async to testpeer");
    let session = Session::send_session_to_server(ws_stream, ports, Duration::from_secs(5))
        .await
        .expect("client handshake against real Go server");
    tokio::spawn(session.clone().run(Duration::ZERO, Duration::ZERO));
    session
}

/// Handshake + RPC round trip against the real Go `WaitSessionFromClient`/
/// `Session.Run`/`RegisterCommandHandler`, proving the codec and CMD/CMD_RESPONSE
/// path are wire-compatible with the actual server-role implementation.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_rpc_round_trip_against_real_go_server() {
    // Flow: [Rust Client] --CMD "echo"--> [Go Test Peer] --CMD_RESPONSE--> [Rust Client]
    init_tracing();
    let peer = TestPeer::spawn(&[]);
    let session = connect_client(&peer, HashMap::new()).await;

    let raw = session
        .call_rpc(
            "echo",
            Some(serde_json::json!({"msg": "hello from rust"})),
            Duration::from_secs(3),
        )
        .await
        .expect("CallRPC against real Go server");
    assert_eq!(raw.unwrap()["msg"], "hello from rust");

    session.cancel_token().cancel();
}

/// Client-initiated (even stream ID) stream conformance: the Rust client
/// opens a stream targeting the Go test-peer's real TCP echo listener, and
/// data written on the local side round-trips through genuine Go
/// `Session.handleOpenStream`/`Stream.Dispatch`/`Stream.ReadPump` code.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_client_initiated_stream_against_real_go_server() {
    // Flow: local_client <-> accepted_conn <-> [Rust Client] <=ws=> [Go Test Peer] <-> echo_server (go_echo_port)
    init_tracing();
    let echo_port = free_port().await;
    let echo_addr = format!("127.0.0.1:{echo_port}");
    let peer = TestPeer::spawn(&["--echo-addr", &echo_addr]);
    let session = connect_client(&peer, HashMap::new()).await;

    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();

    let mut local_client = TcpStream::connect(listener_addr)
        .await
        .expect("connect to local listener");
    let (accepted_conn, _) = listener.accept().await.expect("accept local connection");

    session
        .open_remote_stream(accepted_conn, "127.0.0.1", echo_port as i32, "test-client")
        .await;

    local_client
        .write_all(b"round trip via real go server")
        .await
        .expect("write");

    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), local_client.read(&mut buf))
        .await
        .expect("timed out waiting for echoed data")
        .expect("read error");
    assert_eq!(&buf[..n], b"round trip via real go server");

    session.cancel_token().cancel();
}

/// Sidecar-initiated (odd stream ID) stream conformance: the Go test-peer's
/// `--listen-and-forward` stands in for iptables-redirected traffic, sending
/// an unsolicited OPEN that the Rust client must accept, map to its local
/// echo listener via the SESSION_INIT ports mapping, and relay both ways —
/// the inbound path a real sidecar exercises, required and not optional.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_sidecar_initiated_stream_against_real_go_server() {
    // Flow: forward_client <-> [Go Test Peer] <=ws=> [Rust Client] <-> echo_server (rust_echo_port)
    init_tracing();
    let rust_echo_port = free_port().await;
    tokio::spawn(run_echo_listener(rust_echo_port));

    let forward_local_port = free_port().await;
    const MAPPED_TARGET_PORT: i32 = 9999; // logical identifier, matched on both sides below

    let forward_spec = format!("{forward_local_port}:127.0.0.1:{MAPPED_TARGET_PORT}");
    let peer = TestPeer::spawn(&["--listen-and-forward", &forward_spec]);

    let mut ports = HashMap::new();
    ports.insert(MAPPED_TARGET_PORT, rust_echo_port.to_string());
    let session = connect_client(&peer, ports).await;

    // The Go peer starts its forward listener in a goroutine right after the
    // handshake, not before — retry briefly rather than racing its startup.
    let forward_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), forward_local_port);
    let mut forward_client = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match TcpStream::connect(forward_addr).await {
                Ok(stream) => return stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("timed out connecting to testpeer's forward listener");

    forward_client
        .write_all(b"sidecar-initiated round trip")
        .await
        .expect("write to forward listener");

    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), forward_client.read(&mut buf))
        .await
        .expect("timed out waiting for echoed data via sidecar-initiated stream")
        .expect("read error");
    assert_eq!(&buf[..n], b"sidecar-initiated round trip");

    session.cancel_token().cancel();
}

/// Both directions live on the same session at once, and each direction
/// opens *multiple* independent, repeated streams — not just one — driven
/// concurrently via `futures_util::future::join_all` within each side and
/// `tokio::join!` across both sides, so `2 * STREAMS_PER_SIDE` streams of
/// opposite parity (even client-initiated, odd sidecar-initiated) are alive
/// at once. Every stream's payload is tagged with its own stream index and
/// round number, so if the session ever demultiplexed a frame onto the
/// wrong `Stream` — mixing data between two streams of the *same*
/// direction, not just between the two directions — the mismatched
/// assertion catches it immediately instead of silently passing.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_simultaneous_client_and_sidecar_initiated_streams_against_real_go_server() {
    // Flow (both run concurrently, x STREAMS_PER_SIDE each):
    //   client:  local_client<->accepted_conn<->[Rust Client]<=ws=>[Go Test Peer]<->echo_server(go_echo_port)
    //   sidecar: forward_client<->[Go Test Peer]<=ws=>[Rust Client]<->echo_server(rust_echo_port)
    init_tracing();

    const STREAMS_PER_SIDE: usize = 100;
    const ROUNDS_PER_STREAM: u16 = 50;

    let go_echo_port = free_port().await;
    let go_echo_addr = format!("127.0.0.1:{go_echo_port}");

    let rust_echo_port = free_port().await;
    tokio::spawn(run_echo_listener(rust_echo_port));

    let forward_local_port = free_port().await;
    const MAPPED_TARGET_PORT: i32 = 9998; // logical identifier, matched on both sides below
    let forward_spec = format!("{forward_local_port}:127.0.0.1:{MAPPED_TARGET_PORT}");

    let peer = TestPeer::spawn(&[
        "--echo-addr",
        &go_echo_addr,
        "--listen-and-forward",
        &forward_spec,
    ]);

    let mut ports = HashMap::new();
    ports.insert(MAPPED_TARGET_PORT, rust_echo_port.to_string());
    let session = connect_client(&peer, ports).await;

    // Client-initiated (even stream IDs): open STREAMS_PER_SIDE independent
    // streams to the Go peer's echo listener, all concurrently — each its
    // own local TCP pair, its own OPEN, its own stream ID.
    let client_streams = futures_util::future::join_all((0..STREAMS_PER_SIDE).map(|i| {
        let session = session.clone();
        async move {
            let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
                .await
                .unwrap();
            let listener_addr = listener.local_addr().unwrap();

            let mut local_client = TcpStream::connect(listener_addr)
                .await
                .expect("connect to local listener");
            let (accepted_conn, _) = listener.accept().await.expect("accept local connection");

            session
                .open_remote_stream(
                    accepted_conn,
                    "127.0.0.1",
                    go_echo_port as i32,
                    format!("test-client-{i}"),
                )
                .await;

            for round in 0..ROUNDS_PER_STREAM {
                let payload = format!("client-stream-{i}-round-{round}");
                local_client
                    .write_all(payload.as_bytes())
                    .await
                    .expect("write (client-initiated)");

                let mut buf = vec![0u8; 128];
                let n = tokio::time::timeout(Duration::from_secs(3), local_client.read(&mut buf))
                    .await
                    .expect("timed out waiting for echoed data (client-initiated)")
                    .expect("read error (client-initiated)");
                assert_eq!(
                    &buf[..n],
                    payload.as_bytes(),
                    "client-initiated stream {i} round {round} got corrupted or cross-mixed data"
                );
            }
        }
    }));

    // Sidecar-initiated (odd stream IDs): connect STREAMS_PER_SIDE separate
    // TCP clients to the Go peer's forward listener, all concurrently; each
    // accepted connection triggers its own unsolicited OPEN back to the client.
    let sidecar_streams =
        futures_util::future::join_all((0..STREAMS_PER_SIDE).map(|i| async move {
            let forward_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), forward_local_port);
            let mut forward_client = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match TcpStream::connect(forward_addr).await {
                        Ok(stream) => return stream,
                        Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                    }
                }
            })
            .await
            .expect("timed out connecting to testpeer's forward listener");

            for round in 0..ROUNDS_PER_STREAM {
                let payload = format!("sidecar-stream-{i}-round-{round}");
                forward_client
                    .write_all(payload.as_bytes())
                    .await
                    .expect("write to forward listener");

                let mut buf = vec![0u8; 128];
                let n = tokio::time::timeout(Duration::from_secs(3), forward_client.read(&mut buf))
                    .await
                    .expect("timed out waiting for echoed data (sidecar-initiated)")
                    .expect("read error (sidecar-initiated)");
                assert_eq!(
                    &buf[..n],
                    payload.as_bytes(),
                    "sidecar-initiated stream {i} round {round} got corrupted or cross-mixed data"
                );
            }
        }));

    // Run every client-initiated and sidecar-initiated stream concurrently —
    // 2 * STREAMS_PER_SIDE independent streams alive on the same session at
    // once, not one after another.
    tokio::join!(client_streams, sidecar_streams);

    session.cancel_token().cancel();
}

/// PING/PONG heartbeat conformance: the Go peer (server role) actively
/// sends PING and expects PONG back, exactly like the real sidecar would.
/// The Rust client is passive by design — it never initiates PING itself —
/// so this proves it correctly PONGs a real Go-issued PING across several
/// heartbeat cycles, keeping the session alive rather than being force-closed.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_heartbeat_keeps_session_alive_against_real_go_server() {
    // Flow: [Go Test Peer] --PING--> [Rust Client] --PONG--> [Go Test Peer]  (repeats every --ping-interval)
    init_tracing();
    let peer = TestPeer::spawn(&["--ping-interval", "40ms", "--pong-timeout", "150ms"]);
    let session = connect_client(&peer, HashMap::new()).await;

    // Several real ping/pong cycles; if the client failed to PONG, Go would
    // have force-closed the session well within this window.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !session.cancel_token().is_cancelled(),
        "session was closed — client failed to keep up with the Go peer's real PING/PONG heartbeat"
    );

    // Not just "not cancelled" — confirm the session is genuinely still
    // functional end-to-end after sustained real heartbeats.
    let raw = session
        .call_rpc(
            "echo",
            Some(serde_json::json!({"msg": "still alive"})),
            Duration::from_secs(3),
        )
        .await
        .expect("CallRPC after sustained heartbeat");
    assert_eq!(raw.unwrap()["msg"], "still alive");

    session.cancel_token().cancel();
}

/// NACK conformance: the Rust client opens a stream targeting a port
/// nothing is listening on. The real Go peer dials, fails, retries once
/// after 200ms, then sends a real NACK back — observed here as Rust tearing
/// down the local connection it was handed (`Session::handle_nack` closing
/// the stream), rather than ever receiving an echo.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_nack_on_dial_failure_against_real_go_server() {
    // Flow: local_client<->accepted_conn<->[Rust Client] --OPEN--> [Go Test Peer] --dial--> X (refused) --NACK--> [Rust Client] closes accepted_conn --> local_client sees EOF
    init_tracing();
    let peer = TestPeer::spawn(&[]);
    let session = connect_client(&peer, HashMap::new()).await;

    // free_port() reserves then immediately releases a port, guaranteeing a
    // real "connection refused" on Go's dial attempt.
    let dead_port = free_port().await;

    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();

    let mut local_client = TcpStream::connect(listener_addr)
        .await
        .expect("connect to local listener");
    let (accepted_conn, _) = listener.accept().await.expect("accept local connection");

    session
        .open_remote_stream(accepted_conn, "127.0.0.1", dead_port as i32, "test-nack")
        .await;

    // Go retries its dial once after 200ms before giving up and sending
    // NACK, so allow enough time for that plus the round trip.
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(2), local_client.read(&mut buf))
        .await
        .expect("timed out waiting for the stream to close after NACK")
        .expect("read error");
    assert_eq!(
        n, 0,
        "expected EOF — Rust should have closed the local connection after a real NACK from Go"
    );

    session.cancel_token().cancel();
}

/// FIN / half-close conformance: shutting down only the write half of the
/// local connection lets the in-flight echo reply still arrive, then
/// propagates as a real FIN to Go, through its echo connection, and back —
/// both real implementations must track independent local/remote EOF
/// correctly before fully tearing the stream down.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_half_close_fin_against_real_go_server() {
    // Flow: local_client<->accepted_conn<->[Rust Client]<=ws=>[Go Test Peer]<->echo_server; local_client half-closes --FIN--> both sides drain in-flight data then close in turn
    init_tracing();
    let echo_port = free_port().await;
    let echo_addr = format!("127.0.0.1:{echo_port}");
    let peer = TestPeer::spawn(&["--echo-addr", &echo_addr]);
    let session = connect_client(&peer, HashMap::new()).await;

    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();

    let mut local_client = TcpStream::connect(listener_addr)
        .await
        .expect("connect to local listener");
    let (accepted_conn, _) = listener.accept().await.expect("accept local connection");

    session
        .open_remote_stream(accepted_conn, "127.0.0.1", echo_port as i32, "test-fin")
        .await;

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

    // Once Go's own side also reaches EOF (its echo connection closes after
    // observing our FIN), it sends its own FIN back and the stream fully closes.
    let n = tokio::time::timeout(Duration::from_secs(3), local_client.read(&mut buf))
        .await
        .expect("timed out waiting for the stream to fully close")
        .expect("read error");
    assert_eq!(n, 0, "expected EOF once both sides have half-closed");

    session.cancel_token().cancel();
}

/// RST conformance: forcibly aborting the local connection with
/// `SO_LINGER(0)` (which makes the OS send a real TCP RST instead of a
/// clean FIN on close) gives Rust's `ReadPump` a genuine local read error,
/// which it sends onward as a protocol RST frame. The real Go peer must
/// tear down its own side of just that one stream without taking the whole
/// session down.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_rst_on_local_reset_against_real_go_server() {
    // Flow: local_client<->accepted_conn<->[Rust Client]<=ws=>[Go Test Peer]<->echo_server; local_client aborts (SO_LINGER 0) --RST--> [Rust Client] --RST frame--> [Go Test Peer] closes only that stream
    init_tracing();
    let echo_port = free_port().await;
    let echo_addr = format!("127.0.0.1:{echo_port}");
    let peer = TestPeer::spawn(&["--echo-addr", &echo_addr]);
    let session = connect_client(&peer, HashMap::new()).await;

    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();

    let mut local_client = TcpStream::connect(listener_addr)
        .await
        .expect("connect to local listener");
    let (accepted_conn, _) = listener.accept().await.expect("accept local connection");

    session
        .open_remote_stream(accepted_conn, "127.0.0.1", echo_port as i32, "test-rst")
        .await;

    // Establish the stream is genuinely working before aborting it.
    local_client
        .write_all(b"before reset")
        .await
        .expect("write");
    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), local_client.read(&mut buf))
        .await
        .expect("timed out waiting for echo before reset")
        .expect("read error");
    assert_eq!(&buf[..n], b"before reset");

    // Abort, not close: SO_LINGER(0) makes the OS send a real TCP RST when
    // the socket is dropped, instead of the usual clean FIN.
    let std_stream = local_client.into_std().expect("into_std");
    let socket = socket2::Socket::from(std_stream);
    socket
        .set_linger(Some(Duration::ZERO))
        .expect("set SO_LINGER(0)");
    drop(socket);

    // Give the real Go peer time to observe the reset and tear down its
    // side of just this one stream.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The session itself — not just this one aborted stream — must still be
    // healthy: a fresh RPC round trip proves the RST didn't corrupt shared state.
    assert!(
        !session.cancel_token().is_cancelled(),
        "an aborted stream must not take down the whole session"
    );
    let raw = session
        .call_rpc(
            "echo",
            Some(serde_json::json!({"msg": "still healthy"})),
            Duration::from_secs(3),
        )
        .await
        .expect("CallRPC after a stream-level RST");
    assert_eq!(raw.unwrap()["msg"], "still healthy");

    session.cancel_token().cancel();
}

/// CMD/CMD_RESPONSE conformance: calling a command the real Go peer never
/// registered gets a real error CMD_RESPONSE back (Go's `ErrCommandNotFound`
/// path via `protocol.NewCmdErrorResponse`), not just a client-side timeout.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_cmd_unknown_command_against_real_go_server() {
    // Flow: [Rust Client] --CMD "no_such_command"--> [Go Test Peer] --CMD_RESPONSE(error: not found)--> [Rust Client]
    init_tracing();
    let peer = TestPeer::spawn(&[]);
    let session = connect_client(&peer, HashMap::new()).await;

    let result = session
        .call_rpc("no_such_command", None, Duration::from_secs(3))
        .await;
    assert!(
        matches!(result, Err(RpcError::Callee(_))),
        "expected a real error CMD_RESPONSE, got {result:?}"
    );

    session.cancel_token().cancel();
}

/// CMD/CMD_RESPONSE conformance: the real Go peer's registered `fail`
/// handler returns an error, which must round-trip as `RpcError::Callee`
/// with the exact message Go sent, not be swallowed or misclassified as a
/// transport failure.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_cmd_callee_error_against_real_go_server() {
    // Flow: [Rust Client] --CMD "fail"--> [Go Test Peer] --CMD_RESPONSE(error: "intentional failure")--> [Rust Client]
    init_tracing();
    let peer = TestPeer::spawn(&[]);
    let session = connect_client(&peer, HashMap::new()).await;

    let result = session.call_rpc("fail", None, Duration::from_secs(3)).await;
    match result {
        Err(RpcError::Callee(msg)) => assert_eq!(msg, "intentional failure"),
        other => panic!("expected RpcError::Callee(\"intentional failure\"), got {other:?}"),
    }

    session.cancel_token().cancel();
}

/// CMD/CMD_RESPONSE conformance: a real Go handler that runs longer than
/// the caller's timeout must surface as `RpcError::Timeout` on the Rust
/// side, and the late CMD_RESPONSE that arrives afterward must be discarded
/// safely rather than panicking, deadlocking, or corrupting a later call.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_cmd_timeout_against_real_go_server() {
    // Flow: [Rust Client] --CMD "sleep_echo"(300ms)--> [Rust times out @80ms]; later [Go Test Peer] --CMD_RESPONSE(late)--> [Rust Client] (discarded)
    init_tracing();
    let peer = TestPeer::spawn(&[]);
    let session = connect_client(&peer, HashMap::new()).await;

    let result = session
        .call_rpc(
            "sleep_echo",
            Some(serde_json::json!({"DelayMs": 300})),
            Duration::from_millis(80),
        )
        .await;
    assert!(
        matches!(result, Err(RpcError::Timeout)),
        "expected RpcError::Timeout, got {result:?}"
    );

    // Give the real Go handler time to finish and send its now-late
    // response — asserts no panic/deadlock, and that the session survives.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let raw = session
        .call_rpc(
            "echo",
            Some(serde_json::json!({"msg": "after late response"})),
            Duration::from_secs(3),
        )
        .await
        .expect("CallRPC after a late/discarded response");
    assert_eq!(raw.unwrap()["msg"], "after late response");

    session.cancel_token().cancel();
}

/// CMD/CMD_RESPONSE conformance under real concurrent interleaving: three
/// simultaneous calls to the real Go peer's `sleep_echo` handler, with
/// staggered delays chosen so responses arrive out of send order. Proves
/// `Session::call_rpc`'s `cmd_id` correlation is correct against genuine
/// concurrent goroutines on the Go side racing to respond — not just against
/// this crate's own mock transport, which `test_rpc_concurrent_calls` in
/// `src/session/core.rs` already covers but can't prove by itself: concurrency
/// bugs depend on real timing/interleaving, not just code reading.
#[tokio::test]
#[ignore = "requires the Go toolchain; run with `cargo test --test go_testpeer -- --ignored`"]
async fn test_rpc_concurrent_calls_against_real_go_server() {
    // Flow: [Rust Client] --3x concurrent CMD "sleep_echo"--> [Go Test Peer] --3x CMD_RESPONSE (out of send order)--> [Rust Client]
    init_tracing();
    let peer = TestPeer::spawn(&[]);
    let session = connect_client(&peer, HashMap::new()).await;

    let mut handles = Vec::new();
    for delay in [150, 20, 80] {
        let session = session.clone();
        handles.push(tokio::spawn(async move {
            session
                .call_rpc(
                    "sleep_echo",
                    Some(serde_json::json!({"DelayMs": delay})),
                    Duration::from_secs(5),
                )
                .await
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("concurrent RPC calls against real Go server timed out")
            .expect("task panicked")
            .expect("CallRPC against real Go server");
        results.push(result.unwrap().as_i64().unwrap());
    }

    assert_eq!(
        results,
        vec![150, 20, 80],
        "expected correct cmd_id correlation against the real Go peer, not FIFO/response order"
    );

    session.cancel_token().cancel();
}
