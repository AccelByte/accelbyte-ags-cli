//! Shared helpers for this crate's cross-language conformance tests
//! (`go_testpeer.rs`, Layer 2; `sidecar_conformance.rs`, Layers 3/4).
//! Not itself a test binary — `tests/common/mod.rs` is cargo's convention
//! for a module shared between integration test files without becoming one.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Reserves a free loopback port by binding then releasing it — a brief
/// TOCTOU race, acceptable for test purposes (same pattern used in this
/// crate's forwarder unit tests).
pub async fn free_port() -> u16 {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    listener.local_addr().unwrap().port()
}

/// Runs a plain TCP echo loop on `port` until the listener errors (e.g. is dropped elsewhere).
pub async fn run_echo_listener(port: u16) {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .await
        .unwrap();
    loop {
        let (mut conn, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => return,
        };
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                match conn.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if conn.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
}

/// Installs a `tracing` subscriber reading `RUST_LOG` (defaulting to `debug`
/// for this crate), so `--nocapture` shows the client's session/stream
/// diagnostics alongside the real peer's own log lines. Safe to call from
/// multiple tests in a binary — `try_init` ignores a second call.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("extend_proxy_client=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .try_init();
}
