//! Tree-view rendering for the structured JSON editor.
//!
//! Renders the visible-row order from [`navigation::flatten`] into ratatui
//! [`Line`]s. Each row gets one line: indent, expansion marker, label
//! (with `*` for required), a type chip, and an optional value summary.

use std::cell::Cell;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation};
use ratatui::Frame;

use super::navigation::{flatten, get, NodePath};
use super::node::{Node, NodeKind, ScalarValue};
use crate::frontend::terminal::scrollbar::scrollbar_state;

/// Render the tree into `area` with scroll-to-focus and a right-edge scrollbar
/// when it overflows — mirroring the main form's field list so long JSON bodies
/// stay navigable. `scroll_top` persists the window position across redraws.
pub fn render_tree_view(
    frame: &mut Frame,
    area: Rect,
    root: &Node,
    focus: &NodePath,
    editing: Option<(&NodePath, &str)>,
    scroll_top: &Cell<usize>,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let lines = render_tree(root, focus, editing);
    let total = lines.len();
    let height = area.height as usize;

    // Keep the focused row visible, moving the window as little as possible
    // (same behaviour as `views::fields::render_inline`).
    let focus_idx = flatten(root).iter().position(|p| p == focus).unwrap_or(0);
    let max_start = total.saturating_sub(height);
    let mut start = scroll_top.get().min(max_start);
    if focus_idx < start {
        start = focus_idx;
    } else if focus_idx >= start + height {
        start = focus_idx + 1 - height;
    }
    start = start.min(max_start);
    scroll_top.set(start);

    // Reserve the scrollbar column (+gap) so long rows trail off before it.
    let overflow = total > height;
    let right_reserve = if overflow { 2 } else { 0 };
    let end = (start + height).min(total);
    let windowed: Vec<Line> = lines[start..end].to_vec();
    let text_area = Rect {
        width: area.width.saturating_sub(right_reserve),
        ..area
    };
    frame.render_widget(Paragraph::new(windowed), text_area);

    if let Some(mut sb) = scrollbar_state(total, height, start) {
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area,
            &mut sb,
        );
    }
}

/// The focused node's schema `description`, for the editor hint box. A single
/// trailing full stop is stripped to match the form's hint convention.
pub fn focused_description(root: &Node, focus: &NodePath) -> Option<String> {
    let node = get(root, focus)?;
    let desc = node.schema.get("description")?.as_str()?;
    if desc.is_empty() {
        return None;
    }
    Some(desc.strip_suffix('.').unwrap_or(desc).to_string())
}

/// Render the tree as a flat list of [`Line`]s in visible-row order.
/// `focus` highlights the matching row label; pass an empty path to focus the
/// root. `editing` is `Some((path, buffer))` while a scalar leaf is being edited
/// in place — that row renders `[buffer▌]` instead of its value summary.
pub fn render_tree(
    root: &Node,
    focus: &NodePath,
    editing: Option<(&NodePath, &str)>,
) -> Vec<Line<'static>> {
    let visible = flatten(root);
    visible
        .iter()
        .map(|path| {
            let node = get(root, path).expect("path is visible");
            let edit_buffer = match editing {
                Some((p, buf)) if p == path => Some(buf),
                _ => None,
            };
            render_row(node, path.len(), path == focus, edit_buffer)
        })
        .collect()
}

/// Build the styled line for one tree row: indentation, expansion marker, key,
/// type chip, and value summary, highlighted when `focused`.
fn render_row(
    node: &Node,
    depth: usize,
    focused: bool,
    edit_buffer: Option<&str>,
) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::raw("  ".repeat(depth)));
    spans.push(Span::raw(expansion_marker(node).to_string()));
    spans.push(Span::raw(" "));

    let label_style = if focused {
        Style::default()
            .bg(Color::Indexed(238))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    spans.push(Span::styled(node.name.clone(), label_style));
    if node.required {
        // Orange `*`, matching the inline form's required marker.
        spans.push(Span::styled(" *", Style::default().fg(Color::Indexed(208))));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        type_chip(node),
        Style::default().fg(Color::LightCyan),
    ));
    let dim = Style::default().fg(Color::Indexed(244));
    if let Some(buffer) = edit_buffer {
        // Replaces the static value summary so the user sees a live editing state.
        spans.push(Span::raw(" "));
        spans.push(Span::styled("[", dim));
        spans.push(Span::raw(format!("{buffer}\u{2588}")));
        spans.push(Span::styled("]", dim));
    } else if let NodeKind::Scalar { value } = &node.kind {
        // Always bracket scalar leaves (dim brackets) so the value persists in
        // brackets like the inline form and the live editing state — consistent.
        spans.push(Span::raw(" "));
        spans.push(Span::styled("[", dim));
        spans.push(Span::raw(scalar_text(value)));
        spans.push(Span::styled("]", dim));
    } else if let Some(summary) = value_summary(node) {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(summary, dim));
    }
    Line::from(spans)
}

