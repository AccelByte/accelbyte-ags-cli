//! Shared terminal-backend plumbing for the inline and fullscreen TUIs.
//!
//! Both surfaces install a ratatui terminal over stderr wrapped in
//! [`NoColorBackend`] (stdout stays reserved for `CommandOutput`, so piping
//! `ags … | jq` keeps result bytes clean), and both enter raw mode the same
//! way. The surface-specific divergence — inline installs a fixed-height
//! viewport, fullscreen enters the alternate screen — stays in each surface's
//! `lifecycle` module; the common construction lives here so it can't drift.

use std::io::Stderr;

use crossterm::terminal::enable_raw_mode;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::errors::CliError;
use crate::frontend::terminal::no_color_backend::NoColorBackend;

/// A ratatui terminal over stderr with the no-colour wrapper applied. Both the
/// inline viewport and the fullscreen alt-screen install this exact type; the
/// viewport-vs-alt-screen choice is made by each surface's `lifecycle::acquire`.
pub type Tty = Terminal<NoColorBackend<CrosstermBackend<Stderr>>>;

/// Build the stderr backend, stripping cell colours under `--no-color` /
/// `NO_COLOR` (ratatui renders styles regardless of any colour preference).
pub fn stderr_backend() -> NoColorBackend<CrosstermBackend<Stderr>> {
    NoColorBackend::new(
        CrosstermBackend::new(std::io::stderr()),
        !crate::frontend::style::is_stderr_enabled(),
    )
}

/// Enter raw mode, mapping the failure to the shared user-facing error.
pub fn enter_raw_mode() -> Result<(), CliError> {
    enable_raw_mode().map_err(|e| CliError::Usage {
        message: format!("Cannot enter raw terminal mode: {e}"),
        metadata: None,
    })
}
