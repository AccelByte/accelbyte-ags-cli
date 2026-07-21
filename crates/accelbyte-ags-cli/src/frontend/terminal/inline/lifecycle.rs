//! Terminal acquisition and teardown for the inline TUI.

use crossterm::terminal::disable_raw_mode;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::errors::CliError;
pub use crate::frontend::terminal::backend::Tty;
use crate::frontend::terminal::backend::{enter_raw_mode, stderr_backend};

/// Preferred inline viewport height. Sized to fit the parity chrome: header box
/// (5–7) + `Parameters` box + nav box (5). Bumped from the v0 value of 25. The
/// actual height is capped to the terminal so the pinned submit/nav never fall
/// off the bottom — see [`viewport_height`].
pub const INLINE_HEIGHT: u16 = 30;

/// Floor for the capped viewport: never shrink below this even on a tiny
/// terminal, or the form chrome (fields + pinned submit + nav) has no room.
const MIN_INLINE_HEIGHT: u16 = 12;

/// The viewport height to install: the preferred [`INLINE_HEIGHT`], capped to
/// the terminal's rows (minus one row of headroom so the surrounding scrollback
/// isn't clipped) and floored at [`MIN_INLINE_HEIGHT`]. On a terminal shorter
/// than `INLINE_HEIGHT` this keeps the pinned submit button and nav bar on
/// screen rather than scrolling them past the bottom edge.
fn viewport_height(terminal_rows: u16) -> u16 {
    INLINE_HEIGHT
        .min(terminal_rows.saturating_sub(1))
        .max(MIN_INLINE_HEIGHT)
}

/// Enable raw mode and install an inline ratatui viewport on stderr.
/// The TUI is UI chrome — stdout stays reserved for `CommandOutput`,
/// so piping `ags … | jq` keeps result bytes clean even when the inline surface
/// is active on the terminal.
pub fn acquire() -> Result<Tty, CliError> {
    enter_raw_mode()?;
    // Cap the inline viewport to the terminal height so the bottom chrome (the
    // pinned submit button, the nav bar) stays visible on short terminals.
    let height = crossterm::terminal::size()
        .map(|(_, rows)| viewport_height(rows))
        .unwrap_or(INLINE_HEIGHT);
    let terminal = Terminal::with_options(
        stderr_backend(),
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )
    .map_err(|e| {
        let _ = disable_raw_mode();
        CliError::Usage {
            message: format!("Cannot initialise terminal: {e}"),
            metadata: None,
        }
    })?;
    Ok(terminal)
}

/// Restore cooked-mode state and clear the inline viewport.
pub fn release(mut terminal: Tty) {
    let _ = terminal.clear();
    let _ = terminal.show_cursor();
    let _ = disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::{viewport_height, INLINE_HEIGHT, MIN_INLINE_HEIGHT};

    #[test]
    fn test_viewport_height_uses_preferred_on_tall_terminal() {
        // Plenty of rows → full preferred height.
        assert_eq!(viewport_height(60), INLINE_HEIGHT);
        assert_eq!(viewport_height(INLINE_HEIGHT + 1), INLINE_HEIGHT);
    }

    #[test]
    fn test_viewport_height_caps_to_terminal_with_headroom() {
        // Shorter than preferred → cap to rows - 1 so the bottom chrome stays on
        // screen and a row of scrollback headroom remains.
        assert_eq!(viewport_height(24), 23);
        assert_eq!(viewport_height(20), 19);
    }

    #[test]
    fn test_viewport_height_floors_on_tiny_terminal() {
        // Never shrink below the floor, even on a very short terminal.
        assert_eq!(viewport_height(6), MIN_INLINE_HEIGHT);
        assert_eq!(viewport_height(0), MIN_INLINE_HEIGHT);
    }
}
