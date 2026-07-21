//! Fullscreen context/step header box (top region of the §13 layout).
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

/// Fixed height of the header slot at the top of the main region. Fixed rather
/// than content-derived so the content box below never shifts as the phase
/// advances Fields → Confirm → Running → Result. Sized for the tallest header
/// (heading + blank + description = 3 body rows, plus 4 rows of chrome).
pub(crate) const SLOT_HEIGHT: u16 = 7;

/// Split `area` into the fixed-height header slot (top) and the content rect
/// below it. Every per-step phase and the result phase draw through this so
/// the content frame stays put.
pub(crate) fn split(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(SLOT_HEIGHT), Constraint::Min(1)])
        .split(area);
    (chunks[0], chunks[1])
}

/// Render `header` into the fixed slot at the top of `area` and return the
/// content rect below it.
pub(crate) fn render_slot(frame: &mut Frame, area: Rect, header: &Header) -> Rect {
    let (slot, content) = split(area);
    render(frame, slot, header);
    content
}

#[derive(Debug, Clone)]
pub(crate) struct Header {
    /// Border title, e.g. `" Step 1 "` (fullscreen) or `" iam users create "`
    /// (inline single command — the command lives in the frame title).
    pub title: String,
    /// Optional bold cyan first line. `Some` for fullscreen's step name; `None`
    /// for the inline single-command header, where the command is already the
    /// frame title and only the `description` (operation summary) is shown.
    pub heading: Option<String>,
    /// Optional longer description. Shown under the heading when a heading is
    /// present; shown as the sole body line (white) when `heading` is `None`.
    pub description: Option<String>,
}

impl Header {
    /// Build a per-step header: title `" Step N "`, the step name as the cyan
    /// heading, and the (optional) longer description. An empty description is
    /// dropped so the heading stands alone.
    pub(crate) fn step(step_number: usize, name: &str, description: &str) -> Self {
        Self {
            title: format!(" Step {step_number} "),
            heading: Some(name.to_string()),
            description: if description.is_empty() {
                None
            } else {
                Some(description.to_string())
            },
        }
    }
}

/// Draw the bordered header box (title, heading, optional description) into `area`.
pub(crate) fn render(frame: &mut Frame, area: Rect, header: &Header) {
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1))
        .title(header.title.clone());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    match (&header.heading, &header.description) {
        (Some(heading), desc) => {
            lines.push(Line::from(Span::styled(
                heading.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            if let Some(desc) = desc {
                lines.push(Line::raw(""));
                lines.push(Line::from(Span::styled(
                    desc.clone(),
                    Style::default().fg(Color::White),
                )));
            }
        }
        // No heading: the command is the frame title; show the summary (if any)
        // as the sole white body line.
        (None, Some(desc)) => {
            lines.push(Line::from(Span::styled(
                desc.clone(),
                Style::default().fg(Color::White),
            )));
        }
        (None, None) => {}
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_step_constructor_drops_empty_description() {
        let with_desc = Header::step(2, "Publish", "Publishes the catalog");
        assert_eq!(with_desc.title, " Step 2 ");
        assert_eq!(with_desc.heading.as_deref(), Some("Publish"));
        assert_eq!(
            with_desc.description.as_deref(),
            Some("Publishes the catalog")
        );
        let no_desc = Header::step(1, "Define", "");
        assert!(no_desc.description.is_none(), "empty description dropped");
    }

    #[test]
    fn test_split_returns_content_below_fixed_slot() {
        // The content rect starts exactly SLOT_HEIGHT rows below the top and
        // fills the remaining height — independent of the header's content.
        let (slot, content) = split(Rect::new(0, 0, 40, 20));
        assert_eq!(slot.height, SLOT_HEIGHT);
        assert_eq!(content.y, SLOT_HEIGHT);
        assert_eq!(content.height, 20 - SLOT_HEIGHT);
    }

    #[test]
    fn test_header_renders_title_and_heading_and_description() {
        let h = Header {
            title: " Step 1 ".into(),
            heading: Some("Create user".into()),
            description: Some("Creates a new user account".into()),
        };
        let mut term = Terminal::new(TestBackend::new(60, 7)).unwrap();
        term.draw(|f| render(f, f.area(), &h)).unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("Create user"), "heading present: {s}");
        assert!(
            s.contains("Creates a new user account"),
            "description present: {s}"
        );
    }

    #[test]
    fn test_header_without_heading_shows_command_in_title_and_summary_as_body() {
        // Inline single-command header: command only in the frame title, summary
        // as the sole body line — the command must NOT be repeated as a heading.
        let h = Header {
            title: " iam users create ".into(),
            heading: None,
            description: Some("Creates a new user account".into()),
        };
        let mut term = Terminal::new(TestBackend::new(60, 5)).unwrap();
        term.draw(|f| render(f, f.area(), &h)).unwrap();
        let rows: Vec<String> = {
            let buf = term.backend().buffer();
            (0..buf.area.height)
                .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
                .collect()
        };
        // Title is on the top border row; summary is on an interior row.
        assert!(
            rows[0].contains("iam users create"),
            "command in title: {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("Creates a new user account")),
            "summary in body: {rows:?}"
        );
        // The command appears exactly once (title only), not duplicated as a body heading.
        let occurrences = rows
            .iter()
            .filter(|r| r.contains("iam users create"))
            .count();
        assert_eq!(occurrences, 1, "command not duplicated: {rows:?}");
    }
}
