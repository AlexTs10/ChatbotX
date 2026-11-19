//! Port of `packages/business/__tests__/google-ads-timing.test.ts`.

use chatbotx_business::google_ads_conversion::{
    PROCESSING_REDRIVE_DELAY_MS, REDRIVE_DELAY_MS, is_conversion_expired, send_delay_ms,
};
use chatbotx_business::google_ads_housekeeping::next_processing_backoff_ms;

const HOUR: i64 = 60 * 60 * 1000;
const DAY: i64 = 24 * HOUR;
/// 2026-10-05T12:00:00Z
const NOW: i64 = 1_791_201_600_000;
/// 2026-07-01T00:00:00Z
const R: i64 = 1_782_864_000_000;

fn expired(occurred: i64, now: i64, lookback: Option<i64>) -> bool {
    is_conversion_expired(R + occurred, R, lookback, R + now)
}

/// Port of: "computeSendDelayMs > waits the remainder of the 6h minimum click age" (+ "is the full 6h", "is 0 exactly at 6h", "clamps to 0")
#[test]
fn send_delay_waits_the_rest_of_the_six_hour_click_age() {
    assert_eq!(send_delay_ms(NOW - 2 * HOUR, NOW), 4 * HOUR);
    assert_eq!(send_delay_ms(NOW, NOW), 6 * HOUR);
    assert_eq!(send_delay_ms(NOW - 6 * HOUR, NOW), 0);
    assert_eq!(send_delay_ms(NOW - 30 * HOUR, NOW), 0);
}

/// Port of: "nextProcessingBackoffMs > steps 3h, 6h, 12h, 24h" (+ "caps at 24h", "treats negative attempts as the first poll")
#[test]
fn processing_backoff_steps_and_caps() {
    let steps: Vec<i64> = (0..4).map(next_processing_backoff_ms).collect();
    assert_eq!(steps, [3 * HOUR, 6 * HOUR, 12 * HOUR, 24 * HOUR]);
    assert_eq!(next_processing_backoff_ms(4), 24 * HOUR);
    assert_eq!(next_processing_backoff_ms(100), 24 * HOUR);
    assert_eq!(next_processing_backoff_ms(-5), 3 * HOUR);
}

/// Port of: "isConversionExpired > is not expired when fresh and inside the lookback"
#[test]
fn not_expired_when_fresh_and_inside_the_lookback() {
    for now in [DAY, 2 * DAY] {
        assert!(!expired(DAY, now, Some(30)));
    }
}

/// Port of: "B: not expired exactly on the lookback boundary, expired one second past it"
#[test]
fn rule_b_lookback_boundary() {
    assert!(!expired(30 * DAY, 31 * DAY, Some(30)));
    assert!(expired(30 * DAY + 1000, 31 * DAY, Some(30)));
}

/// Port of: "a conversion inside the lookback delivered after it is not expired"
#[test]
fn delivery_after_the_lookback_is_not_expired() {
    assert!(!expired(10 * DAY, 40 * DAY, Some(30)));
}

/// Port of: "A: not expired exactly 90 d after receipt, expired one second past it"
#[test]
fn rule_a_ninety_day_boundary() {
    assert!(!expired(DAY, 90 * DAY, Some(90)));
    assert!(expired(DAY, 90 * DAY + 1000, Some(90)));
}

/// Port of: "a null lookback behaves as 90 days"
#[test]
fn a_null_lookback_behaves_as_ninety_days() {
    assert!(!expired(89 * DAY, 89 * DAY, None));
    assert!(expired(91 * DAY, 90 * DAY, None));
}

/// Port of: "a lookback above 90 d does not extend rule A"
#[test]
fn a_long_lookback_does_not_extend_rule_a() {
    assert!(expired(100 * DAY, 100 * DAY, Some(120)));
}

/// Port of: "a conversion before the click receipt is not expired locally, however early"
#[test]
fn a_conversion_before_the_click_is_not_expired() {
    assert!(!expired(-HOUR, HOUR, Some(30)));
    assert!(!expired(-100 * DAY, HOUR, Some(30)));
}

/// Port of: "redrive delays > a transient processing failure waits 6h (plan §8); reauth deferral stays 1h"
#[test]
fn redrive_delays() {
    assert_eq!(PROCESSING_REDRIVE_DELAY_MS, 6 * HOUR);
    assert_eq!(REDRIVE_DELAY_MS, HOUR);
}
