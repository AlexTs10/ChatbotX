//! Port of `packages/database/__tests__/working-hours-schema.test.ts` (P3.2b, ported:
//! `safeParse(..).success` is `parse_..(..).is_ok()`; each `test.each` row is one test).

use chatbotx_db::repo::partials::working_hours::{
    WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY, WORKING_HOURS_MAX_INTERVALS_TOTAL,
    parse_working_hours_interval, parse_working_hours_intervals,
};
use serde_json::{Value, json};

fn interval(weekday: i64, start_minute: impl Into<Value>, end_minute: i64) -> Value {
    json!({ "weekday": weekday, "startMinute": start_minute.into(), "endMinute": end_minute })
}

fn list(intervals: Vec<Value>) -> Value {
    Value::Array(intervals)
}

#[test]
fn working_hours_interval_schema_accepts_the_full_day_00_00_24_00_and_sunday_saturday_bounds() {
    assert!(parse_working_hours_interval(&interval(0, 0, 1440)).is_ok());
    assert!(parse_working_hours_interval(&interval(6, 1439, 1440)).is_ok());
}

#[test]
fn working_hours_interval_schema_rejects_weekday_below_range() {
    assert!(parse_working_hours_interval(&interval(-1, 0, 60)).is_err());
}

#[test]
fn working_hours_interval_schema_rejects_weekday_above_range() {
    assert!(parse_working_hours_interval(&interval(7, 0, 60)).is_err());
}

#[test]
fn working_hours_interval_schema_rejects_negative_start() {
    assert!(parse_working_hours_interval(&interval(1, -1, 60)).is_err());
}

#[test]
fn working_hours_interval_schema_rejects_start_at_24_00() {
    assert!(parse_working_hours_interval(&interval(1, 1440, 1441)).is_err());
}

#[test]
fn working_hours_interval_schema_rejects_end_above_24_00() {
    assert!(parse_working_hours_interval(&interval(1, 0, 1441)).is_err());
}

#[test]
fn working_hours_interval_schema_rejects_end_equal_to_start() {
    assert!(parse_working_hours_interval(&interval(1, 60, 60)).is_err());
}

#[test]
fn working_hours_interval_schema_rejects_end_before_start() {
    assert!(parse_working_hours_interval(&interval(1, 120, 60)).is_err());
}

#[test]
fn working_hours_interval_schema_rejects_non_integer_minute() {
    assert!(parse_working_hours_interval(&interval(1, 0.5, 60)).is_err());
}

#[test]
fn working_hours_intervals_schema_accepts_an_empty_list_and_adjacent_touching_intervals() {
    assert!(parse_working_hours_intervals(&list(vec![])).is_ok());
    assert!(
        parse_working_hours_intervals(&list(vec![interval(1, 540, 720), interval(1, 720, 1020)]))
            .is_ok()
    );
}

#[test]
fn working_hours_intervals_schema_rejects_overlapping_intervals_on_the_same_weekday_regardless_of_order()
 {
    assert!(
        parse_working_hours_intervals(&list(vec![interval(1, 600, 900), interval(1, 540, 601)]))
            .is_err()
    );
}

#[test]
fn working_hours_intervals_schema_allows_the_same_minutes_on_different_weekdays() {
    assert!(
        parse_working_hours_intervals(&list(vec![interval(1, 540, 1020), interval(2, 540, 1020)]))
            .is_ok()
    );
}

#[test]
fn working_hours_intervals_schema_rejects_more_than_the_per_weekday_maximum() {
    let too_many: Vec<Value> = (0..WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY as i64 + 1)
        .map(|index| interval(1, index * 60, index * 60 + 30))
        .collect();
    assert!(parse_working_hours_intervals(&list(too_many.clone())).is_err());
    assert!(parse_working_hours_intervals(&list(too_many[..10].to_vec())).is_ok());
}

#[test]
fn working_hours_intervals_schema_rejects_more_than_the_total_maximum() {
    let over: Vec<Value> = (0..WORKING_HOURS_MAX_INTERVALS_TOTAL as i64 + 1)
        .map(|index| interval(index / 11, (index % 11) * 60, (index % 11) * 60 + 30))
        .collect();
    assert!(parse_working_hours_intervals(&list(over)).is_err());
}

#[test]
fn working_hours_intervals_schema_accepts_exactly_the_total_maximum() {
    let exact: Vec<Value> = (0..WORKING_HOURS_MAX_INTERVALS_TOTAL as i64)
        .map(|index| interval(index / 10, (index % 10) * 60, (index % 10) * 60 + 30))
        .collect();
    assert!(parse_working_hours_intervals(&list(exact)).is_ok());
}
