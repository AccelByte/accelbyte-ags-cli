//! Functional tests for `ags extend tunnel`.
//!
//! These tests spawn the real `ags` binary and verify end-to-end behaviour
//! that unit tests cannot reach: exit codes after signal delivery,
//! crypto-provider availability at the binary level, and session event
//! output for connection lifecycle.

use std::io::{BufRead, BufReader, Read};
use std::process::Stdio;

/// Find a free TCP port by binding and immediately releasing.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Spawn `ags extend tunnel` as a child process with piped stderr,
/// using a dummy base URL (the SIGINT test never contacts the upstream).
#[cfg(unix)]
fn spawn_tunnel(
    port: u16,
) -> (
    std::process::Child,
    std::sync::mpsc::Receiver<String>,
    tempfile::TempDir,
) {
    spawn_tunnel_with_base_url(port, "http://localhost:1")
}

/// Spawn `ags extend tunnel` with a custom `AGS_BASE_URL`.
fn spawn_tunnel_with_base_url(
    port: u16,
    base_url: &str,
) -> (
    std::process::Child,
    std::sync::mpsc::Receiver<String>,
    tempfile::TempDir,
) {
    spawn_tunnel_with_args(port, base_url, &[])
}

/// Spawn `ags extend tunnel` with a custom `AGS_BASE_URL` and extra CLI
/// flags (e.g. `--quiet`, `--format`, `json`).
fn spawn_tunnel_with_args(
    port: u16,
    base_url: &str,
    extra_args: &[&str],
) -> (
    std::process::Child,
    std::sync::mpsc::Receiver<String>,
    tempfile::TempDir,
) {
    let bin = assert_cmd::cargo::cargo_bin("ags");
    let tmp = tempfile::tempdir().unwrap();
    let port_str = port.to_string();

    let mut cmd = std::process::Command::new(&bin);
    cmd.args([
        "extend",
        "tunnel",
        "--resource-name",
        "test-res",
        "--local-port",
        &port_str,
        "--namespace",
        "test-ns",
    ])
    .args(extra_args)
    .env("AGS_HOME", tmp.path())
    .env("AGS_NO_KEYCHAIN", "1")
    .env("AGS_ACCESS_TOKEN", "fake-test-token")
    .env("AGS_BASE_URL", base_url)
    .env("AGS_NO_UPDATE_CHECK", "1")
    .stderr(Stdio::piped())
    .stdin(Stdio::null())
    .stdout(Stdio::null());

    let mut child = cmd.spawn().expect("failed to spawn ags extend tunnel");

    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    (child, rx, tmp)
}

/// Spawn `ags extend tunnel` with stdout piped (for stdout-empty checks).
fn spawn_tunnel_capture_stdout(
    port: u16,
    base_url: &str,
    extra_args: &[&str],
) -> (
    std::process::Child,
    std::sync::mpsc::Receiver<String>,
    std::process::ChildStdout,
    tempfile::TempDir,
) {
    let bin = assert_cmd::cargo::cargo_bin("ags");
    let tmp = tempfile::tempdir().unwrap();
    let port_str = port.to_string();

    let mut cmd = std::process::Command::new(&bin);
    cmd.args([
        "extend",
        "tunnel",
        "--resource-name",
        "test-res",
        "--local-port",
        &port_str,
        "--namespace",
        "test-ns",
    ])
    .args(extra_args)
    .env("AGS_HOME", tmp.path())
    .env("AGS_NO_KEYCHAIN", "1")
    .env("AGS_ACCESS_TOKEN", "fake-test-token")
    .env("AGS_BASE_URL", base_url)
    .env("AGS_NO_UPDATE_CHECK", "1")
    .stderr(Stdio::piped())
    .stdin(Stdio::null())
    .stdout(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn ags extend tunnel");

    let stderr = child.stderr.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    (child, rx, stdout, tmp)
}

/// Wait until the tunnel emits its ready line on stderr. Handles both
/// the human format (`listening on`) and JSON format (`"event":"listening"`).
fn wait_for_ready(rx: &std::sync::mpsc::Receiver<String>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut lines = Vec::new();
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(line) => {
                let is_ready = line.to_lowercase().contains("listening on")
                    || line.contains("\"event\":\"listening\"");
                lines.push(line);
                if is_ready {
                    return;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    panic!(
        "tunnel did not emit ready signal within 15 s; stderr so far:\n{}",
        lines.join("\n")
    );
}

/// Wait until the port is accepting TCP connections (for quiet-mode
/// tests where the ready line is suppressed).
fn wait_for_port(port: u16) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("port {port} did not become reachable within 15 s");
}

/// Drain remaining stderr lines with a brief grace period.
fn drain_stderr(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
    drain_stderr_for(rx, std::time::Duration::from_millis(500))
}

/// Drain stderr lines with a configurable timeout. Connection handlers
/// may take longer than the default 500 ms grace period when the TLS
/// dial needs to complete or be rejected.
fn drain_stderr_for(
    rx: &std::sync::mpsc::Receiver<String>,
    timeout: std::time::Duration,
) -> Vec<String> {
    let mut lines = Vec::new();
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(line) => lines.push(line),
            Err(_) => {
                // No more data right now; keep waiting until deadline.
            }
        }
    }
    lines
}

