//! Result phase: scrollable read-only view of the rendered command output.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{Phase, PhaseStep};

pub struct ResultPhase {
    lines: Vec<String>,
    scroll: u16,
    completion: Option<ags_protocol::output_views::WorkflowCompletionView>,
}

impl ResultPhase {
    /// Construct a scrollable result viewer over the rendered text.
    pub fn new(rendered_text: &str) -> Self {
        Self {
            lines: rendered_text.lines().map(|l| l.to_string()).collect(),
            scroll: 0,
            completion: None,
        }
    }

    /// Attach a resolved workflow completion to render below the result text.
    pub fn with_completion(
        mut self,
        completion: Option<ags_protocol::output_views::WorkflowCompletionView>,
    ) -> Self {
        self.completion = completion;
        self
    }
}

impl Phase for ResultPhase {
    type Output = ();

    fn render(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        let mut body_lines: Vec<Line> = self
            .lines
            .iter()
            .map(|l| Line::from(Span::raw(l.clone())))
            .collect();
        if let Some(view) = &self.completion {
            crate::frontend::terminal::views::result::push_completion_lines(&mut body_lines, view);
        }
        let body = Paragraph::new(body_lines)
            .scroll((self.scroll, 0))
            .block(Block::default().borders(Borders::ALL).title("Result"));
        frame.render_widget(body, chunks[0]);

        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(
                "[↑/↓] scroll · [q] / [Enter] / [Esc] exit",
            ))),
            chunks[1],
        );
    }

    fn on_key(&mut self, key: KeyEvent) -> PhaseStep<()> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc => PhaseStep::Done(()),
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                PhaseStep::Continue
            }
            KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
                PhaseStep::Continue
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(10);
                PhaseStep::Continue
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(10);
                PhaseStep::Continue
            }
            _ => PhaseStep::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    /// Construct an unmodified key press for the given key code.
    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_q_exits() {
        let mut phase = ResultPhase::new("a\nb");
        assert!(matches!(
            phase.on_key(key_event(KeyCode::Char('q'))),
            PhaseStep::Done(())
        ));
    }

    #[test]
    fn test_enter_exits() {
        let mut phase = ResultPhase::new("a");
        assert!(matches!(
            phase.on_key(key_event(KeyCode::Enter)),
            PhaseStep::Done(())
        ));
    }

    #[test]
    fn test_esc_exits() {
        let mut phase = ResultPhase::new("a");
        assert!(matches!(
            phase.on_key(key_event(KeyCode::Esc)),
            PhaseStep::Done(())
        ));
    }

    #[test]
    fn test_down_increments_scroll() {
        let mut phase = ResultPhase::new("a\nb\nc");
        let _ = phase.on_key(key_event(KeyCode::Down));
        assert_eq!(phase.scroll, 1);
    }

    #[test]
    fn test_up_at_zero_saturates() {
        let mut phase = ResultPhase::new("a");
        let _ = phase.on_key(key_event(KeyCode::Up));
        assert_eq!(phase.scroll, 0);
    }

    #[test]
    fn test_page_down_jumps_ten() {
        let mut phase = ResultPhase::new("a");
        let _ = phase.on_key(key_event(KeyCode::PageDown));
        assert_eq!(phase.scroll, 10);
    }

    #[test]
    fn test_result_phase_renders_completion() {
        use ratatui::{backend::TestBackend, Terminal};
        let phase = ResultPhase::new("body").with_completion(Some(
            ags_protocol::output_views::WorkflowCompletionView {
                created: vec![ags_protocol::workflow::CompletionResource {
                    label: "Match pool".into(),
                    value: "ranked-pool".into(),
                }],
                next_steps: vec![ags_protocol::workflow::CompletionStep {
                    description: "Inspect the match pool".into(),
                    command: "ags matchmaking match-pools get --namespace dev --pool ranked-pool"
                        .into(),
                }],
            },
        ));
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        term.draw(|f| phase.render(f)).unwrap();
        let buf = term.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Created"), "Created heading: {content}");
        assert!(content.contains("ranked-pool"), "created value");
        // → Next: convention, no "Next steps" header (matches the after-exit render).
        assert!(
            !content.contains("Next steps"),
            "old header dropped: {content}"
        );
        assert!(
            content.contains("Next: Inspect the match pool"),
            "next-step convention label: {content}"
        );
        assert!(content.contains("match-pools get"), "command shown");
    }
}
