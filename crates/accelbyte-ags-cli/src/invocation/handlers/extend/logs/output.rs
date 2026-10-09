//! Output writer for `ags extend logs stream`.
//!
//! `LogStreamOutput` is the ONLY code in the command that writes to stdout or
//! stderr. It decides for every event whether and where it is written, in one
//! place. This is the same single-owner rule `session_log.rs` states for the
//! sibling commands.
//!
//! In JSON mode every record goes to stdout as one JSON object per line,
//! in event order. Nothing goes to stderr on success. Quiet removes nothing.
//!
//! In human mode log lines go to stdout (after control-sequence stripping)
//! and status lines go to stderr with the `[+Ns]` elapsed prefix. Quiet
//! removes status and verbose lines but never log lines.

use std::time::Duration;

use ags_protocol::request::Verbosity;
use serde::Serialize;

use super::resume::pod_from_event_id;
use crate::errors::CliError;
use crate::frontend::StdoutLineWrite;
use crate::invocation::handlers::extend::session_log::format_elapsed;

// ── JSON record types ──

/// Reason a reconnect is initiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReconnectReason {
    ClosedWithoutStreamEnd,
    TransportError,
    IdleTimeout,
    ServerUnavailable,
}

/// A single JSON record emitted by `ags extend logs stream --format json`.
///
/// Every variant serializes with a `event` discriminator and all fields listed
/// in the Spec's record table. An absent value serializes as `null` (no
/// `skip_serializing_if`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum LogStreamRecord {
    Connected {
        reconnect: u32,
        elapsed_ms: u64,
    },
    Log {
        id: Option<String>,
        pod: Option<String>,
        line: String,
        elapsed_ms: u64,
    },
    PodDone {
        pod: Option<String>,
        reason: Option<String>,
        message: Option<String>,
        elapsed_ms: u64,
    },
    Reconnecting {
        reconnect: u32,
        max_reconnects: u32,
        delay_ms: u64,
        last_event_id: Option<String>,
        reason: ReconnectReason,
        detail: Option<String>,
        elapsed_ms: u64,
    },
    StreamEnd {
        reason: Option<String>,
        elapsed_ms: u64,
    },
    Disconnected {
        elapsed_ms: u64,
    },
    Stopped {
        elapsed_ms: u64,
    },
    GaveUp {
        reconnects: u32,
        last_event_id: Option<String>,
        elapsed_ms: u64,
    },
    Error {
        message: String,
        exit_code: i32,
        elapsed_ms: u64,
    },
    DryRun {
        method: String,
        url: String,
        elapsed_ms: u64,
    },
}

// ── Trait the reconnect loop is generic over ──

/// Parameters for the `opening` event, bundled to stay within the
/// argument-count limit.
pub(crate) struct OpeningParams<'a> {
    pub(crate) namespace: &'a str,
    pub(crate) app: &'a str,
    pub(crate) pods: u32,
    pub(crate) tail: u32,
    pub(crate) follow: bool,
    pub(crate) since: Option<u32>,
    pub(crate) previous: bool,
}

