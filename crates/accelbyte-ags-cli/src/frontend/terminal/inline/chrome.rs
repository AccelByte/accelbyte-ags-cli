//! Inline composition: split the frame into main + nav and render a phase body
//! into the main region. Inline-only (fullscreen uses its own regions).
//!
//! The inline chrome is clamped to [`MAX_WIDTH`] columns and left-aligned so
//! it does not sprawl the full terminal width and the host scrollbar stays
//! visible.
use crate::frontend::terminal::views::nav::{self, NavContext};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

/// Max width for the inline chrome so it does not sprawl full-screen and the
/// scrollbar stays visible. Left-aligned within the viewport. Wide enough for the
/// full Fields nav keymap plus the `[o] show optional (+N)` toggle.
const MAX_WIDTH: u16 = 120;

/// Split the frame into a width-clamped main region and nav bar, then render
/// `body` into the main region.
pub(crate) fn render(frame: &mut Frame, nav_ctx: NavContext, body: impl FnOnce(&mut Frame, Rect)) {
    render_with_nav_suffix(frame, nav_ctx, Vec::new(), body);
}

/// Like [`render`] but appends `nav_suffix` spans to the nav keymap — used by the
/// inline form to show the `[o] show optional (+N)` toggle in the nav bar.
pub(crate) fn render_with_nav_suffix(
    frame: &mut Frame,
    nav_ctx: NavContext,
    nav_suffix: Vec<ratatui::text::Span<'static>>,
    body: impl FnOnce(&mut Frame, Rect),
) {
    let full = frame.area();
    let width = full.width.min(MAX_WIDTH);
    let area = Rect {
        x: full.x,
        y: full.y,
        width,
        height: full.height,
    };
    let nav_h = crate::frontend::terminal::fullscreen::layout::NAV_HEIGHT;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(nav_h)])
        .split(area);
    body(frame, chunks[0]);
    nav::render_with_suffix(frame, chunks[1], nav_ctx, nav_suffix);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::widgets::{Block, Borders};
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_chrome_renders_main_and_nav() {
        let mut term = Terminal::new(TestBackend::new(80, 30)).unwrap();
        term.draw(|f| {
            render(f, NavContext::Fields, |frame, area| {
                frame.render_widget(
                    Block::default().borders(Borders::ALL).title(" Parameters "),
                    area,
                );
            })
        })
        .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("Parameters"), "main body rendered");
        assert!(s.contains("Navigation"), "nav box rendered");
        assert!(s.contains("[Tab] move"), "fields keymap rendered");
    }
}
