//! Functional tests for `ags extend logs stream`.
//!
//! These tests spawn the real `ags` binary against local servers (wiremock
//! for finite responses, plain TCP for long-lived connections) and verify
//! end-to-end behaviours that unit tests cannot reach: exit codes, stdout
//! versus stderr routing, the SSE-to-JSON pipeline, reconnect with
//! Last-Event-ID, and graceful shutdown on SIGINT and closed stdout.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::Stdio;
use std::time::Duration;

use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Helpers ──

/// The CSM log-streaming path for the test namespace and app.
const LOGS_PATH: &str = "/csm/v4/admin/namespaces/test-ns/apps/my-app/logs";

/// Build a `std::process::Command` for `ags extend logs stream` pointed at
/// a local server. Every test variable goes to the child; nothing mutates
/// the test process's own environment.
fn ags_logs_cmd(base_url: &str, extra_args: &[&str]) -> (std::process::Command, tempfile::TempDir) {
    let bin = assert_cmd::cargo::cargo_bin("ags");
    let tmp = tempfile::tempdir().unwrap();

    let mut cmd = std::process::Command::new(bin);
    cmd.args([
        "extend",
        "logs",
        "stream",
        "--app",
        "my-app",
        "--namespace",
        "test-ns",
    ])
    .args(extra_args)
    .env("AGS_HOME", tmp.path())
    .env("AGS_NO_KEYCHAIN", "1")
    .env("AGS_ACCESS_TOKEN", "fake-test-token")
    .env("AGS_BASE_URL", base_url)
    .env("AGS_NO_UPDATE_CHECK", "1")
    .stdin(Stdio::null());
    (cmd, tmp)
}

/// Run `ags extend logs stream` to completion and return
/// (exit code, stdout text, stderr text). Bounded by a timeout so a
/// regression cannot hang the suite.
fn run_to_completion(base_url: &str, extra_args: &[&str]) -> (i32, String, String) {
    let (mut cmd, _tmp) = ags_logs_cmd(base_url, extra_args);
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn ags");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                return (status.code().unwrap_or(-1), stdout, stderr);
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("ags did not exit within 30 s");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("wait failed: {e}"),
        }
    }
}

/// Build a complete SSE body from events.
fn sse_body(events: &[(&str, &str, &str)]) -> String {
    let mut body = String::new();
    for (event_name, id, data) in events {
        if !event_name.is_empty() {
            body.push_str(&format!("event: {event_name}\n"));
        }
        if !id.is_empty() {
            body.push_str(&format!("id: {id}\n"));
        }
        body.push_str(&format!("data: {data}\n\n"));
    }
    body
}

/// Mount a 200 SSE response at LOGS_PATH with the given body.
async fn mount_sse_response(server: &MockServer, body: &str) {
    Mock::given(method("GET"))
        .and(path(LOGS_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(body.to_string()),
        )
        .mount(server)
        .await;
}

/// Mount an error response at LOGS_PATH.
async fn mount_error_response(server: &MockServer, status: u16, body: &str) {
    Mock::given(method("GET"))
        .and(path(LOGS_PATH))
        .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
        .mount(server)
        .await;
}

/// Start a plain-TCP server that sends SSE headers then runs `feeder` on each
/// accepted connection. Returns the port, stop flag, and accept-loop handle.
fn start_sse_tcp_server<F>(
    feeder: F,
) -> (
    u16,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
)
where
    F: Fn(&mut std::net::TcpStream) + Send + 'static,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag_clone = flag.clone();
    let handle = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let mut held = Vec::new();
        while flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
                    let _ = stream.flush();
                    feeder(&mut stream);
                    held.push(stream);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
        drop(held);
    });
    (port, flag, handle)
}

/// Stop a TCP server thread.
fn stop_server(
    flag: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: std::thread::JoinHandle<()>,
) {
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = handle.join();
}

// ── Test 1: stdout is exactly the data lines; exit 0 ──

