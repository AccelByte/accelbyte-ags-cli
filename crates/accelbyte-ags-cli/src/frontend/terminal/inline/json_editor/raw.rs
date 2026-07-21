//! Raw-text JSON editor — a small in-house multiline text area used by both
//! the inline and fullscreen surfaces when the user flips the structured tree
//! editor into raw mode (`r`).
//!
//! The editor is a plain `Vec<String>` of logical lines plus a `(row, col)`
//! char-indexed cursor. All editing/movement is pure (no terminal, no ratatui),
//! so the bulk of the behaviour is unit-testable. [`render_raw`] draws the
//! buffer and places a real terminal cursor; [`dispatch`] maps a key event onto
//! an edit or a commit/cancel intent. Both surfaces share this one
//! implementation so their raw modes cannot drift apart.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// A multiline text buffer with a char-indexed cursor. Lines is never empty
/// (an empty buffer is a single empty line), so `lines[cursor.0]` is always
/// valid.
#[derive(Debug, Clone)]
pub struct RawEditor {
    lines: Vec<String>,
    /// `(row, col)` — both char indices (not byte offsets).
    cursor: (usize, usize),
    /// Last-attempted commit error, rendered in red below the buffer.
    error: Option<String>,
}

impl RawEditor {
    /// Build an editor seeded from `text` (typically pretty-printed JSON),
    /// cursor at the start.
    pub fn from_seed(text: &str) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(str::to_owned).collect()
        };
        RawEditor {
            lines,
            cursor: (0, 0),
            error: None,
        }
    }

    /// Serialise the buffer back to text (lines joined with `\n`).
    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }

    /// Record a commit error to surface in the hint box below the buffer.
    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    /// The last-attempted commit error, if any. The caller renders it in the
    /// shared hint box so raw mode matches the form's error presentation.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    // ── editing ──────────────────────────────────────────────────────────────

    /// Insert `c` at the cursor and advance one column.
    pub fn insert_char(&mut self, c: char) {
        let (row, col) = self.cursor;
        let byte = char_to_byte(&self.lines[row], col);
        self.lines[row].insert(byte, c);
        self.cursor.1 = col + 1;
        self.error = None;
    }

    /// Insert each character of `s` at the cursor.
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    /// Split the current line at the cursor, moving the tail onto a new line below.
    pub fn insert_newline(&mut self) {
        let (row, col) = self.cursor;
        let byte = char_to_byte(&self.lines[row], col);
        let tail = self.lines[row].split_off(byte);
        self.lines.insert(row + 1, tail);
        self.cursor = (row + 1, 0);
        self.error = None;
    }

    /// Delete the character before the cursor, joining with the previous line
    /// when at the start of a line.
    pub fn backspace(&mut self) {
        let (row, col) = self.cursor;
        if col > 0 {
            let byte = char_to_byte(&self.lines[row], col - 1);
            self.lines[row].remove(byte);
            self.cursor.1 = col - 1;
        } else if row > 0 {
            let prev_len = char_len(&self.lines[row - 1]);
            let current = self.lines.remove(row);
            self.lines[row - 1].push_str(&current);
            self.cursor = (row - 1, prev_len);
        }
        self.error = None;
    }

    /// Delete the character at the cursor, joining the next line when at the
    /// end of a line.
    pub fn delete(&mut self) {
        let (row, col) = self.cursor;
        if col < char_len(&self.lines[row]) {
            let byte = char_to_byte(&self.lines[row], col);
            self.lines[row].remove(byte);
        } else if row + 1 < self.lines.len() {
            let next = self.lines.remove(row + 1);
            self.lines[row].push_str(&next);
        }
        self.error = None;
    }

    // ── movement ─────────────────────────────────────────────────────────────

    /// Move the cursor one column left, wrapping to the end of the previous line.
    pub fn move_left(&mut self) {
        let (row, col) = self.cursor;
        if col > 0 {
            self.cursor.1 = col - 1;
        } else if row > 0 {
            self.cursor = (row - 1, char_len(&self.lines[row - 1]));
        }
    }

    /// Move the cursor one column right, wrapping to the start of the next line.
    pub fn move_right(&mut self) {
        let (row, col) = self.cursor;
        if col < char_len(&self.lines[row]) {
            self.cursor.1 = col + 1;
        } else if row + 1 < self.lines.len() {
            self.cursor = (row + 1, 0);
        }
    }

    /// Move the cursor up one line, clamping the column to the line length.
    pub fn move_up(&mut self) {
        let (row, col) = self.cursor;
        if row > 0 {
            self.cursor = (row - 1, col.min(char_len(&self.lines[row - 1])));
        }
    }

    /// Move the cursor down one line, clamping the column to the line length.
    pub fn move_down(&mut self) {
        let (row, col) = self.cursor;
        if row + 1 < self.lines.len() {
            self.cursor = (row + 1, col.min(char_len(&self.lines[row + 1])));
        }
    }

    /// Move the cursor to the start of the current line.
    pub fn move_home(&mut self) {
        self.cursor.1 = 0;
    }

    /// Move the cursor to the end of the current line.
    pub fn move_end(&mut self) {
        self.cursor.1 = char_len(&self.lines[self.cursor.0]);
    }
}

