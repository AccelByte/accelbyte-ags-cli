//! Progress indicator for in-flight API requests.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use unicode_width::UnicodeWidthStr;

use crate::frontend::style;

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
            is_active: !quiet && ags_runtime::support::is_stderr_tty(),
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
                let columns = crossterm::terminal::size().map_or(80, |(cols, _)| cols);
                {
                    // Scope the lock guard so it drops BEFORE the 400ms sleep —
                    // otherwise any other thread calling write_stderr_line would
                    // block on stderr for up to one blink cycle.
                    let mut err = anstream::stderr().lock();
                    let _ = write!(err, "{}", status_frame(&status_message, visible, columns));
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

/// Fit `text` to at most `max_cols` display columns.
///
/// Returns `text` unchanged when its display width fits. Otherwise truncates
/// at a character boundary and appends `…` (U+2026, one column) so the
/// result's display width is at most `max_cols`.
pub fn fit_to_columns(text: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let text_width = UnicodeWidthStr::width(text);
    if text_width <= max_cols {
        return text.to_string();
    }
    // Reserve one column for the ellipsis.
    let budget = max_cols.saturating_sub(1);
    let mut used = 0usize;
    let mut result = String::new();
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > budget {
            break;
        }
        result.push(ch);
        used += ch_width;
    }
    result.push('\u{2026}');
    result
}

/// Build the exact bytes of one status-line redraw frame.
///
/// Visible frame: `\r` + `style::status(&fitted, true)` + `\x1b[K`.
/// Blink frame: `\r\x1b[2m  ` + fitted + `\x1b[0m\x1b[K`.
/// `fitted` is `fit_to_columns(message, max)` where `max` leaves room for
/// the two-column prefix and keeps the last terminal column empty:
/// `(columns as usize).saturating_sub(3)`.
pub fn status_frame(message: &str, visible: bool, columns: u16) -> String {
    let max = (columns as usize).saturating_sub(3);
    let fitted = fit_to_columns(message, max);
    if visible {
        format!("\r{}\x1b[K", style::status(&fitted, true))
    } else {
        format!("\r\x1b[2m  {fitted}\x1b[0m\x1b[K")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    /// Quiet mode suppresses all output without panicking
    #[test]
    fn test_status_line_quiet_mode() {
        // In quiet mode, show/clear should be no-ops (no panic, no output)
        let mut status = StatusLine::new(true);
        status.show("Testing...");
        status.clear();
    }

    #[test]
    fn test_fit_to_columns_keeps_short_text() {
        let text = "hello world";
        let result = fit_to_columns(text, 80);
        assert_eq!(result, text);
    }

    #[test]
    fn test_fit_to_columns_cuts_with_ellipsis() {
        // A message longer than 80 columns is cut and ends with the ellipsis,
        // and the result's display width is exactly 80.
        let long = "A".repeat(200);
        let result = fit_to_columns(&long, 80);
        assert!(
            result.ends_with('\u{2026}'),
            "expected trailing ellipsis, got: {result}"
        );
        assert_eq!(
            UnicodeWidthStr::width(result.as_str()),
            80,
            "display width must be exactly 80"
        );
    }

    #[test]
    fn test_fit_to_columns_counts_display_width() {
        // Double-width CJK characters: each is 2 columns. Cutting to 7 columns
        // must not split inside a character. 6 CJK chars = 12 columns.
        let cjk = "\u{6F22}\u{5B57}\u{6F22}\u{5B57}\u{6F22}\u{5B57}";
        let result = fit_to_columns(cjk, 7);
        let width = UnicodeWidthStr::width(result.as_str());
        assert!(
            width <= 7,
            "display width {width} exceeds max_cols 7: {result}"
        );
        // 3 CJK chars (6 cols) + ellipsis (1 col) = 7
        assert!(
            result.ends_with('\u{2026}'),
            "expected trailing ellipsis, got: {result}"
        );
        assert_eq!(width, 7, "should use all available columns: {result}");
    }

    /// Strip ANSI escape sequences for display-width measurement.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut in_escape = false;
        for ch in s.chars() {
            if in_escape {
                if ch.is_ascii_alphabetic() || ch == 'K' {
                    in_escape = false;
                }
                continue;
            }
            if ch == '\x1b' {
                in_escape = true;
                continue;
            }
            if ch == '\r' {
                continue;
            }
            out.push(ch);
        }
        out
    }

    #[test]
    fn test_frame_never_exceeds_terminal_width() {
        let long_message = "X".repeat(200);
        for columns in [3u16, 4, 40, 80, 91, 120] {
            for visible in [true, false] {
                let frame = status_frame(&long_message, visible, columns);
                let stripped = strip_ansi(&frame);
                let width = UnicodeWidthStr::width(stripped.as_str());
                assert!(
                    width <= (columns as usize).saturating_sub(1),
                    "columns={columns} visible={visible}: stripped width {width} \
                     exceeds limit {}, stripped={stripped:?}",
                    columns - 1
                );
            }
        }
    }
}