#[tokio::test]
async fn test_logs_stream_prints_lines_and_exits_zero_on_stream_end() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        ("", "pod-a|2026-09-28T09:14:01Z", "hello from pod"),
        ("", "pod-a|2026-09-28T09:14:02Z", "second line"),
        ("stream-end", "", r#"{"reason":"completed"}"#),
    ]);
    mount_sse_response(&server, &body).await;

    let (code, stdout, stderr) = run_to_completion(&server.uri(), &[]);

    assert_eq!(code, 0, "stream-end must exit 0, stderr:\n{stderr}");

    // stdout contains exactly the data lines.
    let stdout_lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        stdout_lines.iter().any(|l| l.contains("hello from pod")),
        "stdout must contain first log line: {stdout}"
    );
    assert!(
        stdout_lines.iter().any(|l| l.contains("second line")),
        "stdout must contain second log line: {stdout}"
    );

    // stderr has status lines with [+ elapsed prefix.
    let stderr_lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !stderr_lines.is_empty(),
        "stderr must have at least one status line"
    );
    for line in &stderr_lines {
        assert!(
            line.contains("[+"),
            "stderr status line must carry [+Ns] elapsed prefix: {line}"
        );
    }

    // Verify request details via wiremock's received_requests.
    let requests = server.received_requests().await.unwrap();
    assert!(
        !requests.is_empty(),
        "server must have received at least one request"
    );
    let req = &requests[0];

    // The path.
    assert!(
        req.url
            .path()
            .contains("/csm/v4/admin/namespaces/test-ns/apps/my-app/logs"),
        "request path must match: {}",
        req.url.path()
    );

    // The five query parameters.
    let query_str = req.url.query().unwrap_or("");
    assert!(
        query_str.contains("follow="),
        "query must contain follow: {query_str}"
    );
    assert!(
        query_str.contains("tailLines="),
        "query must contain tailLines: {query_str}"
    );
    assert!(
        query_str.contains("previous="),
        "query must contain previous: {query_str}"
    );
    assert!(
        query_str.contains("pods="),
        "query must contain pods: {query_str}"
    );

    // Accept header.
    let accept = req
        .headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        accept.contains("text/event-stream"),
        "Accept header must be text/event-stream: {accept}"
    );

    // Authorization header.
    let auth = req
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        auth, "Bearer fake-test-token",
        "Authorization header must carry the test token: {auth}"
    );
}

// ── Test 2: JSON mode prints one object per line ──

#[tokio::test]
async fn test_logs_stream_json_prints_one_object_per_line() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        ("", "pod-a|2026-09-28T09:14:01Z", "hello"),
        ("pod-done", "", r#"{"pod":"pod-a","reason":"completed"}"#),
        ("stream-end", "", r#"{"reason":"completed"}"#),
    ]);
    mount_sse_response(&server, &body).await;

    let (code, stdout, stderr) = run_to_completion(&server.uri(), &["--format", "json"]);

    assert_eq!(code, 0, "JSON stream must exit 0, stderr:\n{stderr}");
    assert!(
        stderr.trim().is_empty(),
        "stderr must be empty in JSON mode: {stderr}"
    );

    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        lines.len() >= 4,
        "expected at least 4 JSON records (connected, log, pod_done, stream_end), got {}:\n{stdout}",
        lines.len()
    );

    // Every line must parse as JSON.
    let events: Vec<String> = lines
        .iter()
        .map(|l| {
            let parsed: serde_json::Value = serde_json::from_str(l)
                .unwrap_or_else(|e| panic!("each line must be valid JSON ({e}): {l}"));
            parsed["event"].as_str().unwrap_or("").to_string()
        })
        .collect();

    // The sequence must contain connected, log, pod_done, stream_end.
    assert!(
        events.contains(&"connected".to_string()),
        "missing connected: {events:?}"
    );
    assert!(
        events.contains(&"log".to_string()),
        "missing log: {events:?}"
    );
    assert!(
        events.contains(&"pod_done".to_string()),
        "missing pod_done: {events:?}"
    );
    assert!(
        events.contains(&"stream_end".to_string()),
        "missing stream_end: {events:?}"
    );
}

// ── Test 3: follow reconnects with Last-Event-ID ──

