//! Running phase: while a step dispatches, keep the §13 layout intact — the step
//! box (number / name / description) on top, and a bordered panel below whose
//! contents are replaced by the running verb (centred). This mirrors the Fields
//! phase's two-box layout so the screen doesn't collapse to bare text on
//! Continue. (No animated spinner and no elapsed reading — the step dispatch
//! blocks the render loop, so neither would update; see the dynamic-enum picker
//! for the spawn-and-tick pattern a live one would need.)

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::frontend::terminal::fullscreen::header::{self, Header};

pub struct RunningPanel {
    /// 1-based step number for the `Step N` box title. `0` means no step is in
    /// flight yet (initial/placeholder) — only the verb panel is drawn.
    pub step_number: usize,
    pub step_title: String,
    pub description: String,
    pub verb: String,
}

impl RunningPanel {
    /// Draw the step box and the centred running-verb panel into `area`,
    /// preserving the §13 two-box layout while a step dispatches.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        // Mirror the Fields layout: step box in the shared fixed slot, verb
        // panel in the content rect below, so the frame holds across phases.
        // step 0 is the pre-step placeholder ("Starting"/"Working") — it fills
        // the slot with a neutral empty header so the verb box still lines up
        // with the Confirm and Result boxes on either side.
        let header = if self.step_number == 0 {
            Header {
                title: String::new(),
                heading: None,
                description: None,
            }
        } else {
            Header::step(self.step_number, &self.step_title, &self.description)
        };
        let content = header::render_slot(frame, area, &header);
        self.render_verb_panel(frame, content);
    }

    /// The lower panel: the same bordered ` Parameters ` box the Fields phase
    /// draws, with its contents replaced by the running verb, centred.
    fn render_verb_panel(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title(" Parameters ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("{}...", self.verb),
                Style::default().fg(Color::Cyan),
            )))
            .alignment(Alignment::Center),
            rows[1],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_running_panel_holds_step_title_and_verb() {
        let p = RunningPanel {
            step_number: 1,
            step_title: "Define Skill Stat".into(),
            description: "Posts the stat definition.".into(),
            verb: "Sending request".into(),
        };
        assert_eq!(p.step_title, "Define Skill Stat");
        assert_eq!(p.verb, "Sending request");
    }

    #[test]
    fn test_centred_running_renders_without_panic() {
        use ratatui::{backend::TestBackend, Terminal};
        let p = RunningPanel {
            step_number: 2,
            step_title: "Define".into(),
            description: "Posting".into(),
            verb: "Sending".into(),
        };
        let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_step_zero_renders_only_verb_panel_without_panic() {
        use ratatui::{backend::TestBackend, Terminal};
        let p = RunningPanel {
            step_number: 0,
            step_title: String::new(),
            description: String::new(),
            verb: "Starting".into(),
        };
        let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
        term.draw(|f| p.render(f, f.area())).unwrap();
    }
}
