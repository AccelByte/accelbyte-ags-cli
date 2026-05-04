//! Progress indicator for in-flight API requests.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::frontend::style;
use crate::protocol::event::{ProgressEvent, ProgressSink};

/// A blinking status indicator on stderr that auto-clears on drop.
///
/// When is_active, blinks `∘` on and off every 400ms to show work in progress.
/// The message can be updated while the spinner is running via `update()`.
/// All output is suppressed when quiet mode is enabled or stderr is not a TTY.
pub struct StatusLine {
    is_active: bool,
    stop: Option<Arc<AtomicBool>>,
    message: Option<Arc<Mutex<String>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl StatusLine {
    /// Create a new status line. Output is suppressed if `quiet` is true or stderr is not a TTY.
    pub fn new(quiet: bool) -> Self {
        Self {
            is_active: !quiet && crate::support::is_stderr_tty(),
            stop: None,
            message: None,
            handle: None,
        }
    }

    /// Display a pulsing status message on stderr.
    pub fn show(&mut self, message: &str) {
        if !self.is_active {
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let shared_message = Arc::new(Mutex::new(message.to_string()));
        let message_clone = shared_message.clone();

        let handle = std::thread::spawn(move || {
            let mut visible = true;
            loop {
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                let status_message = message_clone
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                {
                    // Scope the lock guard so it drops BEFORE the 400ms sleep —
                    // otherwise any other thread calling write_stderr_line would
                    // block on stderr for up to one blink cycle.
                    let mut err = anstream::stderr().lock();
                    if visible {
                        let _ = write!(err, "\r{}\x1b[K", style::status(&status_message, true));
                    } else {
                        let _ = write!(err, "\r\x1b[2m  {status_message}\x1b[0m\x1b[K");
                    }
                    let _ = err.flush();
                }
                visible = !visible;
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        });

        self.stop = Some(stop);
        self.message = Some(shared_message);
        self.handle = Some(handle);
    }

    /// Update the message while the spinner is running.
    pub fn update(&self, message: &str) {
        if let Some(shared) = &self.message {
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = message.to_string();
        }
    }

    /// Clear the status line and stop the spinner.
    pub fn clear(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        if self.is_active {
            let mut err = anstream::stderr().lock();
            let _ = write!(err, "\r\x1b[K");
            let _ = err.flush();
            self.is_active = false;
        }
    }
}

impl Drop for StatusLine {
    fn drop(&mut self) {
        self.clear();
    }
}

/// A `ProgressSink` implementation that drives a `StatusLine` spinner and
/// writes urgent `Message` events to stderr as persistent lines.
///
/// Used by `invocation/commands/service.rs` to bridge runtime-originated progress
/// events to the existing terminal spinner. Byte-identical to the direct
/// `StatusLine` calls it replaces.
pub struct StatusLineSink {
    status: StatusLine,
}

impl StatusLineSink {
    /// Construct a new sink. When `quiet` is true, all spinner output is
    /// suppressed (same as passing `quiet: true` to `StatusLine::new`).
    /// `Message` events still print to stderr — `--quiet` does not suppress
    /// urgent warnings, matching today's behaviour.
    pub fn new(quiet: bool) -> Self {
        Self {
            status: StatusLine::new(quiet),
        }
    }
}

impl ProgressSink for StatusLineSink {
    fn on_event(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::Started { message } => {
                self.status.show(&message);
            }
            ProgressEvent::Page { current, total } => {
                let text = match total {
                    Some(t) => format!("Fetching page {current}/{t}..."),
                    None => format!("Fetching page {current}..."),
                };
                self.status.update(&text);
            }
            ProgressEvent::Message { text } => {
                // "Urgent, persistent stderr output." Clear the spinner so
                // the text does not get overwritten, then print. The caller
                // typically follows up with `Finished` or exits.
                self.status.clear();
                crate::frontend::write_stderr_line(&text);
            }
            ProgressEvent::Finished => {
                self.status.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quiet mode suppresses all output without panicking
    #[test]
    fn test_status_line_quiet_mode() {
        // In quiet mode, show/clear should be no-ops (no panic, no output)
        let mut status = StatusLine::new(true);
        status.show("Testing...");
        status.clear();
    }

    /// StatusLineSink in quiet mode accepts every ProgressEvent variant
    /// without panicking. The actual terminal output is exercised manually
    /// (not snapshot-tested) because StatusLine blinks on a background
    /// thread.
    #[test]
    fn test_status_line_sink_quiet_handles_all_events() {
        use crate::protocol::event::{ProgressEvent, ProgressSink};

        let mut sink = StatusLineSink::new(true);
        sink.on_event(ProgressEvent::Started {
            message: "Fetching...".to_string(),
        });
        sink.on_event(ProgressEvent::Page {
            current: 2,
            total: Some(5),
        });
        sink.on_event(ProgressEvent::Page {
            current: 3,
            total: None,
        });
        sink.on_event(ProgressEvent::Message {
            text: "Stopped after 10 pages".to_string(),
        });
        sink.on_event(ProgressEvent::Finished);
    }
}