/// Render a scalar node's value as plain text (empty string when unset).
fn scalar_text(value: &Option<ScalarValue>) -> String {
    match value {
        Some(ScalarValue::String(s)) | Some(ScalarValue::Enum(s)) => s.clone(),
        Some(ScalarValue::Integer(i)) => i.to_string(),
        Some(ScalarValue::Number(f)) => f.to_string(),
        Some(ScalarValue::Boolean(b)) => b.to_string(),
        None => String::new(),
    }
}

/// Abbreviate a JSON-schema type word for the type chip, so a string array
/// (`[str]`) and a plain string (`str`) read consistently.
fn abbrev_type(t: &str) -> &str {
    match t {
        "string" => "str",
        "integer" => "int",
        "number" => "num",
        "boolean" => "bool",
        other => other,
    }
}

/// The triangle marker for a container node (▼ expanded, ▶ collapsed), or a
/// space for scalars.
fn expansion_marker(node: &Node) -> &'static str {
    match (&node.kind, node.expanded) {
        (NodeKind::Object { .. } | NodeKind::Array { .. }, true) => "\u{25BC}",
        (NodeKind::Object { .. } | NodeKind::Array { .. }, false) => "\u{25B6}",
        _ => " ",
    }
}

/// The short type chip shown for a node (e.g. `{}`, `[int]`, `enum`, `str`).
fn type_chip(node: &Node) -> String {
    match &node.kind {
        NodeKind::Object { .. } => "{}".into(),
        NodeKind::Array { .. } => match node
            .schema
            .get("items")
            .and_then(|i| i.get("type"))
            .and_then(|t| t.as_str())
        {
            Some(t) => format!("[{}]", abbrev_type(t)),
            None => "[]".into(),
        },
        NodeKind::Scalar { .. } => {
            if node.schema.get("enum").is_some() {
                "enum".into()
            } else {
                match node.schema.get("type").and_then(|t| t.as_str()) {
                    Some(t) => abbrev_type(t).to_string(),
                    None => "?".into(),
                }
            }
        }
        NodeKind::OneOf { .. } => "oneOf".into(),
        NodeKind::FreeMap { .. } => "{str:any}".into(),
    }
}

