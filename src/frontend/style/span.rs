//! Styled text fragments — the IR between templates and frontend backends.

use super::tone::Tone;

#[derive(Debug, Clone)]
pub struct StyledSpan {
    pub text: String,
    pub tone: Tone,
}

impl StyledSpan {
    /// Build an unstyled span — the default when no tone is specified.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Plain,
        }
    }

    /// Build a span carrying a semantic tone for the backend to translate.
    pub fn toned(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StyledLine(pub(crate) Vec<StyledSpan>);

impl StyledLine {
    /// Build an empty line — used as the seed for incremental construction.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a single-span line with no styling applied.
    pub fn plain(text: impl Into<String>) -> Self {
        Self(vec![StyledSpan::plain(text)])
    }

    /// Build a single-span line carrying one semantic tone.
    pub fn toned(text: impl Into<String>, tone: Tone) -> Self {
        Self(vec![StyledSpan::toned(text, tone)])
    }

    /// Append another span to the line.
    pub fn push(&mut self, span: StyledSpan) {
        self.0.push(span);
    }
}
