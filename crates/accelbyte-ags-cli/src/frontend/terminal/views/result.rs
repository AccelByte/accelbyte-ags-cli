//! Surface-neutral result-view helpers shared by the inline and fullscreen
//! surfaces.

use ags_protocol::output_views::WorkflowCompletionView;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Append the workflow completion summary (the `Created` section and the
/// `→ Next:` follow-ups) to `lines`. Both surfaces render this identically
/// below the result body, and it matches the after-exit stderr render, so the
/// formatting lives here to stay in lock-step.
pub(crate) fn push_completion_lines(lines: &mut Vec<Line>, view: &WorkflowCompletionView) {
    if !view.created.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Created",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for r in &view.created {
            // Colon, not an em dash (house style) — matches the after-exit render.
            lines.push(Line::raw(format!("  {}: {}", r.label, r.value)));
        }
    }
    if !view.next_steps.is_empty() {
        lines.push(Line::raw(""));
        for s in &view.next_steps {
            // Shared suggestion convention: `→ Next: <description>` with the
            // command dimmed underneath. No "Next steps" header — each line is
            // self-labelling, matching the after-exit stderr render and the
            // error/warning suggestion lines.
            lines.push(Line::raw(format!(
                "{} Next: {}",
                crate::frontend::style::text::SYMBOL_FIX,
                s.description
            )));
            lines.push(Line::from(Span::styled(
                format!("    {}", s.command),
                Style::default().fg(Color::Indexed(244)),
            )));
        }
    }
}