/// The inline value summary for a node's row, or `None` when there is nothing
/// to show (e.g. an unset scalar or a container).
fn value_summary(node: &Node) -> Option<String> {
    match &node.kind {
        NodeKind::Scalar { value: Some(v) } => Some(match v {
            ScalarValue::String(s) | ScalarValue::Enum(s) => s.clone(),
            ScalarValue::Integer(i) => i.to_string(),
            ScalarValue::Number(f) => f.to_string(),
            ScalarValue::Boolean(b) => b.to_string(),
        }),
        NodeKind::Scalar { value: None } => Some("(unset)".into()),
        NodeKind::Array { children } => Some(if children.len() == 1 {
            "(1 entry)".to_string()
        } else {
            format!("({} entries)", children.len())
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::json_editor::node::from_schema;

    /// Flatten a ratatui [`Line`] into its concatenated text content, for
    /// assertion-friendly tests that don't need to introspect styles.
    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn test_render_tree_shows_required_marker_on_required_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } },
            "required": ["a"]
        });
        let root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let lines = render_tree(&root, &vec![], None);
        let a_row = line_text(&lines[1]);
        assert!(
            a_row.contains("a *"),
            "expected required marker on 'a': {a_row}"
        );
    }

    #[test]
    fn test_render_tree_collapses_marker_for_collapsed_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } }
        });
        let mut root = from_schema("root", &schema, &serde_json::Value::Null, true);
        root.expanded = false;
        let lines = render_tree(&root, &vec![], None);
        assert_eq!(lines.len(), 1);
        let row = line_text(&lines[0]);
        assert!(
            row.starts_with("\u{25B6}"),
            "expected collapsed marker: {row}"
        );
    }

    #[test]
    fn test_render_tree_array_chip_shows_inner_type() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let root = from_schema("tags", &schema, &serde_json::Value::Null, false);
        let row = line_text(&render_tree(&root, &vec![], None)[0]);
        assert!(
            row.contains("[str]"),
            "expected abbreviated '[str]' chip: {row}"
        );
    }

    #[test]
    fn test_render_tree_unset_scalar_renders_empty_brackets() {
        let schema = serde_json::json!({ "type": "string" });
        let root = from_schema("name", &schema, &serde_json::Value::Null, false);
        let row = line_text(&render_tree(&root, &vec![], None)[0]);
        assert!(
            row.contains("[]"),
            "unset scalar renders empty brackets: {row}"
        );
    }

    #[test]
    fn test_render_tree_scalar_value_is_bracketed() {
        let schema = serde_json::json!({ "type": "string" });
        let root = from_schema("name", &schema, &serde_json::json!("alice"), false);
        let row = line_text(&render_tree(&root, &vec![], None)[0]);
        assert!(row.contains("[alice]"), "set scalar is bracketed: {row}");
    }

    #[test]
    fn test_render_tree_enum_chip_is_enum() {
        let schema = serde_json::json!({
            "type": "string",
            "enum": ["RED", "BLUE"]
        });
        let root = from_schema("color", &schema, &serde_json::Value::Null, false);
        let row = line_text(&render_tree(&root, &vec![], None)[0]);
        assert!(row.contains("enum"), "expected 'enum' chip: {row}");
    }

    #[test]
    fn test_render_tree_indents_children_with_two_spaces_per_depth_level() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "inner": { "type": "object", "properties": { "leaf": { "type": "string" } } }
            }
        });
        let root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let lines = render_tree(&root, &vec![], None);
        // root, inner (depth 1), leaf (depth 2)
        assert_eq!(lines.len(), 3);
        assert!(line_text(&lines[0]).starts_with("\u{25BC}"));
        assert!(line_text(&lines[1]).starts_with("  "));
        assert!(line_text(&lines[2]).starts_with("    "));
    }

    #[test]
    fn test_render_tree_shows_edit_buffer_in_place_for_editing_path() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        let root = from_schema("root", &schema, &serde_json::json!({"name": "old"}), true);
        // Editing the child at path [0] with buffer "new".
        let name_path = vec![0usize];
        let lines = render_tree(&root, &name_path, Some((&name_path, "new")));
        let row = line_text(&lines[1]);
        assert!(
            row.contains("[new"),
            "editing row shows the buffer in brackets: {row}"
        );
        assert!(
            row.contains('\u{2588}'),
            "editing row includes the block cursor: {row}"
        );
        assert!(
            !row.contains("old"),
            "editing row hides the prior value: {row}"
        );
    }

    #[test]
    fn test_render_tree_view_scrolls_focused_row_into_view() {
        use ratatui::{backend::TestBackend, Terminal};
        use std::cell::Cell;
        // An object with far more children than the viewport can show.
        let mut props = serde_json::Map::new();
        for i in 0..30 {
            props.insert(format!("f{i:02}"), serde_json::json!({ "type": "string" }));
        }
        let schema = serde_json::json!({ "type": "object", "properties": props });
        let root = from_schema("root", &schema, &serde_json::Value::Null, true);
        // Focus a row well below the fold (root is index 0, children follow).
        let flat = flatten(&root);
        let focus = flat[26].clone();
        let label = get(&root, &focus).unwrap().name.clone();
        let scroll = Cell::new(0usize);
        let mut term = Terminal::new(TestBackend::new(40, 8)).unwrap();
        term.draw(|f| render_tree_view(f, f.area(), &root, &focus, None, &scroll))
            .unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            s.contains(label.as_str()),
            "focused row '{label}' must scroll into view: {s}"
        );
        assert!(scroll.get() > 0, "scroll window advanced past the top");
    }

    #[test]
    fn test_focused_description_reads_schema_and_strips_trailing_period() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string", "description": "The thing." } }
        });
        let root = from_schema("root", &schema, &serde_json::Value::Null, true);
        assert_eq!(
            focused_description(&root, &vec![0]).as_deref(),
            Some("The thing")
        );

        // No description → None.
        let plain = from_schema(
            "x",
            &serde_json::json!({ "type": "string" }),
            &serde_json::Value::Null,
            false,
        );
        assert_eq!(focused_description(&plain, &vec![]), None);
    }
}
