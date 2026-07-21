//! Inline-viewport progress state and redraw helper for the TUI.

use std::sync::{Arc, Mutex};

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::frontend::event::RunOutcome;
use crate::frontend::terminal::inline::lifecycle::Tty;

#[derive(Default, Clone)]
pub struct InlineProgressState {
    inner: Arc<Mutex<InlineProgressInner>>,
}

#[derive(Default)]
struct InlineProgressInner {
    latest_message: Option<String>,
    page: Option<(usize, Option<usize>)>,
    /// `None` while the run is in flight; `Some(_)` once `RunFinished` arrives.
    outcome: Option<RunOutcome>,
    /// For a workflow step in flight: the step's frame name (e.g.
    /// `Step 2: create-ruleset`), shown as the box title so it doesn't flip to
    /// "Running". `None` for single-command progress.
    step_label: Option<String>,
}

impl InlineProgressState {
    /// Replace the latest progress message and clear the terminal outcome.
    pub fn set_message(&self, message: String) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.latest_message = Some(message);
            guard.outcome = None;
        }
    }
    /// Record the current pagination position for display in the inline viewport.
    pub fn set_page(&self, current: usize, total: Option<usize>) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.page = Some((current, total));
        }
    }
    /// Set (or clear) the workflow step's frame name shown as the box title.
    pub fn set_step_label(&self, label: Option<String>) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.step_label = label;
        }
    }
    /// Mark the in-flight operation as complete with the given outcome.
    pub fn finish_with(&self, outcome: RunOutcome) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.outcome = Some(outcome);
        }
    }
    /// Mark the in-flight operation as successfully complete.
    pub fn finish(&self) {
        self.finish_with(RunOutcome::Success);
    }
    /// Copy the current state out for rendering without holding the lock.
    pub fn snapshot(&self) -> ProgressSnapshot {
        let guard = self.inner.lock().expect("progress state poisoned");
        ProgressSnapshot {
            latest_message: guard.latest_message.clone(),
            page: guard.page,
            outcome: guard.outcome,
            step_label: guard.step_label.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProgressSnapshot {
    pub latest_message: Option<String>,
    pub page: Option<(usize, Option<usize>)>,
    pub outcome: Option<RunOutcome>,
    pub step_label: Option<String>,
}

impl ProgressSnapshot {
    /// Title to show on the inline viewport block, derived from the current outcome.
    pub fn title(&self) -> &'static str {
        match self.outcome {
            None => "Running",
            Some(RunOutcome::Success) => "Done",
            Some(RunOutcome::Failed) => "Failed",
            Some(RunOutcome::Cancelled) => "Cancelled",
        }
    }
}

/// Paint the inline progress viewport using the current snapshot.
/// Body colouring is suppressed when `--no-color` / `NO_COLOR` is in effect.
pub fn redraw(terminal: &mut Tty, snapshot: &ProgressSnapshot) {
    let _ = terminal.draw(|f| {
        // A workflow step in flight keeps the step review's layout: render the
        // inline chrome (body box + Navigation box) and replace only the body
        // box's contents with the centred verb, mirroring the fullscreen running
        // panel. Single-command progress and terminal states (Done/Failed/…) use
        // the simple full-viewport status box.
        match (&snapshot.step_label, snapshot.outcome) {
            (Some(label), None) => render_running_step(f, label, snapshot),
            _ => render_status_box(f, snapshot),
        }
    });
}

/// Running display for a workflow step: the same chrome the step review uses
/// (a bordered box titled with the step's frame name, plus the Navigation box),
/// with only the box contents swapped for the centred verb. The Navigation box
/// is kept so the running frame doesn't wipe it.
fn render_running_step(f: &mut ratatui::Frame, label: &str, snapshot: &ProgressSnapshot) {
    use crate::frontend::terminal::inline::chrome;
    use crate::frontend::terminal::views::nav::NavContext;
    chrome::render(f, NavContext::Running, |f, area| {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {label} "));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(inner);
        let style = if crate::frontend::style::is_stderr_enabled() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let verb = snapshot.latest_message.clone().unwrap_or_default();
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(verb, style))).alignment(Alignment::Center),
            rows[1],
        );
    });
}

/// Simple full-viewport status box: single-command progress and terminal
/// states (Done / Failed / Cancelled). Shows the status word as the title, the
/// latest message (plus pagination) centred, and a `[Ctrl-C] cancel` footer.
fn render_status_box(f: &mut ratatui::Frame, snapshot: &ProgressSnapshot) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let title = snapshot.title().to_string();
    let mut text = snapshot.latest_message.clone().unwrap_or_default();
    if let Some((current, total)) = snapshot.page {
        let total_str = total.map(|t| t.to_string()).unwrap_or_else(|| "?".into());
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&format!("(page {current} of {total_str})"));
    }
    let body_style = if crate::frontend::style::is_stderr_enabled() {
        let colour = match snapshot.outcome {
            None => Color::Cyan,
            Some(RunOutcome::Success) => Color::Green,
            Some(RunOutcome::Failed) => Color::Red,
            Some(RunOutcome::Cancelled) => Color::Red,
        };
        Style::default().fg(colour)
    } else {
        Style::default()
    };
    let mut block = Block::default().borders(Borders::ALL);
    if !title.is_empty() {
        block = block.title(format!(" {title} "));
    }
    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);
    // Centre the message vertically and horizontally inside the box.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .split(inner);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(text, body_style))).alignment(Alignment::Center),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::raw("[Ctrl-C] cancel"))),
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_set_message_round_trips() {
        let state = InlineProgressState::default();
        state.set_message("hello".into());
        let snap = state.snapshot();
        assert_eq!(snap.latest_message.as_deref(), Some("hello"));
        assert_eq!(snap.outcome, None);
    }

    #[test]
    fn test_state_finish_records_success_outcome() {
        let state = InlineProgressState::default();
        state.set_message("doing".into());
        state.finish();
        let snap = state.snapshot();
        assert_eq!(snap.outcome, Some(RunOutcome::Success));
    }

    #[test]
    fn test_state_finish_with_records_explicit_outcome() {
        let state = InlineProgressState::default();
        state.finish_with(RunOutcome::Failed);
        let snap = state.snapshot();
        assert_eq!(snap.outcome, Some(RunOutcome::Failed));
        assert_eq!(snap.title(), "Failed");
    }

    #[test]
    fn test_state_set_message_after_finish_clears_outcome() {
        let state = InlineProgressState::default();
        state.finish();
        state.set_message("new work".into());
        let snap = state.snapshot();
        assert_eq!(snap.outcome, None);
    }

    #[test]
    fn test_state_set_page_records_pagination() {
        let state = InlineProgressState::default();
        state.set_page(2, Some(5));
        let snap = state.snapshot();
        assert_eq!(snap.page, Some((2, Some(5))));
    }

    #[test]
    fn test_snapshot_title_reflects_outcome() {
        let mut snap = ProgressSnapshot::default();
        assert_eq!(snap.title(), "Running");
        snap.outcome = Some(RunOutcome::Failed);
        assert_eq!(snap.title(), "Failed");
    }
}