/// Output seam for the reconnect loop. Each method corresponds to one event
/// the loop can report. Methods that write to stdout return the write
/// result; the loop stops the command on a closed pipe or a write error.
pub(crate) trait LogStreamWriter {
    /// The command is opening a log stream. Emitted once before the first
    /// connection attempt.
    fn opening(&mut self, params: &OpeningParams<'_>);

    /// A connection succeeded.
    fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError>;

    /// A log line arrived.
    fn log_line(&mut self, id: Option<&str>, data: &str) -> Result<StdoutLineWrite, CliError>;

    /// A `pod-done` lifecycle event arrived.
    fn pod_done(&mut self, data: &str) -> Result<StdoutLineWrite, CliError>;

    /// The loop is about to wait and reconnect.
    fn reconnecting(
        &mut self,
        reconnect: u32,
        max_reconnects: u32,
        delay_ms: u64,
        last_event_id: Option<&str>,
        reason: ReconnectReason,
        detail: Option<&str>,
    ) -> Result<StdoutLineWrite, CliError>;

    /// The server sent `stream-end`. Terminal.
    fn stream_end(&mut self, data: &str) -> Result<StdoutLineWrite, CliError>;

    /// Without `--follow`, the connection closed without `stream-end`. Terminal.
    fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError>;

    /// Ctrl-C. Terminal.
    fn stopped(&mut self) -> Result<StdoutLineWrite, CliError>;

    /// The reconnect budget is exhausted. Terminal.
    fn gave_up(
        &mut self,
        reconnects: u32,
        last_event_id: Option<&str>,
    ) -> Result<StdoutLineWrite, CliError>;

    /// An error after input validation. Terminal.
    fn error(&mut self, message: &str, exit_code: i32) -> Result<StdoutLineWrite, CliError>;

    /// A verbose status line (request URL, raw lifecycle event data).
    fn verbose(&mut self, message: &str);

    /// `--dry-run`: the request the first connection would send.
    fn dry_run(
        &mut self,
        method: &str,
        url: &str,
        namespace: &str,
        app: &str,
    ) -> Result<StdoutLineWrite, CliError>;
}

// ── Production implementation ──

/// Type aliases for the writer functions, enabling test injection.
pub(crate) type StdoutWriteFn =
    Box<dyn Fn(&str) -> Result<StdoutLineWrite, crate::errors::CliError>>;
pub(crate) type StderrWriteFn = Box<dyn Fn(&str)>;

/// The production output writer. Holds verbosity, format, follow flag,
/// and the two writer functions so tests can capture lines without
/// touching real stdout/stderr.
pub(crate) struct LogStreamOutput {
    verbosity: Verbosity,
    format_json: bool,
    follow: bool,
    elapsed: Box<dyn Fn() -> Duration>,
    write_stdout: StdoutWriteFn,
    write_stderr: StderrWriteFn,
    /// Whether the "receiving logs" hint has been printed.
    receiving_hint_shown: bool,
}

impl LogStreamOutput {
    /// Build a production output writer.
    pub(crate) fn new(
        verbosity: Verbosity,
        format_json: bool,
        follow: bool,
        elapsed: Box<dyn Fn() -> Duration>,
        write_stdout: StdoutWriteFn,
        write_stderr: StderrWriteFn,
    ) -> Self {
        Self {
            verbosity,
            format_json,
            follow,
            elapsed,
            write_stdout,
            write_stderr,
            receiving_hint_shown: false,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        (self.elapsed)().as_millis() as u64
    }

    fn elapsed_duration(&self) -> Duration {
        (self.elapsed)()
    }

    /// Write one JSON record to stdout.
    fn write_json_record(&self, record: &LogStreamRecord) -> Result<StdoutLineWrite, CliError> {
        let line =
            serde_json::to_string(record).unwrap_or_else(|_| r#"{"event":"unknown"}"#.to_string());
        (self.write_stdout)(&line)
    }

    /// Write a status line to stderr (human mode, not quiet).
    ///
    /// Stderr writes are best-effort, as `write_stderr_line` is everywhere
    /// in this CLI, so only stdout writes can stop the command.
    fn write_status(&self, text: &str) {
        if self.format_json || self.verbosity.is_quiet() {
            return;
        }
        let text = ags_runtime::support::strings::strip_terminal_control_sequences(text);
        let line = status_line(&format!(
            "{} {}",
            format_elapsed(self.elapsed_duration()),
            text
        ));
        (self.write_stderr)(&line);
    }

    /// Write a verbose line to stderr (human mode, verbose only).
    fn write_verbose(&self, text: &str) {
        if self.format_json || !self.verbosity.is_verbose() {
            return;
        }
        let text = ags_runtime::support::strings::strip_terminal_control_sequences(text);
        let line = status_line(&format!(
            "{} {}",
            format_elapsed(self.elapsed_duration()),
            text
        ));
        (self.write_stderr)(&line);
    }

    /// Parse `pod-done` data as JSON `{"pod","reason","message"}`.
    /// When the data is not that JSON, `pod` and `reason` are `None` and
    /// `message` holds the raw data.
    fn parse_pod_done(data: &str) -> (Option<String>, Option<String>, Option<String>) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
            let pod = v.get("pod").and_then(|v| v.as_str()).map(String::from);
            let reason = v.get("reason").and_then(|v| v.as_str()).map(String::from);
            let message = v.get("message").and_then(|v| v.as_str()).map(String::from);
            (pod, reason, message)
        } else {
            (None, None, Some(data.to_string()))
        }
    }

