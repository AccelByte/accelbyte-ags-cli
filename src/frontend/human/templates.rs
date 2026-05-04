//! Text adapters for the core template functions.
//!
//! Each function here delegates to the backend-agnostic core in
//! [`crate::frontend::templates`] and applies ANSI colour via
//! [`crate::frontend::style::ansi::render_lines`].

use crate::errors::SuggestionKind;
use crate::frontend::style::ansi;
use crate::frontend::style::Tone;
use crate::frontend::templates as core;
use crate::protocol::output::{FieldEntry, Section};

/// Render a structured error message as a `String`.
///
/// This is the text adapter: it calls [`core::render_error`] and applies ANSI colour
/// based on `color_enabled`.
pub fn render_error_text(
    message: &str,
    reason: Option<&str>,
    detail: Option<&str>,
    suggestion: Option<&str>,
    suggestion_kind: SuggestionKind,
    tip: Option<&str>,
    color_enabled: bool,
) -> String {
    crate::frontend::style::ansi::render_lines(
        &core::render_error(message, reason, detail, suggestion, suggestion_kind, tip),
        color_enabled,
    )
}

/// Render a structured warning message as a `String`.
///
/// This is the text adapter: it calls [`core::render_warning`] and applies ANSI colour
/// based on `color_enabled`.
pub fn render_warning_text(
    message: &str,
    reason: Option<&str>,
    tip: Option<&str>,
    color_enabled: bool,
) -> String {
    crate::frontend::style::ansi::render_lines(
        &core::render_warning(message, reason, tip),
        color_enabled,
    )
}

/// Render a tip as a `String`.
///
/// This is the text adapter: it calls [`core::render_tip`] and applies ANSI colour
/// based on `color_enabled`.
pub fn render_tip_text(message: &str, color_enabled: bool) -> String {
    crate::frontend::style::ansi::render_lines(&core::render_tip(message), color_enabled)
}

/// Render an inspect (single-object) view as a `String`.
///
/// This is the text adapter: it calls [`core::render_inspect`] and applies ANSI colour
/// based on `color_enabled`.
pub fn render_inspect_text(
    heading: &str,
    fields: &[FieldEntry],
    sections: &[Section],
    quiet: bool,
    color_enabled: bool,
) -> String {
    crate::frontend::style::ansi::render_lines(
        &core::render_inspect(heading, fields, sections, quiet),
        color_enabled,
    )
}

/// Render a list view with column headers as a `String`.
///
/// This is the text adapter: it calls [`core::render_list`] and applies ANSI colour
/// based on `color_enabled`.
#[allow(clippy::too_many_arguments)]
pub fn render_list_text(
    count: usize,
    noun: &str,
    headers: &[String],
    rows: &[Vec<String>],
    pagination: Option<crate::frontend::PaginationHint>,
    is_page_all: bool,
    quiet: bool,
    color_enabled: bool,
) -> String {
    crate::frontend::style::ansi::render_lines(
        &core::render_list(count, noun, headers, rows, pagination, is_page_all, quiet),
        color_enabled,
    )
}

/// Render a block of label/value pairs with dynamic label alignment.
///
/// Each row is indented 4 spaces and padded so labels align in a single column.
/// `tone` is applied to the rendered line (use `Tone::Dim` for secondary data,
/// `Tone::Plain` for primary content).
pub fn render_label_value_block_text(
    rows: &[(&str, String)],
    tone: Tone,
    color_enabled: bool,
) -> String {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(label, value)| {
            let line = format!("{label:<width$}  {value}", width = width);
            format!("    {}", ansi::apply_tone(&line, tone, color_enabled))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
