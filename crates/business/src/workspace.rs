//! `workspace/service.ts` `isActiveNow`: the bot on/off switch and its
//! daily active window.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// `isActiveNow`: `is_active`, and the local `HH:mm` inside the
/// `start`..=`end` window (overnight when `end` < `start`). An unknown time
/// zone reads as UTC.
pub fn is_active_now(
    is_active: bool,
    start: Option<&str>,
    end: Option<&str>,
    timezone: &str,
    now_ms: i64,
) -> bool {
    if !is_active {
        return false;
    }
    let (Some(start), Some(end)) = (
        start.filter(|s| !s.is_empty()),
        end.filter(|e| !e.is_empty()),
    ) else {
        return true;
    };
    let tz: Tz = timezone.parse().unwrap_or(Tz::UTC);
    let now = DateTime::<Utc>::from_timestamp_millis(now_ms).unwrap_or_default();
    let current = now.with_timezone(&tz).format("%H:%M").to_string();
    let current = current.as_str();
    if start <= end {
        current >= start && current <= end
    } else {
        current >= start || current <= end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-01-01T12:30:00Z
    const NOON: i64 = 1_767_270_600_000;

    #[test]
    fn window_and_switch() {
        assert!(!is_active_now(false, None, None, "UTC", NOON));
        assert!(is_active_now(true, None, None, "UTC", NOON));
        assert!(is_active_now(
            true,
            Some("09:00"),
            Some("17:00"),
            "UTC",
            NOON
        ));
        assert!(!is_active_now(
            true,
            Some("13:00"),
            Some("17:00"),
            "UTC",
            NOON
        ));
        assert!(is_active_now(
            true,
            Some("22:00"),
            Some("13:00"),
            "UTC",
            NOON
        ));
        // 19:30 in Ho Chi Minh City.
        assert!(!is_active_now(
            true,
            Some("09:00"),
            Some("17:00"),
            "Asia/Ho_Chi_Minh",
            NOON
        ));
    }
}
