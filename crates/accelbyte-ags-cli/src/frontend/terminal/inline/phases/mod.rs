//! Phase abstraction for the TUI's sequential-replace state machine.

pub mod confirm_card;
pub mod form;
pub mod result;

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use crate::errors::CliError;
use crate::frontend::terminal::inline::lifecycle::Tty;

/// The outcome of a key event applied to a phase.
pub enum PhaseStep<O> {
    /// Phase wants the runloop to re-render and keep going.
    Continue,
    /// Phase has produced its output and wants to transition.
    Done(O),
    /// User cancelled this phase.
    Cancelled,
}

/// A phase owns its own state and renders into the inline viewport.
pub trait Phase {
    /// The value this phase produces on `Done`.
    type Output;

    /// Render the current state into the inline viewport.
    fn render(&self, frame: &mut Frame<'_>);

    /// Apply a key event, returning the resulting step.
    fn on_key(&mut self, key: KeyEvent) -> PhaseStep<Self::Output>;
}

/// Run a phase to completion, polling crossterm for key events and rendering
/// between events. Returns `Ok(Some(output))` on success, `Ok(None)` on cancel,
/// `Err(...)` on draw failure.
pub fn run<P: Phase>(terminal: &mut Tty, mut phase: P) -> Result<Option<P::Output>, CliError> {
    loop {
        terminal
            .draw(|f| phase.render(f))
            .map_err(|e| CliError::Usage {
                message: format!("TUI draw failed: {e}"),
                metadata: None,
            })?;
        // Poll with a short timeout so a future progress phase can tick.
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    // Raw mode swallows SIGINT; intercept Ctrl-C explicitly so
                    // it behaves like the rest of the CLI.
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        return Ok(None);
                    }
                    match phase.on_key(key) {
                        PhaseStep::Continue => continue,
                        PhaseStep::Done(output) => return Ok(Some(output)),
                        PhaseStep::Cancelled => return Ok(None),
                    }
                }
                Ok(_) => continue,
                Err(_) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phase that completes on Enter, cancels on Esc, otherwise continues.
    struct CountingPhase {
        keys_seen: u32,
    }
    impl Phase for CountingPhase {
        type Output = u32;
        fn render(&self, _frame: &mut Frame<'_>) {}
        fn on_key(&mut self, key: KeyEvent) -> PhaseStep<u32> {
            self.keys_seen += 1;
            match key.code {
                KeyCode::Enter => PhaseStep::Done(self.keys_seen),
                KeyCode::Esc => PhaseStep::Cancelled,
                _ => PhaseStep::Continue,
            }
        }
    }

    #[test]
    fn test_phase_done_returns_output() {
        let mut phase = CountingPhase { keys_seen: 0 };
        let step = phase.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match step {
            PhaseStep::Done(n) => assert_eq!(n, 1),
            _ => panic!("expected Done"),
        }
    }

    #[test]
    fn test_phase_esc_cancels() {
        let mut phase = CountingPhase { keys_seen: 0 };
        let step = phase.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(step, PhaseStep::Cancelled));
    }

    #[test]
    fn test_phase_other_key_continues() {
        let mut phase = CountingPhase { keys_seen: 0 };
        let step = phase.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(matches!(step, PhaseStep::Continue));
        assert_eq!(phase.keys_seen, 1);
    }
}
