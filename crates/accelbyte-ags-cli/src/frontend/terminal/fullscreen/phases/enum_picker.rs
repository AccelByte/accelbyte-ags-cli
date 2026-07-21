//! Centered, scrollable, type-to-filter modal picker for a resolved
//! `DynamicEnum` field (fullscreen Phase-1 form). Pure state + render; the key
//! loop lives in `fullscreen/interaction.rs`.

use crate::frontend::terminal::picker_list::PickerList;
use ags_protocol::workflow::OptionChoice;
use ratatui::layout::Rect;

/// Non-list rows the modal frame consumes: top + bottom border, top + bottom
/// inner padding, the filter line, a spacer below the filter, and a spacer above
/// the footer, plus the footer itself (2 + 2 + 1 + 1 + 1 + 1 = 8).
const CHROME_ROWS: u16 = 8;
const MIN_MODAL_WIDTH: u16 = 24;
const MIN_MODAL_HEIGHT: u16 = CHROME_ROWS + 1; // at least one list row
const MAX_MODAL_WIDTH: u16 = 96;
/// The modal never grows past this fraction of the screen height — a long list
/// scrolls inside the cap rather than filling the terminal. Expressed as a
/// divisor: `2` ⇒ at most half the screen.
const MAX_HEIGHT_DIVISOR: u16 = 2;

/// Modal rect + list-viewport height for a given outer area.
pub(crate) struct EnumPickerLayout {
    pub modal: Rect,
    pub list_height: u16,
}

/// Modal picker over a resolved, non-empty choice list.
pub(crate) struct EnumPickerModal {
    title: String,
    truncated: bool,
    list: PickerList,
}

impl EnumPickerModal {
    /// Build a picker over `choices`, preselecting `current_value` (falling back
    /// to the first entry) and starting with an empty filter.
    pub fn new(
        title: String,
        choices: Vec<OptionChoice>,
        truncated: bool,
        current_value: Option<&str>,
    ) -> Self {
        Self {
            title,
            truncated,
            list: PickerList::new(choices, current_value),
        }
    }

    /// Append a character to the filter and refilter.
    pub fn push_char(&mut self, c: char) {
        self.list.push_char(c);
    }

    /// Delete the last filter character and refilter.
    pub fn pop_char(&mut self) {
        self.list.pop_char();
    }

    /// Move the highlight up one row.
    pub fn move_up(&mut self) {
        self.list.move_up();
    }

    /// Move the highlight down one row.
    pub fn move_down(&mut self) {
        self.list.move_down();
    }

    /// Move the highlight up `n` rows (at least one).
    pub fn page_up(&mut self, n: usize) {
        self.list.page_up(n);
    }

    /// Move the highlight down `n` rows (at least one).
    pub fn page_down(&mut self, n: usize) {
        self.list.page_down(n);
    }

    /// The highlighted choice's `value`, or `None` when nothing matches.
    pub fn selected_value(&self) -> Option<String> {
        self.list.selected_value()
    }

    /// The trimmed filter text, but only when nothing matches (the free-text
    /// escape hatch). `None` otherwise.
    pub fn custom_value(&self) -> Option<String> {
        self.list.custom_value()
    }

