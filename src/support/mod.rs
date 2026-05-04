//! Shared utilities: filesystem helpers, string transforms, and small process utilities.

pub mod file_system;
pub mod output_sink;
pub mod strings;

pub use file_system::FileLock;

static LOCK_CONTENTION_REPORTER: std::sync::OnceLock<fn(&str)> = std::sync::OnceLock::new();

/// Register the process-global hook that reports advisory-lock contention.
///
/// Invocation wires this to frontend stderr output at startup so lower layers
/// can stay decoupled from presentation concerns. Subsequent registrations are
/// ignored; the first reporter wins for the life of the process.
pub fn register_lock_contention_reporter(reporter: fn(&str)) {
    let _ = LOCK_CONTENTION_REPORTER.set(reporter);
}

/// Report that a named advisory lock is contended.
pub(crate) fn report_lock_contention(lock_name: &str) {
    if let Some(reporter) = LOCK_CONTENTION_REPORTER.get().copied() {
        reporter(lock_name);
    }
}

/// Format a duration in seconds as a two-component human-readable string.
/// Uses the two largest non-zero units: "2h 15m", "8m 30s", or "45s".
pub fn format_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Returns the current Unix timestamp in seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be at or after the UNIX epoch")
        .as_secs()
}

/// Return whether stdin is connected to an interactive terminal.
pub fn is_stdin_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// Return whether stdout is connected to an interactive terminal.
pub fn is_stdout_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Return whether stderr is connected to an interactive terminal.
pub fn is_stderr_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sub-minute durations render as seconds only
    #[test]
    fn test_format_duration_seconds_only() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(59), "59s");
    }

    /// Durations under one hour show minutes and seconds
    #[test]
    fn test_format_duration_minutes_and_seconds() {
        assert_eq!(format_duration(60), "1m 0s");
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3599), "59m 59s");
    }

    /// Durations of one hour or more show hours and minutes
    #[test]
    fn test_format_duration_hours_and_minutes() {
        assert_eq!(format_duration(3600), "1h 0m");
        assert_eq!(format_duration(8100), "2h 15m");
        assert_eq!(format_duration(86400), "24h 0m");
    }

    /// unix_now returns a value between two SystemTime reads, proving it uses the real clock
    #[test]
    fn test_unix_now_returns_reasonable_timestamp() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be at or after the UNIX epoch")
            .as_secs();
        let now = unix_now();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be at or after the UNIX epoch")
            .as_secs();
        assert!(
            now >= before && now <= after,
            "unix_now() = {now} should be between {before} and {after}"
        );
    }
}
