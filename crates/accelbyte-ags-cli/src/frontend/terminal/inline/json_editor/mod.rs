//! Structured JSON body editor.
//!
//! The submodules together form a schema-driven tree editor that replaces
//! the Phase 2 raw-text `JsonBody` field.

pub mod navigation;
pub mod node;
pub mod raw;
pub mod render;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use navigation::{flatten, get, get_mut, NodePath};
use node::{from_schema, Node, NodeKind, ScalarValue};
use raw::RawEditor;

/// The editor box title as a styled line. When `modified`, a leading `●` flags
/// unsaved changes. It reuses the orange of the required-field `*` marker
/// (`Indexed(208)`) to keep the palette tight; the filled-dot glyph already
/// distinguishes it from `*`. `text` is the bare title (e.g.
/// `Edit raw JSON: audience`); padding is added here.
pub fn editor_title_line(modified: bool, text: &str) -> Line<'static> {
    if modified {
        Line::from(vec![
            Span::styled(" \u{25CF} ", Style::default().fg(Color::Indexed(208))),
            Span::raw(format!("{text} ")),
        ])
    } else {
        Line::from(format!(" {text} "))
    }
}

/// Which mode the editor is currently in. Pressing `r` flips
/// between the structured tree view and a raw-text JSON buffer; Ctrl-S
/// from raw mode validates and commits back to the tree.
#[derive(Debug, Clone)]
pub enum EditorMode {
    Structured,
    Raw(RawEditor),
}

impl EditorMode {
    /// Enter raw mode, seeding the buffer from the current tree.
    pub fn into_raw(root: &Node) -> EditorMode {
        let seed =
            serde_json::to_string_pretty(&root.to_value()).unwrap_or_else(|_| "{}".to_string());
        EditorMode::Raw(RawEditor::from_seed(&seed))
    }
}

/// Parse a raw-text JSON buffer, enforcing only that it is syntactically valid
/// JSON — no required-property check. Required-ness is enforced once, at Confirm
/// (form submit), so neither the save (Ctrl-S) nor the view toggle (Ctrl-R)
/// blocks on an incomplete-but-valid edit.
fn parse_raw_value(buffer: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(buffer).map_err(|e| format!("invalid JSON: {e}"))
}

/// Rebuild a structured tree from a raw-text buffer. Enforces only JSON syntax:
/// an incomplete-but-valid edit is accepted, with any missing property showing
/// up unset in the tree. Required-property completeness is checked at Confirm,
/// not here — used by both the save path (Ctrl-S) and the view toggle (Ctrl-R).
/// Preserves edits losslessly modulo property ordering (governed by the schema's
/// `properties` map iteration order).
pub fn commit_raw_to_tree(
    buffer: &str,
    schema: &serde_json::Value,
    name: &str,
    required: bool,
) -> Result<Node, String> {
    let value = parse_raw_value(buffer)?;
    Ok(from_schema(name, schema, &value, required))
}

/// What the outer phase loop should do after applying a key event.
#[derive(Debug, PartialEq)]
pub enum EditorStep {
    /// Re-render and keep going.
    Continue,
    /// User pressed Ctrl-S; the editor produced this JSON value.
    Save(Value),
    /// User pressed Esc; discard any in-progress changes.
    Cancel,
    /// User pressed `r` to flip to raw-text mode (handled by Task 3.7).
    OpenRaw,
    /// User pressed Enter on a focused scalar; the outer loop should open
    /// the scalar editor on the node at this path. The tree editor itself
    /// stays out of scalar editing because that needs an input buffer +
    /// terminal handle the editor doesn't own.
    OpenScalar(NodePath),
}

