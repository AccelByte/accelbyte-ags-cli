//! Shared scrollbar-state construction for ratatui's `Scrollbar` widget.
//!
//! Every scrollable list/tree view in this crate (the picker list, the
//! fullscreen enum/file pickers, the inline fields view, and the JSON tree
//! editor) renders its scrollbar the same way; this used to be five copies
//! of the same off-by-one fix.

use ratatui::widgets::ScrollbarState;

/// Build the `ScrollbarState` for a list of `total` items showing `visible`
/// rows, scrolled to `offset`. Returns `None` when everything fits (no
/// scrollbar needed) — callers should skip rendering the `Scrollbar` widget
/// in that case.
///
/// `content_length` is deliberately `max_start + 1` (the number of scroll
/// POSITIONS: `0..=max_start`), not `total` (the item count) — ratatui maps
/// `position` over `0..content_length`, so feeding it the item count instead
/// leaves the thumb short of the track bottom when scrolled fully down.
pub(crate) fn scrollbar_state(
    total: usize,
    visible: usize,
    offset: usize,
) -> Option<ScrollbarState> {
    if total <= visible {
        return None;
    }
    let max_start = total.saturating_sub(visible);
    Some(
        ScrollbarState::new(max_start + 1)
            .position(offset)
            .viewport_content_length(visible),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fits_within_viewport_returns_none() {
        assert!(scrollbar_state(5, 10, 0).is_none());
        assert!(scrollbar_state(10, 10, 0).is_none());
    }

    #[test]
    fn test_overflow_uses_scroll_positions_not_item_count() {
        // 20 items, 5 visible -> 16 scroll positions (0..=15), not 20.
        let sb = scrollbar_state(20, 5, 3).unwrap();
        let expected = ScrollbarState::new(16)
            .position(3)
            .viewport_content_length(5);
        assert_eq!(sb, expected);
    }
}
