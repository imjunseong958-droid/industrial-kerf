//! The clock a proposal's and a revision's records are stamped with, beside who staged a
//! proposal ([`Author`]).

pub use kerf_protocol::{Author, AUTHOR_KINDS};
use std::time::{SystemTime, UNIX_EPOCH};

/// Now, in RFC 3339 (UTC, microseconds). Later calls in one process always
/// read later, so two records never share a time.
pub fn timestamp() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    let mut last = LAST.load(Ordering::Relaxed);
    let micros = loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break next,
            Err(current) => last = current,
        }
    };
    rfc3339_micros(micros)
}

/// `micros` since the Unix epoch as RFC 3339 in UTC.
pub fn rfc3339_micros(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{fraction:06}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_rfc3339_and_always_later() {
        assert_eq!(rfc3339_micros(0), "1970-01-01T00:00:00.000000Z");
        assert_eq!(
            rfc3339_micros(1_700_000_000_123_456),
            "2023-11-14T22:13:20.123456Z"
        );
        let (first, second) = (timestamp(), timestamp());
        assert!(second > first, "{first} then {second}");
    }
}