/// Apply a key event to the editor state. Pure — no I/O — so the bulk of
/// the editor is unit-testable without a terminal.
pub fn dispatch_key(root: &mut Node, focus: &mut NodePath, key: KeyEvent) -> EditorStep {
    let visible = flatten(root);
    let focus_index = visible.iter().position(|p| p == focus).unwrap_or(0);

    match (key.code, key.modifiers) {
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => EditorStep::Save(root.to_value()),
        (KeyCode::Esc, _) => EditorStep::Cancel,

        (KeyCode::Up, _) => {
            if focus_index > 0 {
                *focus = visible[focus_index - 1].clone();
            }
            EditorStep::Continue
        }
        (KeyCode::Down, _) => {
            if focus_index + 1 < visible.len() {
                *focus = visible[focus_index + 1].clone();
            }
            EditorStep::Continue
        }

        (KeyCode::Right, _) => {
            if let Some(node) = get_mut(root, focus) {
                if matches!(node.kind, NodeKind::Object { .. } | NodeKind::Array { .. }) {
                    node.expanded = true;
                }
            }
            EditorStep::Continue
        }
        (KeyCode::Left, _) => {
            if let Some(node) = get_mut(root, focus) {
                if matches!(node.kind, NodeKind::Object { .. } | NodeKind::Array { .. }) {
                    node.expanded = false;
                }
            }
            EditorStep::Continue
        }

        (KeyCode::Char('+'), _) => {
            // Add an array entry. Focus on the array node itself appends;
            // focus on an entry within an array appends to that array
            // (insert-after positioning is left to raw mode for v1).
            let focused_kind = get(root, focus).map(|n| n.kind.clone());
            match focused_kind {
                Some(NodeKind::Array { .. }) => {
                    if let Some(arr) = get_mut(root, focus) {
                        let _ = arr.array_append_default();
                    }
                }
                Some(_) if !focus.is_empty() => {
                    let mut parent_path = focus.clone();
                    parent_path.pop();
                    if let Some(parent) = get_mut(root, &parent_path) {
                        if matches!(parent.kind, NodeKind::Array { .. }) {
                            let _ = parent.array_append_default();
                        }
                    }
                }
                _ => {}
            }
            EditorStep::Continue
        }

        (KeyCode::Char('-'), _) => {
            // Delete an array entry. Only valid when focus is on an entry.
            if !focus.is_empty() {
                let mut parent_path = focus.clone();
                let entry_index = parent_path.pop().unwrap();
                if let Some(parent) = get_mut(root, &parent_path) {
                    if parent.array_delete_at(entry_index).is_ok() {
                        *focus = parent_path;
                    }
                }
            }
            EditorStep::Continue
        }

        (KeyCode::Char('r'), m) if m.contains(KeyModifiers::CONTROL) => EditorStep::OpenRaw,

        (KeyCode::Enter, _) | (KeyCode::Char(' '), _) => {
            let is_space = matches!(key.code, KeyCode::Char(' '));
            let forward = !key.modifiers.contains(KeyModifiers::SHIFT);
            if let Some(node) = get_mut(root, focus) {
                if matches!(node.kind, NodeKind::Scalar { .. }) {
                    // Enter or Space toggles a bool / cycles an enum in place
                    // (matching the form's Space idiom). A text scalar opens the
                    // in-place editor on Enter; Space no-ops on text.
                    if node.schema.get("type").and_then(|t| t.as_str()) == Some("boolean") {
                        toggle_bool_node(node);
                        return EditorStep::Continue;
                    }
                    if let Some(variants) = enum_variants(&node.schema) {
                        cycle_enum_node(node, &variants, forward);
                        return EditorStep::Continue;
                    }
                    if is_space {
                        return EditorStep::Continue;
                    }
                    return EditorStep::OpenScalar(focus.clone());
                }
            }
            EditorStep::Continue
        }
        _ => EditorStep::Continue,
    }
}

/// Read a schema's `enum` array as owned strings, if present.
fn enum_variants(schema: &Value) -> Option<Vec<String>> {
    schema.get("enum").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    })
}

/// Toggle a bool scalar node's value (unset → true → false → true).
fn toggle_bool_node(node: &mut Node) {
    if let NodeKind::Scalar { value } = &mut node.kind {
        let cur = matches!(value, Some(ScalarValue::Boolean(true)));
        *value = Some(ScalarValue::Boolean(!cur));
    }
}

