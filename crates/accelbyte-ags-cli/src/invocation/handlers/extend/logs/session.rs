//! Reconnect decision table and loop for `ags extend logs stream`.
//!
//! `reconnect_decision` is a pure function: given how an attempt ended and the
//! current state, it returns either "reconnect" or "end". `run_stream_loop`
//! drives the state machine, calling the attempt trait, asking the decision
//! function, and reporting through the output writer trait.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::errors::{ApiErrorCategory, CliError, ErrorMetadata};
use crate::frontend::StdoutLineWrite;

use super::output::{LogStreamWriter, ReconnectReason};
use super::resume::ResumeWindow;
use super::sse::{SseEvent, SseEventKind};
use super::transport::{AttemptOutcome, EventSink, SinkControl, StreamAttempt};

/// Maximum reconnects per run. The fifth used reconnect triggers `gave_up`.
pub(crate) const MAX_RECONNECTS: u32 = 5;

// ── Reconnect Decision ──

/// How the event sink stopped an attempt (distinct from transport-level outcomes).
///
/// Stdout write errors are not stop reasons; see `LoopSink::write_error`.
#[derive(Debug)]
pub(crate) enum SinkStopReason {
    /// The server sent `event: stream-end`. Contains the raw data field.
    StreamEnd(String),
    /// The server sent `event: error`. Contains the raw data field.
    Error(String),
    /// stdout closed (`StdoutLineWrite::ReaderClosed`).
    StdoutClosed,
}

/// Which kind of ending the decision table chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndKind {
    /// A clean ending (stream-end, closed without follow, stdout closed).
    Ok,
    /// Cancellation arrived (possibly after the attempt returned).
    Cancelled,
    /// The reconnect budget was exhausted.
    GaveUp,
    /// Any other error (HTTP, auth, transport, attempt failure, …).
    Error,
}

/// How the reconnect decision says to end the command.
#[derive(Debug)]
pub(crate) struct EndDecision {
    pub(crate) kind: EndKind,
    pub(crate) result: Result<(), CliError>,
}

/// How the reconnect decision says to reconnect.
#[derive(Debug)]
pub(crate) struct ReconnectInfo {
    pub(crate) reason: ReconnectReason,
    pub(crate) detail: Option<String>,
}

/// The outcome of `reconnect_decision`.
#[derive(Debug)]
pub(crate) enum Decision {
    Reconnect(ReconnectInfo),
    End(EndDecision),
}

/// Map an HTTP status and CSM detail to a `CliError`.
///
/// Centralises the status-to-error mapping so reconnect_decision never
/// repeats it across arms.
fn status_to_error(status: u16, detail: &str, namespace: &str) -> CliError {
    // `detail` is either empty or starts with ": " (from
    // `extract_csm_error_detail`), so concatenation gives the right form:
    //   "HTTP 404" or "HTTP 404: errorCode 20008: app not found".
    let msg = format!("HTTP {status}{detail}");
    match status {
        403 => CliError::Api {
            message: msg,
            metadata: Some(Box::new(ErrorMetadata::with_suggestion(
                format!(
                    "The token must carry ADMIN:NAMESPACE:{namespace}:EXTEND:APP [READ]"
                ),
            ))),
            category: ApiErrorCategory::Permission,
        },
        404 => CliError::Api {
            message: msg,
            metadata: Some(Box::new(ErrorMetadata::with_suggestion(
                "The app may not exist or may have no running pods",
            ))),
            category: ApiErrorCategory::NotFound,
        },
        400 => CliError::Api {
            message: msg,
            metadata: None,
            category: ApiErrorCategory::Rejected,
        },
        429 => CliError::Api {
            message: msg,
            metadata: Some(Box::new(ErrorMetadata::with_suggestion(
                "At most 10 log streams may be open per namespace at once, counted for everyone in that namespace. A stream stopped less than a minute ago may still count, so wait a minute before running the command again",
            ))),
            category: ApiErrorCategory::Rejected,
        },
        _ => CliError::Api {
            message: msg,
            metadata: None,
            category: ApiErrorCategory::Upstream,
        },
    }
}

/// Parse an `event: error` data field as `{"message": "..."}`, returning
/// the message when valid JSON with a `message` string, the raw data otherwise.
fn error_event_message(data: &str) -> String {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
        .unwrap_or_else(|| data.to_string())
}

/// Pure reconnect-decision function.
///
/// Inputs:
/// - `outcome`: how the attempt ended (transport-level) or `None` when the
///   sink stopped it (see `sink_stop`).
/// - `sink_stop`: why the sink stopped the attempt, when it did.
/// - `cancelled`: whether a cancellation arrived.
/// - `first_attempt`: true on the very first attempt (not a reconnect).
/// - `follow`: the `--follow` flag.
/// - `reconnects_used`: how many reconnects have been consumed so far.
/// - `namespace`: for error messages that cite the required permission.
///
/// Returns `Decision::Reconnect` or `Decision::End`.
pub(crate) fn reconnect_decision(
    outcome: Result<AttemptOutcome, CliError>,
    sink_stop: Option<&SinkStopReason>,
    cancelled: bool,
    first_attempt: bool,
    follow: bool,
    reconnects_used: u32,
    namespace: &str,
) -> Decision {
    // Cancellation always ends, regardless of how the attempt finished.
    if cancelled {
        return Decision::End(EndDecision {
            kind: EndKind::Cancelled,
            result: Ok(()),
        });
    }

    // Sink stopped the attempt.
    if let Some(reason) = sink_stop {
        return match reason {
            SinkStopReason::StreamEnd(_) => Decision::End(EndDecision {
                kind: EndKind::Ok,
                result: Ok(()),
            }),
            SinkStopReason::Error(data) => {
                let msg = error_event_message(data);
                Decision::End(EndDecision {
                    kind: EndKind::Error,
                    result: Err(CliError::Api {
                        message: msg,
                        metadata: None,
                        category: ApiErrorCategory::Upstream,
                    }),
                })
            }
            SinkStopReason::StdoutClosed => Decision::End(EndDecision {
                kind: EndKind::Ok,
                result: Ok(()),
            }),
        };
    }

    // Attempt itself failed (token resolution, etc.).
    let outcome = match outcome {
        Err(e) => {
            // Forward the error directly; the loop writes the error record.
            return Decision::End(EndDecision {
                kind: EndKind::Error,
                result: Err(e),
            });
        }
        Ok(o) => o,
    };

    let outcome = &outcome;

    match outcome {
        // Budget check: the fifth used reconnect ends with gave_up.
        _ if reconnects_used >= MAX_RECONNECTS && needs_reconnect(outcome, follow) => {
            Decision::End(EndDecision {
                kind: EndKind::GaveUp,
                result: Err(CliError::Network {
                    message: "gave up after 5 reconnects; run the command again".to_string(),
                    metadata: None,
                }),
            })
        }

        AttemptOutcome::StoppedBySink => {
            // Should not reach here (handled above via sink_stop), but safe.
            Decision::End(EndDecision {
                kind: EndKind::Ok,
                result: Ok(()),
            })
        }

        AttemptOutcome::Closed => {
            // EOF without stream-end.
            if follow {
                Decision::Reconnect(ReconnectInfo {
                    reason: ReconnectReason::ClosedWithoutStreamEnd,
                    detail: None,
                })
            } else {
                Decision::End(EndDecision {
                    kind: EndKind::Ok,
                    result: Ok(()),
                })
            }
        }

        AttemptOutcome::IdleTimeout => {
            if follow {
                Decision::Reconnect(ReconnectInfo {
                    reason: ReconnectReason::IdleTimeout,
                    detail: None,
                })
            } else {
                Decision::End(EndDecision {
                    kind: EndKind::Error,
                    result: Err(CliError::Network {
                        message: "stream idle timeout".to_string(),
                        metadata: None,
                    }),
                })
            }
        }

        AttemptOutcome::TransportError(detail) => {
            if follow {
                Decision::Reconnect(ReconnectInfo {
                    reason: ReconnectReason::TransportError,
                    detail: Some(detail.clone()),
                })
            } else {
                Decision::End(EndDecision {
                    kind: EndKind::Error,
                    result: Err(CliError::Network {
                        message: format!("stream transport error: {detail}"),
                        metadata: None,
                    }),
                })
            }
        }

        AttemptOutcome::HttpStatus { status, detail } => match *status {
            502..=504 if !first_attempt && follow => Decision::Reconnect(ReconnectInfo {
                reason: ReconnectReason::ServerUnavailable,
                detail: Some(format!("HTTP {status}{detail}")),
            }),
            _ => Decision::End(EndDecision {
                kind: EndKind::Error,
                result: Err(status_to_error(*status, detail, namespace)),
            }),
        },

        AttemptOutcome::Unauthorized(detail) => {
            // Second 401: exit 2.
            Decision::End(EndDecision {
                kind: EndKind::Error,
                result: Err(CliError::Auth {
                    message: detail.clone(),
                    metadata: None,
                }),
            })
        }

        AttemptOutcome::NoResponse(detail) => {
            if !first_attempt && follow {
                Decision::Reconnect(ReconnectInfo {
                    reason: ReconnectReason::TransportError,
                    detail: Some(detail.clone()),
                })
            } else {
                Decision::End(EndDecision {
                    kind: EndKind::Error,
                    result: Err(CliError::Network {
                        message: format!("no response: {detail}"),
                        metadata: None,
                    }),
                })
            }
        }

        AttemptOutcome::LineTooLong => {
            // A line over the size limit always exits — the server would send
            // the same line again, so reconnecting would not help.
            Decision::End(EndDecision {
                kind: EndKind::Error,
                result: Err(CliError::Network {
                    message: "a single line exceeded the size limit".to_string(),
                    metadata: None,
                }),
            })
        }
    }
}