#[tokio::test]
async fn test_logs_stream_follow_reconnects_with_last_event_id() {
    let server = MockServer::start().await;

    // First request (no Last-Event-ID): two log lines, then EOF
    // (body closes without stream-end, triggering reconnect).
    let first_body = sse_body(&[
        ("", "pod-a|2026-09-28T09:14:01Z", "first-line"),
        ("", "pod-a|2026-09-28T09:14:02Z", "second-line"),
    ]);

    // Second request (with Last-Event-ID): replays second line before a
    // third, then stream-end.
    let second_body = sse_body(&[
        ("", "pod-a|2026-09-28T09:14:02Z", "second-line"),
        ("", "pod-a|2026-09-28T09:14:03Z", "third-line"),
        ("stream-end", "", r#"{"reason":"completed"}"#),
    ]);

    // Mount reconnect mock first (higher priority in wiremock 0.6):
    // matches when Last-Event-ID is present.
    Mock::given(method("GET"))
        .and(path(LOGS_PATH))
        .and(header_exists("Last-Event-ID"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(second_body),
        )
        .named("reconnect-with-last-event-id")
        .expect(1)
        .mount(&server)
        .await;

    // Initial request mock (no Last-Event-ID restriction).
    Mock::given(method("GET"))
        .and(path(LOGS_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(first_body),
        )
        .named("initial-no-last-event-id")
        .expect(1)
        .mount(&server)
        .await;

    // Use --format json so we can parse events precisely.
    // tailLines=1 on reconnect is tested via the query param in wiremock.
    let (code, stdout, stderr) =
        run_to_completion(&server.uri(), &["--follow", "--format", "json"]);

    assert_eq!(code, 0, "follow-reconnect must exit 0, stderr:\n{stderr}");

    // Parse JSON lines.
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    // Must have three distinct log lines total, no duplicate.
    let log_lines: Vec<&str> = events
        .iter()
        .filter(|e| e["event"] == "log")
        .filter_map(|e| e["line"].as_str())
        .collect();

    assert!(
        log_lines.contains(&"first-line"),
        "must see first-line: {log_lines:?}"
    );
    assert!(
        log_lines.contains(&"second-line"),
        "must see second-line: {log_lines:?}"
    );
    assert!(
        log_lines.contains(&"third-line"),
        "must see third-line: {log_lines:?}"
    );

    // Deduplication: second-line must appear exactly once.
    let second_count = log_lines.iter().filter(|l| **l == "second-line").count();
    assert_eq!(
        second_count, 1,
        "second-line must not be duplicated: {log_lines:?}"
    );

    // wiremock's expect(1) on the reconnect mock validates that
    // Last-Event-ID was sent on exactly one request and tailLines=1.
}

// ── Test 4: without follow, one request, disconnected in JSON ──

#[tokio::test]
async fn test_logs_stream_without_follow_makes_one_request() {
    let server = MockServer::start().await;

    // A body without stream-end: just log data, then EOF.
    let body = sse_body(&[("", "pod-a|2026-09-28T09:14:01Z", "only-line")]);

    Mock::given(method("GET"))
        .and(path(LOGS_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let (code, stdout, _stderr) = run_to_completion(&server.uri(), &["--format", "json"]);

    assert_eq!(code, 0, "one-shot must exit 0");

    let events: Vec<serde_json::Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    // Must have a disconnected record (not reconnecting).
    let has_disconnected = events.iter().any(|e| e["event"] == "disconnected");
    assert!(
        has_disconnected,
        "must see disconnected in JSON output: {stdout}"
    );

    // wiremock's expect(1) validates exactly one request was made.
}

// ── Test 5: error event exits 3 ──

#[tokio::test]
async fn test_logs_stream_error_event_exits_3() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        ("", "pod-a|2026-09-28T09:14:01Z", "normal-line"),
        ("error", "", r#"{"message":"pod crashed"}"#),
    ]);
    mount_sse_response(&server, &body).await;

    let (code, stdout, stderr) = run_to_completion(&server.uri(), &["--format", "json"]);

    assert_eq!(code, 3, "error event must exit 3, stderr:\n{stderr}");

    // The message appears on stderr (human) or in the error record (JSON).
    let all_output = format!("{stdout}{stderr}");
    assert!(
        all_output.contains("pod crashed"),
        "error message must appear in output: stdout={stdout}, stderr={stderr}"
    );

    // In JSON mode, the last record must be an error record.
    let json_lines: Vec<serde_json::Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    if !json_lines.is_empty() {
        let last = json_lines.last().unwrap();
        assert_eq!(
            last["event"], "error",
            "last JSON record must be error: {last}"
        );
    }
}

// ── Test 6: HTTP 404 exits 3 with server message ──

#[tokio::test]
async fn test_logs_stream_http_404_exits_3_with_server_message() {
    let server = MockServer::start().await;
    mount_error_response(
        &server,
        404,
        r#"{"errorCode":20008,"errorMessage":"app not found"}"#,
    )
    .await;

    let (code, _stdout, stderr) = run_to_completion(&server.uri(), &[]);

    assert_eq!(code, 3, "404 must exit 3, stderr:\n{stderr}");

    // The errorMessage is shown; the raw JSON body is not shown verbatim.
    assert!(
        stderr.contains("app not found"),
        "stderr must show the errorMessage: {stderr}"
    );
    // The raw body `{"errorCode":...` must not appear as literal JSON.
    assert!(
        !stderr.contains(r#""errorCode""#),
        "stderr must not show the raw JSON body: {stderr}"
    );

    // The error message line uses the CliError::Api human rendering from
    // PlainFrontend::render_error → render_error_text (file:
    // crates/accelbyte-ags-cli/src/frontend/terminal/plain/frontend.rs:114).
    // With stderr piped (no colour), the line starts with "✕ " (SYMBOL_ERROR).
    // The message comes from session.rs status_to_error:
    //   "HTTP 404: errorCode 20008: app not found" (one colon, not two).
    let expected_error_line = "\u{2715} HTTP 404: errorCode 20008: app not found";
    assert!(
        stderr.lines().any(|l| l.trim() == expected_error_line),
        "stderr must contain the exact error headline (no doubled colon):\n  expected: {expected_error_line}\n  stderr: {stderr}"
    );
}

// ── Test 7: HTTP 403 exits 3 with permission suggestion ──

#[tokio::test]
async fn test_logs_stream_http_403_exits_3_with_permission_suggestion() {
    let server = MockServer::start().await;
    mount_error_response(
        &server,
        403,
        r#"{"errorCode":20013,"errorMessage":"forbidden"}"#,
    )
    .await;

    let (code, _stdout, stderr) = run_to_completion(&server.uri(), &[]);

    assert_eq!(code, 3, "403 must exit 3, stderr:\n{stderr}");
    assert!(
        stderr.contains("EXTEND:APP") && stderr.contains("READ"),
        "suggestion must name EXTEND:APP [READ]: {stderr}"
    );
}

// ── Test 8: connection refused exits 4 without retry ──

#[test]
fn test_logs_stream_first_connection_refused_exits_4_without_retry() {
    // Use a port that nothing listens on.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // Close so nothing listens.

    let start = std::time::Instant::now();
    let (code, _stdout, _stderr) = run_to_completion(&format!("http://127.0.0.1:{port}"), &[]);
    let elapsed = start.elapsed();

    assert_eq!(code, 4, "connection refused must exit 4 (Network)");
    assert!(
        elapsed < Duration::from_secs(5),
        "must fail in under 5 seconds, took {:?}",
        elapsed
    );

    // The error text must include the underlying cause (e.g. "refused"),
    // not just the outer reqwest message. Linux says "Connection refused",
    // Windows says "actively refused it"; both contain "refused".
    let stderr_lower = _stderr.to_lowercase();
    assert!(
        stderr_lower.contains("refused"),
        "stderr must contain the connection-refused cause: {_stderr}"
    );
}

// ── Test 9: dry-run sends no request ──

#[tokio::test]
async fn test_logs_stream_dry_run_sends_no_request() {
    let server = MockServer::start().await;

    // Mount a mock that fails if hit.
    Mock::given(method("GET"))
        .and(path(LOGS_PATH))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let (code, _stdout, stderr) = run_to_completion(&server.uri(), &["--dry-run"]);

    assert_eq!(code, 0, "dry-run must exit 0, stderr:\n{stderr}");

    // The URL is printed.
    let all_output = format!("{_stdout}{stderr}");
    assert!(
        all_output.contains("/csm/v4/admin/namespaces/test-ns/apps/my-app/logs"),
        "dry-run must print the URL: {all_output}"
    );

    // wiremock's expect(0) validates no request was sent.
}

// ── Test 10: quiet prints only log lines ──

#[tokio::test]
async fn test_logs_stream_quiet_prints_only_log_lines() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        ("", "pod-a|2026-09-28T09:14:01Z", "hello-quiet"),
        ("stream-end", "", r#"{"reason":"completed"}"#),
    ]);
    mount_sse_response(&server, &body).await;

    let (code, stdout, stderr) = run_to_completion(&server.uri(), &["--quiet"]);

    assert_eq!(code, 0, "quiet stream must exit 0, stderr:\n{stderr}");
    assert!(
        stderr.trim().is_empty(),
        "stderr must be empty in --quiet mode: {stderr}"
    );
    assert!(
        stdout.contains("hello-quiet"),
        "stdout must still contain log lines: {stdout}"
    );
}

// ── Test 11: follow outlives request timeout ──

#[test]
fn test_logs_stream_follow_outlives_request_timeout() {
    // A TCP server that sends keepalives, then a log line after 3 seconds.
    let (port, flag, handle) = start_sse_tcp_server(|stream| {
        // Send a keepalive every 500 ms for 3 seconds, then a log line.
        for _ in 0..6 {
            let _ = stream.write_all(b": keepalive\n");
            let _ = stream.flush();
            std::thread::sleep(Duration::from_millis(500));
        }
        let _ = stream.write_all(
            b"id: pod-a|2026-09-28T09:14:01Z\ndata: late-line\n\n\
              event: stream-end\ndata: {\"reason\":\"completed\"}\n\n",
        );
        let _ = stream.flush();
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let (code, stdout, stderr) = run_to_completion(&base_url, &["--follow", "--timeout", "2"]);

    stop_server(&flag, handle);

    assert_eq!(code, 0, "follow must outlive --timeout; stderr:\n{stderr}");
    assert!(
        stdout.contains("late-line"),
        "the late line must arrive on stdout: {stdout}"
    );
}

// ── Test 12: stops when stdout closes ──

#[test]
fn test_logs_stream_stops_when_stdout_closes() {
    // A TCP server that sends one line every 100 ms indefinitely.
    let (port, flag, handle) = start_sse_tcp_server(|stream| {
        let mut i = 0u32;
        loop {
            let line = format!("id: pod-a|2026-09-28T09:14:{i:02}Z\ndata: line-{i}\n\n");
            if stream.write_all(line.as_bytes()).is_err() {
                break;
            }
            if stream.flush().is_err() {
                break;
            }
            i += 1;
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let (mut cmd, _tmp) = ags_logs_cmd(&base_url, &["--follow"]);
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn ags");

    // Read one line then drop the reader to close the pipe.
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    let _ = reader.read_line(&mut first_line);
    drop(reader);

    // Wait for the child to exit within 5 seconds.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    stop_server(&flag, handle);
                    panic!("child did not exit after stdout closed within 5 s");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("wait error: {e}"),
        }
    };

    stop_server(&flag, handle);

    assert_eq!(
        status.code(),
        Some(0),
        "logs stream must exit 0 when stdout is closed, got {status:?}"
    );
}

// ── Test 13: SIGINT exits 0 ──

#[cfg(unix)]
#[test]
fn test_logs_stream_exits_zero_after_sigint() {
    // A TCP server that holds connections open (sends SSE headers, then idles).
    let (port, flag, handle) = start_sse_tcp_server(|_stream| {
        // Connection stays open; the feeder does nothing extra.
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let (mut cmd, _tmp) = ags_logs_cmd(&base_url, &["--follow", "--format", "json"]);
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn ags");

    // Wait for the binary to connect.
    std::thread::sleep(Duration::from_secs(2));

    // Send SIGINT.
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(pid, libc::SIGINT);
    }

    // Bounded wait: prevent the suite from hanging if SIGINT fails to
    // terminate the child (the read_timeout + reconnect loop can block
    // for minutes in --follow mode).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    stop_server(&flag, handle);
                    panic!("child did not exit within 10 s after SIGINT");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("wait error: {e}"),
        }
    };

    // Read stdout before stopping the server.
    let mut stdout_text = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout_text);
    }

    stop_server(&flag, handle);

    assert_eq!(
        status.code(),
        Some(0),
        "logs stream must exit 0 after SIGINT, got {status:?}"
    );

    // The last JSON record must be "stopped".
    let json_lines: Vec<serde_json::Value> = stdout_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    if !json_lines.is_empty() {
        let last = json_lines.last().unwrap();
        assert_eq!(
            last["event"], "stopped",
            "last JSON record must be stopped: {last}"
        );
    }
}

// ── Test: HTTP 401 exits 2 when the session cannot be refreshed ──

#[tokio::test]
async fn test_logs_stream_http_401_exits_2_when_the_session_cannot_be_refreshed() {
    let server = MockServer::start().await;
    mount_error_response(
        &server,
        401,
        r#"{"errorCode":20001,"errorMessage":"unauthorized"}"#,
    )
    .await;

    let (code, _stdout, stderr) = run_to_completion(&server.uri(), &[]);

    assert_eq!(
        code, 2,
        "401 with no refresh must exit 2, stderr:\n{stderr}"
    );

    // The server must receive exactly one request: a token from the
    // environment has nothing to refresh, so the command does not retry.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "exactly one request when the token cannot be refreshed: {}",
        requests.len()
    );

    // The error line on stderr uses CliError::Auth rendered by
    // PlainFrontend::render_error -> render_error_text
    // (crates/accelbyte-ags-cli/src/frontend/terminal/plain/frontend.rs:127).
    // SYMBOL_ERROR (\u{2715}) is the prefix for the error headline.
    let expected_error_line = "\u{2715} authentication failed and session could not be refreshed";
    assert_eq!(
        stderr
            .lines()
            .find(|l| l.contains("authentication failed")),
        Some(expected_error_line),
        "stderr must contain the exact auth error line:\n  expected: {expected_error_line}\n  stderr: {stderr}"
    );
}

// ── Test: HTTP 401 JSON ends with the error record ──

#[tokio::test]
async fn test_logs_stream_http_401_json_ends_with_the_error_record() {
    let server = MockServer::start().await;
    mount_error_response(
        &server,
        401,
        r#"{"errorCode":20001,"errorMessage":"unauthorized"}"#,
    )
    .await;

    let (code, stdout, stderr) = run_to_completion(&server.uri(), &["--format", "json"]);

    assert_eq!(code, 2, "401 JSON must exit 2, stderr:\n{stderr}");

    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "stdout must have at least one JSON record"
    );

    let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap_or_else(|e| {
        panic!(
            "last line must be valid JSON ({e}): {}",
            lines.last().unwrap()
        )
    });

    assert_eq!(
        last["event"], "error",
        "last JSON record must be error: {last}"
    );
    assert_eq!(last["exit_code"], 2, "exit_code must be 2: {last}");
}

