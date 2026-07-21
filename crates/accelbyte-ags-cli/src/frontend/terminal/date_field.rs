//! Pure date-time widget core: parse/serialise the authored UTC ISO form,
//! clamp days to the month, and drive segmented editing. No terminal
//! ownership — the driver loops call into this. UTC only, minute precision.

use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DateParts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// True if `year` is a Gregorian leap year (used by `days_in_month`).
fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

/// Pull `day` into `1..=days_in_month(year, month)` after a month/year change.
pub(crate) fn clamp_day(parts: &mut DateParts) {
    let max = days_in_month(parts.year, parts.month);
    if parts.day > max {
        parts.day = max;
    }
    if parts.day < 1 {
        parts.day = 1;
    }
}

/// Parse a fixed-width, all-digit numeric component. Anything but exactly
/// `width` ASCII digits returns `None`.
fn fixed(s: &str, width: usize) -> Option<u32> {
    if s.len() != width || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Parse strictly the authored fixed-width UTC form `YYYY-MM-DDThh:mm:ssZ`.
/// Any other value — a numeric offset, fractional seconds, a missing `Z`, a
/// variable-width component like `2026-8-1T0:0Z`, or a calendar-invalid date
/// like `2026-02-31` — returns `None`, so the caller keeps it as raw text and
/// never rewrites it. Only exactly-authored values become editable date parts.
pub(crate) fn parse_iso(s: &str) -> Option<DateParts> {
    let body = s.strip_suffix('Z')?;
    let (date, time) = body.split_once('T')?;

    let mut d = date.split('-');
    let year = fixed(d.next()?, 4)? as i32;
    let month = fixed(d.next()?, 2)?;
    let day = fixed(d.next()?, 2)?;
    if d.next().is_some() {
        return None;
    }

    let mut t = time.split(':');
    let hour = fixed(t.next()?, 2)?;
    let minute = fixed(t.next()?, 2)?;
    fixed(t.next()?, 2)?; // seconds required and fixed-width, but discarded
    if t.next().is_some() {
        return None;
    }

    if !(1..=12).contains(&month) || hour > 23 || minute > 59 {
        return None;
    }
    // Reject calendar-invalid days (e.g. 2026-02-31, leap-year aware).
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(DateParts {
        year,
        month,
        day,
        hour,
        minute,
    })
}

/// Assemble the wire value. Seconds are pinned to `00` (minute precision).
pub(crate) fn to_iso(p: &DateParts) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:00Z",
        p.year, p.month, p.day, p.hour, p.minute
    )
}

/// Friendly rest display: `2026-08-01  00:00  UTC`. If the value does not parse
/// as the authored UTC form, return it verbatim (the display never lies).
pub(crate) fn friendly_display(iso: &str) -> String {
    match parse_iso(iso) {
        Some(p) => format!(
            "{:04}-{:02}-{:02}  {:02}:{:02}  UTC",
            p.year, p.month, p.day, p.hour, p.minute
        ),
        None => iso.to_string(),
    }
}

const SEGMENT_COUNT: usize = 5;

/// Digit width of each segment: year is 4, everything else 2.
fn segment_width(segment: usize) -> usize {
    if segment == 0 {
        4
    } else {
        2
    }
}

/// Live segmented-edit state for a DateTime field.
#[derive(Debug, Clone)]
pub(crate) struct DateEditState {
    pub parts: DateParts,
    pub segment: usize,
    typed: String,
}

impl DateEditState {
    pub fn new(parts: DateParts) -> Self {
        Self {
            parts,
            segment: 0,
            typed: String::new(),
        }
    }

    pub fn to_iso(&self) -> String {
        to_iso(&self.parts)
    }

    /// `2026-[08]-01  00:00  UTC` with the active segment bracketed.
    pub fn render_editing(&self) -> String {
        let seg = |i: usize, s: String| -> String {
            if i == self.segment {
                format!("[{s}]")
            } else {
                s
            }
        };
        let y = seg(0, format!("{:04}", self.parts.year));
        let mo = seg(1, format!("{:02}", self.parts.month));
        let d = seg(2, format!("{:02}", self.parts.day));
        let h = seg(3, format!("{:02}", self.parts.hour));
        let mi = seg(4, format!("{:02}", self.parts.minute));
        format!("{y}-{mo}-{d}  {h}:{mi}  UTC")
    }