/// Start a plain-TCP upstream that accepts connections and holds them
/// (never speaks TLS or WebSocket). Returns the upstream port, a stop
/// flag, and the accept-loop thread handle.
fn start_holding_upstream() -> (
    u16,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag_clone = flag.clone();
    let handle = std::thread::spawn(move || {
        let mut held = Vec::new();
        upstream
            .set_nonblocking(true)
            .expect("set_nonblocking failed");
        while flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
            match upstream.accept() {
                Ok((stream, _)) => held.push(stream),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
        drop(held);
    });

    (upstream_port, flag, handle)
}

/// Start a plain-TCP upstream that accepts and immediately closes
/// connections. The TLS handshake fails quickly (connection reset or
/// EOF), so the tunnel's connection handler completes in < 1 s instead
/// of waiting for the 30 s dial timeout.
fn start_closing_upstream() -> (
    u16,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag_clone = flag.clone();
    let handle = std::thread::spawn(move || {
        upstream
            .set_nonblocking(true)
            .expect("set_nonblocking failed");
        while flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
            match upstream.accept() {
                Ok((_stream, _)) => {
                    // Immediately drop — TLS handshake sees EOF/reset.
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
    });

    (upstream_port, flag, handle)
}

/// Stop an upstream acceptor thread.
fn stop_upstream(
    flag: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: std::thread::JoinHandle<()>,
) {
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = handle.join();
}

// ── F-01: Cooperative Ctrl-C exits 0 ──

/// After a single SIGINT the tunnel must exit with code 0, not 130.
///
/// The tunnel declares ownership of its signal path via
/// `declare_command_owns_interrupt_path()` before entering the accept
/// loop. When SIGINT fires, `spawn_interrupt_handler` reads the sticky
/// one-way flag and defers; `finish_self_owned` reads the same flag and
/// reports the command's own outcome (exit 0).
#[cfg(unix)]
#[test]
fn test_tunnel_exits_zero_after_sigint() {
    let port = free_port();
    let (mut child, rx, _tmp) = spawn_tunnel(port);
    wait_for_ready(&rx);

    // Send SIGINT to the child process.
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(pid, libc::SIGINT);
    }

    let status = child.wait().expect("failed to wait for ags");

    assert_eq!(
        status.code(),
        Some(0),
        "tunnel must exit 0 after SIGINT, got {status:?}",
    );
}

// ── F-03: Crypto provider is installed at the binary level ──

/// Making a TCP connection to the tunnel's local port triggers an
/// upstream TLS dial. Without `ensure_crypto_provider()` in `main.rs`,
/// that dial panics ("no process-level CryptoProvider available").
/// This test asserts the panic does NOT occur — proving the call in
/// `main.rs` is exercised by the production binary.
///
/// The upstream is a real `TcpListener` that accepts connections and
/// holds them, so the client proceeds past TCP connect into the TLS
/// handshake where the crypto provider is required. The expected
/// passing outcome is a TLS/handshake error (the listener speaks no
/// TLS); the failing outcome is a rustls no-provider panic.
#[test]
fn test_tunnel_no_crypto_panic_on_tcp_connection() {
    let (upstream_port, flag, accept_handle) = start_holding_upstream();
    let base_url = format!("https://127.0.0.1:{upstream_port}");

    let port = free_port();
    let (mut child, rx, _tmp) = spawn_tunnel_with_base_url(port, &base_url);
    wait_for_ready(&rx);

    // Connect TCP to the tunnel's local port, which triggers the
    // upstream TLS dial in the accept loop.
    let _ = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));

    // Grace period for the connection handler to fire.
    std::thread::sleep(std::time::Duration::from_millis(1000));

    // Stop the tunnel.
    let _ = child.kill();
    let _ = child.wait();

    stop_upstream(&flag, accept_handle);

    // Collect any remaining stderr.
    let trailing = drain_stderr(&rx);
    let all_stderr: String = trailing.join("\n");

    assert!(
        !all_stderr.contains("panicked"),
        "ags must not panic on TLS connection — \
         ensure_crypto_provider() in main.rs is missing or broken.\n\
         Stderr:\n{all_stderr}"
    );
}

