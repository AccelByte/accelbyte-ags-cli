//! Reusable output templates: error, success, tip, inspect, and list layouts.

use crate::errors::SuggestionKind;
use crate::protocol::output::{FieldEntry, Section};
use crate::support::strings::pluralize;

use super::style::span::{StyledLine, StyledSpan};
use super::style::text::{SYMBOL_ERROR, SYMBOL_FIX, SYMBOL_INFO, SYMBOL_WARNING};
use super::style::tone::Tone;

/// Render a structured error message as a sequence of styled lines.
///
/// ```text
/// ✖ <message>.
///     Reason: <reason>.
///     Detail: <detail>.
///
/// ↳ Fix: <suggestion>.       (SuggestionKind::Fix)
/// ↳ Next: <suggestion>.      (SuggestionKind::Next)
/// ```
pub fn render_error(
    message: &str,
    reason: Option<&str>,
    detail: Option<&str>,
    suggestion: Option<&str>,
    suggestion_kind: SuggestionKind,
    tip: Option<&str>,
) -> Vec<StyledLine> {
    let mut lines: Vec<StyledLine> = Vec::new();

    let (headline, rest) = message.split_once('\n').unwrap_or((message, ""));
    lines.push(StyledLine::toned(
        format!("{SYMBOL_ERROR} {headline}"),
        Tone::Error,
    ));
    if !rest.is_empty() {
        lines.push(StyledLine::plain(rest));
    }

    if let Some(reason_text) = reason {
        lines.push(StyledLine::plain(format!("    Reason: {reason_text}")));
    }
    if let Some(detail_text) = detail {
        lines.push(StyledLine::toned(
            format!("    Detail: {detail_text}"),
            Tone::Dim,
        ));
    }

    if let Some(suggestion_text) = suggestion {
        let label = match suggestion_kind {
            SuggestionKind::Fix => "Fix",
            SuggestionKind::Next => "Next",
        };
        lines.push(StyledLine::plain(format!(
            "{SYMBOL_FIX} {label}: {suggestion_text}"
        )));
    }

    if let Some(tip_text) = tip {
        lines.push(StyledLine::toned(format!("    Tip: {tip_text}"), Tone::Dim));
    }

    lines
}

/// Render a structured warning message as a sequence of styled lines.
///
/// ```text
/// ! <message>
///     Reason: <reason>
///     Tip: <tip>
/// ```
pub fn render_warning(message: &str, reason: Option<&str>, tip: Option<&str>) -> Vec<StyledLine> {
    let mut lines: Vec<StyledLine> = Vec::new();
    lines.push(StyledLine::toned(
        format!("{SYMBOL_WARNING} {message}"),
        Tone::Warning,
    ));
    if let Some(reason_text) = reason {
        lines.push(StyledLine::plain(format!("    Reason: {reason_text}")));
    }
    if let Some(tip_text) = tip {
        lines.push(StyledLine::toned(format!("    Tip: {tip_text}"), Tone::Dim));
    }
    lines
}

/// Render a tip as a sequence of styled lines — dimmed, indented, no symbol.
///
/// ```text
///     Tip: <message>
/// ```
pub fn render_tip(message: &str) -> Vec<StyledLine> {
    vec![StyledLine::toned(format!("    Tip: {message}"), Tone::Dim)]
}

