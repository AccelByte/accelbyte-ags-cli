//! Terminal acquisition and teardown for the fullscreen TUI.
//!
//! Parallel to `inline::lifecycle` but enters the alternate screen so the
//! workflow surface owns the entire terminal viewport for its lifetime,
//! then leaves it on teardown to restore the user's scrollback.

use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;

use crate::errors::CliError;
pub use crate::frontend::terminal::backend::Tty;
use crate::frontend::terminal::backend::{enter_raw_mode, stderr_backend};

/// Enable raw mode, enter the alternate screen, and install a ratatui
/// terminal on stderr. Mirrors the stream-ownership choice made for the
/// inline backend — stdout stays reserved for `CommandOutput`
/// so the fullscreen surface doesn't corrupt piped output.
pub fn acquire() -> Result<Tty, CliError> {
    enter_raw_mode()?;
    execute!(std::io::stderr(), EnterAlternateScreen).map_err(|e| {
        let _ = disable_raw_mode();
        CliError::Usage {
            message: format!("Cannot enter alternate screen: {e}"),
            metadata: None,
        }
    })?;
    let mut terminal = Terminal::new(stderr_backend()).map_err(|e| {
        let _ = execute!(std::io::stderr(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        CliError::Usage {
            message: format!("Cannot initialise terminal: {e}"),
            metadata: None,
        }
    })?;
    // Force ratatui to sync its internal buffer to the actual terminal size
    // before the first draw. `clear()` alone doesn't trigger autoresize, so
    // the first frame would still compute layouts against the default buffer
    // size; the second frame would detect the real size and shift panel
    // widths — visible to the user as a one-shot width change on the first
    // key event. `autoresize()` queries the backend and resizes the buffer.
    let _ = terminal.autoresize();
    let _ = terminal.clear();
    Ok(terminal)
}

/// Leave the alternate screen, restore cooked mode, and show the cursor.
/// Idempotent — calling twice is a no-op aside from disable_raw_mode
/// returning an error the caller doesn't care about.
pub fn release(mut terminal: Tty) {
    let _ = terminal.show_cursor();
    let _ = execute!(std::io::stderr(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
}