// ── F1: client-connected line appears after a TCP connection ──

/// After a real TCP client connects to the tunnel's local port, stderr
/// must contain a "client connected" event line. The session log emits
/// this event in the accept branch, before the handler task is spawned.
#[test]
fn test_tunnel_client_connected_event_on_stderr() {
    let (upstream_port, flag, accept_handle) = start_closing_upstream();
    let base_url = format!("https://127.0.0.1:{upstream_port}");

    let port = free_port();
    let (mut child, rx, _tmp) = spawn_tunnel_with_base_url(port, &base_url);
    wait_for_ready(&rx);

    // Connect a TCP client to the tunnel.
    let _client = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));

    // Wait for the connection handler to fire and emit events.
    let lines = drain_stderr_for(&rx, std::time::Duration::from_secs(3));

    let _ = child.kill();
    let _ = child.wait();
    stop_upstream(&flag, accept_handle);

    let has_connected = lines
        .iter()
        .any(|l| l.to_lowercase().contains("client connected"));
    assert!(
        has_connected,
        "stderr must contain a 'client connected' line after TCP connect;\n\
         stderr lines:\n{}",
        lines.join("\n")
    );
}

// ── F2: connection lifecycle completes with a session-log event ──

/// After the TCP client connects and the connection handler completes,
/// stderr must contain a session-log lifecycle event with an elapsed
/// prefix. In this test setup the WS dial fails (upstream closes
/// immediately), so `connection_error` fires via the session log. The
/// `client_disconnected` path requires a successful WS relay and is
/// verified by unit tests.
#[test]
fn test_tunnel_connection_lifecycle_event_on_stderr() {
    let (upstream_port, flag, accept_handle) = start_closing_upstream();
    let base_url = format!("https://127.0.0.1:{upstream_port}");

    let port = free_port();
    let (mut child, rx, _tmp) = spawn_tunnel_with_base_url(port, &base_url);
    wait_for_ready(&rx);

    {
        // Connect and immediately drop to trigger the lifecycle.
        let _client = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));
    }

    // Wait for the connection handler to complete.
    let lines = drain_stderr_for(&rx, std::time::Duration::from_secs(5));

    let _ = child.kill();
    let _ = child.wait();
    stop_upstream(&flag, accept_handle);

    // The lifecycle event must come through the session log, which adds
    // the elapsed prefix `[+<n>s]`. The old bare `write_stderr_line`
    // call has no elapsed prefix, so this distinguishes the two paths.
    let has_lifecycle = lines.iter().any(|l| {
        let lower = l.to_lowercase();
        (lower.contains("client disconnected") || lower.contains("connection error"))
            && l.contains("[+")
    });
    assert!(
        has_lifecycle,
        "stderr must contain a session-log lifecycle event (with [+<n>s] prefix)\n\
         after the handler completes;\n\
         stderr lines:\n{}",
        lines.join("\n")
    );
}