/// Whether the outcome warrants a reconnect (used only for the budget check).
fn needs_reconnect(outcome: &AttemptOutcome, follow: bool) -> bool {
    match outcome {
        AttemptOutcome::Closed | AttemptOutcome::IdleTimeout => follow,
        AttemptOutcome::TransportError(_) => follow,
        AttemptOutcome::HttpStatus { status, .. } => follow && matches!(*status, 502..=504),
        AttemptOutcome::NoResponse(_) => follow,
        // LineTooLong never reconnects — the server would send the same line.
        AttemptOutcome::LineTooLong => false,
        AttemptOutcome::StoppedBySink | AttemptOutcome::Unauthorized(_) => false,
    }
}

// ── Stream Loop ──

/// Private event sink used by `run_stream_loop`. Dispatches each SSE event
/// to the output writer and the resume window, and tracks why the attempt
/// was stopped.
///
/// Stdout write errors are kept in `write_error`, not in `stop_reason`.
struct LoopSink<'a, W: LogStreamWriter> {
    writer: &'a mut W,
    window: &'a mut ResumeWindow,
    reconnect_number: u32,
    stop_reason: Option<SinkStopReason>,
    /// A non-BrokenPipe stdout write failure. Taken by `run_stream_loop`
    /// after the attempt returns and returned directly, bypassing the
    /// decision table.
    write_error: Option<CliError>,
}

impl<W: LogStreamWriter> EventSink for LoopSink<'_, W> {
    fn opened(&mut self) -> SinkControl {
        match self.writer.connected(self.reconnect_number) {
            Ok(StdoutLineWrite::ReaderClosed) => {
                self.stop_reason = Some(SinkStopReason::StdoutClosed);
                SinkControl::Stop
            }
            Err(e) => {
                self.write_error = Some(e);
                SinkControl::Stop
            }
            Ok(_) => SinkControl::Continue,
        }
    }

    fn event(&mut self, event: SseEvent) -> SinkControl {
        match event.kind() {
            SseEventKind::LogLine => {
                if !self.window.admit(event.id.as_deref()) {
                    return SinkControl::Continue;
                }
                match self.writer.log_line(event.id.as_deref(), &event.data) {
                    Ok(StdoutLineWrite::ReaderClosed) => {
                        self.stop_reason = Some(SinkStopReason::StdoutClosed);
                        return SinkControl::Stop;
                    }
                    Err(e) => {
                        self.write_error = Some(e);
                        return SinkControl::Stop;
                    }
                    Ok(_) => {}
                }
            }
            SseEventKind::PodDone => match self.writer.pod_done(&event.data) {
                Ok(StdoutLineWrite::ReaderClosed) => {
                    self.stop_reason = Some(SinkStopReason::StdoutClosed);
                    return SinkControl::Stop;
                }
                Err(e) => {
                    self.write_error = Some(e);
                    return SinkControl::Stop;
                }
                Ok(_) => {}
            },
            SseEventKind::StreamEnd => {
                self.stop_reason = Some(SinkStopReason::StreamEnd(event.data.clone()));
                return SinkControl::Stop;
            }
            SseEventKind::Error => {
                self.stop_reason = Some(SinkStopReason::Error(event.data.clone()));
                return SinkControl::Stop;
            }
            SseEventKind::Other(name) => {
                self.writer
                    .verbose(&format!("unrecognised event: {name}  data={}", event.data));
            }
        }
        SinkControl::Continue
    }
}

