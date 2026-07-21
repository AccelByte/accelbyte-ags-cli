//! Summary panel — right side of the fullscreen layout.
//!
//! Per-step entry with leading coloured glyph + indented key/value
//! captures + optional error line. Title and capture text stay in the
//! default foreground; only the glyph carries colour.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::step_strip::StepState;

#[derive(Debug, Clone)]
pub struct SummaryEntry {
    pub title: String,
    pub state: StepState,
    pub captures: Vec<(String, String)>,
    pub error_line: Option<String>,
}

/// Draw the run-summary panel into `area`: an optional cleanup warning followed
/// by one scrollable entry per completed step (header, captures, error line).
pub fn render(
    frame: &mut Frame,
    area: Rect,
    entries: &[SummaryEntry],
    cleanup_required: bool,
    scroll_offset: u16,
) -> u16 {
    let mut lines = Vec::new();
    if cleanup_required {
        lines.push(Line::from(Span::styled(
            "Cleanup required",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "Earlier steps completed and were not undone. Review the captures below.",
            Style::default().fg(Color::Indexed(244)),
        )));
        lines.push(Line::raw(""));
    }
    if entries.is_empty() && !cleanup_required {
        lines.push(Line::from(Span::styled(
            "No completed steps yet.",
            Style::default().fg(Color::Indexed(244)),
        )));
    }
    for (i, entry) in entries.iter().enumerate() {
        // One blank line between entries so steps don't run together.
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(entry_header(entry));
        for (k, v) in &entry.captures {
            // Dim the label so the value carries the eye — same intent as
            // the dimmed `Query:` / `Headers:` prefixes in the Result panel.
            lines.push(Line::from(vec![
                Span::styled(
                    format!("    {k}: "),
                    Style::default().fg(Color::Indexed(244)),
                ),
                Span::raw(v.clone()),
            ]));
        }
        if let Some(err) = &entry.error_line {
            lines.push(Line::from(Span::styled(
                format!("    {err}"),
                Style::default().fg(Color::Red),
            )));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1))
        .title(" Summary ");
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

/// Build the header line for one summary entry: a state glyph plus the step title.
fn entry_header(entry: &SummaryEntry) -> Line<'static> {
    let (glyph, glyph_style) = match entry.state {
        StepState::Complete => ("\u{2714}", Style::default().fg(Color::Green)),
        StepState::Failed => (
            crate::frontend::style::text::SYMBOL_ERROR,
            Style::default().fg(Color::Red),
        ),
        StepState::Skipped => (
            crate::frontend::style::text::SYMBOL_SKIPPED,
            Style::default().fg(Color::Indexed(244)),
        ),
        StepState::Current | StepState::Pending => {
            ("\u{25CF}", Style::default().fg(Color::Indexed(244)))
        }
    };
    Line::from(vec![
        Span::styled(glyph, glyph_style),
        Span::raw(" "),
        // Default text colour — only the glyph carries colour.
        Span::raw(entry.title.clone()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summary_empty_shows_placeholder() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
        term.draw(|f| {
            let _ = render(f, f.area(), &[], false, 0);
        })
        .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains("No completed steps yet."),
            "empty placeholder shown: {buf}"
        );
    }

    #[test]
    fn test_entry_header_uses_default_text_colour_for_title() {
        let e = SummaryEntry {
            title: "Define Skill Stat".into(),
            state: StepState::Complete,
            captures: vec![],
            error_line: None,
        };
        let line = entry_header(&e);
        assert_eq!(line.spans[2].content, "Define Skill Stat");
        assert_eq!(line.spans[2].style.fg, None);
    }

    #[test]
    fn test_entry_header_uses_green_check_for_complete() {
        let e = SummaryEntry {
            title: "t".into(),
            state: StepState::Complete,
            captures: vec![],
            error_line: None,
        };
        let line = entry_header(&e);
        assert_eq!(line.spans[0].content, "\u{2714}");
        assert_eq!(line.spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn test_entry_header_uses_red_cross_for_failed() {
        let e = SummaryEntry {
            title: "t".into(),
            state: StepState::Failed,
            captures: vec![],
            error_line: None,
        };
        let line = entry_header(&e);
        assert_eq!(
            line.spans[0].content,
            crate::frontend::style::text::SYMBOL_ERROR
        );
        assert_eq!(line.spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn test_entry_header_uses_em_dash_for_skipped() {
        let e = SummaryEntry {
            title: "t".into(),
            state: StepState::Skipped,
            captures: vec![],
            error_line: None,
        };
        assert_eq!(entry_header(&e).spans[0].content, "\u{2014}");
    }
}