    /// Parse `stream-end` data for the `reason` field.
    fn parse_stream_end_reason(data: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(data)
            .ok()
            .and_then(|v| v.get("reason").and_then(|v| v.as_str()).map(String::from))
    }
}

impl LogStreamWriter for LogStreamOutput {
    fn opening(&mut self, params: &OpeningParams<'_>) {
        if self.format_json {
            return;
        }
        if self.verbosity.is_quiet() {
            return;
        }

        let mut parts = format!(
            "opening log stream  namespace={}  app={}  pods={}  tail={}  follow={}",
            params.namespace, params.app, params.pods, params.tail, params.follow,
        );
        if let Some(s) = params.since {
            parts.push_str(&format!("  since={}s", s));
        }
        if params.previous {
            parts.push_str("  previous=true");
        }

        let line = status_line(&format!(
            "{} {}",
            format_elapsed(self.elapsed_duration()),
            parts
        ));
        (self.write_stderr)(&line);
    }

    fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError> {
        if self.format_json {
            return self.write_json_record(&LogStreamRecord::Connected {
                reconnect,
                elapsed_ms: self.elapsed_ms(),
            });
        }

        let text = if reconnect == 0 {
            "connected, fetching recent lines from pods (Ctrl-C to stop)".to_string()
        } else {
            "connected, resuming log stream".to_string()
        };
        self.write_status(&text);
        Ok(StdoutLineWrite::Written)
    }

    fn log_line(&mut self, id: Option<&str>, data: &str) -> Result<StdoutLineWrite, CliError> {
        // Show the "receiving logs" hint once, at the first log line, only
        // with --follow, in human mode, not quiet.
        if self.follow && !self.receiving_hint_shown {
            self.receiving_hint_shown = true;
            self.write_status("receiving logs; quiet apps may log only every few seconds");
        }

        if self.format_json {
            let pod = id.and_then(pod_from_event_id).map(String::from);
            return self.write_json_record(&LogStreamRecord::Log {
                id: id.map(String::from),
                pod,
                line: data.to_string(),
                elapsed_ms: self.elapsed_ms(),
            });
        }

        // Human mode: strip control sequences and write to stdout.
        let cleaned = ags_runtime::support::strings::strip_terminal_control_sequences(data);
        (self.write_stdout)(&cleaned)
    }

    fn pod_done(&mut self, data: &str) -> Result<StdoutLineWrite, CliError> {
        let (pod, reason, message) = Self::parse_pod_done(data);

        if self.format_json {
            return self.write_json_record(&LogStreamRecord::PodDone {
                pod,
                reason,
                message,
                elapsed_ms: self.elapsed_ms(),
            });
        }

        // Human mode: status line.
        if let (Some(p), Some(r)) = (&pod, &reason) {
            let mut text = format!("pod stream finished  pod={}  reason={}", p, r);
            if let Some(ref m) = message {
                let trimmed = m.trim();
                if !trimmed.is_empty() {
                    text.push_str(&format!("  message={}", trimmed));
                }
            }
            self.write_status(&text);
        } else {
            self.write_status(&format!("pod stream finished  data={}", data));
        }
        Ok(StdoutLineWrite::Written)
    }

    fn reconnecting(
        &mut self,
        reconnect: u32,
        max_reconnects: u32,
        delay_ms: u64,
        last_event_id: Option<&str>,
        reason: ReconnectReason,
        detail: Option<&str>,
    ) -> Result<StdoutLineWrite, CliError> {
        if self.format_json {
            let record = LogStreamRecord::Reconnecting {
                reconnect,
                max_reconnects,
                delay_ms,
                last_event_id: last_event_id.map(String::from),
                reason,
                detail: detail.map(String::from),
                elapsed_ms: self.elapsed_ms(),
            };
            return self.write_json_record(&record);
        }

        // Human mode: disconnect cause line, then reconnecting line.
        let cause = match reason {
            ReconnectReason::ClosedWithoutStreamEnd => CLOSED_WITHOUT_STREAM_END_CAUSE.to_string(),
            ReconnectReason::IdleTimeout => {
                format!(
                    "no data for {}s",
                    super::transport::STREAM_IDLE_READ_TIMEOUT.as_secs()
                )
            }
            ReconnectReason::TransportError => match detail {
                Some(d) => format!("transport error: {d}"),
                None => "transport error".to_string(),
            },
            ReconnectReason::ServerUnavailable => match detail {
                Some(d) => format!("server unavailable: {d}"),
                None => "server unavailable".to_string(),
            },
        };
        self.write_status(&format!("stream disconnected: {cause}"));

        let delay_secs = delay_ms / 1000;
        let resume_part = match last_event_id {
            Some(id) => format!(", resuming after {}", id),
            None => String::new(),
        };
        self.write_status(&format!(
            "reconnecting ({} of {}) in {}s{}",
            reconnect, max_reconnects, delay_secs, resume_part
        ));
        Ok(StdoutLineWrite::Written)
    }

