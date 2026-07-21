//! Surface-neutral contextual nav bar (the bordered `Navigation` box).
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // ConfirmCard used by the inline surface in a subsequent task
pub(crate) enum NavContext {
    Briefing,
    Fields,
    /// Fields phase for an optional step — adds `[s] skip` to the keymap.
    FieldsSkippable,
    Confirm,
    /// Confirm phase for an optional step — adds `[s] skip` to the keymap.
    ConfirmSkippable,
    /// Interactive failure gate: retry / cancel.
    StepFailure,
    /// Interactive failure gate for a safely-skippable step: retry / skip / cancel.
    StepFailureSkippable,
    ConfirmCard,
    JsonEditTree,
    JsonEditRaw,
    JsonEditScalar,
    Running,
    Result,
    Loading,
    /// The inline dynamic-enum picker list: move / select / cancel, type to filter.
    Picker,
}

/// Draw the navigation bar for `ctx`'s keymap into `area`.
pub(crate) fn render(frame: &mut Frame, area: Rect, ctx: NavContext) {
    render_spans(frame, area, spans(ctx));
}

/// Like [`render`] but appends `suffix` spans after the context keymap — used by
/// the inline single-command form to show the `[o] show optional (+N)` toggle
/// inline in the nav bar. Pass an empty `suffix` for the plain keymap.
pub(crate) fn render_with_suffix(
    frame: &mut Frame,
    area: Rect,
    ctx: NavContext,
    suffix: Vec<Span<'static>>,
) {
    let mut all = spans(ctx);
    all.extend(suffix);
    render_spans(frame, area, all);
}

/// Build the optional-row toggle affordance shown in the Fields nav bar:
/// ` · [o] show optional (+N)` (collapsed) or ` · [o] hide optional` (expanded).
/// Empty when collapsed with nothing hidden.
pub(crate) fn optional_toggle_suffix(count: usize, show_optional: bool) -> Vec<Span<'static>> {
    let dim = Style::default().fg(Color::Indexed(244));
    let key = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let action = Style::default().fg(Color::White);
    let label = if show_optional {
        " hide optional".to_string()
    } else if count > 0 {
        format!(" show optional (+{count})")
    } else {
        return Vec::new();
    };
    vec![
        Span::styled(" \u{00B7} ", dim),
        Span::styled("[o]", key),
        Span::styled(label, action),
    ]
}

/// Draw the bordered "Navigation" box with `spans` as its keymap line.
fn render_spans(frame: &mut Frame, area: Rect, spans: Vec<Span<'static>>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1))
        .title(" Navigation ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

/// Build the styled keymap spans for a nav context (the per-phase key legend).
fn spans(ctx: NavContext) -> Vec<Span<'static>> {
    let action = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::Indexed(244));
    let key = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let sep = || Span::styled(" \u{00B7} ", dim);
    let k = |s: &'static str| Span::styled(format!("[{s}]"), key);
    let a = |s: &'static str| Span::styled(s, action);
    match ctx {
        NavContext::Briefing => vec![
            k("Enter"),
            a(" continue"),
            sep(),
            k("\u{2191}\u{2193}"),
            a(" scroll"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::Fields => vec![
            k("Tab"),
            a(" move"),
            sep(),
            k("Enter"),
            a(" edit/cycle"),
            sep(),
            k("Space"),
            a(" toggle/cycle"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::FieldsSkippable => vec![
            k("Tab"),
            a(" move"),
            sep(),
            k("Enter"),
            a(" edit/cycle"),
            sep(),
            k("\u{2190}/\u{2192}"),
            a(" select"),
            sep(),
            k("s"),
            a(" skip"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::Confirm => vec![k("Enter"), a(" confirm"), sep(), k("Esc"), a(" cancel")],
        NavContext::ConfirmSkippable => vec![
            k("\u{2190}/\u{2192}"),
            a(" select"),
            sep(),
            k("Enter"),
            a(" confirm"),
            sep(),
            k("s"),
            a(" skip"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::StepFailure => vec![k("Enter"), a(" retry"), sep(), k("Esc"), a(" cancel")],
        NavContext::StepFailureSkippable => vec![
            k("\u{2190}/\u{2192}"),
            a(" select"),
            sep(),
            k("Enter"),
            a(" retry"),
            sep(),
            k("s"),
            a(" skip"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::ConfirmCard => vec![
            k("Enter"),
            a(" confirm"),
            sep(),
            k("b"),
            a(" back to edit"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::JsonEditScalar => vec![
            a("Type a value"),
            sep(),
            k("Enter"),
            a(" save"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::JsonEditRaw => vec![
            k("\u{2191}\u{2193}\u{2190}\u{2192}"),
            a(" move"),
            sep(),
            k("Ctrl-R"),
            a(" tree view"),
            sep(),
            k("Ctrl-S"),
            a(" save"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::JsonEditTree => vec![
            k("\u{2191}\u{2193}"),
            a(" move"),
            sep(),
            k("\u{2192}"),
            a(" expand"),
            sep(),
            k("Enter"),
            a(" edit"),
            sep(),
            k("+/\u{2212}"),
            a(" add/remove entry"),
            sep(),
            k("Ctrl-R"),
            a(" raw view"),
            sep(),
            k("Ctrl-S"),
            a(" save"),
            sep(),
            k("Esc"),
            a(" cancel"),
        ],
        NavContext::Running => vec![a("working\u{2026}")],
        NavContext::Result => vec![
            k("q"),
            a(" / "),
            k("Enter"),
            a(" exit"),
            sep(),
            k("\u{2191}\u{2193}"),
            a(" scroll result"),
            sep(),
            k("PgUp/Dn"),
            a(" scroll summary"),
        ],
        NavContext::Loading => vec![
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel", Style::default().fg(Color::White)),
        ],
        NavContext::Picker => vec![
            k("\u{2191}\u{2193}"),
            a(" move"),
            sep(),
            k("Enter"),
            a(" select"),
            sep(),
            k("Esc"),
            a(" cancel"),
            sep(),
            a("type to filter"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_optional_toggle_suffix_collapsed_shows_count() {
        let spans = optional_toggle_suffix(3, false);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert!(text.contains("[o]"), "key shown: {text}");
        assert!(text.contains("show optional (+3)"), "count shown: {text}");
    }

    #[test]
    fn test_optional_toggle_suffix_expanded_shows_hide() {
        let spans = optional_toggle_suffix(3, true);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert!(text.contains("hide optional"), "hide shown: {text}");
    }

    #[test]
    fn test_optional_toggle_suffix_empty_when_collapsed_and_none_hidden() {
        assert!(optional_toggle_suffix(0, false).is_empty());
    }

    #[test]
    fn test_fields_nav_renders_bracketed_tab_move() {
        let mut term = Terminal::new(TestBackend::new(80, 5)).unwrap();
        term.draw(|f| render(f, f.area(), NavContext::Fields))
            .unwrap();
        let content: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("[Tab] move"), "got: {content}");
    }
}
