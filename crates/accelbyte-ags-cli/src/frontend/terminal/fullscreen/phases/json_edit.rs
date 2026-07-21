//! JSON-edit phase: the structured JSON body editor embedded in the main
//! region, so editing a `JsonBody` field keeps the surrounding header / step
//! strip / Summary / Navigation chrome rather than taking over the screen.

use std::cell::Cell;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Padding};
use ratatui::Frame;

use crate::frontend::terminal::inline::json_editor::navigation::NodePath;
use crate::frontend::terminal::inline::json_editor::node::Node;
use crate::frontend::terminal::inline::json_editor::render::{
    focused_description, render_tree_view,
};
use crate::frontend::terminal::inline::json_editor::{editor_title_line, EditorMode};
use crate::frontend::terminal::views::fields::render_hint_box;

/// The in-surface JSON editor for one `JsonBody` field.
pub struct JsonEditPanel {
    /// Field label shown in the panel title (e.g. `more body`).
    pub title: String,
    /// Editor tree built from the field's schema + current value.
    pub root: Node,
    /// Currently focused tree path.
    pub focus: NodePath,
    /// Structured tree view or raw-text mode.
    pub mode: EditorMode,
    /// `Some((path, buffer))` while editing a single scalar value in-panel.
    pub scalar: Option<(NodePath, String)>,
    /// Normalised value when the editor opened — baseline for the unsaved
    /// changes `●` in the title.
    pub baseline: serde_json::Value,
    /// Persisted scroll window for the tree view, so long bodies stay navigable.
    pub scroll_top: Cell<usize>,
}

impl JsonEditPanel {
    /// Draw the structured JSON request-body editor into `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        // The title names the mode so the nav bar doesn't have to: raw mode reads
        // `Edit raw JSON: <field>`, the tree reads `Edit: <field>`. A leading `●`
        // flags unsaved changes versus the value the editor opened with.
        let baseline_text = serde_json::to_string_pretty(&self.baseline)
            .unwrap_or_else(|_| self.baseline.to_string());
        let modified = match &self.mode {
            EditorMode::Raw(editor) if self.scalar.is_none() => editor.to_text() != baseline_text,
            _ => self.root.to_value() != self.baseline,
        };
        let label = if self.scalar.is_none() && matches!(self.mode, EditorMode::Raw(_)) {
            format!("Edit raw JSON: {}", self.title)
        } else {
            format!("Edit: {}", self.title)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .padding(Padding::new(2, 2, 1, 1))
            .title(editor_title_line(modified, &label));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // Scalar sub-edit: render the tree with the focused leaf editing in place.
        if let Some((path, buffer)) = &self.scalar {
            self.render_tree_with_hint(frame, inner, Some((path, buffer.as_str())));
            return;
        }

        match &self.mode {
            EditorMode::Structured => self.render_tree_with_hint(frame, inner, None),
            EditorMode::Raw(editor) => {
                // Shared with the inline surface (form_runner) so both raw modes
                // stay identical, including the real terminal cursor. Raw mode has
                // no descriptions, so the hint box only appears to carry a commit
                // error (red) — matching the form's error box. With no error the
                // editor fills the whole area.
                use crate::frontend::terminal::inline::json_editor::raw::render_raw;
                match editor.error() {
                    Some(err) => {
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([Constraint::Min(1), Constraint::Length(4)])
                            .split(inner);
                        render_raw(frame, chunks[0], editor);
                        render_hint_box(frame, chunks[1], Some(&(err.to_owned(), true)));
                    }
                    None => render_raw(frame, inner, editor),
                }
            }
        }
    }