// ── F3: every event line carries the elapsed-time prefix ──

/// Every session event line in human format carries a `[+<n>s]`
/// elapsed-time prefix.
#[test]
fn test_tunnel_event_lines_carry_elapsed_prefix() {
    let (upstream_port, flag, accept_handle) = start_closing_upstream();
    let base_url = format!("https://127.0.0.1:{upstream_port}");

    let port = free_port();
    let (mut child, rx, _tmp) = spawn_tunnel_with_base_url(port, &base_url);
    wait_for_ready(&rx);

    let _client = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));

    let lines = drain_stderr_for(&rx, std::time::Duration::from_secs(3));

    let _ = child.kill();
    let _ = child.wait();
    stop_upstream(&flag, accept_handle);

    // Filter to non-empty lines.
    let event_lines: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();

    assert!(
        !event_lines.is_empty(),
        "must see at least one event line after TCP connect"
    );

    for line in &event_lines {
        assert!(
            line.contains("[+"),
            "event line must carry [+<n>s] elapsed prefix: {line}"
        );
    }
}

// ── F4: with --quiet, no event lines appear ──

/// With `--quiet`, the same connection sequence produces no event
/// output on stderr. This protects the existing quiet-mode contract.
#[test]
fn test_tunnel_quiet_suppresses_all_events() {
    let (upstream_port, flag, accept_handle) = start_closing_upstream();
    let base_url = format!("https://127.0.0.1:{upstream_port}");

    let port = free_port();
    let (mut child, rx, _tmp) = spawn_tunnel_with_args(port, &base_url, &["--quiet"]);

    // Ready line is suppressed; wait for port binding instead.
    wait_for_port(port);

    // Allow the wait_for_port connection handler to complete.
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Now make an explicit connection.
    let _client = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));

    // Wait for the handler to complete and any potential output.
    let lines = drain_stderr_for(&rx, std::time::Duration::from_secs(3));

    let _ = child.kill();
    let _ = child.wait();
    stop_upstream(&flag, accept_handle);

    assert!(
        lines.is_empty(),
        "stderr must be empty in --quiet mode; got:\n{}",
        lines.join("\n")
    );
}

// ── F5: with --format json, events are JSON objects ──

/// With `--format json`, every event line parses as JSON and carries
/// `event` and `elapsed_ms` fields.
#[test]
fn test_tunnel_json_format_events() {
    let (upstream_port, flag, accept_handle) = start_closing_upstream();
    let base_url = format!("https://127.0.0.1:{upstream_port}");

    let port = free_port();
    let (mut child, rx, _tmp) = spawn_tunnel_with_args(port, &base_url, &["--format", "json"]);
    wait_for_ready(&rx);

    let _client = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));

    let lines = drain_stderr_for(&rx, std::time::Duration::from_secs(3));

    let _ = child.kill();
    let _ = child.wait();
    stop_upstream(&flag, accept_handle);

    // Every non-empty line must parse as JSON with `event` and `elapsed_ms`.
    let event_lines: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();

    assert!(
        !event_lines.is_empty(),
        "must see at least one event line in JSON mode after TCP connect"
    );

    for line in &event_lines {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("every event line must be valid JSON: {e}\nline: {line}"));
        assert!(
            parsed.get("event").is_some(),
            "JSON event must carry 'event' field: {line}"
        );
        assert!(
            parsed.get("elapsed_ms").is_some(),
            "JSON event must carry 'elapsed_ms' field: {line}"
        );
    }
}

// ── F6: stdout is empty for the whole run ──

