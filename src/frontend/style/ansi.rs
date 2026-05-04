//! ANSI backend: apply Tones, write styled lines to terminals.

use std::sync::atomic::{AtomicBool, Ordering};

use clap::builder::Styles;

use super::span::{StyledLine, StyledSpan};
use super::text::{
    SYMBOL_ERROR, SYMBOL_FIX, SYMBOL_INFO, SYMBOL_STATUS, SYMBOL_SUCCESS, SYMBOL_WARNING,
};
use super::tone::Tone;

// Separate flags for stdout vs stderr (check each stream independently)
static STDOUT_COLOR: AtomicBool = AtomicBool::new(true);
static STDERR_COLOR: AtomicBool = AtomicBool::new(true);

/// Initialize color support. Call once at startup.
/// Disables color when: --no-color flag, NO_COLOR environment variable, or the target stream is not a TTY.
/// TTY detection is per-stream: stdout and stderr are checked independently.
pub fn init(no_color_flag: bool) {
    let force_off = no_color_flag || std::env::var("NO_COLOR").is_ok();
    STDOUT_COLOR.store(
        !force_off && crate::support::is_stdout_tty(),
        Ordering::Relaxed,
    );
    STDERR_COLOR.store(
        !force_off && crate::support::is_stderr_tty(),
        Ordering::Relaxed,
    );
}

/// Use for text going to stdout (via `frontend::write_stdout_line`)
pub fn is_stdout_enabled() -> bool {
    STDOUT_COLOR.load(Ordering::Relaxed)
}

/// Use for text going to stderr (via `frontend::write_stderr_line` / `write_stderr`)
pub fn is_stderr_enabled() -> bool {
    STDERR_COLOR.load(Ordering::Relaxed)
}

/// Wrap text in green ANSI escape codes when color is enabled
pub fn green(text: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[32m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Wrap text in red ANSI escape codes when color is enabled
pub fn red(text: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[31m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Wrap text in cyan ANSI escape codes when color is enabled
pub fn cyan(text: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[36m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Wrap text in yellow ANSI escape codes when color is enabled
pub fn yellow(text: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[33m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Wrap text in dim ANSI escape codes when color is enabled
pub fn dim(text: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[2m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Wrap text in bold ANSI escape codes when color is enabled
pub fn bold(text: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Format a success message with a green checkmark prefix
pub fn success(msg: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_SUCCESS} {msg}"), Tone::Success, enabled)
}

/// Format an error message with a red cross prefix
pub fn error(msg: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_ERROR} {msg}"), Tone::Error, enabled)
}

/// Format a status message with a dimmed bullet prefix
pub fn status(msg: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_STATUS} {msg}"), Tone::Dim, enabled)
}

/// Format a warning message with a yellow exclamation prefix
pub fn warning(msg: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_WARNING} {msg}"), Tone::Warning, enabled)
}

/// Format an informational message with a cyan angle-bracket prefix
pub fn info(msg: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_INFO} {msg}"), Tone::Info, enabled)
}

/// Return the arrow symbol used before fix/next-step suggestions
pub fn fix_prefix() -> &'static str {
    SYMBOL_FIX
}

/// Format text using Clap's built-in literal style (bold).
pub fn styled_literal(text: &str) -> String {
    let styles = Styles::styled();
    let style = styles.get_literal();
    format!("{}{text}{}", style.render(), style.render_reset())
}

/// Format text using Clap's built-in header style (bold+underline).
pub fn styled_header(text: &str) -> String {
    let styles = Styles::styled();
    let style = styles.get_header();
    format!("{}{text}{}", style.render(), style.render_reset())
}

/// Apply a tone to a raw text fragment.
pub fn apply_tone(text: &str, tone: Tone, color_enabled: bool) -> String {
    match tone {
        Tone::Plain => text.to_string(),
        Tone::Dim => dim(text, color_enabled),
        Tone::Bold => bold(text, color_enabled),
        Tone::Success => green(text, color_enabled),
        Tone::Error => red(text, color_enabled),
        Tone::Warning => yellow(text, color_enabled),
        Tone::Info => cyan(text, color_enabled),
    }
}

/// Render a single `StyledSpan` by translating its tone into the matching ANSI sequence.
fn render_span(span: &StyledSpan, color_enabled: bool) -> String {
    apply_tone(&span.text, span.tone, color_enabled)
}

/// Render a `StyledLine` by concatenating each span with its tone-mapped ANSI sequence.
fn render_line(line: &StyledLine, color_enabled: bool) -> String {
    line.0
        .iter()
        .map(|s| render_span(s, color_enabled))
        .collect()
}

/// Render multiple styled lines joined by `\n`.
pub fn render_lines(lines: &[StyledLine], color_enabled: bool) -> String {
    lines
        .iter()
        .map(|l| render_line(l, color_enabled))
        .collect::<Vec<_>>()
        .join("\n")
}