// ── Test: HTTP 429 exits 3 with the stream limit hint ──

#[tokio::test]
async fn test_logs_stream_http_429_exits_3_with_the_stream_limit_hint() {
    let server = MockServer::start().await;
    mount_error_response(
        &server,
        429,
        r#"{"errorCode":20024,"errorMessage":"rate limited"}"#,
    )
    .await;

    let (code, _stdout, stderr) = run_to_completion(&server.uri(), &[]);

    assert_eq!(code, 3, "429 must exit 3, stderr:\n{stderr}");

    // The suggestion line is rendered by render_error -> SYMBOL_FIX + " Fix: " + suggestion
    // (crates/accelbyte-ags-cli/src/frontend/output/templates.rs:70-72).
    let expected_hint_line = "\u{2192} Fix: At most 10 log streams may be open per namespace at once, counted for everyone in that namespace. A stream stopped less than a minute ago may still count, so wait a minute before running the command again";
    assert_eq!(
        stderr.lines().find(|l| l.contains("10 log streams")),
        Some(expected_hint_line),
        "stderr must contain the exact hint line:\n  expected: {expected_hint_line}\n  stderr: {stderr}"
    );
}

// ── Test: --pods 0 exits without the 32-bit maximum ──

#[test]
fn test_logs_stream_pods_zero_exits_1_without_the_32_bit_maximum() {
    // Use a dummy URL; clap rejects --pods 0 before any network call.
    let (code, _stdout, stderr) = run_to_completion("http://127.0.0.1:1", &["--pods", "0"]);

    // The custom value parser (`parse_at_least_one_u32`) error is caught by
    // the CLI's clap error handler, which wraps it as CliError::Usage (exit 1).
    assert_eq!(
        code, 1,
        "pods=0 must exit 1 (CliError::Usage), stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("4294967295"),
        "stderr must not contain the 32-bit maximum: {stderr}"
    );

    // The stderr line that carries the custom message.
    let expected_line = "\u{2715} Invalid value '0' for '--pods <count>': must be at least 1";
    assert_eq!(
        stderr.lines().find(|l| l.contains("must be at least 1")),
        Some(expected_line),
        "stderr must contain the exact error line:\n  expected: {expected_line}\n  stderr: {stderr}"
    );
}

