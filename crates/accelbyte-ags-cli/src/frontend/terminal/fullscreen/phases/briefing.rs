//! Briefing phase: long-form welcome screen shown before gather-inputs
//! when the workflow declares a briefing.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::Frame;

use crate::frontend::terminal::views::inline_format::{classify, parse, InlineSpan};
use ags_protocol::workflow::WorkflowBriefing;

pub struct BriefingPanel {
    title: String,
    briefing: WorkflowBriefing,
}

impl BriefingPanel {
    /// Build a panel from a briefing + workflow display name. The briefing
    /// text is kept as source data; wrapping happens at render time so
    /// continuation lines can hang-indent under their bullet glyphs to the
    /// actual terminal width.
    pub fn new(briefing: &WorkflowBriefing, workflow_name: &str) -> Self {
        Self {
            title: workflow_name.to_string(),
            briefing: briefing.clone(),
        }
    }

    /// Render the panel into `area` scrolled by `scroll_offset` rows.
    /// Returns the maximum useful scroll offset so the surface clamps the
    /// stored scroll state — same contract as `ResultPanel::render`.
    pub fn render(&self, frame: &mut Frame, area: Rect, scroll_offset: u16) -> u16 {
        let block = Block::default()
            .borders(Borders::ALL)
            .padding(Padding::new(2, 2, 1, 1))
            .title(" Briefing ");
        let inner = block.inner(area);
        let lines = self.build_lines(inner.width as usize);
        let content_lines = lines.len() as u16;
        let visible_lines = inner.height;
        let max_offset = content_lines.saturating_sub(visible_lines);
        let effective = scroll_offset.min(max_offset);
        let para = Paragraph::new(lines)
            .block(block)
            // No `.wrap(...)`: we pre-wrap with hanging indents so ratatui's
            // built-in wrap would only undo our column-0 vs column-4 split.
            .scroll((effective, 0));
        frame.render_widget(para, area);
        max_offset
    }

    /// Build the displayable line list at a given inner width. Wraps long
    /// paragraphs and bullets manually so continuation lines align under
    /// their leading text (bullets get a 4-column hanging indent; overview
    /// paragraphs wrap to column 0).
    fn build_lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Title (panel content area, distinct from the block's " Briefing " title).
        lines.push(Line::from(Span::styled(
            self.title.clone(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::raw(""));

        for (i, paragraph) in self.briefing.overview.split("\n\n").enumerate() {
            if i > 0 {
                lines.push(Line::raw(""));
            }
            lines.extend(wrap_paragraph(paragraph, "", "", width));
        }

        if !self.briefing.prerequisites.is_empty() {
            lines.push(Line::raw(""));
            lines.push(section_heading("Prerequisites"));
            lines.push(Line::raw(""));
            for bullet in &self.briefing.prerequisites {
                lines.extend(wrap_paragraph(bullet, "  \u{2022} ", "    ", width));
            }
        }

        if !self.briefing.creates.is_empty() {
            lines.push(Line::raw(""));
            lines.push(section_heading("This workflow creates"));
            lines.push(Line::raw(""));
            for bullet in &self.briefing.creates {
                lines.extend(wrap_paragraph(bullet, "  \u{2022} ", "    ", width));
            }
        }

        // Get Started button — single focusable action, always focused.
        // Same black-on-cyan focused styling as the form's Submit button
        // (`Form::render_button`), so the affordance reads as "the focused
        // button" the way every other phase's primary action does.
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "[ Get Started \u{2192} ]",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));

        lines
    }

    /// For test/state assertions.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn title(&self) -> &str {
        &self.title
    }
}

