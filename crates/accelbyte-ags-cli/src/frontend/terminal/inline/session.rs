//! `InlineSession` — shared ownership of the live terminal handle.
//!
//! The `select_workflow_phase_surfaces` factory builds one `InlineSession` and
//! hands the *same* `Rc<RefCell<InlineSession>>` to both `InlineFrontend` and
//! `InlineInteraction` so both reach the terminal without acquiring it twice.
//!
//! # Borrow-safety invariant
//!
//! Rendering and interaction borrow the session (via `RefCell::borrow_mut`)
//! at strictly disjoint times — a render phase completes fully before any
//! interaction phase starts, and vice versa. They are never nested. This
//! means `RefCell` will never panic at runtime due to a concurrent borrow.

use crate::errors::CliError;
use crate::frontend::terminal::inline::lifecycle::{acquire, release, Tty};

/// Owns the live terminal handle for the duration of a TUI session.
///
/// Construct with `InlineSession::new()` (acquires the terminal) or, in tests,
/// with `InlineSession::without_terminal()` (no acquisition, `terminal: None`).
/// The `Drop` impl calls `teardown` as a panic-safety fallback; the normal
/// path is `finish` on `InlineFrontend`, which also calls `teardown` first.
pub struct InlineSession {
    terminal: Option<Tty>,
}

impl InlineSession {
    /// Acquire the terminal and enable raw mode. Returns `Err` if the
    /// terminal cannot be installed (no TTY on stderr, raw mode failure, etc.).
    pub fn new() -> Result<Self, CliError> {
        Self::acquire_if_stderr_tty(ags_runtime::support::is_stderr_tty)
    }

    /// Core of [`new`](Self::new) with the stderr-TTY predicate injected, so the
    /// no-TTY error path is unit-testable without depending on the test
    /// process's real stderr (which, under `cargo test`, inherits the
    /// terminal). The inline viewport renders on stderr, so that is the channel
    /// the gate reports on; raw-mode acquisition remains the backstop for
    /// stdin. When the predicate is true this acquires the real terminal.
    pub(crate) fn acquire_if_stderr_tty(
        is_stderr_tty: impl FnOnce() -> bool,
    ) -> Result<Self, CliError> {
        if !is_stderr_tty() {
            return Err(CliError::Usage {
                message: "Inline UI requires an interactive terminal on stderr".into(),
                metadata: None,
            });
        }
        let terminal = acquire()?;
        Ok(Self {
            terminal: Some(terminal),
        })
    }

    /// Borrow the active terminal, returning a `Usage` error if it has
    /// already been released.
    pub fn terminal_mut(&mut self) -> Result<&mut Tty, CliError> {
        self.terminal.as_mut().ok_or_else(|| CliError::Usage {
            message: "TUI frontend terminal already released".into(),
            metadata: None,
        })
    }

    /// Tear down the terminal exactly once. Idempotent: safe to call from
    /// both `InlineFrontend::finish` and `InlineSession::drop`.
    pub fn teardown(&mut self) {
        if let Some(terminal) = self.terminal.take() {
            release(terminal);
        }
    }

    /// Test-only constructor: builds a `InlineSession` with `terminal: None`
    /// and performs NO `acquire()`. Allows session-sharing wiring to be
    /// unit-tested in CI, where there is no TTY.
    #[cfg(test)]
    pub fn without_terminal() -> Self {
        Self { terminal: None }
    }
}

impl Drop for InlineSession {
    fn drop(&mut self) {
        // Panic-safety fallback. Errors are swallowed; `InlineFrontend::finish`
        // is the path that surfaces them.
        self.teardown();
    }
}

#[cfg(test)]
mod session_tests {
    use super::InlineSession;
    use crate::errors::CliError;

    #[test]
    fn test_without_terminal_returns_usage_error_on_terminal_mut() {
        let mut session = InlineSession::without_terminal();
        let result = session.terminal_mut();
        assert!(
            matches!(result, Err(CliError::Usage { .. })),
            "expected Usage error from terminal_mut on a no-terminal session"
        );
    }
}
