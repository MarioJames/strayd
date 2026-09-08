use std::time::{SystemTime, UNIX_EPOCH};

use time::{OffsetDateTime, UtcOffset};

use crate::Translator;

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn format_started_at(started_at: Option<u64>, tr: Translator) -> String {
    let Some(date) = started_at
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
    else {
        return tr.text("tui_unknown").into();
    };
    let offset = UtcOffset::local_offset_at(date).unwrap_or(UtcOffset::UTC);
    format_date(date.to_offset(offset))
}

fn format_date(date: OffsetDateTime) -> String {
    let offset = date.offset();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}{:02}:{:02}",
        date.year(),
        date.month() as u8,
        date.day(),
        date.hour(),
        date.minute(),
        date.second(),
        if offset.is_negative() { '-' } else { '+' },
        offset.whole_hours().unsigned_abs(),
        offset.minutes_past_hour().unsigned_abs(),
    )
}

pub fn format_uptime(started_at: Option<u64>, now: u64, tr: Translator) -> String {
    let Some(elapsed) = started_at
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| now.checked_sub(seconds))
    else {
        return tr.text("tui_unknown").into();
    };
    let clock = format!(
        "{:02}:{:02}:{:02}",
        elapsed / 3600 % 24,
        elapsed / 60 % 60,
        elapsed % 60
    );
    if elapsed < 86400 {
        clock
    } else {
        tr.format(
            "process_uptime_days",
            &[("days", (elapsed / 86400).to_string()), ("clock", clock)],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Language;

    #[test]
    fn uptime_handles_unit_boundaries_and_unknown_times() {
        let tr = Translator::new(Language::English);
        for (elapsed, expected) in [
            (0, "00:00:00"),
            (59, "00:00:59"),
            (60, "00:01:00"),
            (3600, "01:00:00"),
            (86399, "23:59:59"),
            (86400, "1d 00:00:00"),
            (176461, "2d 01:01:01"),
        ] {
            assert_eq!(format_uptime(Some(100), 100 + elapsed, tr), expected);
        }
        for started_at in [None, Some(0), Some(101)] {
            assert_eq!(format_uptime(started_at, 100, tr), tr.text("tui_unknown"));
        }
        assert_eq!(
            format_uptime(Some(100), 86500, Translator::new(Language::ZhCn)),
            "1天 00:00:00"
        );
    }

    #[test]
    fn creation_time_preserves_date_and_timezone() {
        let date = OffsetDateTime::from_unix_timestamp(1704067200).unwrap();
        assert_eq!(
            format_date(date.to_offset(UtcOffset::from_hms(8, 0, 0).unwrap())),
            "2024-01-01 08:00:00 +08:00"
        );
        assert_eq!(
            format_date(date.to_offset(UtcOffset::from_hms(-3, -30, 0).unwrap())),
            "2023-12-31 20:30:00 -03:30"
        );
        let tr = Translator::new(Language::English);
        for value in [None, Some(0), Some(u64::MAX)] {
            assert_eq!(format_started_at(value, tr), tr.text("tui_unknown"));
        }
    }
}
