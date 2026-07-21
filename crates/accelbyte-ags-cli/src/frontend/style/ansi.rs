//! ANSI backend: apply Tones, write styled lines to terminals.

use std::sync::atomic::{AtomicBool, Ordering};

use clap::builder::Styles;

use super::span::{StyledLine, StyledSpan};
use super::text::{
    SYMBOL_ERROR, SYMBOL_FIX, SYMBOL_INFO, SYMBOL_STATUS, SYMBOL_SUCCESS, SYMBOL_WARNING,
};
use super::tone::Tone;

static STDOUT_COLOR: AtomicBool = AtomicBool::new(true);
static STDERR_COLOR: AtomicBool = AtomicBool::new(true);

/// Pure core of the colour force-off rule: colour is disabled when either
/// the `--no-color` flag was passed or the `NO_COLOR` environment variable
/// is present. Split out from `color_force_off` so both outcomes are
/// deterministically testable without depending on ambient environment.
fn color_force_off_inner(no_color_flag: bool, no_color_env_present: bool) -> bool {
    no_color_flag || no_color_env_present
}

/// True when colour must be disabled regardless of the target stream:
/// either the `--no-color` flag was passed or the `NO_COLOR` environment
/// variable is present. This is the single source of truth for the
/// force-off rule — `init` and the frontend-context resolver both use it.
pub fn color_force_off(no_color_flag: bool) -> bool {
    color_force_off_inner(no_color_flag, std::env::var("NO_COLOR").is_ok())
}

/// Initialize color support. Call once at startup.
/// Disables color when: --no-color flag, NO_COLOR environment variable, or the target stream is not a TTY.
/// TTY detection is per-stream: stdout and stderr are checked independently.
pub fn init(no_color_flag: bool) {
    let force_off = color_force_off(no_color_flag);
    STDOUT_COLOR.store(
        !force_off && ags_runtime::support::is_stdout_tty(),
        Ordering::Relaxed,
    );
    STDERR_COLOR.store(
        !force_off && ags_runtime::support::is_stderr_tty(),
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
pub fn success(message: &str, enabled: bool) -> String {
    apply_tone(
        &format!("{SYMBOL_SUCCESS} {message}"),
        Tone::Success,
        enabled,
    )
}

/// Format an error message with a red cross prefix
pub fn error(message: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_ERROR} {message}"), Tone::Error, enabled)
}

/// Format a status message with a dimmed bullet prefix
pub fn status(message: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_STATUS} {message}"), Tone::Dim, enabled)
}

/// Format a warning message with a yellow exclamation prefix
pub fn warning(message: &str, enabled: bool) -> String {
    apply_tone(
        &format!("{SYMBOL_WARNING} {message}"),
        Tone::Warning,
        enabled,
    )
}

/// Format an informational message with a cyan angle-bracket prefix
pub fn info(message: &str, enabled: bool) -> String {
    apply_tone(&format!("{SYMBOL_INFO} {message}"), Tone::Info, enabled)
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

#[cfg(test)]
mod tests {
    use super::color_force_off_inner;

    #[test]
    fn test_color_force_off_inner_flag_forces_off() {
        assert!(color_force_off_inner(true, false));
    }

    #[test]
    fn test_color_force_off_inner_env_forces_off() {
        assert!(color_force_off_inner(false, true));
    }

    #[test]
    fn test_color_force_off_inner_no_signal_keeps_color() {
        assert!(!color_force_off_inner(false, false));
    }

    #[test]
    fn test_color_force_off_inner_both_signals() {
        assert!(color_force_off_inner(true, true));
    }
}
