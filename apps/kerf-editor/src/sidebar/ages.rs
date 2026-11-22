//! A conversation's age, as its tip says it ("4m", "Mar 4"): read from the
//! RFC 3339 time its file keeps, and the sidebar drawn again as the age
//! turns over.

use super::SidebarView;
use crate::model::ConversationSummary;
use gpui::Context;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Seconds since 1970 of an RFC 3339 time ("2026-11-20T03:49:08Z",
/// "2026-11-20T12:49:08.5+09:00"); None when it does not read.
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let bytes = text.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let zone = &rest[1..];
            if zone.len() != 5 || zone.as_bytes()[2] != b':' {
                return None;
            }
            let hours: i64 = zone[..2].parse().ok()?;
            let minutes: i64 = zone[3..].parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_cycle - 719_468
}

/// The date of a day since 1970-01-01: year, month, day.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// How long ago `when` was at `now` (both seconds since 1970): "Now",
/// "4m", "2h", "3d", then "Mar 4" this year and "Mar 2025" before.
pub fn relative_time(when: i64, now: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let seconds = (now - when).max(0);
    if seconds < 60 {
        return "Now".into();
    }
    if seconds < 3600 {
        return format!("{}m", seconds / 60);
    }
    if seconds < 86_400 {
        return format!("{}h", seconds / 3600);
    }
    if seconds < 7 * 86_400 {
        return format!("{}d", seconds / 86_400);
    }
    let (year, month, day) = civil_from_days(when.div_euclid(86_400));
    let (this_year, _, _) = civil_from_days(now.div_euclid(86_400));
    let month = MONTHS[(month - 1).clamp(0, 11) as usize];
    if year == this_year {
        format!("{month} {day}")
    } else {
        format!("{month} {year}")
    }
}

/// When (seconds since 1970) what [`relative_time`] says of `when` at `now`
/// changes next: at the next minute, hour or day it counts, then as the
/// year turns ("Mar 4" becomes "Mar 2025"); None once it names a year.
pub fn relative_time_changes(when: i64, now: i64) -> Option<i64> {
    let seconds = (now - when).max(0);
    let unit = match seconds {
        0..3600 => 60,
        3600..86_400 => 3600,
        86_400..604_800 => 86_400,
        _ => {
            let (year, _, _) = civil_from_days(now.div_euclid(86_400));
            let (then, _, _) = civil_from_days(when.div_euclid(86_400));
            return (then == year).then(|| days_from_civil(year + 1, 1, 1) * 86_400);
        }
    };
    Some(when + (seconds / unit + 1) * unit)
}

/// Seconds since 1970, now.
pub(super) fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

impl SidebarView {
    /// Draws the sidebar again when the age a tip gives any of
    /// `conversations` ([`relative_time`]) turns over.
    pub(super) fn follow_ages<'a>(
        &self,
        conversations: impl IntoIterator<Item = &'a ConversationSummary>,
        now: i64,
        cx: &Context<Self>,
    ) {
        let next = conversations
            .into_iter()
            .filter_map(|conversation| parse_time(&conversation.updated))
            .filter_map(|when| relative_time_changes(when, now))
            .min();
        if let Some(next) = next {
            let wait = std::time::Duration::from_secs(u64::try_from(next - now).unwrap_or(0));
            self.ages.at(Instant::now() + wait, cx);
        }
    }
}
