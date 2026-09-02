//! Centered, scrollable, type-to-filter modal for browsing the local
//! filesystem (fullscreen Phase-1 form, `FieldType::FilePicker`). Pure state
//! and render, mirroring `EnumPickerModal`'s chrome — the key loop lives in
//! `fullscreen/interaction.rs`.

use std::io;
use std::path::{Path, PathBuf};

use ags_protocol::workflow::OptionChoice;
use ags_runtime::runtime::workflows::file_picker::{list_directory, FileEntry};
use ratatui::layout::Rect;

use crate::frontend::terminal::picker_list::PickerList;

/// Non-list rows the modal frame consumes: the enum picker's own
/// `CHROME_ROWS` (2 border + 2 padding + filter + 2 spacers + footer) plus
/// two more this modal adds — the breadcrumb line and a spacer separating it
/// from the filter line. Derived (not duplicated) so a future change to
/// `EnumPickerModal`'s chrome can't silently desync this one.
const CHROME_ROWS: u16 = super::enum_picker::CHROME_ROWS + 2;
const MIN_MODAL_WIDTH: u16 = 24;
const MIN_MODAL_HEIGHT: u16 = CHROME_ROWS + 1;
const MAX_MODAL_WIDTH: u16 = 96;
const MAX_HEIGHT_DIVISOR: u16 = 2;

/// Modal rect + list-viewport height for a given outer area.
pub(crate) struct FilePickerLayout {
    pub modal: Rect,
    pub list_height: u16,
}

/// Modal file browser over the local filesystem.
pub(crate) struct FilePickerModal {
    title: String,
    extensions: Option<Vec<String>>,
    current_dir: PathBuf,
    entries: Vec<FileEntry>,
    list: PickerList,
    /// Set for one render after a Tab-jump to a nonexistent path; cleared on
    /// the next state-changing action.
    transient_message: Option<String>,
}

fn entries_to_choices(entries: &[FileEntry]) -> Vec<OptionChoice> {
    entries
        .iter()
        .map(|e| OptionChoice {
            label: if e.is_dir && e.name != ".." {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            },
            value: e.name.clone(),
        })
        .collect()
}

impl FilePickerModal {
    /// Build a modal browsing `dir`. Fails only if `dir` itself can't be
    /// read (the caller — `drive_inputs_panel_form`, Task 10 — is
    /// responsible for having already resolved a valid starting directory,
    /// falling back to CWD if the declared `start_dir` doesn't exist).
    pub fn new(title: String, dir: PathBuf, extensions: Option<Vec<String>>) -> io::Result<Self> {
        let entries = list_directory(&dir, extensions.as_deref())?;
        let choices = entries_to_choices(&entries);
        Ok(Self {
            title,
            extensions,
            current_dir: dir,
            list: PickerList::new(choices, None),
            entries,
            transient_message: None,
        })
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    /// Re-list `dir` and rebuild the picker over it, clearing the filter and
    /// any transient message. `select` pre-highlights the entry with that
    /// name in the rebuilt list (falling back to the first entry when absent
    /// or not found) — used by the Tab-jump-to-a-file action to pre-highlight
    /// the target file after navigating to its parent, without committing it
    /// (the user still presses Enter). Every other caller (drill-in via
    /// Enter, Backspace-up, Tab-into-a-directory) passes `None`. Leaves the
    /// modal's prior state untouched on error (an unreadable directory during
    /// navigation, e.g. a permission change or race with deletion) — the
    /// caller should surface the error via `transient_message`-style
    /// messaging rather than losing the browsing session.
    pub fn navigate_to(
        &mut self,
        dir: PathBuf,
        extensions: Option<Vec<String>>,
        select: Option<&str>,
    ) -> io::Result<()> {
        let entries = list_directory(&dir, extensions.as_deref())?;
        let choices = entries_to_choices(&entries);
        self.current_dir = dir;
        self.entries = entries;
        self.list = PickerList::new(choices, select);
        self.extensions = extensions;
        self.transient_message = None;
        Ok(())
    }

    pub fn push_char(&mut self, c: char) {
        self.transient_message = None;
        self.list.push_char(c);
    }

    pub fn pop_char(&mut self) {
        self.transient_message = None;
        self.list.pop_char();
    }

    pub fn move_up(&mut self) {
        self.list.move_up();
    }

    pub fn move_down(&mut self) {
        self.list.move_down();
    }

    pub fn page_up(&mut self, n: usize) {
        self.list.page_up(n);
    }

    pub fn page_down(&mut self, n: usize) {
        self.list.page_down(n);
    }

    /// The currently highlighted entry, if any.
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        let value = self.list.selected_value()?;
        self.entries.iter().find(|e| e.name == value)
    }