/// Run the reconnect loop for `ags extend logs stream`.
///
/// Generic over:
/// - `A`: the connection attempt (production or test fake)
/// - `D`: a delay function `(reconnect_number: u32) -> Duration`
/// - `W`: the output writer
pub(crate) async fn run_stream_loop<A, D, W>(
    attempt: &A,
    mut delay_fn: D,
    writer: &mut W,
    follow: bool,
    namespace: &str,
    cancel: CancellationToken,
) -> Result<(), CliError>
where
    A: StreamAttempt,
    D: FnMut(u32) -> Duration,
    W: LogStreamWriter,
{
    let mut window = ResumeWindow::new();
    let mut reconnects_used: u32 = 0;
    let mut first_attempt = true;

    loop {
        let is_reconnect = !first_attempt;
        let last_event_id = window.resume_point().map(|s| s.to_string());

        let mut sink = LoopSink {
            writer,
            window: &mut window,
            reconnect_number: reconnects_used,
            stop_reason: None,
            write_error: None,
        };

        // Run one attempt, honoring cancellation.
        let outcome = tokio::select! {
            _ = cancel.cancelled() => {
                // Cancellation during the attempt.
                match sink.writer.stopped() {
                    Ok(_) => return Ok(()),
                    Err(write_err) => return Err(write_err),
                }
            }
            result = attempt.run_attempt(
                is_reconnect,
                last_event_id.as_deref(),
                &mut sink,
            ) => result,
        };

        // A stdout write error stops the command immediately: return the
        // write error, no terminal record, regardless of cancellation.
        if let Some(write_err) = sink.write_error.take() {
            return Err(write_err);
        }

        let sink_stop = sink.stop_reason.take();

        // Determine what to do next.
        let decision = reconnect_decision(
            outcome,
            sink_stop.as_ref(),
            cancel.is_cancelled(),
            first_attempt,
            follow,
            reconnects_used,
            namespace,
        );

        first_attempt = false;

        match decision {
            Decision::Reconnect(info) => {
                reconnects_used += 1;
                let delay = delay_fn(reconnects_used);
                match writer.reconnecting(
                    reconnects_used,
                    MAX_RECONNECTS,
                    delay.as_millis() as u64,
                    window.resume_point(),
                    info.reason,
                    info.detail.as_deref(),
                ) {
                    Ok(StdoutLineWrite::ReaderClosed) => return Ok(()),
                    Err(write_err) => return Err(write_err),
                    Ok(StdoutLineWrite::Written) => {}
                }

                // Wait for the delay or cancellation.
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = cancel.cancelled() => {
                        match writer.stopped() {
                            Ok(_) => return Ok(()),
                            Err(write_err) => return Err(write_err),
                        }
                    }
                }
            }
            Decision::End(end) => {
                // Write exactly one terminal record, decided by the ending
                // kind the decision table chose.
                match end.kind {
                    EndKind::Ok => {
                        match &sink_stop {
                            Some(SinkStopReason::StreamEnd(data)) => {
                                match writer.stream_end(data) {
                                    Ok(StdoutLineWrite::ReaderClosed) => return Ok(()),
                                    Err(write_err) => return Err(write_err),
                                    Ok(StdoutLineWrite::Written) => {}
                                }
                            }
                            Some(SinkStopReason::StdoutClosed) => {
                                // No terminal record when stdout is closed.
                            }
                            _ => {
                                // Closed without stream-end (no --follow).
                                match writer.disconnected() {
                                    Ok(StdoutLineWrite::ReaderClosed) => return Ok(()),
                                    Err(write_err) => return Err(write_err),
                                    Ok(StdoutLineWrite::Written) => {}
                                }
                            }
                        }
                    }
                    EndKind::Cancelled => {
                        // Cancellation after the attempt returned. Write
                        // `stopped` unless stdout was already closed.
                        if !matches!(&sink_stop, Some(SinkStopReason::StdoutClosed)) {
                            match writer.stopped() {
                                Ok(StdoutLineWrite::ReaderClosed) => return Ok(()),
                                Err(write_err) => return Err(write_err),
                                Ok(StdoutLineWrite::Written) => {}
                            }
                        }
                    }
                    EndKind::GaveUp => {
                        // The ending's own error must not be replaced by
                        // success or by the write error, so all outcomes
                        // of the record write fall through to `end.result`.
                        let _ = writer.gave_up(reconnects_used, window.resume_point());
                    }
                    EndKind::Error => {
                        // The ending's own error must not be replaced by
                        // success or by the write error, so all outcomes
                        // of the record write fall through to `end.result`.
                        if let Err(ref e) = end.result {
                            let _ = writer.error(&e.to_string(), e.exit_code());
                        }
                    }
                }
                return end.result;
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::handlers::extend::remote_debug::{
        apply_jitter, backoff_base_secs, reconnect_delay,
    };
    use std::sync::Mutex;

    // ── Decision table tests ──

    fn ok_outcome(outcome: AttemptOutcome) -> Result<AttemptOutcome, CliError> {
        Ok(outcome)
    }

    #[test]
    fn decision_stream_end_exits_zero() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::StoppedBySink),
            Some(&SinkStopReason::StreamEnd(
                r#"{"reason":"completed"}"#.to_string(),
            )),
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => assert!(e.result.is_ok()),
            _ => panic!("expected End, got Reconnect"),
        }
    }

    #[test]
    fn decision_closed_without_stream_end_follow_reconnects() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::Closed),
            None,
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(info.reason, ReconnectReason::ClosedWithoutStreamEnd);
            }
            _ => panic!("expected Reconnect"),
        }
    }

    #[test]
    fn decision_closed_without_stream_end_no_follow_exits_zero() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::Closed),
            None,
            false,
            false,
            false,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => assert!(e.result.is_ok()),
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_idle_timeout_follow_reconnects() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::IdleTimeout),
            None,
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(info.reason, ReconnectReason::IdleTimeout);
            }
            _ => panic!("expected Reconnect"),
        }
    }

    #[test]
    fn decision_idle_timeout_no_follow_exits_four() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::IdleTimeout),
            None,
            false,
            false,
            false,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert!(matches!(err, CliError::Network { .. }));
                assert_eq!(err.exit_code(), 4);
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_transport_error_follow_reconnects() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::TransportError("conn reset".to_string())),
            None,
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(info.reason, ReconnectReason::TransportError);
                assert!(info.detail.as_deref().unwrap().contains("conn reset"));
            }
            _ => panic!("expected Reconnect"),
        }
    }

    #[test]
    fn decision_transport_error_no_follow_exits_four() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::TransportError("conn reset".to_string())),
            None,
            false,
            false,
            false,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert!(matches!(err, CliError::Network { .. }));
                assert_eq!(err.exit_code(), 4);
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_502_reconnect_on_reconnect() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 502,
                detail: "bad gw".to_string(),
            }),
            None,
            false,
            false, // not first attempt
            true,
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(info.reason, ReconnectReason::ServerUnavailable);
            }
            _ => panic!("expected Reconnect"),
        }
    }

    #[test]
    fn decision_503_reconnect_on_reconnect() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 503,
                detail: "service unavailable".to_string(),
            }),
            None,
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(info.reason, ReconnectReason::ServerUnavailable);
            }
            _ => panic!("expected Reconnect"),
        }
    }

    #[test]
    fn decision_504_reconnect_on_reconnect() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 504,
                detail: "gateway timeout".to_string(),
            }),
            None,
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(info.reason, ReconnectReason::ServerUnavailable);
            }
            _ => panic!("expected Reconnect"),
        }
    }

    #[test]
    fn decision_502_first_attempt_exits_three() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 502,
                detail: "bad gw".to_string(),
            }),
            None,
            false,
            true, // first attempt
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_403_exits_three_permission() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 403,
                detail: "forbidden".to_string(),
            }),
            None,
            false,
            true,
            true,
            0,
            "my-ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
                assert!(matches!(
                    err,
                    CliError::Api {
                        category: ApiErrorCategory::Permission,
                        ..
                    }
                ));
                // Check suggestion mentions the permission.
                let msg = format!("{err:?}");
                assert!(msg.contains("ADMIN:NAMESPACE:my-ns:EXTEND:APP"));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_404_exits_three_not_found() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 404,
                detail: "not found".to_string(),
            }),
            None,
            false,
            true,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
                assert!(matches!(
                    err,
                    CliError::Api {
                        category: ApiErrorCategory::NotFound,
                        ..
                    }
                ));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_400_exits_three_rejected() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 400,
                detail: "bad request".to_string(),
            }),
            None,
            false,
            true,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
                assert!(matches!(
                    err,
                    CliError::Api {
                        category: ApiErrorCategory::Rejected,
                        ..
                    }
                ));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_429_exits_three_rejected_with_suggestion() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 429,
                detail: "rate limited".to_string(),
            }),
            None,
            false,
            true,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
                assert!(matches!(
                    err,
                    CliError::Api {
                        category: ApiErrorCategory::Rejected,
                        ..
                    }
                ));
                let suggestion = err
                    .metadata()
                    .and_then(|m| m.suggestion.as_deref())
                    .expect("429 error must have a suggestion");
                assert_eq!(
                    suggestion,
                    "At most 10 log streams may be open per namespace at once, counted for everyone in that namespace. A stream stopped less than a minute ago may still count, so wait a minute before running the command again",
                    "429 suggestion"
                );
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_500_exits_three_upstream() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 500,
                detail: "internal".to_string(),
            }),
            None,
            false,
            true,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
                assert!(matches!(
                    err,
                    CliError::Api {
                        category: ApiErrorCategory::Upstream,
                        ..
                    }
                ));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_401_unauthorized_exits_two() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::Unauthorized(
                "token refresh failed".to_string(),
            )),
            None,
            false,
            true,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 2);
                assert!(matches!(err, CliError::Auth { .. }));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_error_event_exits_three() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::StoppedBySink),
            Some(&SinkStopReason::Error(
                r#"{"message":"server overload"}"#.to_string(),
            )),
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
                assert!(err.to_string().contains("server overload"));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_error_event_raw_data_when_not_json() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::StoppedBySink),
            Some(&SinkStopReason::Error("not json".to_string())),
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert!(err.to_string().contains("not json"));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_budget_exhausted_exits_four() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::Closed),
            None,
            false,
            false,
            true,
            MAX_RECONNECTS, // 5 used
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 4);
                assert!(err.to_string().contains("gave up"));
                assert!(err.to_string().contains("run the command again"));
            }
            _ => panic!("expected End (gave_up)"),
        }
    }

    #[test]
    fn decision_cancellation_exits_zero() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::Closed),
            None,
            true, // cancelled
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => assert!(e.result.is_ok()),
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_stdout_closed_exits_zero() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::StoppedBySink),
            Some(&SinkStopReason::StdoutClosed),
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => assert!(e.result.is_ok()),
            _ => panic!("expected End"),
        }
    }

    // ── Added rows: NoResponse on reconnect, sub-400 non-200 status ──

    #[test]
    fn decision_no_response_reconnect_follows() {
        // NoResponse on a RECONNECT with --follow: reconnect with transport_error.
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::NoResponse("DNS failed".to_string())),
            None,
            false,
            false, // not first attempt
            true,
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(info.reason, ReconnectReason::TransportError);
                assert!(info.detail.as_deref().unwrap().contains("DNS failed"));
            }
            _ => panic!("expected Reconnect"),
        }
    }

    #[test]
    fn decision_no_response_first_attempt_exits_four() {
        // NoResponse on the FIRST attempt: exit 4.
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::NoResponse("connection refused".to_string())),
            None,
            false,
            true, // first attempt
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 4);
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_sub400_non200_exits_three_upstream() {
        // A status below 400 other than 200 (e.g. 301 redirect): exit 3, Upstream.
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 301,
                detail: "moved".to_string(),
            }),
            None,
            false,
            true,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 3);
                assert!(matches!(
                    err,
                    CliError::Api {
                        category: ApiErrorCategory::Upstream,
                        ..
                    }
                ));
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_attempt_error_forwarded() {
        // When the attempt itself returns Err (e.g. token resolution failed).
        let d = reconnect_decision(
            Err(CliError::Auth {
                message: "token expired".to_string(),
                metadata: None,
            }),
            None,
            false,
            true,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert_eq!(err.exit_code(), 2);
            }
            _ => panic!("expected End"),
        }
    }

    #[test]
    fn decision_line_too_long_follow_exits_four() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::LineTooLong),
            None,
            false,
            false,
            true,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert!(matches!(err, CliError::Network { .. }));
                assert_eq!(err.exit_code(), 4);
                assert!(err.to_string().contains("size limit"), "message: {}", err);
            }
            _ => panic!("expected End, got Reconnect"),
        }
    }

    #[test]
    fn server_unavailable_without_follow_does_not_reconnect() {
        // 503 on a reconnect (not first attempt) without --follow must end
        // with exit code 3, not reconnect. Tests both low and exhausted
        // reconnect budgets.
        for reconnects_used in [1, 5] {
            let d = reconnect_decision(
                ok_outcome(AttemptOutcome::HttpStatus {
                    status: 503,
                    detail: "service unavailable".to_string(),
                }),
                None,
                false,
                false, // not first attempt
                false, // no --follow
                reconnects_used,
                "ns",
            );
            match d {
                Decision::End(e) => {
                    let err = e.result.unwrap_err();
                    assert_eq!(
                        err.exit_code(),
                        3,
                        "503 without --follow (reconnects_used={reconnects_used}) must exit 3"
                    );
                }
                Decision::Reconnect(_) => {
                    panic!(
                        "503 without --follow (reconnects_used={reconnects_used}) must not reconnect"
                    );
                }
            }
        }
    }

    // ── Test: doubled-colon fix ──

    #[test]
    fn http_status_messages_have_one_colon_before_the_server_detail() {
        // (a) status_to_error with a CSM detail (starts with ": ")
        let err = status_to_error(404, ": errorCode 20008: app not found", "ns");
        assert_eq!(
            err.to_string(),
            "HTTP 404: errorCode 20008: app not found",
            "status_to_error with detail"
        );

        // (b) status_to_error with empty detail
        let err = status_to_error(404, "", "ns");
        assert_eq!(
            err.to_string(),
            "HTTP 404",
            "status_to_error without detail"
        );

        // (c) reconnect_decision for a 502 on a reconnect with --follow and detail
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::HttpStatus {
                status: 502,
                detail: ": errorCode 1: busy".to_string(),
            }),
            None,
            false,
            false, // not first attempt
            true,  // --follow
            0,
            "ns",
        );
        match d {
            Decision::Reconnect(info) => {
                assert_eq!(
                    info.detail,
                    Some("HTTP 502: errorCode 1: busy".to_string()),
                    "reconnect detail for 502"
                );
            }
            _ => panic!("expected Reconnect for 502 on reconnect with --follow"),
        }
    }

    #[test]
    fn decision_line_too_long_no_follow_exits_four() {
        let d = reconnect_decision(
            ok_outcome(AttemptOutcome::LineTooLong),
            None,
            false,
            false,
            false,
            0,
            "ns",
        );
        match d {
            Decision::End(e) => {
                let err = e.result.unwrap_err();
                assert!(matches!(err, CliError::Network { .. }));
                assert_eq!(err.exit_code(), 4);
            }
            _ => panic!("expected End"),
        }
    }

    // ── Fake attempt for loop tests ──

    type ScriptEntry = (Vec<SseEvent>, Result<AttemptOutcome, CliError>);

    struct FakeStreamAttempt {
        /// Scripted sequence of (events-to-feed, outcome).
        script: Mutex<Vec<ScriptEntry>>,
    }

    impl FakeStreamAttempt {
        fn new(items: Vec<(Vec<SseEvent>, Result<AttemptOutcome, CliError>)>) -> Self {
            Self {
                script: Mutex::new(items),
            }
        }
    }

    impl StreamAttempt for FakeStreamAttempt {
        fn run_attempt<'a>(
            &'a self,
            _reconnect: bool,
            _last_event_id: Option<&'a str>,
            sink: &'a mut dyn EventSink,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<AttemptOutcome, CliError>> + 'a>,
        > {
            Box::pin(async move {
                let (events, outcome) = {
                    let mut script = self.script.lock().unwrap();
                    assert!(!script.is_empty(), "fake script exhausted");
                    script.remove(0)
                };

                // Feed events to sink.
                if sink.opened() == SinkControl::Stop {
                    return Ok(AttemptOutcome::StoppedBySink);
                }
                for event in events {
                    if sink.event(event) == SinkControl::Stop {
                        return outcome;
                    }
                }
                outcome
            })
        }
    }

    // ── Recording writer for loop tests ──

    #[derive(Debug, Clone)]
    enum WriterCall {
        Connected(u32),
        Log(Option<String>, String),
        PodDone,
        Reconnecting { reconnect: u32 },
        StreamEnd,
        Disconnected,
        Stopped,
        GaveUp(u32),
        Error(i32),
        Verbose,
    }

    struct RecordingWriter {
        calls: Vec<WriterCall>,
    }

    impl RecordingWriter {
        fn new() -> Self {
            Self { calls: Vec::new() }
        }
    }

    impl LogStreamWriter for RecordingWriter {
        fn opening(
            &mut self,
            _params: &crate::invocation::handlers::extend::logs::output::OpeningParams<'_>,
        ) {
        }

        fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Connected(reconnect));
            Ok(StdoutLineWrite::Written)
        }

        fn log_line(&mut self, id: Option<&str>, data: &str) -> Result<StdoutLineWrite, CliError> {
            self.calls
                .push(WriterCall::Log(id.map(String::from), data.to_string()));
            Ok(StdoutLineWrite::Written)
        }

        fn pod_done(&mut self, _data: &str) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::PodDone);
            Ok(StdoutLineWrite::Written)
        }

        fn reconnecting(
            &mut self,
            reconnect: u32,
            _max_reconnects: u32,
            _delay_ms: u64,
            _last_event_id: Option<&str>,
            _reason: ReconnectReason,
            _detail: Option<&str>,
        ) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Reconnecting { reconnect });
            Ok(StdoutLineWrite::Written)
        }

        fn stream_end(&mut self, _data: &str) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::StreamEnd);
            Ok(StdoutLineWrite::Written)
        }

        fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Disconnected);
            Ok(StdoutLineWrite::Written)
        }

        fn stopped(&mut self) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Stopped);
            Ok(StdoutLineWrite::Written)
        }

        fn gave_up(
            &mut self,
            reconnects: u32,
            _last_event_id: Option<&str>,
        ) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::GaveUp(reconnects));
            Ok(StdoutLineWrite::Written)
        }

        fn error(&mut self, _message: &str, exit_code: i32) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Error(exit_code));
            Ok(StdoutLineWrite::Written)
        }

        fn verbose(&mut self, _message: &str) {
            self.calls.push(WriterCall::Verbose);
        }

        fn dry_run(
            &mut self,
            _method: &str,
            _url: &str,
            _ns: &str,
            _app: &str,
        ) -> Result<StdoutLineWrite, CliError> {
            Ok(StdoutLineWrite::Written)
        }
    }

    fn make_log_event(id: &str, data: &str) -> SseEvent {
        SseEvent {
            name: None,
            id: Some(id.to_string()),
            data: data.to_string(),
        }
    }

    fn make_named_event(name: &str, data: &str) -> SseEvent {
        SseEvent {
            name: Some(name.to_string()),
            id: None,
            data: data.to_string(),
        }
    }

    // ── Loop tests ──

    #[tokio::test]
    async fn loop_stream_end_exits_zero() {
        let attempt = FakeStreamAttempt::new(vec![(
            vec![
                make_log_event("pod|1", "hello"),
                make_named_event("stream-end", r#"{"reason":"completed"}"#),
            ],
            Ok(AttemptOutcome::StoppedBySink),
        )]);

        let mut writer = RecordingWriter::new();
        let mut delays = Vec::new();
        let result = run_stream_loop(
            &attempt,
            |n| {
                delays.push(n);
                Duration::ZERO
            },
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        assert!(result.is_ok());
        assert!(delays.is_empty(), "no reconnects");
        // Should have: connected, log, stream_end
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::StreamEnd)));
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::Connected(0))));
    }

    #[tokio::test]
    async fn loop_reconnects_on_closed_with_follow() {
        let attempt = FakeStreamAttempt::new(vec![
            // First attempt: Closed (no stream-end).
            (
                vec![make_log_event("pod|1", "line1")],
                Ok(AttemptOutcome::Closed),
            ),
            // Second attempt (reconnect): stream-end.
            (
                vec![make_named_event("stream-end", r#"{"reason":"done"}"#)],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let mut writer = RecordingWriter::new();
        let mut delays = Vec::new();
        let result = run_stream_loop(
            &attempt,
            |n| {
                delays.push(n);
                Duration::ZERO
            },
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(delays, vec![1]);
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::Reconnecting { reconnect: 1, .. })));
    }

    #[tokio::test]
    async fn loop_does_not_reconnect_without_follow() {
        let attempt = FakeStreamAttempt::new(vec![(
            vec![make_log_event("pod|1", "line1")],
            Ok(AttemptOutcome::Closed),
        )]);

        let mut writer = RecordingWriter::new();
        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            false,
            "ns",
            CancellationToken::new(),
        )
        .await;

        assert!(result.is_ok());
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::Disconnected)));
        // Exactly one attempt: no reconnecting record.
        assert!(
            !writer
                .calls
                .iter()
                .any(|c| matches!(c, WriterCall::Reconnecting { .. })),
            "must not reconnect without --follow"
        );
    }

    #[tokio::test]
    async fn loop_first_connection_failure_does_not_retry() {
        let attempt = FakeStreamAttempt::new(vec![(
            vec![],
            Ok(AttemptOutcome::NoResponse("connection refused".to_string())),
        )]);

        let mut writer = RecordingWriter::new();
        let mut delays = Vec::new();
        let result = run_stream_loop(
            &attempt,
            |n| {
                delays.push(n);
                Duration::ZERO
            },
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(delays.is_empty(), "no delay: first failure does not retry");
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::Error(4))));
    }

    #[tokio::test]
    async fn loop_cancellation_during_attempt_exits_zero() {
        // The attempt blocks until cancelled.
        struct BlockingAttempt;
        impl StreamAttempt for BlockingAttempt {
            fn run_attempt<'a>(
                &'a self,
                _reconnect: bool,
                _last_event_id: Option<&'a str>,
                _sink: &'a mut dyn EventSink,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<AttemptOutcome, CliError>> + 'a>,
            > {
                Box::pin(async {
                    // Block forever.
                    std::future::pending::<()>().await;
                    unreachable!()
                })
            }
        }

        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            trigger.cancel();
        });

        let mut writer = RecordingWriter::new();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run_stream_loop(
                &BlockingAttempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                cancel,
            ),
        )
        .await
        .expect("should not time out");

        assert!(result.is_ok());
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::Stopped)));
    }

    #[tokio::test]
    async fn loop_stops_during_wait_on_cancel() {
        let attempt = FakeStreamAttempt::new(vec![
            // First attempt: Closed -> reconnect.
            (
                vec![make_log_event("pod|1", "line1")],
                Ok(AttemptOutcome::Closed),
            ),
            // Second attempt: should not be reached.
            (
                vec![make_named_event("stream-end", "{}")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let cancel = CancellationToken::new();
        let trigger = cancel.clone();

        let mut writer = RecordingWriter::new();
        // Use a very long delay; cancellation should interrupt it.
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run_stream_loop(
                &attempt,
                move |_| {
                    trigger.cancel();
                    Duration::from_secs(300)
                },
                &mut writer,
                true,
                "ns",
                cancel.clone(),
            ),
        )
        .await
        .expect("should not time out");

        assert!(result.is_ok());
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::Stopped)));
        // No further attempt after cancellation.
        let connected_count = writer
            .calls
            .iter()
            .filter(|c| matches!(c, WriterCall::Connected(_)))
            .count();
        assert_eq!(connected_count, 1, "only one attempt before cancel");
    }

    #[tokio::test]
    async fn loop_gives_up_after_five_reconnects() {
        // 1 initial attempt (Closed) + 5 reconnects (all Closed) = budget exhausted.
        let mut script = Vec::new();
        for _ in 0..=MAX_RECONNECTS {
            script.push((
                vec![make_log_event("pod|1", "line")],
                Ok(AttemptOutcome::Closed),
            ));
        }
        let attempt = FakeStreamAttempt::new(script);

        let mut writer = RecordingWriter::new();
        let mut delays = Vec::new();
        let result = run_stream_loop(
            &attempt,
            |n| {
                delays.push(n);
                Duration::ZERO
            },
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(err.to_string().contains("gave up"));
        // Five delays with reconnect numbers 1 to 5.
        assert_eq!(delays, vec![1, 2, 3, 4, 5]);
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::GaveUp(5))));
        // gave_up is the sole terminal record; no error record follows.
        assert!(
            !writer
                .calls
                .iter()
                .any(|c| matches!(c, WriterCall::Error(_))),
            "gave_up replaces error for the give-up ending"
        );
    }

    #[tokio::test]
    async fn loop_error_event_exits_three() {
        let attempt = FakeStreamAttempt::new(vec![(
            vec![make_named_event("error", r#"{"message":"overloaded"}"#)],
            Ok(AttemptOutcome::StoppedBySink),
        )]);

        let mut writer = RecordingWriter::new();
        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("overloaded"));
        assert!(writer
            .calls
            .iter()
            .any(|c| matches!(c, WriterCall::Error(3))));
    }

    #[tokio::test]
    async fn loop_reconnects_with_last_printed_id_and_skips_duplicate() {
        // Attempt 1: prints ids a and b, then closes.
        // Attempt 2: receives resume point b, replays b (duplicate), then c, stream-end.
        // Output should be a, b, c — no duplicate.
        let attempt = FakeStreamAttempt::new(vec![
            (
                vec![
                    make_log_event("pod|a", "line-a"),
                    make_log_event("pod|b", "line-b"),
                ],
                Ok(AttemptOutcome::Closed),
            ),
            (
                vec![
                    make_log_event("pod|b", "line-b"),
                    make_log_event("pod|c", "line-c"),
                    make_named_event("stream-end", r#"{"reason":"completed"}"#),
                ],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let mut writer = RecordingWriter::new();
        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        assert!(result.is_ok());
        // Collect the log lines written.
        let logs: Vec<(&str, &str)> = writer
            .calls
            .iter()
            .filter_map(|c| match c {
                WriterCall::Log(id, data) => Some((id.as_deref().unwrap_or(""), data.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            logs,
            vec![
                ("pod|a", "line-a"),
                ("pod|b", "line-b"),
                ("pod|c", "line-c")
            ],
            "duplicate b must be suppressed"
        );
    }

    #[tokio::test]
    async fn loop_budget_is_not_restored_by_a_successful_reconnect() {
        // A successful reconnect followed by more drops still stops at five
        // reconnects in total. Script: 1 initial + 6 reconnects = 7 attempts.
        // After the second reconnect (which is "successful" in that some logs
        // arrive), reconnects 3-5 exhaust the budget.
        let mut script = Vec::new();
        for _ in 0..=MAX_RECONNECTS {
            // Each attempt prints a log line and then closes — always reconnectable.
            script.push((
                vec![make_log_event("pod|1", "line")],
                Ok(AttemptOutcome::Closed),
            ));
        }
        let attempt = FakeStreamAttempt::new(script);

        let mut writer = RecordingWriter::new();
        let mut delays = Vec::new();
        let result = run_stream_loop(
            &attempt,
            |n| {
                delays.push(n);
                Duration::ZERO
            },
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(err.to_string().contains("gave up"));
        // Budget is 5 total, never restored.
        assert_eq!(delays.len(), MAX_RECONNECTS as usize);
    }

    #[tokio::test]
    async fn loop_stops_when_output_reports_closed_stdout() {
        // Writer reports StdoutClosed on log_line => exit 0, no further attempt,
        // no terminal record written.
        struct ClosingWriter {
            calls: Vec<WriterCall>,
        }

        impl LogStreamWriter for ClosingWriter {
            fn opening(&mut self, _: &super::super::output::OpeningParams<'_>) {}
            fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Connected(reconnect));
                Ok(StdoutLineWrite::Written)
            }
            fn log_line(
                &mut self,
                id: Option<&str>,
                data: &str,
            ) -> Result<StdoutLineWrite, CliError> {
                self.calls
                    .push(WriterCall::Log(id.map(String::from), data.to_string()));
                Ok(StdoutLineWrite::ReaderClosed) // stdout is closed
            }
            fn pod_done(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
            fn reconnecting(
                &mut self,
                _: u32,
                _: u32,
                _: u64,
                _: Option<&str>,
                _: ReconnectReason,
                _: Option<&str>,
            ) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
            fn stream_end(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::StreamEnd);
                Ok(StdoutLineWrite::Written)
            }
            fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Disconnected);
                Ok(StdoutLineWrite::Written)
            }
            fn stopped(&mut self) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Stopped);
                Ok(StdoutLineWrite::Written)
            }
            fn gave_up(&mut self, _: u32, _: Option<&str>) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
            fn error(&mut self, _msg: &str, code: i32) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Error(code));
                Ok(StdoutLineWrite::Written)
            }
            fn verbose(&mut self, _: &str) {}
            fn dry_run(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
        }

        let attempt = FakeStreamAttempt::new(vec![
            (
                vec![make_log_event("pod|1", "line1")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
            // Second attempt: should never be reached.
            (
                vec![make_named_event("stream-end", "{}")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let mut writer = ClosingWriter { calls: Vec::new() };
        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        assert!(result.is_ok());
        // No terminal record (no StreamEnd, Disconnected, Stopped, or Error).
        assert!(
            !writer.calls.iter().any(|c| matches!(
                c,
                WriterCall::StreamEnd
                    | WriterCall::Disconnected
                    | WriterCall::Stopped
                    | WriterCall::Error(_)
            )),
            "no terminal record when stdout closed"
        );
        // Only one attempt.
        let connected_count = writer
            .calls
            .iter()
            .filter(|c| matches!(c, WriterCall::Connected(_)))
            .count();
        assert_eq!(connected_count, 1, "no further attempt after closed stdout");
    }

    // ── opened() → Stop stops the loop ──

    #[tokio::test]
    async fn loop_ends_when_connected_write_finds_stdout_closed() {
        // A writer that returns ReaderClosed from connected(), simulating
        // stdout closed at the moment the "connected" record is written.
        // The loop must stop after one attempt, write no log line, and
        // emit no terminal record.
        struct ConnectedClosingWriter {
            calls: Vec<WriterCall>,
        }

        impl LogStreamWriter for ConnectedClosingWriter {
            fn opening(&mut self, _: &super::super::output::OpeningParams<'_>) {}
            fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Connected(reconnect));
                Ok(StdoutLineWrite::ReaderClosed)
            }
            fn log_line(
                &mut self,
                id: Option<&str>,
                data: &str,
            ) -> Result<StdoutLineWrite, CliError> {
                self.calls
                    .push(WriterCall::Log(id.map(String::from), data.to_string()));
                Ok(StdoutLineWrite::Written)
            }
            fn pod_done(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
            fn reconnecting(
                &mut self,
                _: u32,
                _: u32,
                _: u64,
                _: Option<&str>,
                _: ReconnectReason,
                _: Option<&str>,
            ) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
            fn stream_end(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::StreamEnd);
                Ok(StdoutLineWrite::Written)
            }
            fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Disconnected);
                Ok(StdoutLineWrite::Written)
            }
            fn stopped(&mut self) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Stopped);
                Ok(StdoutLineWrite::Written)
            }
            fn gave_up(&mut self, _: u32, _: Option<&str>) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
            fn error(&mut self, _: &str, code: i32) -> Result<StdoutLineWrite, CliError> {
                self.calls.push(WriterCall::Error(code));
                Ok(StdoutLineWrite::Written)
            }
            fn verbose(&mut self, _: &str) {}
            fn dry_run(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<StdoutLineWrite, CliError> {
                Ok(StdoutLineWrite::Written)
            }
        }

        let attempt = FakeStreamAttempt::new(vec![
            (
                vec![make_log_event("pod|1", "should-not-print")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
            // Second attempt: should never be reached.
            (
                vec![make_named_event("stream-end", "{}")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let mut writer = ConnectedClosingWriter { calls: Vec::new() };
        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        assert!(
            result.is_ok(),
            "must return Ok when stdout closed at connected"
        );
        // No log lines should have been written.
        assert!(
            !writer
                .calls
                .iter()
                .any(|c| matches!(c, WriterCall::Log(..))),
            "no log lines when stdout closed at connected: {:?}",
            writer.calls
        );
        // No terminal record.
        assert!(
            !writer.calls.iter().any(|c| matches!(
                c,
                WriterCall::StreamEnd
                    | WriterCall::Disconnected
                    | WriterCall::Stopped
                    | WriterCall::Error(_)
            )),
            "no terminal record when stdout closed at connected: {:?}",
            writer.calls
        );
        // Only one attempt (one Connected call).
        let connected_count = writer
            .calls
            .iter()
            .filter(|c| matches!(c, WriterCall::Connected(_)))
            .count();
        assert_eq!(
            connected_count, 1,
            "only one attempt before stdout-closed exit"
        );
    }

    // ── Terminal-record-count tests ──

    /// Helper: count terminal records in the writer's call log.
    /// Terminal records are: StreamEnd, Disconnected, Stopped, GaveUp, Error.
    fn terminal_record_count(calls: &[WriterCall]) -> usize {
        calls
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    WriterCall::StreamEnd
                        | WriterCall::Disconnected
                        | WriterCall::Stopped
                        | WriterCall::GaveUp(_)
                        | WriterCall::Error(_)
                )
            })
            .count()
    }

    /// Helper: the last terminal record, if any.
    fn last_terminal_record(calls: &[WriterCall]) -> Option<&WriterCall> {
        calls.iter().rev().find(|c| {
            matches!(
                c,
                WriterCall::StreamEnd
                    | WriterCall::Disconnected
                    | WriterCall::Stopped
                    | WriterCall::GaveUp(_)
                    | WriterCall::Error(_)
            )
        })
    }

    /// Every ending of the reconnect loop writes exactly one terminal record
    /// (or none when stdout was closed). This test drives each ending and
    /// asserts the count and kind.
    #[tokio::test]
    async fn loop_writes_exactly_one_terminal_record_per_ending() {
        // ── stream-end: exactly one, StreamEnd ──
        {
            let attempt = FakeStreamAttempt::new(vec![(
                vec![make_named_event("stream-end", r#"{"reason":"done"}"#)],
                Ok(AttemptOutcome::StoppedBySink),
            )]);
            let mut writer = RecordingWriter::new();
            let _ = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                CancellationToken::new(),
            )
            .await;
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "stream-end: exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::StreamEnd)
                ),
                "stream-end: last terminal record must be StreamEnd"
            );
        }

        // ── closed without stream-end, no --follow: exactly one, Disconnected ──
        {
            let attempt = FakeStreamAttempt::new(vec![(
                vec![make_log_event("pod|1", "line")],
                Ok(AttemptOutcome::Closed),
            )]);
            let mut writer = RecordingWriter::new();
            let _ = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                false,
                "ns",
                CancellationToken::new(),
            )
            .await;
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "closed/no-follow: exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::Disconnected)
                ),
                "closed/no-follow: last terminal record must be Disconnected"
            );
        }

        // ── gave-up (budget exhausted): exactly one, GaveUp ──
        {
            let mut script = Vec::new();
            for _ in 0..=MAX_RECONNECTS {
                script.push((
                    vec![make_log_event("pod|1", "line")],
                    Ok(AttemptOutcome::Closed),
                ));
            }
            let attempt = FakeStreamAttempt::new(script);
            let mut writer = RecordingWriter::new();
            let _ = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                CancellationToken::new(),
            )
            .await;
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "gave-up: exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::GaveUp(_))
                ),
                "gave-up: last terminal record must be GaveUp, got: {:?}",
                last_terminal_record(&writer.calls)
            );
        }

        // ── error (decision table returns an error, e.g. HTTP 403): exactly one, Error ──
        {
            let attempt = FakeStreamAttempt::new(vec![(
                vec![],
                Ok(AttemptOutcome::HttpStatus {
                    status: 403,
                    detail: "forbidden".to_string(),
                }),
            )]);
            let mut writer = RecordingWriter::new();
            let _ = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                CancellationToken::new(),
            )
            .await;
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "http-error: exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::Error(_))
                ),
                "http-error: last terminal record must be Error"
            );
        }

        // ── attempt error (token resolution failure): exactly one, Error ──
        {
            let attempt = FakeStreamAttempt::new(vec![(
                vec![],
                Err(CliError::Auth {
                    message: "token expired".to_string(),
                    metadata: None,
                }),
            )]);
            let mut writer = RecordingWriter::new();
            let _ = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                CancellationToken::new(),
            )
            .await;
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "attempt-error: exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::Error(_))
                ),
                "attempt-error: last terminal record must be Error"
            );
        }

        // ── cancellation: exactly one, Stopped ──
        {
            let cancel = CancellationToken::new();
            let trigger = cancel.clone();
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                trigger.cancel();
            });

            struct BlockingAttempt2;
            impl StreamAttempt for BlockingAttempt2 {
                fn run_attempt<'a>(
                    &'a self,
                    _reconnect: bool,
                    _last_event_id: Option<&'a str>,
                    _sink: &'a mut dyn EventSink,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<AttemptOutcome, CliError>> + 'a>,
                > {
                    Box::pin(async {
                        std::future::pending::<()>().await;
                        unreachable!()
                    })
                }
            }

            let mut writer = RecordingWriter::new();
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                run_stream_loop(
                    &BlockingAttempt2,
                    |_| Duration::ZERO,
                    &mut writer,
                    true,
                    "ns",
                    cancel,
                ),
            )
            .await
            .expect("should not time out");
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "cancellation: exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::Stopped)
                ),
                "cancellation: last terminal record must be Stopped"
            );
        }

        // ── stdout closed: zero terminal records ──
        {
            struct ClosingWriter2 {
                calls: Vec<WriterCall>,
            }

            impl LogStreamWriter for ClosingWriter2 {
                fn opening(&mut self, _: &super::super::output::OpeningParams<'_>) {}
                fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError> {
                    self.calls.push(WriterCall::Connected(reconnect));
                    Ok(StdoutLineWrite::Written)
                }
                fn log_line(
                    &mut self,
                    id: Option<&str>,
                    data: &str,
                ) -> Result<StdoutLineWrite, CliError> {
                    self.calls
                        .push(WriterCall::Log(id.map(String::from), data.to_string()));
                    Ok(StdoutLineWrite::ReaderClosed)
                }
                fn pod_done(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
                    Ok(StdoutLineWrite::Written)
                }
                fn reconnecting(
                    &mut self,
                    _: u32,
                    _: u32,
                    _: u64,
                    _: Option<&str>,
                    _: ReconnectReason,
                    _: Option<&str>,
                ) -> Result<StdoutLineWrite, CliError> {
                    Ok(StdoutLineWrite::Written)
                }
                fn stream_end(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
                    self.calls.push(WriterCall::StreamEnd);
                    Ok(StdoutLineWrite::Written)
                }
                fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError> {
                    self.calls.push(WriterCall::Disconnected);
                    Ok(StdoutLineWrite::Written)
                }
                fn stopped(&mut self) -> Result<StdoutLineWrite, CliError> {
                    self.calls.push(WriterCall::Stopped);
                    Ok(StdoutLineWrite::Written)
                }
                fn gave_up(
                    &mut self,
                    n: u32,
                    _: Option<&str>,
                ) -> Result<StdoutLineWrite, CliError> {
                    self.calls.push(WriterCall::GaveUp(n));
                    Ok(StdoutLineWrite::Written)
                }
                fn error(&mut self, _: &str, code: i32) -> Result<StdoutLineWrite, CliError> {
                    self.calls.push(WriterCall::Error(code));
                    Ok(StdoutLineWrite::Written)
                }
                fn verbose(&mut self, _: &str) {}
                fn dry_run(
                    &mut self,
                    _: &str,
                    _: &str,
                    _: &str,
                    _: &str,
                ) -> Result<StdoutLineWrite, CliError> {
                    Ok(StdoutLineWrite::Written)
                }
            }

            let attempt = FakeStreamAttempt::new(vec![(
                vec![make_log_event("pod|1", "line1")],
                Ok(AttemptOutcome::StoppedBySink),
            )]);
            let mut writer = ClosingWriter2 { calls: Vec::new() };
            let _ = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                CancellationToken::new(),
            )
            .await;
            assert_eq!(
                terminal_record_count(&writer.calls),
                0,
                "stdout-closed: zero terminal records, got: {:?}",
                writer.calls
            );
        }

        // ── late cancellation (cancel after attempt returns): exactly one, Stopped ──
        {
            let cancel = CancellationToken::new();
            let trigger = cancel.clone();

            struct LateCancelAttempt {
                cancel: CancellationToken,
            }
            impl StreamAttempt for LateCancelAttempt {
                fn run_attempt<'a>(
                    &'a self,
                    _reconnect: bool,
                    _last_event_id: Option<&'a str>,
                    sink: &'a mut dyn EventSink,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<AttemptOutcome, CliError>> + 'a>,
                > {
                    Box::pin(async {
                        let _ = sink.opened();
                        // Cancel after the attempt body but before returning.
                        self.cancel.cancel();
                        Ok(AttemptOutcome::Closed)
                    })
                }
            }

            let attempt = LateCancelAttempt { cancel: trigger };
            let mut writer = RecordingWriter::new();
            let _ = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                cancel,
            )
            .await;
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "late-cancel: exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::Stopped)
                ),
                "late-cancel: last terminal record must be Stopped, got: {:?}",
                last_terminal_record(&writer.calls)
            );
        }
    }

    // ── Late-cancellation test (cancel arrives after attempt returns) ──

    #[tokio::test]
    async fn loop_writes_stopped_when_cancel_arrives_after_attempt_ends() {
        // A fake attempt that cancels the token and then returns Closed.
        // Once with follow and once without: both must write exactly one
        // terminal record (stopped), return Ok, and not reconnect.
        for follow in [true, false] {
            let cancel = CancellationToken::new();
            let trigger = cancel.clone();

            struct CancellingAttempt {
                cancel: CancellationToken,
            }
            impl StreamAttempt for CancellingAttempt {
                fn run_attempt<'a>(
                    &'a self,
                    _reconnect: bool,
                    _last_event_id: Option<&'a str>,
                    sink: &'a mut dyn EventSink,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<AttemptOutcome, CliError>> + 'a>,
                > {
                    Box::pin(async {
                        sink.opened();
                        // Cancel after the attempt body but before returning.
                        self.cancel.cancel();
                        Ok(AttemptOutcome::Closed)
                    })
                }
            }

            let attempt = CancellingAttempt {
                cancel: trigger.clone(),
            };
            let mut writer = RecordingWriter::new();
            let result = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                follow,
                "ns",
                cancel,
            )
            .await;

            assert!(
                result.is_ok(),
                "late-cancel (follow={follow}) must return Ok"
            );
            assert_eq!(
                terminal_record_count(&writer.calls),
                1,
                "late-cancel (follow={follow}): exactly one terminal record, got: {:?}",
                writer.calls
            );
            assert!(
                matches!(
                    last_terminal_record(&writer.calls),
                    Some(WriterCall::Stopped)
                ),
                "late-cancel (follow={follow}): terminal record must be Stopped, got: {:?}",
                last_terminal_record(&writer.calls)
            );
        }
    }

    // ── Backoff reuse test ──

    // ── Stdout write-error propagation tests ──

    /// Helper: make a Usage error matching the helper's format.
    fn write_error() -> CliError {
        CliError::Usage {
            message: "Cannot write to stdout: No space left on device.".to_string(),
            metadata: None,
        }
    }

    /// A writer where `log_line` always returns an error.
    struct FailOnLogLineWriter {
        calls: Vec<WriterCall>,
    }

    impl LogStreamWriter for FailOnLogLineWriter {
        fn opening(&mut self, _: &super::super::output::OpeningParams<'_>) {}
        fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Connected(reconnect));
            Ok(StdoutLineWrite::Written)
        }
        fn log_line(
            &mut self,
            _id: Option<&str>,
            _data: &str,
        ) -> Result<StdoutLineWrite, CliError> {
            Err(write_error())
        }
        fn pod_done(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
            Ok(StdoutLineWrite::Written)
        }
        fn reconnecting(
            &mut self,
            _: u32,
            _: u32,
            _: u64,
            _: Option<&str>,
            _: ReconnectReason,
            _: Option<&str>,
        ) -> Result<StdoutLineWrite, CliError> {
            Ok(StdoutLineWrite::Written)
        }
        fn stream_end(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::StreamEnd);
            Ok(StdoutLineWrite::Written)
        }
        fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Disconnected);
            Ok(StdoutLineWrite::Written)
        }
        fn stopped(&mut self) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Stopped);
            Ok(StdoutLineWrite::Written)
        }
        fn gave_up(&mut self, _: u32, _: Option<&str>) -> Result<StdoutLineWrite, CliError> {
            Ok(StdoutLineWrite::Written)
        }
        fn error(&mut self, _: &str, code: i32) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Error(code));
            Ok(StdoutLineWrite::Written)
        }
        fn verbose(&mut self, _: &str) {}
        fn dry_run(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<StdoutLineWrite, CliError> {
            Ok(StdoutLineWrite::Written)
        }
    }

    /// A configurable writer where specific methods return an error or
    /// report the reader as closed.
    struct FailOnWriter {
        calls: Vec<WriterCall>,
        fail_stream_end: bool,
        fail_error: bool,
        fail_reconnecting: bool,
        fail_pod_done: bool,
        close_error: bool,
        close_gave_up: bool,
    }

    impl FailOnWriter {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                fail_stream_end: false,
                fail_error: false,
                fail_reconnecting: false,
                fail_pod_done: false,
                close_error: false,
                close_gave_up: false,
            }
        }
    }

    impl LogStreamWriter for FailOnWriter {
        fn opening(&mut self, _: &super::super::output::OpeningParams<'_>) {}
        fn connected(&mut self, reconnect: u32) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Connected(reconnect));
            Ok(StdoutLineWrite::Written)
        }
        fn log_line(&mut self, id: Option<&str>, data: &str) -> Result<StdoutLineWrite, CliError> {
            self.calls
                .push(WriterCall::Log(id.map(String::from), data.to_string()));
            Ok(StdoutLineWrite::Written)
        }
        fn pod_done(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::PodDone);
            if self.fail_pod_done {
                return Err(write_error());
            }
            Ok(StdoutLineWrite::Written)
        }
        fn reconnecting(
            &mut self,
            reconnect: u32,
            _: u32,
            _: u64,
            _: Option<&str>,
            _: ReconnectReason,
            _: Option<&str>,
        ) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Reconnecting { reconnect });
            if self.fail_reconnecting {
                return Err(write_error());
            }
            Ok(StdoutLineWrite::Written)
        }
        fn stream_end(&mut self, _: &str) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::StreamEnd);
            if self.fail_stream_end {
                return Err(write_error());
            }
            Ok(StdoutLineWrite::Written)
        }
        fn disconnected(&mut self) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Disconnected);
            Ok(StdoutLineWrite::Written)
        }
        fn stopped(&mut self) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Stopped);
            Ok(StdoutLineWrite::Written)
        }
        fn gave_up(&mut self, n: u32, _: Option<&str>) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::GaveUp(n));
            if self.close_gave_up {
                return Ok(StdoutLineWrite::ReaderClosed);
            }
            Ok(StdoutLineWrite::Written)
        }
        fn error(&mut self, _: &str, code: i32) -> Result<StdoutLineWrite, CliError> {
            self.calls.push(WriterCall::Error(code));
            if self.fail_error {
                return Err(write_error());
            }
            if self.close_error {
                return Ok(StdoutLineWrite::ReaderClosed);
            }
            Ok(StdoutLineWrite::Written)
        }
        fn verbose(&mut self, _: &str) {}
        fn dry_run(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<StdoutLineWrite, CliError> {
            Ok(StdoutLineWrite::Written)
        }
    }

    #[tokio::test]
    async fn loop_stops_with_the_write_error_when_stdout_fails() {
        // --follow, the first log line's write fails. The loop returns the
        // write error, one attempt, no reconnect, no terminal record.
        let attempt = FakeStreamAttempt::new(vec![
            (
                vec![make_log_event("pod|1", "line1")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
            // Should never be reached.
            (
                vec![make_named_event("stream-end", "{}")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let mut writer = FailOnLogLineWriter { calls: Vec::new() };
        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 1, "write error must exit 1");
        assert_eq!(
            err.to_string(),
            "Cannot write to stdout: No space left on device.",
            "error must be the exact write error"
        );
        // One attempt only (one Connected call).
        let connected_count = writer
            .calls
            .iter()
            .filter(|c| matches!(c, WriterCall::Connected(_)))
            .count();
        assert_eq!(connected_count, 1, "only one attempt");
        // No terminal record.
        assert_eq!(
            terminal_record_count(&writer.calls),
            0,
            "no terminal record on write error: {:?}",
            writer.calls
        );
        // No reconnecting record.
        assert!(
            !writer
                .calls
                .iter()
                .any(|c| matches!(c, WriterCall::Reconnecting { .. })),
            "no reconnect on write error"
        );
    }

    #[tokio::test]
    async fn loop_returns_the_write_error_when_a_terminal_record_write_fails() {
        // stream-end arrives (Ok ending); the stream_end terminal record
        // write fails: the loop returns the write error.
        let attempt = FakeStreamAttempt::new(vec![(
            vec![
                make_log_event("pod|1", "hello"),
                make_named_event("stream-end", r#"{"reason":"completed"}"#),
            ],
            Ok(AttemptOutcome::StoppedBySink),
        )]);

        let mut writer = FailOnWriter::new();
        writer.fail_stream_end = true;

        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 1, "write error must exit 1");
        assert!(
            err.to_string().contains("Cannot write to stdout"),
            "error must be the write error: {err}"
        );
    }

    #[tokio::test]
    async fn loop_keeps_the_original_error_when_the_error_record_write_fails() {
        // HTTP 404 ending; the error terminal record's write fails:
        // the loop returns the original exit-3 error, not the write error.
        let attempt = FakeStreamAttempt::new(vec![(
            vec![],
            Ok(AttemptOutcome::HttpStatus {
                status: 404,
                detail: "not found".to_string(),
            }),
        )]);

        let mut writer = FailOnWriter::new();
        writer.fail_error = true;

        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(
            err.exit_code(),
            3,
            "original error (HTTP 404, exit 3) takes precedence over write error"
        );
    }

    #[tokio::test]
    async fn loop_stops_when_the_reconnecting_write_fails() {
        // First attempt closes (triggers reconnect with --follow); the
        // reconnecting record's write fails: the loop returns the write
        // error without waiting or reconnecting.
        let attempt = FakeStreamAttempt::new(vec![
            (
                vec![make_log_event("pod|1", "line1")],
                Ok(AttemptOutcome::Closed),
            ),
            // Should never be reached.
            (
                vec![make_named_event("stream-end", "{}")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let mut writer = FailOnWriter::new();
        writer.fail_reconnecting = true;

        let mut delays = Vec::new();
        let result = run_stream_loop(
            &attempt,
            |n| {
                delays.push(n);
                Duration::from_secs(300) // long delay: should not be reached
            },
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 1, "write error must exit 1");
        assert!(
            err.to_string().contains("Cannot write to stdout"),
            "error must be the write error: {err}"
        );
        // The delay function was called (to compute the delay for the
        // reconnecting record) but the loop did not wait.
        assert!(
            delays.len() <= 1,
            "at most one delay computation, got: {:?}",
            delays
        );
        // Only one attempt.
        let connected_count = writer
            .calls
            .iter()
            .filter(|c| matches!(c, WriterCall::Connected(_)))
            .count();
        assert_eq!(connected_count, 1, "only one attempt");
    }

    #[tokio::test]
    async fn loop_keeps_the_original_error_when_stdout_closes_at_the_final_record() {
        // Case 1: HTTP 404 ending (exit 3). The `error` terminal record's
        // write returns ReaderClosed. The loop must return the original
        // exit-3 error, not Ok(()).
        {
            let attempt = FakeStreamAttempt::new(vec![(
                vec![],
                Ok(AttemptOutcome::HttpStatus {
                    status: 404,
                    detail: "not found".to_string(),
                }),
            )]);

            let mut writer = FailOnWriter::new();
            writer.close_error = true;

            let result = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                CancellationToken::new(),
            )
            .await;

            let err = result.unwrap_err();
            assert_eq!(
                err.exit_code(),
                3,
                "HTTP 404 ending must keep exit 3 when stdout closes at the error record"
            );
        }

        // Case 2: give-up ending (exit 4, budget exhausted). The `gave_up`
        // terminal record's write returns ReaderClosed. The loop must return
        // the original exit-4 error, not Ok(()).
        {
            let mut script = Vec::new();
            for _ in 0..=MAX_RECONNECTS {
                script.push((
                    vec![make_log_event("pod|1", "line")],
                    Ok(AttemptOutcome::Closed),
                ));
            }
            let attempt = FakeStreamAttempt::new(script);

            let mut writer = FailOnWriter::new();
            writer.close_gave_up = true;

            let result = run_stream_loop(
                &attempt,
                |_| Duration::ZERO,
                &mut writer,
                true,
                "ns",
                CancellationToken::new(),
            )
            .await;

            let err = result.unwrap_err();
            assert_eq!(
                err.exit_code(),
                4,
                "give-up ending must keep exit 4 when stdout closes at the gave_up record"
            );
        }
    }

    #[tokio::test]
    async fn loop_stops_with_the_write_error_when_pod_done_write_fails() {
        // --follow, a pod-done event whose write fails with the helper's
        // Usage error. The loop returns the write error, one attempt, no
        // reconnect, no terminal record.
        let attempt = FakeStreamAttempt::new(vec![
            (
                vec![make_named_event("pod-done", r#"{"pod":"x"}"#)],
                Ok(AttemptOutcome::StoppedBySink),
            ),
            // Should never be reached.
            (
                vec![make_named_event("stream-end", "{}")],
                Ok(AttemptOutcome::StoppedBySink),
            ),
        ]);

        let mut writer = FailOnWriter::new();
        writer.fail_pod_done = true;

        let result = run_stream_loop(
            &attempt,
            |_| Duration::ZERO,
            &mut writer,
            true,
            "ns",
            CancellationToken::new(),
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.exit_code(), 1, "write error must exit 1");
        assert_eq!(
            err.to_string(),
            "Cannot write to stdout: No space left on device.",
            "error must be the exact write error"
        );
        // One attempt only.
        let connected_count = writer
            .calls
            .iter()
            .filter(|c| matches!(c, WriterCall::Connected(_)))
            .count();
        assert_eq!(connected_count, 1, "only one attempt");
        // No terminal record.
        assert_eq!(
            terminal_record_count(&writer.calls),
            0,
            "no terminal record on write error: {:?}",
            writer.calls
        );
        // No reconnecting record.
        assert!(
            !writer
                .calls
                .iter()
                .any(|c| matches!(c, WriterCall::Reconnecting { .. })),
            "no reconnect on write error"
        );
    }

    // ── Backoff reuse test ──

    #[test]
    fn loop_uses_backoff_from_remote_debug() {
        // The production delay function's base values should be 5, 10, 20, 20, 20.
        let bases: Vec<u64> = (1..=5).map(backoff_base_secs).collect();
        assert_eq!(bases, vec![5, 10, 20, 20, 20]);

        // With a fixed jitter factor of 0.5 (midpoint), delays should be
        // exactly the base values.
        for attempt in 1..=5 {
            let base = backoff_base_secs(attempt);
            let jittered = apply_jitter(base, 0.5);
            assert!(
                (jittered - base as f64).abs() < 1e-9,
                "midpoint jitter must be identity for attempt {attempt}"
            );
        }

        // The reconnect_delay function should produce values within ±20% of
        // the base (we cannot pin the exact value due to randomness).
        for attempt in 1..=5 {
            let delay = reconnect_delay(attempt);
            let base = backoff_base_secs(attempt) as f64;
            let secs = delay.as_secs_f64();
            assert!(
                secs >= base * 0.80 - 0.01 && secs <= base * 1.20 + 0.01,
                "reconnect_delay({attempt}) = {secs}s, expected ±20% of {base}s"
            );
        }
    }
}
