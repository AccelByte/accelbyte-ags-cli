//! Surface-neutral picker list: type-to-filter, scrollable, highlight-tracked.
//! Used by both the fullscreen `EnumPickerModal` (centered-modal chrome) and
//! the inline picker sub-loop (viewport render with no centering or `Clear`).

use ags_protocol::workflow::OptionChoice;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
    Frame,
};

/// Surface-neutral picker list: type-to-filter, scrollable, highlight-tracked.
///
/// Holds the filter text, the filtered index list, and the `ListState` scroll
/// position. Navigation and filtering are surface-independent; rendering is
/// done via [`PickerList::render_in`] for a viewport-style render (no `Clear`
/// or centering). The fullscreen surface wraps this in `EnumPickerModal` to
/// add centered-modal chrome.
pub(crate) struct PickerList {
    pub(crate) choices: Vec<OptionChoice>,
    /// Raw filter text; exposed read-only via [`PickerList::filter`].
    pub(crate) filter_text: String,
    /// Indices into `choices` matching `filter_text`, in original order.
    pub(crate) filtered: Vec<usize>,
    pub(crate) list_state: ListState,
}

impl PickerList {
    /// Build a picker over `choices`, preselecting the entry whose `value`
    /// matches `current` (falling back to the first entry).
    pub(crate) fn new(choices: Vec<OptionChoice>, current: Option<&str>) -> Self {
        let filtered: Vec<usize> = (0..choices.len()).collect();
        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            let initial = current
                .and_then(|v| choices.iter().position(|c| c.value == v))
                .unwrap_or(0)
                .min(filtered.len() - 1);
            list_state.select(Some(initial));
        }
        Self {
            choices,
            filter_text: String::new(),
            filtered,
            list_state,
        }
    }

    /// Rebuild the filtered index list from the current filter text; reset
    /// the highlight and scroll offset to the top of the new matches.
    fn recompute(&mut self) {
        let needle = self.filter_text.to_lowercase();
        self.filtered = self
            .choices
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                needle.is_empty()
                    || c.label.to_lowercase().contains(&needle)
                    || (c.label != c.value && c.value.to_lowercase().contains(&needle))
            })
            .map(|(i, _)| i)
            .collect();
        self.list_state
            .select((!self.filtered.is_empty()).then_some(0));
        *self.list_state.offset_mut() = 0;
    }

    /// Append a character to the filter and refilter.
    pub(crate) fn push_char(&mut self, c: char) {
        self.filter_text.push(c);
        self.recompute();
    }

    /// Delete the last filter character and refilter.
    pub(crate) fn pop_char(&mut self) {
        self.filter_text.pop();
        self.recompute();
    }

    /// Move the highlight up one row.
    pub(crate) fn move_up(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(i.saturating_sub(1)));
    }

    /// Move the highlight down one row.
    pub(crate) fn move_down(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state
            .select(Some((i + 1).min(self.filtered.len() - 1)));
    }

    /// Move the highlight up `n` rows (at least one).
    pub(crate) fn page_up(&mut self, n: usize) {
        if self.filtered.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(i.saturating_sub(n.max(1))));
    }

    /// Move the highlight down `n` rows (at least one).
    pub(crate) fn page_down(&mut self, n: usize) {
        if self.filtered.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state
            .select(Some((i + n.max(1)).min(self.filtered.len() - 1)));
    }

    /// The highlighted choice's `value`, or `None` when nothing matches.
    pub(crate) fn selected_value(&self) -> Option<String> {
        let sel = self.list_state.selected()?;
        let idx = *self.filtered.get(sel)?;
        Some(self.choices[idx].value.clone())
    }

    /// The trimmed filter text, but only when nothing matches (the free-text
    /// escape hatch). `None` otherwise.
    pub(crate) fn custom_value(&self) -> Option<String> {
        if self.filtered.is_empty() {
            let trimmed = self.filter_text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        None
    }

    /// All currently matching choices with a flag indicating whether each is
    /// highlighted, in filtered (original alphabetical) order.
    #[allow(dead_code)] // read accessor used by this module's tests to assert the filtered view
    pub(crate) fn visible(&self) -> Vec<(&OptionChoice, bool)> {
        let selected = self.list_state.selected();
        self.filtered
            .iter()
            .enumerate()
            .map(|(pos, &idx)| (&self.choices[idx], selected == Some(pos)))
            .collect()
    }

    /// Render the picker list into `area` — a bordered viewport with a filter
    /// line, scrollable choice list, and an optional "list capped" hint row
    /// when `truncated` is set. Does not `Clear` or center; the caller owns
    /// the viewport.
    ///
    /// Takes `&mut self` so ratatui can write the computed scroll offset back
    /// into the persisted `list_state`; a `&self` render (cloning the state)
    /// would reset long-list scrolling on every frame.
    pub(crate) fn render_in(&mut self, frame: &mut Frame, area: Rect, truncated: bool) {
        let dim = Style::default().add_modifier(Modifier::DIM);
        const FILTER_LABEL: &str = "Filter: ";

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 2 {
            return;
        }

        let rows = Layout::vertical([
            Constraint::Length(1),                // filter line
            Constraint::Min(1),                   // choice list
            Constraint::Length(truncated as u16), // optional truncation hint
        ])
        .split(inner);
        let (filter_row, list_row) = (rows[0], rows[1]);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(FILTER_LABEL, dim),
                Span::raw(self.filter_text.clone()),
            ])),
            filter_row,
        );

        let items: Vec<ListItem> = if self.filtered.is_empty() {
            vec![ListItem::new("<no matches>").style(dim)]
        } else {
            self.filtered
                .iter()
                .map(|&i| ListItem::new(self.choices[i].label.clone()))
                .collect()
        };
        let list_widget = List::new(items)
            .highlight_symbol("\u{203a} ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD));

        // Render through the persisted `list_state` so ratatui writes the
        // scroll offset back into it — long lists keep their scroll position
        // across frames instead of resetting each redraw.
        frame.render_stateful_widget(list_widget, list_row, &mut self.list_state);

        let visible_height = list_row.height as usize;
        if self.filtered.len() > visible_height {
            let mut sb = ScrollbarState::new(self.filtered.len())
                .position(self.list_state.offset())
                .viewport_content_length(visible_height);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                list_row,
                &mut sb,
            );
        }

        if truncated {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("list capped, type to narrow", dim))),
                rows[2],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::workflow::OptionChoice;

    fn choices() -> Vec<OptionChoice> {
        vec![
            OptionChoice {
                label: "ada.lovelace".into(),
                value: "u-1".into(),
            },
            OptionChoice {
                label: "adam.smith".into(),
                value: "u-2".into(),
            },
            OptionChoice {
                label: "grace.hopper".into(),
                value: "u-3".into(),
            },
        ]
    }

    #[test]
    fn test_filter_narrows_and_selected_value_tracks_highlight() {
        let mut list = PickerList::new(choices(), None);
        for c in "ada".chars() {
            list.push_char(c);
        }
        // "ada" matches ada.lovelace + adam.smith.
        let visible: Vec<&str> = list
            .visible()
            .iter()
            .map(|(c, _)| c.value.as_str())
            .collect();
        assert_eq!(visible, vec!["u-1", "u-2"]);
        assert_eq!(list.selected_value().as_deref(), Some("u-1"));
        list.move_down();
        assert_eq!(list.selected_value().as_deref(), Some("u-2"));
    }

    #[test]
    fn test_custom_value_only_when_no_match() {
        let mut list = PickerList::new(choices(), None);
        for c in "zzz".chars() {
            list.push_char(c);
        }
        assert!(list.selected_value().is_none());
        assert_eq!(list.custom_value().as_deref(), Some("zzz"));
    }
}