    /// Render the structured tree with scroll-to-focus, plus the focused node's
    /// description in a bordered hint box below — matching the main form layout.
    fn render_tree_with_hint(
        &self,
        frame: &mut Frame,
        inner: Rect,
        editing: Option<(&NodePath, &str)>,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(4)])
            .split(inner);
        render_tree_view(
            frame,
            chunks[0],
            &self.root,
            &self.focus,
            editing,
            &self.scroll_top,
        );
        let hint = focused_description(&self.root, &self.focus).map(|d| (d, false));
        render_hint_box(frame, chunks[1], hint.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::terminal::inline::json_editor::node::from_schema;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_panel_renders_editing_leaf_in_place_not_lone_buffer() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        let root = from_schema("body", &schema, &serde_json::json!({"name": "old"}), false);
        let baseline = root.to_value();
        let panel = JsonEditPanel {
            title: "body".into(),
            root,
            focus: vec![0],
            mode: EditorMode::Structured,
            scalar: Some((vec![0], "new".into())),
            baseline,
            scroll_top: Cell::new(0),
        };
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // The tree label "name" is still visible (in-place), alongside the buffer.
        assert!(s.contains("name"), "tree stays visible during edit: {s}");
        assert!(s.contains("new"), "buffer content shown in place: {s}");
    }

    #[test]
    fn test_panel_raw_mode_renders_buffer_and_places_cursor() {
        use crate::frontend::terminal::inline::json_editor::raw::RawEditor;
        let schema = serde_json::json!({"type": "object"});
        let root = from_schema("body", &schema, &serde_json::Value::Null, false);
        let baseline = root.to_value();
        let panel = JsonEditPanel {
            title: "body".into(),
            root,
            focus: vec![],
            mode: EditorMode::Raw(RawEditor::from_seed("{\"a\":1}")),
            scalar: None,
            baseline,
            scroll_top: Cell::new(0),
        };
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
        // The raw buffer text is rendered (no `█` glyph — a real terminal cursor
        // is placed via `set_cursor_position` instead).
        let s: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("{\"a\":1}"), "raw buffer rendered: {s}");
        assert!(!s.contains('\u{2588}'), "no block-cursor glyph: {s}");
    }

    /// Render the panel to a string for assertions.
    fn render_to_string(panel: &JsonEditPanel) -> String {
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| panel.render(f, f.area())).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn test_panel_shows_focused_field_description_in_hint() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string", "description": "Who to greet" } }
        });
        let root = from_schema("body", &schema, &serde_json::Value::Null, false);
        let baseline = root.to_value();
        let panel = JsonEditPanel {
            title: "body".into(),
            root,
            focus: vec![0], // the "name" child
            mode: EditorMode::Structured,
            scalar: None,
            baseline,
            scroll_top: Cell::new(0),
        };
        let s = render_to_string(&panel);
        assert!(
            s.contains("Who to greet"),
            "focused field description must show in the hint box: {s}"
        );
    }

    #[test]
    fn test_panel_title_shows_modified_dot_when_changed() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        let root = from_schema("body", &schema, &serde_json::json!({"name": "new"}), false);
        // Baseline differs from the current tree → unsaved changes.
        let panel = JsonEditPanel {
            title: "body".into(),
            root,
            focus: vec![],
            mode: EditorMode::Structured,
            scalar: None,
            baseline: serde_json::json!({}),
            scroll_top: Cell::new(0),
        };
        let s = render_to_string(&panel);
        assert!(s.contains('\u{25CF}'), "modified dot present in title: {s}");
        assert!(s.contains("Edit: body"), "title text present: {s}");
    }

    #[test]
    fn test_panel_title_has_no_dot_when_unchanged() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        let root = from_schema("body", &schema, &serde_json::json!({"name": "new"}), false);
        let baseline = root.to_value();
        let panel = JsonEditPanel {
            title: "body".into(),
            root,
            focus: vec![],
            mode: EditorMode::Structured,
            scalar: None,
            baseline,
            scroll_top: Cell::new(0),
        };
        let s = render_to_string(&panel);
        assert!(
            !s.contains('\u{25CF}'),
            "no modified dot when unchanged: {s}"
        );
        assert!(s.contains("Edit: body"), "title text present: {s}");
    }
}