    /// Modal rect + list viewport height centered in `area`. `None` when the
    /// terminal is too small to host a usable modal — the caller falls back to
    /// inline free-text edit. All arithmetic saturates (`u16`).
    pub fn layout(&self, area: Rect) -> Option<EnumPickerLayout> {
        let avail_w = area.width.saturating_sub(4);
        let avail_h = area.height.saturating_sub(4);
        if avail_w < MIN_MODAL_WIDTH || avail_h < MIN_MODAL_HEIGHT {
            return None;
        }
        // The guard above ensures `avail_w >= MIN_MODAL_WIDTH` and
        // `avail_h >= MIN_MODAL_HEIGHT`, so only the upper bound needs applying.
        let width = avail_w.min(MAX_MODAL_WIDTH);
        // Cap the height at a fraction of the *screen* (never below the minimum,
        // never above what's available), then size to the list within that cap.
        // Size to the *total* choice count, not the filtered count, so the modal
        // height stays fixed while the user types in the filter.
        let cap = avail_h.min((area.height / MAX_HEIGHT_DIVISOR).max(MIN_MODAL_HEIGHT));
        let desired = (self.list.choices.len() as u16).saturating_add(CHROME_ROWS);
        let height = desired.clamp(MIN_MODAL_HEIGHT, cap);
        let list_height = height.saturating_sub(CHROME_ROWS).max(1);
        let modal = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };
        Some(EnumPickerLayout { modal, list_height })
    }

    /// Build the footer hint line: the match count plus the key legend, or the
    /// custom-value prompt when the filter matches nothing.
    fn footer_text(&self) -> String {
        if self.list.choices.is_empty() && self.list.filter_text.trim().is_empty() {
            return "No matches \u{00B7} type a value or Esc to cancel".to_string();
        }
        if self.list.filtered.is_empty() && !self.list.filter_text.trim().is_empty() {
            return format!(
                "Enter to use \"{}\" as a custom value",
                self.list.filter_text.trim()
            );
        }
        let mut s = format!(
            "{} of {} \u{00B7} \u{2191}\u{2193} move \u{00B7} Enter select \u{00B7} Esc cancel",
            self.list.filtered.len(),
            self.list.choices.len()
        );
        if self.truncated {
            s.push_str(" \u{00B7} list capped, type to narrow");
        }
        s
    }

    /// Draw the centered modal — border, filter line, scrollable choice list
    /// with scrollbar, and footer — over `area`. No-op when the terminal is too
    /// small to host a usable modal.
    pub fn render(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        use ratatui::layout::{Constraint, Layout};
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{
            Block, Borders, Clear, List, ListItem, Padding, Paragraph, Scrollbar,
            ScrollbarOrientation, ScrollbarState,
        };

        let Some(layout) = self.layout(area) else {
            return; // too small — caller has already fallen back; draw nothing.
        };
        frame.render_widget(Clear, layout.modal);
        // Padding gives horizontal breathing room (2 cols each side) and a blank
        // row above the filter / below the footer (1 each) — see CHROME_ROWS.
        let block = Block::default()
            .borders(Borders::ALL)
            .padding(Padding::new(2, 2, 1, 1))
            .title(format!(" Select {} ", self.title));
        let inner = block.inner(layout.modal);
        frame.render_widget(block, layout.modal);

        // Spacer rows above the list (below the filter) and above the footer give
        // the contents room to breathe.
        let rows = Layout::vertical([
            Constraint::Length(1), // filter
            Constraint::Length(1), // spacer
            Constraint::Min(1),    // list
            Constraint::Length(1), // spacer
            Constraint::Length(1), // footer
        ])
        .split(inner);
        let (filter_row, list_row, footer_row) = (rows[0], rows[2], rows[4]);

        let dim = Style::default().add_modifier(Modifier::DIM);
        const FILTER_LABEL: &str = "Filter: ";
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(FILTER_LABEL, dim),
                Span::raw(self.list.filter_text.clone()),
            ])),
            filter_row,
        );
        // Park the real terminal cursor at the end of the filter text (ratatui
        // shows it only on frames that set it, so it stays hidden elsewhere).
        let cursor_x =
            filter_row.x + FILTER_LABEL.len() as u16 + self.list.filter_text.chars().count() as u16;
        frame.set_cursor_position((
            cursor_x.min(filter_row.x + filter_row.width.saturating_sub(1)),
            filter_row.y,
        ));

        let items: Vec<ListItem> = if self.list.filtered.is_empty() {
            // No choices (or the filter excludes them all): show a dim
            // placeholder so the dialog clearly reads as "searched, found
            // nothing" rather than an empty box.
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
        // Render through `self.list.list_state` so the scroll offset *persists*
        // across frames. Discarding it (a clone) would make ratatui re-derive
        // the offset from the selected index every frame, pinning the selection
        // to the bottom row so moving up scrolls immediately instead of moving
        // within the viewport.
        frame.render_stateful_widget(list, list_row, &mut self.list.list_state);

        // Drive the scrollbar from the post-render scroll *offset* and viewport
        // size (not the selected index), so the thumb is a continuous block sized
        // to the viewport and only moves when the list actually scrolls.
        let visible = list_row.height as usize;
        if self.list.filtered.len() > visible {
            let mut sb = ScrollbarState::new(self.list.filtered.len())
                .position(self.list.list_state.offset())
                .viewport_content_length(visible);
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

    /// Build an `OptionChoice` fixture from a label/value pair.
    fn choice(label: &str, value: &str) -> OptionChoice {
        OptionChoice {
            label: label.into(),
            value: value.into(),
        }
    }

    /// Build a sample picker modal over a fixed choice list.
    fn sample() -> EnumPickerModal {
        EnumPickerModal::new(
            "Pick".into(),
            vec![
                choice("alpha", "a"),
                choice("Mike", "m"),
                choice("Zeta", "z"),
            ],
            false,
            None,
        )
    }

    #[test]
    fn test_new_preselects_current_value() {
        let modal = EnumPickerModal::new(
            "Pick".into(),
            vec![choice("alpha", "a"), choice("Mike", "m")],
            false,
            Some("m"),
        );
        assert_eq!(modal.selected_value().as_deref(), Some("m"));
    }

    #[test]
    fn test_filter_narrows_case_insensitively() {
        let mut modal = sample();
        for c in "mi".chars() {
            modal.push_char(c);
        }
        assert_eq!(modal.list.filtered.len(), 1);
        assert_eq!(modal.selected_value().as_deref(), Some("m"));
    }

    #[test]
    fn test_filter_matches_value_when_label_differs() {
        // The filter text is a substring of the VALUE only (not the label), so a
        // match proves value-matching is live — a label-only filter would drop it.
        let mut modal = EnumPickerModal::new(
            "Pick".into(),
            vec![choice("Ada", "user-prod-1"), choice("Bob", "user-stg-1")],
            false,
            None,
        );
        for c in "prod".chars() {
            modal.push_char(c);
        }
        assert_eq!(modal.selected_value().as_deref(), Some("user-prod-1"));
    }

    #[test]
    fn test_pop_char_widens_again() {
        let mut modal = sample();
        modal.push_char('z');
        assert_eq!(modal.list.filtered.len(), 1);
        modal.pop_char();
        assert_eq!(modal.list.filtered.len(), 3);
    }

    #[test]
    fn test_move_and_page_clamp_at_ends() {
        let mut modal = sample();
        modal.move_up(); // already at 0
        assert_eq!(modal.selected_value().as_deref(), Some("a"));
        modal.page_down(100);
        assert_eq!(modal.selected_value().as_deref(), Some("z"));
        modal.move_down(); // clamped at last
        assert_eq!(modal.selected_value().as_deref(), Some("z"));
    }

    #[test]
    fn test_page_down_moves_by_n() {
        let mut modal = sample();
        modal.page_down(1);
        assert_eq!(modal.selected_value().as_deref(), Some("m"));
    }

    #[test]
    fn test_custom_value_only_when_zero_matches() {
        let mut modal = sample();
        assert_eq!(modal.custom_value(), None);
        for c in "zzz".chars() {
            modal.push_char(c);
        }
        assert!(modal.list.filtered.is_empty());
        assert_eq!(modal.custom_value().as_deref(), Some("zzz"));
        assert_eq!(modal.selected_value(), None);
    }

    #[test]
    fn test_layout_none_when_too_small() {
        let modal = sample();
        assert!(modal.layout(Rect::new(0, 0, 1, 1)).is_none());
        assert!(modal
            .layout(Rect::new(0, 0, MIN_MODAL_WIDTH + 3, 4))
            .is_none());
    }

    #[test]
    fn test_layout_sizes_and_centers() {
        let modal = sample(); // 3 choices
        let l = modal.layout(Rect::new(0, 0, 100, 40)).expect("fits");
        assert_eq!(l.modal.width, MAX_MODAL_WIDTH);
        assert_eq!(l.modal.height, 3 + CHROME_ROWS);
        assert_eq!(l.list_height, 3);
        assert_eq!(l.modal.x, (100 - MAX_MODAL_WIDTH) / 2);
    }

    #[test]
    fn test_layout_list_height_at_least_one() {
        let modal = sample();
        let l = modal
            .layout(Rect::new(0, 0, 40, MIN_MODAL_HEIGHT + 4))
            .expect("fits");
        assert!(l.list_height >= 1);
    }

    #[test]
    fn test_layout_height_stable_while_filtering() {
        let choices: Vec<OptionChoice> = (0..30)
            .map(|i| choice(&format!("item-{i}"), &format!("v{i}")))
            .collect();
        let mut modal = EnumPickerModal::new("Pick".into(), choices, false, None);
        let area = Rect::new(0, 0, 100, 40);
        let before = modal.layout(area).unwrap().modal.height;
        modal.push_char('1'); // narrows the list substantially
        modal.push_char('5');
        let after = modal.layout(area).unwrap().modal.height;
        assert_eq!(
            before, after,
            "modal height must not change as the filter narrows"
        );
    }

    #[test]
    fn test_layout_caps_height_at_half_screen() {
        // A long list on a tall screen must not exceed half the screen height.
        let choices: Vec<OptionChoice> = (0..200)
            .map(|i| choice(&format!("item-{i}"), &format!("v{i}")))
            .collect();
        let modal = EnumPickerModal::new("Pick".into(), choices, false, None);
        let l = modal.layout(Rect::new(0, 0, 100, 40)).expect("fits");
        assert_eq!(l.modal.height, 40 / 2, "height capped at 50% of the screen");
    }

    use ratatui::{backend::TestBackend, Terminal};

    /// Render the modal to a string for assertions.
    fn render_to_string(modal: &mut EnumPickerModal, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| modal.render(f, f.area())).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Render once to a TestBackend so the persisted scroll offset updates.
    fn render_once(modal: &mut EnumPickerModal, w: u16, h: u16) {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| modal.render(f, f.area())).unwrap();
    }

    #[test]
    fn test_scroll_offset_persists_when_moving_up_from_bottom() {
        // 30 items in a small viewport so the list must scroll.
        let choices: Vec<OptionChoice> = (0..30)
            .map(|i| choice(&format!("item-{i}"), &format!("v{i}")))
            .collect();
        let mut modal = EnumPickerModal::new("Pick".into(), choices, false, None);
        // 100x20 ⇒ modal capped at height 10 ⇒ list viewport = 10 - CHROME_ROWS = 2.
        let (w, h) = (100, 20);
        let viewport = modal.layout(Rect::new(0, 0, w, h)).unwrap().list_height as usize;
        assert!(viewport >= 1);

        modal.page_down(100); // jump to the last item
        render_once(&mut modal, w, h);
        let bottom_offset = modal.list.list_state.offset();
        assert_eq!(
            bottom_offset,
            30 - viewport,
            "scrolled so the last item shows"
        );

        // Moving up one keeps the selection within the viewport — the offset
        // must NOT change yet (the bug scrolled immediately).
        modal.move_up();
        render_once(&mut modal, w, h);
        assert_eq!(
            modal.list.list_state.offset(),
            bottom_offset,
            "offset stays put while the selection moves up within the viewport"
        );

        // Moving up until the selection reaches the top of the viewport finally
        // scrolls the list.
        for _ in 0..viewport {
            modal.move_up();
            render_once(&mut modal, w, h);
        }
        assert!(
            modal.list.list_state.offset() < bottom_offset,
            "offset decreases once the selection reaches the viewport top"
        );
    }

    #[test]
    fn test_render_shows_title_filter_counts_and_choice() {
        let mut modal = sample();
        modal.push_char('m'); // filters to "Mike"
        let buf = render_to_string(&mut modal, 80, 24);
        assert!(
            buf.contains("Select Pick"),
            "heading reads 'Select <field>': {buf}"
        );
        assert!(buf.contains("Filter: m"), "filter line: {buf}");
        assert!(buf.contains("Mike"), "matching choice shown: {buf}");
        assert!(buf.contains("1 of 3"), "filtered/total counts: {buf}");
    }

    #[test]
    fn test_render_zero_match_shows_custom_value_footer() {
        let mut modal = sample();
        for c in "zzz".chars() {
            modal.push_char(c);
        }
        let buf = render_to_string(&mut modal, 80, 24);
        assert!(buf.contains("custom value"), "custom-value hint: {buf}");
    }
}