    fn stream_end(&mut self, data: &str) -> Result<StdoutLineWrite, CliError> {
        let reason = Self::parse_stream_end_reason(data);

        if self.format_json {
            return self.write_json_record(&LogStreamRecord::StreamEnd {
                reason,
                elapsed_ms: self.elapsed_ms(),
            });
        }

        self.write_status("stream ended normally");
        Ok(StdoutLineWrite::Written)
    }

    fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError> {
        if self.format_json {
            return self.write_json_record(&LogStreamRecord::Disconnected {
                elapsed_ms: self.elapsed_ms(),
            });
        }

        self.write_status(&format!(
            "stream disconnected: {CLOSED_WITHOUT_STREAM_END_CAUSE}"
        ));
        Ok(StdoutLineWrite::Written)
    }

    fn stopped(&mut self) -> Result<StdoutLineWrite, CliError> {
        if self.format_json {
            return self.write_json_record(&LogStreamRecord::Stopped {
                elapsed_ms: self.elapsed_ms(),
            });
        }

        self.write_status("stopped");
        Ok(StdoutLineWrite::Written)
    }

    fn gave_up(
        &mut self,
        reconnects: u32,
        last_event_id: Option<&str>,
    ) -> Result<StdoutLineWrite, CliError> {
        if self.format_json {
            return self.write_json_record(&LogStreamRecord::GaveUp {
                reconnects,
                last_event_id: last_event_id.map(String::from),
                elapsed_ms: self.elapsed_ms(),
            });
        }

        // Human mode: gave up and error write nothing — the frontend renders
        // the error once.
        Ok(StdoutLineWrite::Written)
    }

    fn error(&mut self, message: &str, exit_code: i32) -> Result<StdoutLineWrite, CliError> {
        if self.format_json {
            return self.write_json_record(&LogStreamRecord::Error {
                message: message.to_string(),
                exit_code,
                elapsed_ms: self.elapsed_ms(),
            });
        }

        // Human mode: gave up and error write nothing — the frontend renders
        // the error once.
        Ok(StdoutLineWrite::Written)
    }

    fn verbose(&mut self, message: &str) {
        self.write_verbose(message);
    }