    /// Adjust the active segment by one unit (`up` increments). Month, day,
    /// hour, and minute wrap at their bounds; year saturates within 1..=9999.
    /// Re-clamps the day after a year/month change so a now-invalid day (e.g.
    /// Jan 31 into February) is pulled into range.
    fn bump(&mut self, up: bool) {
        let delta: i64 = if up { 1 } else { -1 };
        match self.segment {
            0 => {
                self.parts.year = (self.parts.year as i64 + delta).clamp(1, 9999) as i32;
                clamp_day(&mut self.parts);
            }
            1 => {
                let m = (self.parts.month as i64 - 1 + delta).rem_euclid(12) + 1;
                self.parts.month = m as u32;
                clamp_day(&mut self.parts);
            }
            2 => {
                let max = days_in_month(self.parts.year, self.parts.month) as i64;
                let d = (self.parts.day as i64 - 1 + delta).rem_euclid(max) + 1;
                self.parts.day = d as u32;
            }
            3 => {
                self.parts.hour = (self.parts.hour as i64 + delta).rem_euclid(24) as u32;
            }
            _ => {
                self.parts.minute = (self.parts.minute as i64 + delta).rem_euclid(60) as u32;
            }
        }
    }

    /// Type digit `c` into the active segment, overwriting its value; once the
    /// segment's fixed width is filled, clear the buffer and advance to the next
    /// segment. Re-clamps the day after a year/month/day digit.
    fn type_digit(&mut self, c: char) {
        let width = segment_width(self.segment);
        self.typed.push(c);
        if self.typed.len() > width {
            self.typed = c.to_string();
        }
        // `typed` is at most `width` (<= 4) ASCII digits, so it always fits in a
        // u32; the `unwrap_or(0)` can never be reached and only satisfies the type.
        let n: u32 = self.typed.parse().unwrap_or(0);
        match self.segment {
            0 => self.parts.year = (n as i32).clamp(1, 9999),
            1 => self.parts.month = n.clamp(1, 12),
            2 => {
                let max = days_in_month(self.parts.year, self.parts.month);
                self.parts.day = n.clamp(1, max);
            }
            3 => self.parts.hour = n.min(23),
            _ => self.parts.minute = n.min(59),
        }
        if self.segment <= 2 {
            clamp_day(&mut self.parts);
        }
        if self.typed.len() == width {
            self.typed.clear();
            self.segment = (self.segment + 1).min(SEGMENT_COUNT - 1);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DateStep {
    Continue,
    Commit,
    Cancel,
}

/// Dispatch one key against the segmented editor. Arrow/segment nav and digit
/// entry return `Continue`; enter commits; esc cancels.
pub(crate) fn apply_date_key(state: &mut DateEditState, key: KeyEvent) -> DateStep {
    match key.code {
        KeyCode::Enter => DateStep::Commit,
        KeyCode::Esc => DateStep::Cancel,
        KeyCode::Left => {
            state.typed.clear();
            state.segment = state.segment.saturating_sub(1);
            DateStep::Continue
        }
        KeyCode::Right => {
            state.typed.clear();
            state.segment = (state.segment + 1).min(SEGMENT_COUNT - 1);
            DateStep::Continue
        }
        KeyCode::Up => {
            state.typed.clear();
            state.bump(true);
            DateStep::Continue
        }
        KeyCode::Down => {
            state.typed.clear();
            state.bump(false);
            DateStep::Continue
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            state.type_digit(c);
            DateStep::Continue
        }
        _ => DateStep::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn state_at(iso: &str) -> DateEditState {
        DateEditState::new(parse_iso(iso).unwrap())
    }

    #[test]
    fn test_arrows_move_between_segments() {
        let mut s = state_at("2026-08-01T00:00:00Z");
        assert_eq!(s.segment, 0);
        assert_eq!(
            apply_date_key(&mut s, key(KeyCode::Right)),
            DateStep::Continue
        );
        assert_eq!(s.segment, 1);
        assert_eq!(
            apply_date_key(&mut s, key(KeyCode::Left)),
            DateStep::Continue
        );
        assert_eq!(s.segment, 0);
        // Clamped at the ends.
        apply_date_key(&mut s, key(KeyCode::Left));
        assert_eq!(s.segment, 0);
    }

    #[test]
    fn test_up_down_bump_with_rollover_on_month() {
        let mut s = state_at("2026-12-01T00:00:00Z");
        s.segment = 1; // month
        apply_date_key(&mut s, key(KeyCode::Up));
        assert_eq!(s.parts.month, 1); // 12 -> 1 rollover
        apply_date_key(&mut s, key(KeyCode::Down));
        assert_eq!(s.parts.month, 12);
    }

    #[test]
    fn test_bump_month_reclamps_day() {
        let mut s = state_at("2026-01-31T00:00:00Z");
        s.segment = 1; // month
        apply_date_key(&mut s, key(KeyCode::Up)); // -> February
        assert_eq!(s.parts.month, 2);
        assert_eq!(s.parts.day, 28, "Jan 31 -> Feb clamps day to 28");
    }

    #[test]
    fn test_typing_digits_overwrites_segment_and_advances() {
        let mut s = state_at("2026-08-01T00:00:00Z");
        s.segment = 1; // month
        apply_date_key(&mut s, key(KeyCode::Char('1')));
        apply_date_key(&mut s, key(KeyCode::Char('2')));
        assert_eq!(s.parts.month, 12);
        assert_eq!(
            s.segment, 2,
            "filling a 2-digit segment advances to the next"
        );
    }

    #[test]
    fn test_enter_commits_esc_cancels() {
        let mut s = state_at("2026-08-01T00:00:00Z");
        assert_eq!(
            apply_date_key(&mut s, key(KeyCode::Enter)),
            DateStep::Commit
        );
        assert_eq!(apply_date_key(&mut s, key(KeyCode::Esc)), DateStep::Cancel);
    }

    #[test]
    fn test_render_editing_brackets_active_segment() {
        let mut s = state_at("2026-08-01T00:00:00Z");
        s.segment = 1;
        assert_eq!(s.render_editing(), "2026-[08]-01  00:00  UTC");
    }

    #[test]
    fn test_parse_iso_round_trips_authored_form() {
        let p = parse_iso("2026-08-01T00:00:00Z").unwrap();
        assert_eq!(
            p,
            DateParts {
                year: 2026,
                month: 8,
                day: 1,
                hour: 0,
                minute: 0
            }
        );
        assert_eq!(to_iso(&p), "2026-08-01T00:00:00Z");
    }

    #[test]
    fn test_parse_iso_discards_seconds_on_serialise() {
        let p = parse_iso("2099-12-31T23:59:59Z").unwrap();
        assert_eq!(p.minute, 59);
        assert_eq!(to_iso(&p), "2099-12-31T23:59:00Z");
    }

    #[test]
    fn test_parse_iso_rejects_offset_and_missing_z() {
        assert!(parse_iso("2026-08-01T00:00:00+01:00").is_none());
        assert!(parse_iso("2026-08-01T00:00:00").is_none());
        assert!(parse_iso("not-a-date").is_none());
    }

    #[test]
    fn test_parse_iso_rejects_calendar_invalid_day() {
        assert!(parse_iso("2026-02-31T00:00:00Z").is_none());
        assert!(
            parse_iso("2024-02-29T00:00:00Z").is_some(),
            "leap day is valid"
        );
        assert!(
            parse_iso("2026-02-29T00:00:00Z").is_none(),
            "not a leap year"
        );
    }

    #[test]
    fn test_parse_iso_requires_fixed_width_and_seconds() {
        // Only the exact authored form parses; a non-authored-but-close value is
        // kept as raw text (never silently normalised on commit).
        assert!(
            parse_iso("2026-8-1T0:0:0Z").is_none(),
            "variable width rejected"
        );
        assert!(
            parse_iso("2026-08-01T00:00Z").is_none(),
            "missing seconds rejected"
        );
    }

    #[test]
    fn test_days_in_month_leap_year() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn test_clamp_day_pulls_into_month() {
        let mut p = DateParts {
            year: 2026,
            month: 2,
            day: 31,
            hour: 0,
            minute: 0,
        };
        clamp_day(&mut p);
        assert_eq!(p.day, 28);
    }

    #[test]
    fn test_friendly_display_formats_and_falls_back() {
        assert_eq!(
            friendly_display("2026-08-01T00:00:00Z"),
            "2026-08-01  00:00  UTC"
        );
        assert_eq!(friendly_display("garbage"), "garbage");
    }
}
