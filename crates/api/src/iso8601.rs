//! UTC timestamps in the format of JS `Date.prototype.toISOString`.
//!
//! Only std is used. Years 0000 to 9999 match JS exactly; JS writes later
//! years with a `+` and six digits, which is not needed here.

use std::time::{SystemTime, UNIX_EPOCH};

const MS_PER_DAY: u64 = 86_400_000;

/// The current time as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn now() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    format_millis(ms)
}

/// Format milliseconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_millis(ms: u64) -> String {
    let (year, month, day) = civil_from_days(ms / MS_PER_DAY);
    let ms_of_day = ms % MS_PER_DAY;
    let secs = ms_of_day / 1000;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        ms_of_day % 1000
    )
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian
/// calendar. Howard Hinnant's `civil_from_days`, for days >= 0.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Count from 0000-03-01 so that the leap day is the last day of a year.
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153; // 0 = March .. 11 = February
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era * 400 + year_of_era + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::format_millis;

    #[test]
    fn epoch_zero() {
        assert_eq!(format_millis(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn leap_day_2024() {
        assert_eq!(format_millis(1_709_210_096_789), "2024-02-29T12:34:56.789Z");
    }

    #[test]
    fn leap_day_2000() {
        // 2000 is divisible by 400, so it is a leap year.
        assert_eq!(format_millis(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn year_2100_is_not_a_leap_year() {
        assert_eq!(format_millis(4_107_542_399_999), "2100-02-28T23:59:59.999Z");
        assert_eq!(format_millis(4_107_542_400_000), "2100-03-01T00:00:00.000Z");
    }

    #[test]
    fn end_of_year_after_2100() {
        // 2150-12-31T23:59:59.999Z
        assert_eq!(format_millis(5_711_817_599_999), "2150-12-31T23:59:59.999Z");
    }
}