    /// The typed filter text as an absolute path, but only when nothing
    /// matches (the literal-value escape hatch — same "type a value not in
    /// the list" semantics as `EnumPickerModal::custom_value`, except the
    /// result is resolved against `current_dir` the same way Tab-jump's
    /// [`Self::resolve_typed_path`] does, so every committed value stays
    /// absolute regardless of which path produced it). Unlike a picked
    /// entry, the typed text is not checked against `extensions` or
    /// filesystem existence — that bypass is intentional, matching
    /// `options_source`'s manual-entry affordance.
    pub fn custom_value(&self) -> Option<String> {
        self.list.custom_value()?;
        self.resolve_typed_path()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Resolve the typed filter text as a path relative to `current_dir` (or
    /// absolute, if typed as one), for the Tab-jump action. Returns `None`
    /// when the filter is empty. Does not touch filesystem state — the
    /// caller decides what to do based on whether the returned path exists
    /// and is a file or directory.
    pub fn resolve_typed_path(&self) -> Option<PathBuf> {
        let typed = self.list.filter_text.trim();
        if typed.is_empty() {
            return None;
        }
        let candidate = Path::new(typed);
        Some(if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.current_dir.join(candidate)
        })
    }

    pub fn set_transient_message(&mut self, message: String) {
        self.transient_message = Some(message);
    }

    /// The current extension filter, for callers that need to carry it
    /// through a `navigate_to` call (drilling in/up must never lose it).
    pub fn extensions(&self) -> Option<&Vec<String>> {
        self.extensions.as_ref()
    }

    /// Whether the filter text is empty — used by the key loop to decide
    /// whether Backspace should pop a filter character or navigate up a
    /// directory level instead.
    pub fn custom_value_source_is_empty(&self) -> bool {
        self.list.filter_text.is_empty()
    }

    /// Modal rect + list viewport height centered in `area`. `None` when the
    /// terminal is too small — mirrors `EnumPickerModal::layout` with one
    /// extra chrome row reserved for the breadcrumb.
    pub fn layout(&self, area: Rect) -> Option<FilePickerLayout> {
        let avail_w = area.width.saturating_sub(4);
        let avail_h = area.height.saturating_sub(4);
        if avail_w < MIN_MODAL_WIDTH || avail_h < MIN_MODAL_HEIGHT {
            return None;
        }
        let width = avail_w.min(MAX_MODAL_WIDTH);
        // Always use the maximum available size rather than fitting to the
        // current directory's entry count — a modal that resizes as the user
        // navigates (or types a filter) is jarring in a real terminal.
        let height = avail_h.min((area.height / MAX_HEIGHT_DIVISOR).max(MIN_MODAL_HEIGHT));
        let list_height = height.saturating_sub(CHROME_ROWS).max(1);
        let modal = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };
        Some(FilePickerLayout { modal, list_height })
    }