/// Byte offset of char index `col` within `line` (clamped to the line end).
fn char_to_byte(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

/// Number of characters (not bytes) in `line`.
fn char_len(line: &str) -> usize {
    line.chars().count()
}

/// What the editor loop should do after a key in raw mode.
#[derive(Debug, PartialEq)]
pub enum RawStep {
    /// Re-render and keep editing.
    Continue,
    /// Ctrl-S — caller should parse [`RawEditor::to_text`] and save the whole
    /// edit (exit the editor).
    Commit,
    /// Esc — caller should cancel the whole edit (exit the editor).
    Cancel,
    /// Ctrl-R — caller should parse the buffer back into the tree and switch to
    /// the structured view (staying in the editor).
    ToTree,
}

/// Map a key event onto an edit or a save/cancel/switch intent. `r` is an
/// ordinary character here (unlike the tree view, where bare `r` opens raw
/// mode), so raw JSON containing the letter can be typed; Ctrl-R switches back
/// to the tree.
pub fn dispatch(editor: &mut RawEditor, key: KeyEvent) -> RawStep {
    match (key.code, key.modifiers) {
        (KeyCode::Char('s'), m) if m.contains(KeyModifiers::CONTROL) => RawStep::Commit,
        (KeyCode::Char('r'), m) if m.contains(KeyModifiers::CONTROL) => RawStep::ToTree,
        (KeyCode::Esc, _) => RawStep::Cancel,
        (KeyCode::Enter, _) => {
            editor.insert_newline();
            RawStep::Continue
        }
        (KeyCode::Backspace, _) => {
            editor.backspace();
            RawStep::Continue
        }
        (KeyCode::Delete, _) => {
            editor.delete();
            RawStep::Continue
        }
        (KeyCode::Left, _) => {
            editor.move_left();
            RawStep::Continue
        }
        (KeyCode::Right, _) => {
            editor.move_right();
            RawStep::Continue
        }
        (KeyCode::Up, _) => {
            editor.move_up();
            RawStep::Continue
        }
        (KeyCode::Down, _) => {
            editor.move_down();
            RawStep::Continue
        }
        (KeyCode::Home, _) => {
            editor.move_home();
            RawStep::Continue
        }
        (KeyCode::End, _) => {
            editor.move_end();
            RawStep::Continue
        }
        (KeyCode::Tab, _) => {
            editor.insert_str("  ");
            RawStep::Continue
        }
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
            editor.insert_char(c);
            RawStep::Continue
        }
        _ => RawStep::Continue,
    }
}