    fn dry_run(
        &mut self,
        method: &str,
        url: &str,
        namespace: &str,
        app: &str,
    ) -> Result<StdoutLineWrite, CliError> {
        if self.format_json {
            return self.write_json_record(&LogStreamRecord::DryRun {
                method: method.to_string(),
                url: url.to_string(),
                elapsed_ms: self.elapsed_ms(),
            });
        }

        // Human mode: four-line block to stderr.
        if !self.verbosity.is_quiet() {
            let block = format!(
                "Dry run: would open a log stream. No request sent.\n\
                 \x20 Namespace: {namespace}\n\
                 \x20 App:       {app}\n\
                 \x20 Request:   {method} {url}",
            );
            (self.write_stderr)(&block);
        }
        Ok(StdoutLineWrite::Written)
    }
}

// ── Shared cause text ──

/// The human-readable cause text for a connection that closed without
/// `stream-end`. Used by both `disconnected()` (terminal) and
/// `reconnecting()` (non-terminal) so the wording is always identical.
const CLOSED_WITHOUT_STREAM_END_CAUSE: &str = "connection closed without stream-end";

// ── Shared formatting helpers ──

/// Style a status line the same way `session_log.rs` does: dim text with the
/// `SYMBOL_STATUS` prefix, respecting NO_COLOR.
fn status_line(text: &str) -> String {
    crate::frontend::style::ansi::status(text, crate::frontend::style::ansi::is_stderr_enabled())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::rc::Rc;
    use std::time::Duration;

    /// Captured lines for test assertions.
    struct CapturedOutput {
        stdout: Rc<RefCell<Vec<String>>>,
        stderr: Rc<RefCell<Vec<String>>>,
    }

    impl CapturedOutput {
        fn new() -> Self {
            Self {
                stdout: Rc::new(RefCell::new(Vec::new())),
                stderr: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    fn make_output(
        captured: &CapturedOutput,
        verbosity: Verbosity,
        format_json: bool,
        follow: bool,
    ) -> LogStreamOutput {
        let stdout = captured.stdout.clone();
        let stderr = captured.stderr.clone();
        LogStreamOutput::new(
            verbosity,
            format_json,
            follow,
            Box::new(|| Duration::from_secs(42)),
            Box::new(move |text: &str| {
                stdout.borrow_mut().push(text.to_string());
                Ok(StdoutLineWrite::Written)
            }),
            Box::new(move |text: &str| {
                stderr.borrow_mut().push(text.to_string());
            }),
        )
    }

    // ── Test: each_record_serializes_with_exactly_its_documented_keys ──

    fn key_set(json_str: &str) -> BTreeSet<String> {
        let v: serde_json::Value = serde_json::from_str(json_str).unwrap();
        v.as_object().unwrap().keys().cloned().collect()
    }

    fn expected_keys(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn each_record_serializes_with_exactly_its_documented_keys() {
        // connected: event, reconnect, elapsed_ms
        let r = LogStreamRecord::Connected {
            reconnect: 0,
            elapsed_ms: 100,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "reconnect", "elapsed_ms"]),
            "connected keys"
        );

        // log: event, id, pod, line, elapsed_ms
        let r = LogStreamRecord::Log {
            id: Some("a|1".into()),
            pod: Some("a".into()),
            line: "hello".into(),
            elapsed_ms: 200,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "id", "pod", "line", "elapsed_ms"]),
            "log keys"
        );

        // log with null id/pod
        let r = LogStreamRecord::Log {
            id: None,
            pod: None,
            line: "hello".into(),
            elapsed_ms: 200,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "id", "pod", "line", "elapsed_ms"]),
            "log keys with nulls"
        );