/// Build a styled section-heading line for the briefing body.
fn section_heading(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Word-wrap an inline-formatted paragraph into a sequence of styled
/// `Line`s, applying `first_prefix` to the first line and `hang_prefix`
/// to continuations. Wrap width includes the prefix; words longer than
/// the remaining budget force a soft break.
///
/// A "word" here is a run of non-whitespace characters that may span
/// multiple styled fragments (e.g. `**MMR**)` is one word with two
/// styles — bold "MMR" + plain ")"). The word boundary is the absence
/// of whitespace, not the span boundary, so adjacent styled spans glue
/// together without an inserted space.
fn wrap_paragraph(
    text: &str,
    first_prefix: &'static str,
    hang_prefix: &'static str,
    width: usize,
) -> Vec<Line<'static>> {
    let words = tokenise_styled_words(parse(text));

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut is_first_line = true;

    let prefix_width = |first: bool| -> usize {
        if first {
            display_width(first_prefix)
        } else {
            display_width(hang_prefix)
        }
    };

    let push_prefix = |spans: &mut Vec<Span<'static>>, first: bool| {
        let prefix = if first { first_prefix } else { hang_prefix };
        if !prefix.is_empty() {
            spans.push(Span::raw(prefix));
        }
    };

    push_prefix(&mut current, is_first_line);
    let mut current_width = prefix_width(is_first_line);

    for word in words {
        let word_width: usize = word.iter().map(|(t, _)| display_width(t)).sum();
        let on_prefix = current_width == prefix_width(is_first_line);
        // If current line already has content beyond the prefix, account
        // for the separating space between words.
        let need = word_width + if on_prefix { 0 } else { 1 };
        if current_width + need > width && !on_prefix {
            lines.push(Line::from(std::mem::take(&mut current)));
            is_first_line = false;
            push_prefix(&mut current, false);
            current_width = prefix_width(false);
        }
        if current_width > prefix_width(is_first_line) {
            current.push(Span::raw(" "));
            current_width += 1;
        }
        for (text, style) in word {
            current.push(Span::styled(text, style));
        }
        current_width += word_width;
    }

    if !current.is_empty() {
        lines.push(Line::from(current));
    }

    // Empty paragraphs still produce a blank anchor line so vertical
    // spacing stays predictable.
    if lines.is_empty() {
        lines.push(Line::raw(""));
    }

    lines
}

/// Walk the parsed inline spans and group their characters into "words"
/// (whitespace-delimited runs). A word is a `Vec<(text, style)>` because
/// a single word can carry multiple styles when the source has adjacent
/// styled spans without intervening whitespace (e.g. `**MMR**)`).
fn tokenise_styled_words(spans: Vec<InlineSpan>) -> Vec<Vec<(String, Style)>> {
    let mut words: Vec<Vec<(String, Style)>> = Vec::new();
    let mut current_word: Vec<(String, Style)> = Vec::new();
    let mut fragment = String::new();

    for span in spans {
        let (raw, style) = classify(span);
        for ch in raw.chars() {
            if ch.is_whitespace() {
                if !fragment.is_empty() {
                    current_word.push((std::mem::take(&mut fragment), style));
                }
                if !current_word.is_empty() {
                    words.push(std::mem::take(&mut current_word));
                }
            } else {
                fragment.push(ch);
            }
        }
        // End of span — flush any in-progress fragment into the current
        // word, but DO NOT close the word: the next span (if adjacent
        // without whitespace) continues it.
        if !fragment.is_empty() {
            current_word.push((std::mem::take(&mut fragment), style));
        }
    }
    if !current_word.is_empty() {
        words.push(current_word);
    }
    words
}

