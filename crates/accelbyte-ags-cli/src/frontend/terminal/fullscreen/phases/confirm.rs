//! Confirm phase: the confirm card embedded in the main region.
//!
//! Wraps the shared [`ConfirmCardPhase`] and draws it via its area-aware
//! `render_in`, so only the offered actions appear (gather: Confirm / Back /
//! Cancel; per-step: Confirm / Cancel). The interaction drives the wrapped
//! phase's `on_key` against this panel (driven by `FullscreenInteraction`).

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::frontend::terminal::fullscreen::header::{self, Header};
use crate::frontend::terminal::inline::phases::confirm_card::ConfirmCardPhase;

pub struct ConfirmPanel {
    /// Step header drawn in the shared fixed slot above the card, so the card
    /// box lines up with the Running verb box and the Result box.
    pub(crate) header: Header,
    pub card_phase: ConfirmCardPhase,
}

impl ConfirmPanel {
    /// Draw the step header in the shared slot and the confirmation card in the
    /// content rect below it.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let content = header::render_slot(frame, area, &self.header);
        self.card_phase.render_in(frame, content);
    }

    /// Mutable access to the wrapped phase for the interaction's key loop.
    pub fn phase_mut(&mut self) -> &mut ConfirmCardPhase {
        &mut self.card_phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::phases::confirm_card::{ConfirmAction, ConfirmCard};
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_render_into_test_backend_does_not_panic() {
        let panel = ConfirmPanel {
            header: Header::step(1, "Create user", ""),
            card_phase: ConfirmCardPhase::new(ConfirmCard::new(
                "POST /iam/v3/x",
                vec![("namespace".into(), "test-ns".into())],
            ))
            .with_actions(&[ConfirmAction::Confirm, ConfirmAction::Cancel]),
        };
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_snapshot_confirm_panel_layout() {
        let panel = ConfirmPanel {
            header: Header::step(1, "Create user", ""),
            card_phase: ConfirmCardPhase::new(ConfirmCard::new(
                "POST /iam/v3/admin/namespaces/{namespace}/users",
                vec![
                    ("namespace".into(), "dev".into()),
                    ("email".into(), "a@b.com".into()),
                ],
            )),
        };
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
        let buf = term.backend().buffer();
        let text = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::with_settings!({snapshot_path => "../../../../../tests/snapshot/snapshots"}, {
            insta::assert_snapshot!(text);
        });
    }
}
