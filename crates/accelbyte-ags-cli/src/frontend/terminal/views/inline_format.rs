//! Minimal inline-format parser for workflow briefing text, shared by the
//! inline and fullscreen surfaces so both render emphasis identically.
//!
//! Recognises `**bold**` and `` `code` ``. Anything else is plain.
//! Markers do not nest and do not honour escaping. An unterminated
//! marker is rendered as literal text (no panic, no input lost).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum InlineSpan {
    Plain(String),
    Bold(String),
    Code(String),
}

/// Map a parsed span to its display text and ratatui style. The single source
/// of truth for briefing emphasis styling (bold → BOLD, code → dim fg) so the
/// inline and fullscreen briefings can never drift apart.
pub fn classify(span: InlineSpan) -> (String, Style) {
    match span {
        InlineSpan::Plain(t) => (t, Style::default()),
        InlineSpan::Bold(t) => (t, Style::default().add_modifier(Modifier::BOLD)),
        InlineSpan::Code(t) => (t, Style::default().fg(Color::Indexed(244))),
    }
}

/// Parse `input` and map it straight to styled ratatui spans, for surfaces that
/// delegate wrapping to ratatui (the inline briefing). The fullscreen briefing
/// instead tokenises into words for manual hang-indent wrapping, but applies
/// the same [`classify`] styling.
pub fn styled_spans(input: &str) -> Vec<Span<'static>> {
    parse(input)
        .into_iter()
        .map(|span| {
            let (text, style) = classify(span);
            Span::styled(text, style)
        })
        .collect()
}

/// Parse inline markup (`**bold**`, `` `code` ``) in `input` into styled spans.
pub fn parse(input: &str) -> Vec<InlineSpan> {
    let bytes = input.as_bytes();
    let mut out: Vec<InlineSpan> = Vec::new();
    let mut plain_start = 0usize;
    let mut i = 0usize;

    let push_plain = |out: &mut Vec<InlineSpan>, slice: &[u8]| {
        if !slice.is_empty() {
            // SAFETY: slice is a contiguous range of the original &str's bytes,
            // so it is valid UTF-8 by construction.
            let s = std::str::from_utf8(slice).expect("plain slice valid utf-8");
            out.push(InlineSpan::Plain(s.to_string()));
        }
    };

    while i < bytes.len() {
        // `**…**`
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(end) = find_marker(bytes, i + 2, b"**") {
                push_plain(&mut out, &bytes[plain_start..i]);
                let inner = std::str::from_utf8(&bytes[i + 2..end])
                    .expect("bold inner valid utf-8")
                    .to_string();
                out.push(InlineSpan::Bold(inner));
                i = end + 2;
                plain_start = i;
                continue;
            }
        }
        // `` `…` ``
        if bytes[i] == b'`' {
            if let Some(end) = find_marker(bytes, i + 1, b"`") {
                push_plain(&mut out, &bytes[plain_start..i]);
                let inner = std::str::from_utf8(&bytes[i + 1..end])
                    .expect("code inner valid utf-8")
                    .to_string();
                out.push(InlineSpan::Code(inner));
                i = end + 1;
                plain_start = i;
                continue;
            }
        }
        i += 1;
    }
    push_plain(&mut out, &bytes[plain_start..]);
    out
}

/// Return the byte offset of `needle` in `haystack` starting at `from`,
/// or `None` if it doesn't occur. Byte-level so we can keep `bytes`-driven
/// indexing without per-iteration UTF-8 decoding.
fn find_marker(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from + needle.len() > haystack.len() {
        return None;
    }
    let mut i = from;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_round_trips() {
        assert_eq!(
            parse("hello world"),
            vec![InlineSpan::Plain("hello world".into())]
        );
    }

    #[test]
    fn test_single_bold_run() {
        assert_eq!(
            parse("a **b** c"),
            vec![
                InlineSpan::Plain("a ".into()),
                InlineSpan::Bold("b".into()),
                InlineSpan::Plain(" c".into()),
            ]
        );
    }

    #[test]
    fn test_single_code_run() {
        assert_eq!(
            parse("run `cmd` now"),
            vec![
                InlineSpan::Plain("run ".into()),
                InlineSpan::Code("cmd".into()),
                InlineSpan::Plain(" now".into()),
            ]
        );
    }

    #[test]
    fn test_interleaved_bold_and_code() {
        assert_eq!(
            parse("**a** then `b` and **c**"),
            vec![
                InlineSpan::Bold("a".into()),
                InlineSpan::Plain(" then ".into()),
                InlineSpan::Code("b".into()),
                InlineSpan::Plain(" and ".into()),
                InlineSpan::Bold("c".into()),
            ]
        );
    }

    #[test]
    fn test_adjacent_bold_then_code() {
        assert_eq!(
            parse("**a**`b`"),
            vec![InlineSpan::Bold("a".into()), InlineSpan::Code("b".into())]
        );
    }

    #[test]
    fn test_unterminated_bold_renders_as_literal() {
        assert_eq!(parse("a **b c"), vec![InlineSpan::Plain("a **b c".into())]);
    }

    #[test]
    fn test_unterminated_code_renders_as_literal() {
        assert_eq!(parse("a `b c"), vec![InlineSpan::Plain("a `b c".into())]);
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(parse(""), Vec::<InlineSpan>::new());
    }
}
