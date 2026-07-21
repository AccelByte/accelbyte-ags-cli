//! Error phase: message + context + suggested next step. Final state
//! of a failed workflow run — same line/context/next-step
//! structure as the plain error wording.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub struct ErrorPanel {
    pub message: String,
    pub context: Option<String>,
    pub suggestion: Option<String>,
}

impl ErrorPanel {
    /// Draw the error message, optional context, and suggested next step into `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", crate::frontend::style::text::SYMBOL_ERROR),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                self.message.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        if let Some(ctx) = &self.context {
            lines.push(Line::from(format!("  {ctx}")));
        }
        if let Some(hint) = &self.suggestion {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                format!("  {hint}"),
                Style::default().fg(Color::Indexed(244)),
            )));
        }
        let para = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Error"))
            .wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_panel_holds_message_context_and_suggestion() {
        let p = ErrorPanel {
            message: "Authentication failed".into(),
            context: Some("Identity provider rejected the request.".into()),
            suggestion: Some("Run `ags auth login` again.".into()),
        };
        assert_eq!(p.message, "Authentication failed");
        assert!(p.context.as_deref().unwrap().contains("rejected"));
        assert!(p.suggestion.as_deref().unwrap().contains("auth login"));
    }
}
