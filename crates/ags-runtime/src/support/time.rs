//! Human-readable timestamp formatting, shared by every command that
//! displays an RFC 3339 timestamp (e.g. `createdAt`) to a person.

/// Format an RFC 3339 timestamp (e.g. `"2026-08-10T10:17:56Z"`) as
/// `"Aug 10, 2026, 10:17:56 UTC"`. Falls back to the raw string on parse
/// failure — callers always emit valid RFC 3339, but display must never
/// panic or blank out on an unexpected format.
pub fn format_rfc3339_human(raw: &str) -> String {
    parse_rfc3339_utc(raw).unwrap_or_else(|| raw.to_string())
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn parse_rfc3339_utc(raw: &str) -> Option<String> {
    let (date, rest) = raw.split_once('T')?;
    let mut date_parts = date.splitn(3, '-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;

    let (time, offset_seconds) = split_offset(rest)?;
    let time = time.split('.').next()?; // drop fractional seconds
    let mut time_parts = time.splitn(3, ':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next()?.parse().ok()?;

    let total_seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
            - offset_seconds;
    let days_utc = total_seconds.div_euclid(86_400);
    let mut sec_of_day = total_seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days_utc);
    let h = sec_of_day / 3600;
    sec_of_day %= 3600;
    let mi = sec_of_day / 60;
    let s = sec_of_day % 60;

    Some(format!(
        "{} {d}, {y}, {h:02}:{mi:02}:{s:02} UTC",
        MONTH_NAMES[(m - 1) as usize]
    ))
}

/// Split trailing `Z` or `±HH:MM` off a time-of-day string, returning the
/// offset in seconds to subtract to get UTC.
///
/// Only a colon-delimited offset (`±HH:MM`) is accepted, matching the only
/// forms CSM actually returns. A same-length offset without the colon (e.g.
/// `+0700`) is rejected — not accepted-and-misparsed — so callers fall back
/// to the raw string instead of computing a silently wrong time.
fn split_offset(time: &str) -> Option<(&str, i64)> {
    if let Some(t) = time.strip_suffix('Z') {
        return Some((t, 0));
    }
    let sign_pos = time.rfind(['+', '-'])?;
    let (t, offset) = time.split_at(sign_pos);
    let sign: i64 = if offset.starts_with('-') { -1 } else { 1 };
    let (oh_str, om_str) = offset[1..].split_once(':')?;
    let oh: i64 = oh_str.parse().ok()?;
    let om: i64 = om_str.parse().ok()?;
    Some((t, sign * (oh * 3600 + om * 60)))
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
/// http://howardhinnant.github.io/date_algorithms.html#days_from_civil
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_timestamp_with_z_suffix() {
        assert_eq!(
            format_rfc3339_human("2026-08-10T10:17:56Z"),
            "Aug 10, 2026, 10:17:56 UTC"
        );
    }

    #[test]
    fn drops_fractional_seconds() {
        assert_eq!(
            format_rfc3339_human("2026-08-10T10:17:56.123456789Z"),
            "Aug 10, 2026, 10:17:56 UTC"
        );
    }

    #[test]
    fn converts_positive_offset_with_day_rollover() {
        // 2026-08-11T01:17:56+07:00 == 2026-08-10T18:17:56Z
        assert_eq!(
            format_rfc3339_human("2026-08-11T01:17:56+07:00"),
            "Aug 10, 2026, 18:17:56 UTC"
        );
    }

    /// Mirrors `converts_positive_offset_with_day_rollover` for a negative
    /// offset — `split_offset`'s sign handling and the earlier positive-only
    /// coverage left this direction untested.
    #[test]
    fn converts_negative_offset_with_day_rollover() {
        // 2026-08-10T22:17:56-07:00 == 2026-08-11T05:17:56Z
        assert_eq!(
            format_rfc3339_human("2026-08-10T22:17:56-07:00"),
            "Aug 11, 2026, 05:17:56 UTC"
        );
    }

    #[test]
    fn falls_back_to_raw_string_on_unparseable_input() {
        assert_eq!(format_rfc3339_human("not-a-date"), "not-a-date");
    }

    // A same-length offset without the `:` (e.g. `+0700` instead of
    // `+07:00`) must fall back to the raw string, not silently misinterpret
    // the digits as a ~700-hour offset.
    #[test]
    fn falls_back_to_raw_string_on_non_colon_offset() {
        assert_eq!(
            format_rfc3339_human("2026-08-10T10:17:56+0700"),
            "2026-08-10T10:17:56+0700"
        );
    }
}