/// Cycle an enum scalar node to the next (or previous) variant, wrapping.
fn cycle_enum_node(node: &mut Node, variants: &[String], forward: bool) {
    if variants.is_empty() {
        return;
    }
    if let NodeKind::Scalar { value } = &mut node.kind {
        let cur = match value {
            Some(ScalarValue::Enum(s)) | Some(ScalarValue::String(s)) => {
                variants.iter().position(|v| v == s)
            }
            _ => None,
        };
        let next = match (cur, forward) {
            (Some(i), true) => (i + 1) % variants.len(),
            (Some(i), false) => (i + variants.len() - 1) % variants.len(),
            (None, _) => 0,
        };
        *value = Some(ScalarValue::Enum(variants[next].clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use node::{from_schema, NodeKind};

    /// Build a plain key-press event for `code`.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Build a Ctrl-modified key event for `code`.
    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn test_ctrl_s_saves_current_tree_as_json() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } }
        });
        let mut root = from_schema("root", &schema, &serde_json::json!({"a": "v"}), true);
        let mut focus = vec![];
        let step = dispatch_key(&mut root, &mut focus, ctrl(KeyCode::Char('s')));
        match step {
            EditorStep::Save(v) => assert_eq!(v, serde_json::json!({"a": "v"})),
            other => panic!("expected Save, got {other:?}"),
        }
    }

    #[test]
    fn test_esc_cancels_editor() {
        let schema = serde_json::json!({ "type": "string" });
        let mut root = from_schema("name", &schema, &serde_json::Value::Null, false);
        let mut focus = vec![];
        assert_eq!(
            dispatch_key(&mut root, &mut focus, key(KeyCode::Esc)),
            EditorStep::Cancel
        );
    }

    #[test]
    fn test_down_arrow_advances_focus() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" }, "b": { "type": "string" } }
        });
        let mut root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let mut focus = vec![];
        dispatch_key(&mut root, &mut focus, key(KeyCode::Down));
        assert_eq!(focus, vec![0]);
    }

    #[test]
    fn test_left_collapses_focused_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } }
        });
        let mut root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let mut focus = vec![];
        dispatch_key(&mut root, &mut focus, key(KeyCode::Left));
        assert!(!root.expanded);
    }

    #[test]
    fn test_plus_on_array_node_appends_default_entry() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let mut root = from_schema("items", &schema, &serde_json::Value::Null, false);
        let mut focus = vec![];
        dispatch_key(&mut root, &mut focus, key(KeyCode::Char('+')));
        let NodeKind::Array { children } = &root.kind else {
            panic!("expected array")
        };
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn test_minus_on_array_entry_deletes_and_focuses_parent_array() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let value = serde_json::json!(["a", "b"]);
        let mut root = from_schema("items", &schema, &value, false);
        let mut focus = vec![0];
        dispatch_key(&mut root, &mut focus, key(KeyCode::Char('-')));
        let NodeKind::Array { children } = &root.kind else {
            panic!("expected array")
        };
        assert_eq!(children.len(), 1);
        assert_eq!(focus, Vec::<usize>::new());
    }

    #[test]
    fn test_ctrl_r_opens_raw_mode() {
        let schema = serde_json::json!({ "type": "string" });
        let mut root = from_schema("name", &schema, &serde_json::Value::Null, false);
        let mut focus = vec![];
        // Ctrl-R toggles to raw (symmetric with Ctrl-R returning from raw); bare
        // `r` is a no-op in the tree.
        assert_eq!(
            dispatch_key(&mut root, &mut focus, key(KeyCode::Char('r'))),
            EditorStep::Continue
        );
        assert_eq!(
            dispatch_key(&mut root, &mut focus, ctrl(KeyCode::Char('r'))),
            EditorStep::OpenRaw
        );
    }

    #[test]
    fn test_enter_on_scalar_emits_open_scalar_with_focus_path() {
        let schema = serde_json::json!({ "type": "string" });
        let mut root = from_schema("name", &schema, &serde_json::Value::Null, false);
        let mut focus = vec![];
        match dispatch_key(&mut root, &mut focus, key(KeyCode::Enter)) {
            EditorStep::OpenScalar(p) => assert_eq!(p, Vec::<usize>::new()),
            other => panic!("expected OpenScalar, got {other:?}"),
        }
    }

    #[test]
    fn test_into_raw_seeds_buffer_from_tree_to_value() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } }
        });
        let root = from_schema("root", &schema, &serde_json::json!({"a": "v"}), true);
        let mode = EditorMode::into_raw(&root);
        match mode {
            EditorMode::Raw(editor) => {
                // Parsing the buffer must reproduce the tree's serialised value.
                let parsed: serde_json::Value = serde_json::from_str(&editor.to_text()).unwrap();
                assert_eq!(parsed, root.to_value());
            }
            EditorMode::Structured => panic!("expected Raw"),
        }
    }

    #[test]
    fn test_commit_raw_to_tree_returns_err_on_invalid_json() {
        // Syntactically broken JSON can't become a tree — save/toggle stay in raw
        // with the error.
        let schema = serde_json::json!({ "type": "object" });
        let err = commit_raw_to_tree("{not json", &schema, "root", true).unwrap_err();
        assert!(err.starts_with("invalid JSON"), "unexpected error: {err}");
    }

    #[test]
    fn test_commit_raw_to_tree_accepts_missing_required() {
        // Required-ness is enforced at Confirm, not at save/toggle: an incomplete
        // but syntactically-valid edit builds the tree, with `b` showing up unset.
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" }, "b": { "type": "string" } },
            "required": ["a", "b"]
        });
        let tree = commit_raw_to_tree(r#"{"a": "x"}"#, &schema, "root", true)
            .expect("save/toggle must not enforce required properties");
        assert_eq!(tree.to_value().get("a"), Some(&serde_json::json!("x")));
        // `b` is unset, so it is omitted from the value (Confirm will flag it).
        assert_eq!(tree.to_value().get("b"), None);
    }

    #[test]
    fn test_commit_raw_to_tree_accepts_valid_buffer_with_required_present() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } },
            "required": ["a"]
        });
        let tree = commit_raw_to_tree(r#"{"a": "x"}"#, &schema, "root", true).unwrap();
        assert_eq!(tree.to_value(), serde_json::json!({ "a": "x" }));
    }

    #[test]
    fn test_raw_mode_roundtrips_structured_tree() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" }, "b": { "type": "integer" } }
        });
        let original = serde_json::json!({ "a": "x", "b": 42 });
        let tree = from_schema("root", &schema, &original, true);
        let raw = serde_json::to_string_pretty(&tree.to_value()).unwrap();
        let tree2 = commit_raw_to_tree(&raw, &schema, "root", true).unwrap();
        assert_eq!(tree.to_value(), tree2.to_value());
    }

    #[test]
    fn test_enter_on_bool_scalar_toggles_in_place() {
        let schema = serde_json::json!({ "type": "boolean" });
        let mut root = from_schema("flag", &schema, &serde_json::json!(false), false);
        let mut focus = vec![];
        let step = dispatch_key(&mut root, &mut focus, key(KeyCode::Enter));
        assert_eq!(step, EditorStep::Continue, "bool toggle stays in the tree");
        assert_eq!(root.to_value(), serde_json::json!(true));
    }

    #[test]
    fn test_enter_on_enum_scalar_cycles_in_place() {
        let schema = serde_json::json!({ "type": "string", "enum": ["RED", "BLUE"] });
        let mut root = from_schema("color", &schema, &serde_json::json!("RED"), false);
        let mut focus = vec![];
        let step = dispatch_key(&mut root, &mut focus, key(KeyCode::Enter));
        assert_eq!(step, EditorStep::Continue, "enum cycle stays in the tree");
        assert_eq!(root.to_value(), serde_json::json!("BLUE"));
    }

    // Text-scalar Enter → OpenScalar is already covered by
    // `test_enter_on_scalar_emits_open_scalar_with_focus_path`.

    #[test]
    fn test_space_on_bool_scalar_toggles_in_place() {
        let schema = serde_json::json!({ "type": "boolean" });
        let mut root = from_schema("flag", &schema, &serde_json::json!(false), false);
        let mut focus = vec![];
        let step = dispatch_key(&mut root, &mut focus, key(KeyCode::Char(' ')));
        assert_eq!(step, EditorStep::Continue, "space toggles bool in place");
        assert_eq!(root.to_value(), serde_json::json!(true));
    }

    #[test]
    fn test_shift_enter_on_enum_cycles_backward() {
        let schema = serde_json::json!({ "type": "string", "enum": ["RED", "GREEN", "BLUE"] });
        let mut root = from_schema("color", &schema, &serde_json::json!("RED"), false);
        let mut focus = vec![];
        let step = dispatch_key(
            &mut root,
            &mut focus,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        );
        assert_eq!(
            step,
            EditorStep::Continue,
            "shift-enter cycles enum backward"
        );
        assert_eq!(root.to_value(), serde_json::json!("BLUE"));
    }

    #[test]
    fn test_enter_on_object_is_no_op_continue() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" } }
        });
        let mut root = from_schema("root", &schema, &serde_json::Value::Null, true);
        let mut focus = vec![];
        assert_eq!(
            dispatch_key(&mut root, &mut focus, key(KeyCode::Enter)),
            EditorStep::Continue
        );
    }
}
