//! Per-phase widget for the fullscreen main area.

pub mod briefing;
pub mod confirm;
pub mod enum_picker;
pub mod error;
pub mod fields;
pub mod json_edit;
pub mod result;
pub mod running;

use ratatui::layout::Rect;
use ratatui::Frame;

/// Which widget the main area is currently rendering. The fullscreen
/// surface morphs the main area between these states as the
/// workflow advances; the surrounding step strip / summary / nav bar
/// stay rendered around them.
pub enum Phase {
    Briefing(briefing::BriefingPanel),
    Fields(fields::FieldsPanel),
    Confirm(confirm::ConfirmPanel),
    JsonEdit(json_edit::JsonEditPanel),
    Running(running::RunningPanel),
    Result(result::ResultPanel),
    // Not yet wired in production; kept as the planned in-surface error-display
    // path for the fullscreen surface.
    #[allow(dead_code)]
    Error(error::ErrorPanel),
}

impl Phase {
    /// Render the active phase. Returns the maximum useful Result-panel
    /// scroll offset (0 for non-Result phases) so callers can clamp stored
    /// scroll state to what the rendered content actually permits.
    pub fn render_scrolled(&self, frame: &mut Frame, area: Rect, result_scroll: u16) -> u16 {
        match self {
            Phase::Briefing(p) => p.render(frame, area, result_scroll),
            Phase::Fields(p) => {
                p.render(frame, area);
                0
            }
            Phase::Confirm(p) => {
                p.render(frame, area);
                0
            }
            Phase::JsonEdit(p) => {
                p.render(frame, area);
                0
            }
            Phase::Running(p) => {
                p.render(frame, area);
                0
            }
            Phase::Result(p) => p.render(frame, area, result_scroll),
            Phase::Error(p) => {
                p.render(frame, area);
                0
            }
        }
    }
}