        // pod_done: event, pod, reason, message, elapsed_ms
        let r = LogStreamRecord::PodDone {
            pod: Some("a".into()),
            reason: Some("eof".into()),
            message: None,
            elapsed_ms: 300,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "pod", "reason", "message", "elapsed_ms"]),
            "pod_done keys"
        );

        // reconnecting: event, reconnect, max_reconnects, delay_ms,
        //   last_event_id, reason, detail, elapsed_ms
        let r = LogStreamRecord::Reconnecting {
            reconnect: 1,
            max_reconnects: 5,
            delay_ms: 5000,
            last_event_id: Some("a|1".into()),
            reason: ReconnectReason::ClosedWithoutStreamEnd,
            detail: None,
            elapsed_ms: 400,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&[
                "event",
                "reconnect",
                "max_reconnects",
                "delay_ms",
                "last_event_id",
                "reason",
                "detail",
                "elapsed_ms"
            ]),
            "reconnecting keys"
        );

        // stream_end: event, reason, elapsed_ms
        let r = LogStreamRecord::StreamEnd {
            reason: Some("completed".into()),
            elapsed_ms: 500,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "reason", "elapsed_ms"]),
            "stream_end keys"
        );

        // disconnected: event, elapsed_ms
        let r = LogStreamRecord::Disconnected { elapsed_ms: 600 };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "elapsed_ms"]),
            "disconnected keys"
        );

        // stopped: event, elapsed_ms
        let r = LogStreamRecord::Stopped { elapsed_ms: 700 };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "elapsed_ms"]),
            "stopped keys"
        );

        // gave_up: event, reconnects, last_event_id, elapsed_ms
        let r = LogStreamRecord::GaveUp {
            reconnects: 5,
            last_event_id: Some("a|1".into()),
            elapsed_ms: 800,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "reconnects", "last_event_id", "elapsed_ms"]),
            "gave_up keys"
        );

        // error: event, message, exit_code, elapsed_ms
        let r = LogStreamRecord::Error {
            message: "oops".into(),
            exit_code: 3,
            elapsed_ms: 900,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "message", "exit_code", "elapsed_ms"]),
            "error keys"
        );

        // dry_run: event, method, url, elapsed_ms
        let r = LogStreamRecord::DryRun {
            method: "GET".into(),
            url: "https://example.com".into(),
            elapsed_ms: 1000,
        };
        assert_eq!(
            key_set(&serde_json::to_string(&r).unwrap()),
            expected_keys(&["event", "method", "url", "elapsed_ms"]),
            "dry_run keys"
        );
    }

    // ── Test: human_log_line_strips_control_sequences ──

    #[test]
    fn human_log_line_strips_control_sequences() {
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);

        // An ANSI escape in the data is removed; a tab is kept.
        let data = "hello\x1b[31m red \tworld";
        let _ = out.log_line(Some("pod|ts"), data);

        let stdout = cap.stdout.borrow();
        assert_eq!(stdout.len(), 1);
        assert!(
            !stdout[0].contains("\x1b"),
            "escape should be stripped: {}",
            stdout[0]
        );
        assert!(
            stdout[0].contains('\t'),
            "tab should be kept: {}",
            stdout[0]
        );
        assert!(
            stdout[0].contains("hello"),
            "text should be kept: {}",
            stdout[0]
        );
    }

    // ── Test: json_log_line_keeps_data_unchanged ──

    #[test]
    fn json_log_line_keeps_data_unchanged() {
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, true, false);

        let data = "hello\x1b[31m red \tworld";
        let _ = out.log_line(Some("pod|ts"), data);

        let stdout = cap.stdout.borrow();
        assert_eq!(stdout.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&stdout[0]).unwrap();
        assert_eq!(
            parsed["line"].as_str().unwrap(),
            data,
            "JSON mode keeps data unchanged"
        );
    }

    // ── Test: status_lines_carry_elapsed_prefix ──

    #[test]
    fn status_lines_carry_elapsed_prefix() {
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);

        let _ = out.connected(0);
        out.opening(&OpeningParams {
            namespace: "ns",
            app: "app",
            pods: 1,
            tail: 100,
            follow: false,
            since: None,
            previous: false,
        });

        let stderr = cap.stderr.borrow();
        assert!(stderr.len() >= 2, "expected at least 2 stderr lines");
        for line in stderr.iter() {
            assert!(
                line.contains("[+42s]"),
                "status line must contain elapsed prefix: {}",
                line
            );
        }
    }

    // ── Test: quiet_removes_status_lines_but_not_log_lines ──

    #[test]
    fn quiet_removes_status_lines_but_not_log_lines() {
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Quiet, false, false);

        out.opening(&OpeningParams {
            namespace: "ns",
            app: "app",
            pods: 1,
            tail: 100,
            follow: false,
            since: None,
            previous: false,
        });
        let _ = out.connected(0);
        let _ = out.log_line(Some("pod|ts"), "hello");
        let _ = out.disconnected();

        let stderr = cap.stderr.borrow();
        assert!(
            stderr.is_empty(),
            "quiet mode must suppress all status lines, got: {:?}",
            *stderr
        );

        let stdout = cap.stdout.borrow();
        assert_eq!(
            stdout.len(),
            1,
            "quiet mode must keep log lines, got: {:?}",
            *stdout
        );
        assert!(stdout[0].contains("hello"));
    }

    // ── Test: quiet_does_not_remove_json_records ──

    #[test]
    fn quiet_does_not_remove_json_records() {
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Quiet, true, false);

        let _ = out.connected(0);
        let _ = out.log_line(Some("pod|ts"), "hello");
        let _ = out.disconnected();

        let stdout = cap.stdout.borrow();
        // All three records must be on stdout.
        assert_eq!(
            stdout.len(),
            3,
            "quiet JSON mode must keep all records, got: {:?}",
            *stdout
        );

        let stderr = cap.stderr.borrow();
        assert!(
            stderr.is_empty(),
            "JSON mode must never write to stderr, got: {:?}",
            *stderr
        );
    }

    // ── Test: reconnect_reason_serializes_correctly ──

    // ── Test: human_reconnect_prints_disconnect_cause_before_reconnecting ──

    #[test]
    fn human_reconnect_prints_disconnect_cause_before_reconnecting() {
        use super::ReconnectReason;

        let cases: Vec<(ReconnectReason, Option<&str>, &str)> = vec![
            (
                ReconnectReason::ClosedWithoutStreamEnd,
                None,
                "connection closed without stream-end",
            ),
            (ReconnectReason::IdleTimeout, None, "no data for"),
            (
                ReconnectReason::TransportError,
                Some("conn reset"),
                "transport error: conn reset",
            ),
            (ReconnectReason::TransportError, None, "transport error"),
            (
                ReconnectReason::ServerUnavailable,
                Some("HTTP 503: bad"),
                "server unavailable: HTTP 503: bad",
            ),
        ];

        for (reason, detail, expected_fragment) in &cases {
            let cap = CapturedOutput::new();
            let mut out = make_output(&cap, Verbosity::Normal, false, true);

            let _ = out.reconnecting(1, 5, 5000, Some("pod|ts"), *reason, detail.as_deref());

            let stderr = cap.stderr.borrow();
            // Must have at least two lines: disconnect cause, then reconnecting.
            assert!(
                stderr.len() >= 2,
                "reason={reason:?}: expected at least 2 stderr lines, got {}: {:?}",
                stderr.len(),
                *stderr
            );
            // First line: the disconnect cause.
            assert!(
                stderr[0].contains("stream disconnected:"),
                "reason={reason:?}: first line must contain 'stream disconnected:', got: {}",
                stderr[0]
            );
            assert!(
                stderr[0].contains(expected_fragment),
                "reason={reason:?}: first line must contain '{expected_fragment}', got: {}",
                stderr[0]
            );
            // Second line: the reconnecting message.
            assert!(
                stderr[1].contains("reconnecting"),
                "reason={reason:?}: second line must contain 'reconnecting', got: {}",
                stderr[1]
            );
        }

        // For ClosedWithoutStreamEnd, the exact Spec text must match what
        // disconnected() prints.
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);
        let _ = out.disconnected();
        let disc_stderr = cap.stderr.borrow();
        let disc_text = &disc_stderr[0];
        // Both must contain the same cause text.
        assert!(
            disc_text.contains("connection closed without stream-end"),
            "disconnected() must use the Spec text: {disc_text}"
        );
    }

    // ── Test: json_reconnect_writes_only_the_reconnecting_record ──

    #[test]
    fn json_reconnect_writes_only_the_reconnecting_record() {
        use super::ReconnectReason;

        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, true, true);

        let _ = out.reconnecting(
            1,
            5,
            5000,
            Some("pod|ts"),
            ReconnectReason::ClosedWithoutStreamEnd,
            None,
        );

        let stdout = cap.stdout.borrow();
        // Exactly one record on stdout.
        assert_eq!(
            stdout.len(),
            1,
            "JSON reconnect must write exactly one record, got: {:?}",
            *stdout
        );
        let parsed: serde_json::Value = serde_json::from_str(&stdout[0]).unwrap();
        assert_eq!(
            parsed["event"], "reconnecting",
            "the record must be 'reconnecting', got: {}",
            stdout[0]
        );

        let stderr = cap.stderr.borrow();
        assert!(
            stderr.is_empty(),
            "JSON mode must not write to stderr, got: {:?}",
            *stderr
        );
    }

    // ── Test: writer_returns_stdout_write_error_in_both_modes ──

    #[test]
    fn writer_returns_stdout_write_error_in_both_modes() {
        use crate::errors::CliError;

        // A stdout closure that returns the helper's Usage error.
        fn make_failing_output(format_json: bool) -> LogStreamOutput {
            LogStreamOutput::new(
                Verbosity::Normal,
                format_json,
                false,
                Box::new(|| Duration::from_secs(1)),
                Box::new(|_text: &str| {
                    Err(CliError::Usage {
                        message: "Cannot write to stdout: No space left on device.".to_string(),
                        metadata: None,
                    })
                }),
                Box::new(|_text: &str| {}),
            )
        }

        // JSON mode: log_line returns the error.
        let mut out = make_failing_output(true);
        let result = out.log_line(Some("pod|ts"), "hello");
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 1, "Usage error must exit 1");
        assert!(
            err.to_string().contains("Cannot write to stdout"),
            "error must contain the cause: {err}"
        );

        // Human mode: log_line returns the error.
        let mut out = make_failing_output(false);
        let result = out.log_line(Some("pod|ts"), "hello");
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 1, "Usage error must exit 1");

        // Terminal record (stream_end in JSON mode) returns the error.
        let mut out = make_failing_output(true);
        let result = out.stream_end(r#"{"reason":"completed"}"#);
        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 1, "Usage error must exit 1");
    }

    // ── Test: pod_done_human_line_shows_the_server_message ──

    #[test]
    fn pod_done_human_line_shows_the_server_message() {
        // (a) Data with pod, reason, and message: the message appears.
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);
        let data = r#"{"pod":"p1","reason":"error","message":"open log stream for pod p1: previous terminated container \"service\" in pod \"p1\" not found"}"#;
        let _ = out.pod_done(data);

        let stderr = cap.stderr.borrow();
        assert_eq!(
            stderr.len(),
            1,
            "expected exactly one stderr line, got: {:?}",
            *stderr
        );

        // Build the expected line the same way the production code does:
        // status_line( format!("{} {}", format_elapsed(...), text) )
        let elapsed_prefix = crate::invocation::handlers::extend::session_log::format_elapsed(
            Duration::from_secs(42),
        );
        let text = "pod stream finished  pod=p1  reason=error  message=open log stream for pod p1: previous terminated container \"service\" in pod \"p1\" not found";
        let inner = format!("{} {}", elapsed_prefix, text);
        let expected = status_line(&inner);
        assert_eq!(stderr[0], expected, "pod_done with message");

        // (b) message is null: no message= field.
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);
        let _ = out.pod_done(r#"{"pod":"p1","reason":"eof","message":null}"#);
        let stderr = cap.stderr.borrow();
        assert_eq!(stderr.len(), 1);
        let text_no_msg = "pod stream finished  pod=p1  reason=eof";
        let inner_no_msg = format!("{} {}", elapsed_prefix, text_no_msg);
        let expected_no_msg = status_line(&inner_no_msg);
        assert_eq!(stderr[0], expected_no_msg, "pod_done with null message");

        // (c) message is empty string: no message= field.
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);
        let _ = out.pod_done(r#"{"pod":"p1","reason":"eof","message":""}"#);
        let stderr = cap.stderr.borrow();
        assert_eq!(stderr.len(), 1);
        assert_eq!(stderr[0], expected_no_msg, "pod_done with empty message");
    }

    // ── Test: status_lines_strip_terminal_control_sequences ──

    #[test]
    fn status_lines_strip_terminal_control_sequences() {
        // (a) pod-done whose pod, reason, and message contain ANSI and BEL.
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);
        let data = r#"{"pod":"p\u001b[31m1\u0007","reason":"r\u001b[31med\u0007","message":"m\u001b[31msg\u0007"}"#;
        let _ = out.pod_done(data);
        let stderr = cap.stderr.borrow();
        assert_eq!(stderr.len(), 1);
        assert!(
            !stderr[0].contains("\x1b[31m"),
            "ANSI escape must be stripped from pod_done: {}",
            stderr[0]
        );
        assert!(
            !stderr[0].contains("\x07"),
            "BEL must be stripped from pod_done: {}",
            stderr[0]
        );
        drop(stderr);

        // (b) pod-done whose data is not JSON and contains ANSI.
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Normal, false, false);
        let _ = out.pod_done("raw \x1b[31mdata\x07 here");
        let stderr = cap.stderr.borrow();
        assert_eq!(stderr.len(), 1);
        assert!(
            !stderr[0].contains("\x1b[31m"),
            "ANSI escape must be stripped from raw pod_done: {}",
            stderr[0]
        );
        assert!(
            !stderr[0].contains("\x07"),
            "BEL must be stripped from raw pod_done: {}",
            stderr[0]
        );
        drop(stderr);

        // (c) verbose call whose text contains ANSI.
        let cap = CapturedOutput::new();
        let mut out = make_output(&cap, Verbosity::Verbose, false, false);
        out.verbose("lifecycle \x1b[31mdata\x07");
        let stderr = cap.stderr.borrow();
        assert_eq!(stderr.len(), 1);
        assert!(
            !stderr[0].contains("\x1b[31m"),
            "ANSI escape must be stripped from verbose: {}",
            stderr[0]
        );
        assert!(
            !stderr[0].contains("\x07"),
            "BEL must be stripped from verbose: {}",
            stderr[0]
        );
    }

    #[test]
    fn reconnect_reason_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&ReconnectReason::ClosedWithoutStreamEnd).unwrap(),
            "\"closed_without_stream_end\""
        );
        assert_eq!(
            serde_json::to_string(&ReconnectReason::TransportError).unwrap(),
            "\"transport_error\""
        );
        assert_eq!(
            serde_json::to_string(&ReconnectReason::IdleTimeout).unwrap(),
            "\"idle_timeout\""
        );
        assert_eq!(
            serde_json::to_string(&ReconnectReason::ServerUnavailable).unwrap(),
            "\"server_unavailable\""
        );
    }
}
