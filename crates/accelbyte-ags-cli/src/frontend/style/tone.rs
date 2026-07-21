//! Semantic style vocabulary. Backends map Tones to their own styling (ANSI,
//! ratatui, plain text, …).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Dim,
    Bold,
    Success,
    Error,
    Warning,
    Info,
}