// ── Test 14: help lists flags ──

#[test]
fn test_logs_stream_help_lists_flags() {
    let bin = assert_cmd::cargo::cargo_bin("ags");
    let tmp = tempfile::tempdir().unwrap();

    let output = std::process::Command::new(bin)
        .args(["extend", "logs", "stream", "--help"])
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let all = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));

    // The six flags.
    assert!(all.contains("--app"), "help must list --app: {all}");
    assert!(all.contains("--follow"), "help must list --follow: {all}");
    assert!(
        all.contains("--tail-lines") || all.contains("--tail"),
        "help must list --tail-lines: {all}"
    );
    assert!(all.contains("--pods"), "help must list --pods: {all}");
    assert!(
        all.contains("--since-seconds"),
        "help must list --since-seconds: {all}"
    );
    assert!(
        all.contains("--previous"),
        "help must list --previous: {all}"
    );

    // The global namespace flag.
    assert!(
        all.contains("--namespace"),
        "help must list --namespace: {all}"
    );

    // The note about JSON output under --follow never ending.
    assert!(
        all.contains("never ends") || all.contains("stop it with Ctrl-C"),
        "help must note that JSON output under --follow never ends: {all}"
    );
}

// ── Test 15: describe lists logs stream ──

#[test]
fn test_describe_lists_logs_stream() {
    let bin = assert_cmd::cargo::cargo_bin("ags");
    let tmp = tempfile::tempdir().unwrap();

    let output = std::process::Command::new(&bin)
        .args(["--format", "json", "describe", "extend", "logs", "stream"])
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("describe output must be valid JSON ({e}):\n{stdout}");
    });

    // The describe envelope must report the command.
    let kind = parsed["kind"].as_str().unwrap_or("");
    assert!(
        !kind.is_empty(),
        "describe must have a kind field: {stdout}"
    );

    // Flags must be listed.
    let data_str = parsed["data"].to_string();
    assert!(
        data_str.contains("app") || data_str.contains("follow"),
        "describe must report flags: {stdout}"
    );
}