/// Render an inspect (single-object) view.
///
/// ```text
/// ▸ <heading>
///     <Label>: <value>
///
///     <Section heading>
///         <Label>: <value>
/// ```
pub fn render_inspect(
    heading: &str,
    fields: &[FieldEntry],
    sections: &[Section],
    quiet: bool,
) -> Vec<StyledLine> {
    let mut lines: Vec<StyledLine> = Vec::new();

    if !quiet {
        lines.push(StyledLine::toned(
            format!("{SYMBOL_INFO} {heading}"),
            Tone::Info,
        ));
    }

    let max_label_width = fields.iter().map(|f| f.label.len()).max().unwrap_or(0);
    for field in fields {
        lines.push(StyledLine::plain(format!(
            "    {:<width$}  {}",
            format!("{}:", field.label),
            field.value,
            width = max_label_width + 1
        )));
    }

    for section in sections {
        lines.push(StyledLine::empty());
        let mut section_heading_line = StyledLine::default();
        section_heading_line.push(StyledSpan::plain("    "));
        section_heading_line.push(StyledSpan::toned(section.heading.clone(), Tone::Bold));
        lines.push(section_heading_line);
        let section_max = section
            .fields
            .iter()
            .map(|f| f.label.len())
            .max()
            .unwrap_or(0);
        for field in &section.fields {
            lines.push(StyledLine::plain(format!(
                "        {:<width$}  {}",
                format!("{}:", field.label),
                field.value,
                width = section_max + 1
            )));
        }
    }

    lines
}

/// Render a list view with column headers.
///
/// ```text
/// ▸ Found N <noun>
///     HEADER1   HEADER2
///     item1     col2
///     item2     col2
/// ```
#[allow(clippy::too_many_arguments)]
pub fn render_list(
    count: usize,
    noun: &str,
    headers: &[String],
    rows: &[Vec<String>],
    pagination: Option<crate::frontend::PaginationHint>,
    is_page_all: bool,
    quiet: bool,
) -> Vec<StyledLine> {
    let mut lines: Vec<StyledLine> = Vec::new();
    let mut trailing_tip: Option<String> = None;

    if !quiet {
        let plural = pluralize(noun, count);
        let header_line = match &pagination {
            Some(hint) if hint.total.is_some_and(|t| t as usize > count) => {
                let total = hint.total.unwrap();
                if !is_page_all {
                    trailing_tip = Some(format!("Use --page-all to fetch all {total} {plural}"));
                }
                format!("{SYMBOL_INFO} Showing {count} of {total} {plural}")
            }
            Some(hint) if hint.has_next => {
                if !is_page_all {
                    trailing_tip = Some("Use --page-all to fetch additional pages".to_string());
                }
                format!("{SYMBOL_INFO} Found {count} {plural} (more available)")
            }
            _ => format!("{SYMBOL_INFO} Found {count} {plural}"),
        };
        lines.push(StyledLine::toned(header_line, Tone::Info));
    }

    // Calculate column widths including headers
    let column_count = headers.len().max(rows.first().map_or(0, |row| row.len()));
    let mut column_widths = vec![0usize; column_count];

    for (i, header) in headers.iter().enumerate() {
        if i < column_widths.len() {
            column_widths[i] = column_widths[i].max(header.len());
        }
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < column_widths.len() {
                column_widths[i] = column_widths[i].max(cell.len());
            }
        }
    }

    // Render header row
    if !headers.is_empty() {
        let mut header_cells = Vec::new();
        for (i, header) in headers.iter().enumerate() {
            if i < column_widths.len() && i < headers.len() - 1 {
                header_cells.push(format!("{:<width$}", header, width = column_widths[i]));
            } else {
                header_cells.push(header.clone());
            }
        }
        let mut header_line = StyledLine::default();
        header_line.push(StyledSpan::plain("    "));
        header_line.push(StyledSpan::toned(header_cells.join("  "), Tone::Dim));
        lines.push(header_line);
    }

    // Render data rows
    for row in rows {
        let mut formatted_cells = Vec::new();
        for (i, cell) in row.iter().enumerate() {
            if i < column_widths.len() && i < row.len() - 1 {
                formatted_cells.push(format!("{:<width$}", cell, width = column_widths[i]));
            } else {
                formatted_cells.push(cell.clone());
            }
        }
        lines.push(StyledLine::plain(format!(
            "    {}",
            formatted_cells.join("  ")
        )));
    }

    if let Some(tip) = trailing_tip {
        lines.extend(render_tip(&tip));
    }

    lines
}
