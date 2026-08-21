//! Relative time formatting for commit and artifact timestamps.

use std::time::SystemTime;

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const MONTH: i64 = 30 * DAY;

/// Describe `timestamp` relative to `now`.
///
/// Timestamps use GitHub ISO 8601 form (`2026-08-21T00:00:00Z`). Values the
/// parser rejects fall back to the date portion, then to the raw value.
pub fn relative_time(timestamp: &str, now: SystemTime) -> String {
    let Some(utc) = parse_utc_seconds(timestamp) else {
        return timestamp.get(..10).unwrap_or(timestamp).to_owned();
    };
    let Some(elapsed) = now.elapsed().ok().map(|duration| duration.as_secs() as i64) else {
        return timestamp.get(..10).unwrap_or(timestamp).to_owned();
    };
    match elapsed - utc {
        delta if delta < MINUTE => "just now".to_owned(),
        delta if delta < HOUR => format!("{}m ago", delta / MINUTE),
        delta if delta < DAY => format!("{}h ago", delta / HOUR),
        delta if delta < MONTH => format!("{}d ago", delta / DAY),
        _ => timestamp.get(..10).unwrap_or(timestamp).to_owned(),
    }
}

/// Parse `YYYY-MM-DDTHH:MM:SS` UTC timestamps into seconds since the epoch.
fn parse_utc_seconds(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = digits(value.get(0..4)?)?;
    let month = digits(value.get(5..7)?)?;
    let day = digits(value.get(8..10)?)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (hour, minute, second) = match bytes[10] {
        b'T' | b' ' | b't' => (
            digits(value.get(11..13)?)?,
            digits(value.get(14..16)?)?,
            digits(value.get(17..19)?)?,
        ),
        _ => (0, 0, 0),
    };
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * DAY + hour * HOUR + minute * MINUTE + second)
}

fn digits(value: &str) -> Option<i64> {
    value.parse().ok()
}

/// Days since the Unix epoch for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_shift = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shift + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{parse_utc_seconds, relative_time};
    use std::time::{Duration, SystemTime};

    fn now_plus(delta: i64) -> SystemTime {
        let base = SystemTime::now()
            .checked_sub(Duration::from_secs(2))
            .unwrap();
        if delta >= 0 {
            base.checked_sub(Duration::from_secs(delta as u64)).unwrap()
        } else {
            base.checked_add(Duration::from_secs(delta.unsigned_abs()))
                .unwrap()
        }
    }

    #[test]
    fn parses_epoch_correctly() {
        assert_eq!(parse_utc_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_utc_seconds("2026-08-21T00:00:00Z"),
            Some(1_787_270_400)
        );
        assert_eq!(parse_utc_seconds("not-a-date"), None);
        assert_eq!(parse_utc_seconds("2026-08-21"), None);
    }

    #[test]
    fn buckets_relative_times() {
        const NOON: i64 = 1_787_313_600;
        let timestamp = "2026-08-21T12:00:00Z";
        assert_eq!(relative_time(timestamp, now_plus(NOON + 30)), "just now");
        assert_eq!(relative_time(timestamp, now_plus(NOON + 5 * 60)), "5m ago");
        assert_eq!(
            relative_time(timestamp, now_plus(NOON + 3 * 3600)),
            "3h ago"
        );
        assert_eq!(
            relative_time(timestamp, now_plus(NOON + 2 * 86_400)),
            "2d ago"
        );
        assert_eq!(
            relative_time(timestamp, now_plus(NOON + 400 * 86_400)),
            "2026-08-21"
        );
    }

    #[test]
    fn unparsable_values_fall_back_to_the_date() {
        assert_eq!(relative_time("garbage", SystemTime::now()), "garbage");
    }
}