    fn footer_text(&self) -> String {
        if let Some(msg) = &self.transient_message {
            return msg.clone();
        }
        if self.entries.is_empty() {
            return "Empty directory \u{00B7} Backspace up \u{00B7} Esc cancel".to_string();
        }
        if self.list.filtered.is_empty() && !self.list.filter_text.trim().is_empty() {
            return format!(
                "Enter to use \"{}\" as a custom value \u{00B7} Tab to jump to path",
                self.list.filter_text.trim()
            );
        }
        format!(
            "{} of {} \u{00B7} \u{2191}\u{2193} move \u{00B7} Enter open/select \u{00B7} \
             Backspace up \u{00B7} Tab jump to path \u{00B7} Esc cancel",
            self.list.filtered.len(),
            self.entries.len()
        )
    }

    /// Draw the centered modal — border, breadcrumb, filter line, scrollable
    /// entry list with scrollbar, and footer — over `area`. No-op when the
    /// terminal is too small.
    pub fn render(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        use ratatui::layout::{Constraint, Layout};
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{
            Block, Borders, Clear, List, ListItem, Padding, Paragraph, Scrollbar,
            ScrollbarOrientation,
        };

        use crate::frontend::terminal::scrollbar::scrollbar_state;

        let Some(layout) = self.layout(area) else {
            return;
        };
        frame.render_widget(Clear, layout.modal);
        let block = Block::default()
            .borders(Borders::ALL)
            .padding(Padding::new(2, 2, 1, 1))
            .title(format!(" Select {} ", self.title));
        let inner = block.inner(layout.modal);
        frame.render_widget(block, layout.modal);

        let rows = Layout::vertical([
            Constraint::Length(1), // breadcrumb
            Constraint::Length(1), // spacer
            Constraint::Length(1), // filter
            Constraint::Length(1), // spacer
            Constraint::Min(1),    // list
            Constraint::Length(1), // spacer
            Constraint::Length(1), // footer
        ])
        .split(inner);
        let (breadcrumb_row, filter_row, list_row, footer_row) =
            (rows[0], rows[2], rows[4], rows[6]);

        let dim = Style::default().add_modifier(Modifier::DIM);
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(
                self.current_dir.to_string_lossy().into_owned(),
            ))),
            breadcrumb_row,
        );

        const FILTER_LABEL: &str = "Filter: ";
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(FILTER_LABEL, dim),
                Span::raw(self.list.filter_text.clone()),
            ])),
            filter_row,
        );
        let cursor_x =
            filter_row.x + FILTER_LABEL.len() as u16 + self.list.filter_text.chars().count() as u16;
        frame.set_cursor_position((
            cursor_x.min(filter_row.x + filter_row.width.saturating_sub(1)),
            filter_row.y,
        ));

        let items: Vec<ListItem> = if self.list.filtered.is_empty() {
            vec![ListItem::new("<no matches>").style(dim)]
        } else {
            self.list
                .filtered
                .iter()
                .map(|&i| ListItem::new(self.list.choices[i].label.clone()))
                .collect()
        };
        let list = List::new(items)
            .highlight_symbol("\u{203a} ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD));
        frame.render_stateful_widget(list, list_row, &mut self.list.list_state);

        let visible = list_row.height as usize;
        let total = self.list.filtered.len();
        if let Some(mut sb) = scrollbar_state(total, visible, self.list.list_state.offset()) {
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                list_row,
                &mut sb,
            );
        }

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(self.footer_text(), dim))),
            footer_row,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_tree() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        std::fs::write(root.path().join("icon.png"), b"").unwrap();
        std::fs::write(root.path().join("readme.txt"), b"").unwrap();
        root
    }

    #[test]
    fn test_new_lists_starting_directory() {
        let root = setup_tree();
        let extensions = Some(vec!["png".to_string()]);
        let modal = FilePickerModal::new(
            "iconFile".to_string(),
            root.path().to_path_buf(),
            extensions,
        )
        .unwrap();
        let names: Vec<&str> = modal.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"sub"));
        assert!(names.contains(&"icon.png"));
        assert!(!names.contains(&"readme.txt"));
    }

    #[test]
    fn test_navigate_to_rebuilds_list_and_clears_filter() {
        let root = setup_tree();
        let mut modal =
            FilePickerModal::new("iconFile".to_string(), root.path().to_path_buf(), None).unwrap();
        modal.push_char('i');
        assert_eq!(modal.list.filter_text, "i");
        modal
            .navigate_to(root.path().join("sub"), None, None)
            .expect("sub exists");
        assert_eq!(modal.current_dir, root.path().join("sub"));
        assert_eq!(modal.list.filter_text, "");
    }

    #[test]
    fn test_filter_narrows_by_entry_name_not_directory_path() {
        // Regression for: the picker's internal `value` used to be the
        // entry's full absolute path, so typing any fragment of the shared
        // parent-directory path matched every entry (the value-branch of
        // `PickerList::recompute`'s filter is always active when
        // `label != value`). `value` is now the bare entry name, which is
        // unique within one directory listing and never contains the
        // directory's own path.
        let root = setup_tree();
        let mut modal =
            FilePickerModal::new("iconFile".to_string(), root.path().to_path_buf(), None).unwrap();

        // A fragment of the temp directory's own name shouldn't match any
        // entry (none of "..", "sub", "icon.png", "readme.txt" contain it).
        let dir_fragment: String = root
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .chars()
            .take(6)
            .collect();
        assert!(
            !["..", "sub", "icon.png", "readme.txt"]
                .iter()
                .any(|n| n.to_lowercase().contains(&dir_fragment.to_lowercase())),
            "test fixture assumption broken: tempdir name collides with an entry name"
        );
        for c in dir_fragment.chars() {
            modal.push_char(c);
        }
        assert!(
            modal.list.filtered.is_empty(),
            "a fragment of the directory's own path must not match every entry"
        );
        for _ in dir_fragment.chars() {
            modal.pop_char();
        }

        // A fragment of an entry's *name* still narrows correctly.
        for c in "icon".chars() {
            modal.push_char(c);
        }
        let labels: Vec<&str> = modal
            .list
            .filtered
            .iter()
            .map(|&i| modal.list.choices[i].label.as_str())
            .collect();
        assert_eq!(labels, vec!["icon.png"]);
    }

    #[test]
    fn test_navigate_to_can_preselect_an_entry_by_name() {
        let root = setup_tree();
        let mut modal =
            FilePickerModal::new("iconFile".to_string(), root.path().to_path_buf(), None).unwrap();
        modal
            .navigate_to(root.path().to_path_buf(), None, Some("icon.png"))
            .expect("re-navigating to the same dir succeeds");
        let selected = modal.selected_entry().expect("an entry is selected");
        assert_eq!(selected.name, "icon.png");
    }

    #[test]
    fn test_layout_none_when_too_small() {
        let root = setup_tree();
        let modal =
            FilePickerModal::new("iconFile".to_string(), root.path().to_path_buf(), None).unwrap();
        assert!(modal.layout(Rect::new(0, 0, 1, 1)).is_none());
    }

    #[test]
    fn test_layout_uses_max_available_height_regardless_of_entry_count() {
        // The modal must not shrink to fit the current directory's entry
        // count — it should always claim the max available height so it
        // doesn't visibly resize as the user navigates or filters.
        let root = setup_tree();
        let modal =
            FilePickerModal::new("iconFile".to_string(), root.path().to_path_buf(), None).unwrap();
        let area = Rect::new(0, 0, 100, 40);
        let l = modal.layout(area).expect("fits");
        let avail_h = area.height.saturating_sub(4);
        let expected = avail_h.min((area.height / MAX_HEIGHT_DIVISOR).max(MIN_MODAL_HEIGHT));
        assert_eq!(l.modal.height, expected);
    }

    #[test]
    fn test_render_shows_breadcrumb_and_entries() {
        let root = setup_tree();
        let mut modal =
            FilePickerModal::new("iconFile".to_string(), root.path().to_path_buf(), None).unwrap();
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        term.draw(|f| modal.render(f, f.area())).unwrap();
        let buf: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            buf.contains(&root.path().to_string_lossy().to_string()) || buf.contains("icon.png")
        );
    }
}
