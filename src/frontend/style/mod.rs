//! Styling: semantic tones, styled-line IR, and backends.

pub mod ansi;
pub mod span;
pub mod text;
pub mod tone;

pub use tone::Tone;

pub use ansi::{
    apply_tone, error, fix_prefix, info, init, is_stderr_enabled, is_stdout_enabled, status,
    styled_header, styled_literal, success, warning,
};