/// Display width in terminal columns. Briefings are mostly ASCII; for the
/// occasional `•` / em-dash / curly quote we approximate via `chars()` —
/// good enough for the wrap budget without pulling in `unicode-width`.
fn display_width(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Build a `WorkflowBriefing` fixture.
    fn briefing() -> WorkflowBriefing {
        WorkflowBriefing {
            overview: "Hello **world**.".into(),
            prerequisites: vec!["A `thing`.".into()],
            creates: vec!["A stat.".into()],
        }
    }

    #[test]
    fn test_briefing_panel_renders_title_and_sections() {
        let panel = BriefingPanel::new(&briefing(), "WF");
        let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
        term.draw(|f| {
            panel.render(f, f.area(), 0);
        })
        .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf.contains("WF"), "workflow name as title");
        assert!(
            buf.contains("Hello world"),
            "overview rendered with inline-format stripped"
        );
        assert!(buf.contains("Prerequisites"));
        assert!(buf.contains("This workflow creates"));
        assert!(
            buf.contains("A thing"),
            "code marker stripped, content present"
        );
    }

    #[test]
    fn test_briefing_panel_omits_empty_sections() {
        let b = WorkflowBriefing {
            overview: "only overview".into(),
            prerequisites: vec![],
            creates: vec![],
        };
        let panel = BriefingPanel::new(&b, "WF");
        let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
        term.draw(|f| {
            panel.render(f, f.area(), 0);
        })
        .unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!buf.contains("Prerequisites"));
        assert!(!buf.contains("This workflow creates"));
    }

    #[test]
    fn test_briefing_panel_render_returns_clamping_max_offset() {
        let b = WorkflowBriefing {
            // Plenty of lines so content > visible.
            overview: (0..30)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n\n"),
            prerequisites: vec![],
            creates: vec![],
        };
        let panel = BriefingPanel::new(&b, "WF");
        let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
        let mut max = 0u16;
        term.draw(|f| {
            max = panel.render(f, f.area(), 0);
        })
        .unwrap();
        assert!(
            max > 0,
            "tall content should produce a scrollable max_offset"
        );
    }

    #[test]
    fn test_wrap_paragraph_hangs_continuation_under_first_word() {
        // A 30-column budget against a long bullet should produce at least
        // two lines; the second line must start with the hanging indent
        // (4 spaces) and the first word that didn't fit on line 1.
        let lines = wrap_paragraph(
            "alpha beta gamma delta epsilon zeta eta theta",
            "  \u{2022} ",
            "    ",
            30,
        );
        assert!(lines.len() >= 2, "expected wrap, got {lines:?}");

        // Line 0 starts with the bullet prefix.
        let line0_text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            line0_text.starts_with("  \u{2022} "),
            "first line starts with bullet prefix; got {line0_text:?}"
        );

        // Line 1 starts with the four-space hanging indent.
        let line1_text: String = lines[1]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            line1_text.starts_with("    "),
            "continuation hangs under first word; got {line1_text:?}"
        );
        assert!(
            !line1_text.starts_with("\u{2022}") && !line1_text.starts_with(" \u{2022}"),
            "continuation must NOT have a bullet glyph; got {line1_text:?}"
        );
    }

    #[test]
    fn test_wrap_paragraph_glues_adjacent_styled_spans_without_space() {
        // `**MMR**)` is one word in source — bold "MMR" immediately
        // followed by plain ")". The wrap must not insert a space.
        let lines = wrap_paragraph("called **MMR**) please", "", "", 80);
        let text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            text.contains("MMR) "),
            "adjacent styled spans must glue without a space; got {text:?}"
        );
        assert!(
            !text.contains("MMR )"),
            "no space between MMR and ): {text:?}"
        );
    }

    #[test]
    fn test_wrap_paragraph_respects_inline_formatting() {
        let lines = wrap_paragraph("plain **bold** then `code` here", "", "", 80);
        assert_eq!(lines.len(), 1, "fits on one 80-wide line");
        // The middle bold and code spans retain their styling.
        let styles: Vec<(String, Style)> = lines[0]
            .spans
            .iter()
            .map(|s| (s.content.to_string(), s.style))
            .collect();
        let bold_word = styles
            .iter()
            .find(|(t, _)| t == "bold")
            .expect("bold word present");
        assert!(
            bold_word.1.add_modifier.contains(Modifier::BOLD),
            "bold word carries BOLD modifier: {:?}",
            bold_word.1
        );
        let code_word = styles
            .iter()
            .find(|(t, _)| t == "code")
            .expect("code word present");
        assert_eq!(
            code_word.1.fg,
            Some(Color::Indexed(244)),
            "code word carries dim fg"
        );
    }
}
