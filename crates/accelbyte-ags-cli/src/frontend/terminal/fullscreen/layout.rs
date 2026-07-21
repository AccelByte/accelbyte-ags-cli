//! Layout regions for the fullscreen workflow surface.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub struct Regions {
    pub header: Rect,
    pub main: Rect,
    pub summary: Rect,
    // Set by split_with_strip; read in tests to verify layout geometry.
    #[allow(dead_code)]
    pub nav: Rect,
}

pub const MIN_SUMMARY_WIDTH: u16 = 32;
pub const MAX_SUMMARY_WIDTH: u16 = 64;
const SUMMARY_RATIO_PERCENT: u32 = 35;

/// Summary panel width: a fixed percentage of the body width, clamped to a
/// sensible `[MIN, MAX]` range so the panel stays usable on narrow terminals
/// and doesn't bloat on very wide ones.
pub fn summary_width(body_width: u16) -> u16 {
    let raw = ((body_width as u32 * SUMMARY_RATIO_PERCENT) / 100) as u16;
    raw.clamp(MIN_SUMMARY_WIDTH, MAX_SUMMARY_WIDTH)
}

/// Header box height: borders (2) + top/bottom padding (2) + step-strip line.
/// Production code calls `header_height(true, false)`; this is the equivalent
/// constant kept for test assertions.
#[allow(dead_code)]
pub const HEADER_HEIGHT: u16 = 5;

/// Navigation box height: borders (2) + top/bottom padding (2) + key-hint line.
pub const NAV_HEIGHT: u16 = 5;

/// Single-shot header height: borders (2) + top/bottom padding (2) + one line.
pub const SINGLE_SHOT_HEADER_HEIGHT: u16 = 5;

/// Header box height with the step strip.
pub fn header_height(has_strip: bool, _has_description: bool) -> u16 {
    if !has_strip {
        SINGLE_SHOT_HEADER_HEIGHT
    } else {
        5
    }
}

/// Region split that optionally omits the step strip. When the
/// fullscreen surface is forced for a single-shot command
/// (`--ui=fullscreen ags auth status`), the step strip has nothing to
/// show — collapse the header to a single brand line so the main area
/// gets the freed rows back.
pub fn split_with_strip(area: Rect, has_strip: bool, has_description: bool) -> Regions {
    let h = header_height(has_strip, has_description);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(h),
            Constraint::Min(1),
            Constraint::Length(NAV_HEIGHT),
        ])
        .split(area);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(summary_width(vertical[1].width)),
        ])
        .split(vertical[1]);

    Regions {
        header: vertical[0],
        main: body[0],
        summary: body[1],
        nav: vertical[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Equivalent to `split_with_strip(area, true, false)`. Used in tests as a
    /// convenience wrapper.
    fn split(area: Rect) -> Regions {
        split_with_strip(area, true, false)
    }

    #[test]
    fn test_split_allocates_header_main_summary_nav() {
        let area = Rect::new(0, 0, 100, 30);
        let r = split(area);
        assert_eq!(r.header.height, HEADER_HEIGHT);
        assert_eq!(r.nav.height, NAV_HEIGHT);
        // At 100 cols, 35% = 35, within [32, 64] → 35.
        assert_eq!(r.summary.width, summary_width(100));
        assert_eq!(r.main.width, 100 - summary_width(100));
    }

    #[test]
    fn test_main_area_height_is_total_minus_header_and_nav() {
        let area = Rect::new(0, 0, 100, 30);
        let r = split(area);
        assert_eq!(r.main.height, 30 - HEADER_HEIGHT - NAV_HEIGHT);
        assert_eq!(r.summary.height, r.main.height);
    }

    #[test]
    fn test_summary_starts_to_the_right_of_main_with_no_gap() {
        let area = Rect::new(0, 0, 100, 30);
        let r = split(area);
        assert_eq!(r.summary.x, r.main.x + r.main.width);
    }

    #[test]
    fn test_split_without_strip_uses_single_header_line() {
        let r = split_with_strip(Rect::new(0, 0, 100, 30), false, false);
        assert_eq!(r.header.height, SINGLE_SHOT_HEADER_HEIGHT);
        // 30 - 1 (header) - 1 (nav) = 28 rows for the main area.
        assert_eq!(r.main.height, 30 - SINGLE_SHOT_HEADER_HEIGHT - NAV_HEIGHT);
    }

    #[test]
    fn test_split_with_strip_matches_default_split() {
        let area = Rect::new(0, 0, 100, 30);
        let default = split(area);
        let with_strip = split_with_strip(area, true, false);
        assert_eq!(default.header, with_strip.header);
        assert_eq!(default.main, with_strip.main);
        assert_eq!(default.summary, with_strip.summary);
        assert_eq!(default.nav, with_strip.nav);
    }

    #[test]
    fn test_summary_width_ratio_with_clamp() {
        use ratatui::layout::Rect;
        // At a wide terminal (200 cols), summary is clamped to MAX (64).
        let wide = split_with_strip(Rect::new(0, 0, 200, 30), true, false);
        assert_eq!(wide.summary.width, 64);
        // At a narrow terminal (60 cols), summary is clamped to MIN (32).
        let narrow = split_with_strip(Rect::new(0, 0, 60, 30), true, false);
        assert_eq!(narrow.summary.width, 32);
        // At 120 cols, summary is ~35% within bounds: 35% * 120 = 42.
        let mid = split_with_strip(Rect::new(0, 0, 120, 30), true, false);
        assert_eq!(mid.summary.width, 42);
    }
}