/// Stdout must stay empty for the tunnel command, in both human and
/// JSON formats. All output goes to stderr.
#[test]
fn test_tunnel_stdout_is_empty() {
    let (upstream_port, flag, accept_handle) = start_closing_upstream();
    let base_url = format!("https://127.0.0.1:{upstream_port}");

    // Human format.
    {
        let port = free_port();
        let (mut child, rx, stdout, _tmp) = spawn_tunnel_capture_stdout(port, &base_url, &[]);
        wait_for_ready(&rx);

        let _client = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));
        std::thread::sleep(std::time::Duration::from_millis(500));

        let _ = child.kill();
        let _ = child.wait();

        let mut stdout_buf = Vec::new();
        let _ = BufReader::new(stdout).read_to_end(&mut stdout_buf);
        assert!(
            stdout_buf.is_empty(),
            "stdout must be empty in human format; got {} bytes: {:?}",
            stdout_buf.len(),
            String::from_utf8_lossy(&stdout_buf)
        );
    }

    // JSON format.
    {
        let port = free_port();
        let (mut child, rx, stdout, _tmp) =
            spawn_tunnel_capture_stdout(port, &base_url, &["--format", "json"]);
        wait_for_ready(&rx);

        let _client = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));
        std::thread::sleep(std::time::Duration::from_millis(500));

        let _ = child.kill();
        let _ = child.wait();

        let mut stdout_buf = Vec::new();
        let _ = BufReader::new(stdout).read_to_end(&mut stdout_buf);
        assert!(
            stdout_buf.is_empty(),
            "stdout must be empty in JSON format; got {} bytes: {:?}",
            stdout_buf.len(),
            String::from_utf8_lossy(&stdout_buf)
        );
    }

    stop_upstream(&flag, accept_handle);
}

// ── F7: `stopped` event emitted in JSON mode after SIGINT ──
//
// After a graceful SIGINT, the tunnel emits a `stopped` JSON object on
// stderr carrying all six fields: event, status, local_port,
// resource_name, exit_code, and elapsed_ms. This is the only functional
// test that exercises the `session_log.stopped()` wiring.
//
// This test is `#[cfg(unix)]` because it requires `libc::kill` for
// reliable SIGINT delivery. On Windows, `GenerateConsoleCtrlEvent`
// targets a process group and cannot be directed at a single child
// without a dedicated console — no cross-platform equivalent is
// practical. The unit tests in `session_log.rs` cover the format logic
// on all platforms; this test covers the wiring (the `stopped()` call
// in `handle_tunnel` after `run_tunnel` returns `Ok`).
#[cfg(unix)]
#[test]
fn test_tunnel_stopped_event_in_json_after_sigint() {
    let port = free_port();
    let (mut child, rx, _tmp) =
        spawn_tunnel_with_args(port, "http://localhost:1", &["--format", "json"]);
    wait_for_ready(&rx);

    // Send SIGINT for a graceful shutdown.
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(pid, libc::SIGINT);
    }

    let status = child.wait().expect("failed to wait for ags");

    assert_eq!(
        status.code(),
        Some(0),
        "tunnel must exit 0 after SIGINT, got {status:?}",
    );

    // Drain remaining stderr lines — the stopped event is the last line
    // emitted before exit.
    let lines = drain_stderr(&rx);

    let stopped_line = lines.iter().find(|l| l.contains("\"event\":\"stopped\""));
    assert!(
        stopped_line.is_some(),
        "stderr must contain a stopped JSON event after SIGINT;\nstderr lines:\n{}",
        lines.join("\n")
    );

    let parsed: serde_json::Value =
        serde_json::from_str(stopped_line.unwrap()).unwrap_or_else(|e| {
            panic!(
                "stopped line must be valid JSON: {e}\nline: {}",
                stopped_line.unwrap()
            )
        });

    // All six fields from the contract.
    assert_eq!(parsed["event"], "stopped", "event field");
    assert_eq!(parsed["status"], "stopped", "status field");
    assert_eq!(parsed["local_port"], port, "local_port field");
    assert_eq!(parsed["resource_name"], "test-res", "resource_name field");
    assert_eq!(parsed["exit_code"], 0, "exit_code field");
    assert!(
        parsed.get("elapsed_ms").is_some(),
        "stopped JSON must carry elapsed_ms: {parsed}"
    );
}
