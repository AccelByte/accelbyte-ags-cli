//! Result phase: title + description + assembled result body. Final
//! state of a successful workflow run before the dismiss loop hands
//! control back. No future-labelled buttons.

use ags_runtime::support::strings::strip_terminal_control_sequences;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

use crate::frontend::terminal::fullscreen::header;

/// How the panel's title reads: a successful run is green, a cancelled run is
/// yellow (warning tone) — never green, which would imply success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Success,
    Cancelled,
}

pub struct ResultPanel {
    pub title: String,
    pub description: String,
    pub body: String,
    pub completion: Option<ags_protocol::output_views::WorkflowCompletionView>,
    pub status: ResultStatus,
}

impl ResultPanel {
    /// Render the panel. Returns the maximum useful scroll offset for the
    /// given area so callers can clamp their stored scroll state and avoid
    /// "phantom" scroll keystrokes that have to be undone.
    ///
    /// The result is run-level, not a step, so the fixed header slot holds a
    /// neutral run headline (coloured by status) and the output box sits in the
    /// content rect below — the same stable frame the step phases draw into.
    pub fn render(&self, frame: &mut Frame, area: Rect, scroll_offset: u16) -> u16 {
        let (slot, content) = header::split(area);
        self.render_headline(frame, slot);
        self.render_output(frame, content, scroll_offset)
    }

    /// Run headline in the fixed slot: the completion title, green on success
    /// and yellow on a cancelled run (never green, which would imply success),
    /// with the optional description below it.
    fn render_headline(&self, frame: &mut Frame, area: Rect) {
        let title_color = match self.status {
            ResultStatus::Success => Color::Green,
            ResultStatus::Cancelled => Color::Yellow,
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .padding(Padding::new(2, 2, 1, 1))
            .title(" Result ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let mut lines = vec![Line::from(Span::styled(
            self.title.clone(),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ))];
        if !self.description.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::raw(self.description.clone()));
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    /// The response body and completion resources in the content box below the
    /// slot. Returns the maximum useful scroll offset for this box.
    fn render_output(&self, frame: &mut Frame, area: Rect, scroll_offset: u16) -> u16 {
        let mut lines = Vec::new();
        for body_line in self.body.lines() {
            // The renderer (output/human/commands/service.rs) wraps Query /
            // Headers / Body content in the dim ANSI escape (`\x1b[2m…\x1b[0m`)
            // and leaves the endpoint line and step/workflow headers bright.
            // Detect the dim escape here and strip all CSI codes for display,
            // so the TUI mirrors stdout instead of guessing by indentation.
            let is_dim = body_line.contains("\x1b[2m");
            let stripped = strip_terminal_control_sequences(body_line);
            // For a workflow result (completion present), the output's section
            // headings are un-indented, un-dimmed lines. Give them the same cyan
            // bold as the Created header so the panel reads
            // uniformly. Gated on `completion` so a service-command body (whose
            // bright col-0 line is the endpoint) is left alone.
            let is_section_heading = self.completion.is_some()
                && !is_dim
                && !stripped.is_empty()
                && !stripped.starts_with(|c: char| c.is_whitespace());
            let style = if is_section_heading {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_dim {
                Style::default().fg(Color::Indexed(244))
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(stripped, style)));
        }
        if let Some(view) = &self.completion {
            crate::frontend::terminal::views::result::push_completion_lines(&mut lines, view);
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .padding(Padding::new(2, 2, 1, 1))
            .title(" Output ");
        let inner = block.inner(area);
        let content_lines = lines.len() as u16;
        let visible_lines = inner.height;
        let max_offset = content_lines.saturating_sub(visible_lines);
        let effective_scroll = scroll_offset.min(max_offset);
        let para = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0));
        frame.render_widget(para, area);
        max_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_panel_holds_title_description_body() {
        let p = ResultPanel {
            title: "Workflow complete".into(),
            description: "All steps succeeded.".into(),
            body: "{\"id\":\"abc\"}".into(),
            completion: None,
            status: ResultStatus::Success,
        };
        assert_eq!(p.title, "Workflow complete");
        assert!(p.body.contains("abc"));
    }

    #[test]
    fn test_cancelled_title_is_yellow_not_green() {
        use ratatui::{backend::TestBackend, Terminal};
        let p = ResultPanel {
            title: "my-workflow cancelled".into(),
            description: "Run cancelled.".into(),
            body: String::new(),
            completion: None,
            status: ResultStatus::Cancelled,
        };
        let mut term = Terminal::new(TestBackend::new(80, 10)).unwrap();
        term.draw(|f| {
            p.render(f, f.area(), 0);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        // Find the 'c' of "cancelled" in the title and check its colour.
        let cells = buf.content();
        let title_cell = cells
            .iter()
            .find(|c| c.symbol() == "m" && c.fg == Color::Yellow)
            .or_else(|| cells.iter().find(|c| c.fg == Color::Yellow));
        assert!(
            title_cell.is_some(),
            "cancelled title rendered in yellow, not green"
        );
        assert!(
            !cells
                .iter()
                .any(|c| c.symbol() == "c" && c.fg == Color::Green),
            "no part of the cancelled title is green"
        );
    }

    #[test]
    fn test_result_panel_renders_completion_created_and_next_steps() {
        use ratatui::{backend::TestBackend, Terminal};
        let p = ResultPanel {
            title: "Set up competitive multiplayer complete".into(),
            description: String::new(),
            body: String::new(),
            status: ResultStatus::Success,
            completion: Some(ags_protocol::output_views::WorkflowCompletionView {
                created: vec![ags_protocol::workflow::CompletionResource {
                    label: "Match pool".into(),
                    value: "ranked-pool".into(),
                }],
                next_steps: vec![ags_protocol::workflow::CompletionStep {
                    description: "Inspect the match pool".into(),
                    command: "ags matchmaking match-pools get --namespace dev --pool ranked-pool"
                        .into(),
                }],
            }),
        };
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        term.draw(|f| {
            p.render(f, f.area(), 0);
        })
        .unwrap();
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