/// Render the buffer into `area` and place the real terminal cursor at the
/// editing position. A dim line-number gutter precedes each line so commit
/// errors (which report `line N column M`) are locatable. The buffer fills the
/// whole `area`; the caller renders any commit error in the shared hint box
/// below (see [`RawEditor::error`]) so raw mode matches the form's error box.
/// Vertical scroll keeps the cursor visible; horizontal scroll is out of scope
/// for v1 (pretty-printed JSON lines are short), so cursor x clamps to the edge.
pub fn render_raw(frame: &mut Frame, area: Rect, editor: &RawEditor) {
    let text_area = area;
    let height = text_area.height as usize;

    // Scroll so the cursor row stays visible: keep the top until the cursor
    // passes the last visible row, then pin the cursor to the bottom row.
    let scroll = if height == 0 || editor.cursor.0 < height {
        0
    } else {
        editor.cursor.0 - height + 1
    };
    let last = (scroll + height).min(editor.lines.len());

    // Gutter is wide enough for the highest line number plus a trailing space.
    let digits = editor.lines.len().to_string().len();
    let gutter_width = digits + 1;
    let gutter_style = Style::default().fg(Color::Indexed(244));
    let rows: Vec<Line> = editor.lines[scroll..last]
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let lineno = scroll + i + 1;
            Line::from(vec![
                Span::styled(format!("{lineno:>digits$} "), gutter_style),
                Span::raw(line.clone()),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), text_area);

    if height > 0 {
        let rel_row = (editor.cursor.0 - scroll) as u16;
        let max_x = text_area.width.saturating_sub(1);
        let col = (gutter_width as u16 + editor.cursor.1 as u16).min(max_x);
        frame.set_cursor_position((text_area.x + col, text_area.y + rel_row));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    /// Build a plain key-press event for `code`.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_from_seed_empty_is_single_empty_line() {
        let ed = RawEditor::from_seed("");
        assert_eq!(ed.to_text(), "");
        assert_eq!(ed.cursor, (0, 0));
    }

    #[test]
    fn test_from_seed_splits_lines() {
        let ed = RawEditor::from_seed("{\n  \"a\": 1\n}");
        assert_eq!(ed.lines.len(), 3);
        assert_eq!(ed.to_text(), "{\n  \"a\": 1\n}");
    }

    #[test]
    fn test_insert_char_in_middle_of_line() {
        let mut ed = RawEditor::from_seed("ac");
        ed.cursor = (0, 1);
        ed.insert_char('b');
        assert_eq!(ed.to_text(), "abc");
        assert_eq!(ed.cursor, (0, 2));
    }

    #[test]
    fn test_insert_newline_splits_line() {
        let mut ed = RawEditor::from_seed("ab");
        ed.cursor = (0, 1);
        ed.insert_newline();
        assert_eq!(ed.to_text(), "a\nb");
        assert_eq!(ed.cursor, (1, 0));
    }

    #[test]
    fn test_backspace_mid_line_deletes_previous_char() {
        let mut ed = RawEditor::from_seed("abc");
        ed.cursor = (0, 2);
        ed.backspace();
        assert_eq!(ed.to_text(), "ac");
        assert_eq!(ed.cursor, (0, 1));
    }

    #[test]
    fn test_backspace_at_line_start_joins_previous_line() {
        let mut ed = RawEditor::from_seed("a\nb");
        ed.cursor = (1, 0);
        ed.backspace();
        assert_eq!(ed.to_text(), "ab");
        assert_eq!(ed.cursor, (0, 1));
    }

    #[test]
    fn test_delete_at_line_end_joins_next_line() {
        let mut ed = RawEditor::from_seed("a\nb");
        ed.cursor = (0, 1);
        ed.delete();
        assert_eq!(ed.to_text(), "ab");
        assert_eq!(ed.cursor, (0, 1));
    }

    #[test]
    fn test_arrow_movement_wraps_across_lines() {
        let mut ed = RawEditor::from_seed("ab\ncd");
        ed.cursor = (0, 2); // end of first line
        ed.move_right(); // wraps to start of next line
        assert_eq!(ed.cursor, (1, 0));
        ed.move_left(); // wraps back to end of first line
        assert_eq!(ed.cursor, (0, 2));
    }

    #[test]
    fn test_move_down_clamps_column_to_shorter_line() {
        let mut ed = RawEditor::from_seed("abcd\nxy");
        ed.cursor = (0, 4);
        ed.move_down();
        assert_eq!(ed.cursor, (1, 2));
    }

    #[test]
    fn test_dispatch_r_is_typed_not_an_exit() {
        let mut ed = RawEditor::from_seed("");
        assert_eq!(
            dispatch(&mut ed, key(KeyCode::Char('r'))),
            RawStep::Continue
        );
        assert_eq!(ed.to_text(), "r");
    }

    #[test]
    fn test_dispatch_ctrl_r_switches_to_tree() {
        let mut ed = RawEditor::from_seed("{}");
        assert_eq!(
            dispatch(
                &mut ed,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)
            ),
            RawStep::ToTree
        );
        // The buffer is untouched — switching is the caller's job.
        assert_eq!(ed.to_text(), "{}");
    }

    #[test]
    fn test_dispatch_ctrl_s_commits_and_esc_cancels() {
        let mut ed = RawEditor::from_seed("{}");
        assert_eq!(
            dispatch(
                &mut ed,
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
            ),
            RawStep::Commit
        );
        assert_eq!(dispatch(&mut ed, key(KeyCode::Esc)), RawStep::Cancel);
    }

    #[test]
    fn test_dispatch_tab_inserts_two_spaces() {
        let mut ed = RawEditor::from_seed("");
        dispatch(&mut ed, key(KeyCode::Tab));
        assert_eq!(ed.to_text(), "  ");
        assert_eq!(ed.cursor, (0, 2));
    }

    #[test]
    fn test_render_raw_shows_line_number_gutter() {
        use ratatui::{backend::TestBackend, Terminal};
        let ed = RawEditor::from_seed("{\n  \"a\": 1\n}");
        let mut term = Terminal::new(TestBackend::new(40, 8)).unwrap();
        term.draw(|f| render_raw(f, f.area(), &ed)).unwrap();
        let rows: Vec<String> = {
            let buf = term.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect()
        };
        // Line numbers 1..3 precede their content.
        assert!(rows[0].starts_with("1 {"), "row 0: {:?}", rows[0]);
        assert!(rows[1].starts_with("2   \"a\": 1"), "row 1: {:?}", rows[1]);
        assert!(rows[2].starts_with("3 }"), "row 2: {:?}", rows[2]);
    }
}