// ── Test 16: stdout write failure exits 1 ──

#[cfg(target_os = "linux")]
#[test]
fn test_logs_stream_stdout_write_failure_exits_1() {
    // /dev/full returns ENOSPC on every write, simulating a non-BrokenPipe
    // stdout failure. This device exists only on Linux, so the test is
    // compiled out on other platforms; the unit-test siblings in output.rs
    // and session.rs are the CI-runnable coverage on every platform.
    let (port, flag, handle) = start_sse_tcp_server(|stream| {
        let mut i = 0u32;
        loop {
            let line = format!("id: pod-a|2026-09-28T09:14:{i:02}Z\ndata: line-{i}\n\n");
            if stream.write_all(line.as_bytes()).is_err() {
                break;
            }
            if stream.flush().is_err() {
                break;
            }
            i += 1;
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let (mut cmd, _tmp) = ags_logs_cmd(&base_url, &["--follow"]);

    let dev_full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full must be openable");

    let mut child = cmd
        .stdout(Stdio::from(dev_full))
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn ags");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    stop_server(&flag, handle);
                    panic!("child did not exit within 10 s with stdout to /dev/full");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("wait error: {e}"),
        }
    };

    let mut stderr_text = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr_text);
    }

    stop_server(&flag, handle);

    assert_eq!(
        status.code(),
        Some(1),
        "stdout write failure must exit 1, got {status:?}, stderr:\n{stderr_text}"
    );
    assert!(
        stderr_text.contains("Cannot write to stdout"),
        "stderr must contain 'Cannot write to stdout': {stderr_text}"
    );
}

// ── Test 17: dry-run JSON exits 0 when stdout is closed (SIGPIPE) ──
// Windows has no SIGPIPE; a closed pipe there is a BrokenPipe error the helper already reports.
#[cfg(unix)]
#[tokio::test]
async fn test_logs_stream_dry_run_json_exits_zero_when_stdout_is_closed() {
    let server = MockServer::start().await;

    // Mount a mock that fails if hit — dry-run must not send a request.
    Mock::given(method("GET"))
        .and(path(LOGS_PATH))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let (mut cmd, _tmp) = ags_logs_cmd(&server.uri(), &["--dry-run", "--format", "json"]);

    // Create a pipe and drop the reader before spawning, so stdout is
    // immediately closed for the child.
    let (reader, writer) = std::io::pipe().expect("failed to create pipe");
    drop(reader);

    let mut child = cmd
        .stdout(Stdio::from(writer))
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn ags");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not exit within 10 s");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("wait error: {e}"),
        }
    };

    let mut stderr_text = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr_text);
    }

    assert_eq!(
        status.code(),
        Some(0),
        "dry-run JSON must exit 0 when stdout is closed, got {status:?}, stderr:\n{stderr_text}"
    );
}
