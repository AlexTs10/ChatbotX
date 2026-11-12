//! Port of `packages/utils/__tests__/date-expression.test.ts`.

mod support;

use chatbotx_core::utils::field_value_calculator::date_expression::{
    DATE_EXPRESSION_LIMITS, DateClock, DateUnit, add_date_unit, evaluate_date_expression,
};

fn now() -> i64 {
    support::iso_ms("2026-03-10T05:30:00.000Z")
}

fn evaluate_at(expression: &str, time_zone: &str, now_ms: i64) -> Option<String> {
    evaluate_date_expression(
        expression,
        &DateClock {
            now_ms,
            time_zone: time_zone.to_owned(),
        },
    )
    .map(support::iso)
}

fn evaluate(expression: &str) -> Option<String> {
    evaluate_at(expression, "UTC", now())
}

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > now"
#[test]
fn evaluatedateexpression_now() {
    assert_eq!(evaluate("now"), s("2026-03-10T05:30:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > NOW + 1 day"
#[test]
fn evaluatedateexpression_now_1_day() {
    assert_eq!(evaluate("NOW + 1 day"), s("2026-03-11T05:30:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > now - 2 hours"
#[test]
fn evaluatedateexpression_now_2_hours() {
    assert_eq!(evaluate("now - 2 hours"), s("2026-03-10T03:30:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > now + 15min"
#[test]
fn evaluatedateexpression_now_15min() {
    assert_eq!(evaluate("now + 15min"), s("2026-03-10T05:45:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > now +1w"
#[test]
fn evaluatedateexpression_now_1w() {
    assert_eq!(evaluate("now +1w"), s("2026-03-17T05:30:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > now + 1 month"
#[test]
fn evaluatedateexpression_now_1_month() {
    assert_eq!(evaluate("now + 1 month"), s("2026-04-10T05:30:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > now + 1 year"
#[test]
fn evaluatedateexpression_now_1_year() {
    assert_eq!(evaluate("now + 1 year"), s("2027-03-10T05:30:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > now + 1 day - 3 h + 20 minutes"
#[test]
fn evaluatedateexpression_now_1_day_3_h_20_minutes() {
    assert_eq!(
        evaluate("now + 1 day - 3 h + 20 minutes"),
        s("2026-03-11T02:50:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > today"
#[test]
fn evaluatedateexpression_today() {
    assert_eq!(evaluate("today"), s("2026-03-10T00:00:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:13 "evaluateDateExpression > today + 2d"
#[test]
fn evaluatedateexpression_today_2d() {
    assert_eq!(evaluate("today + 2d"), s("2026-03-12T00:00:00.000Z"));
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:28 "evaluateDateExpression > units are case-insensitive like the keywords"
#[test]
fn evaluatedateexpression_units_are_case_insensitive_like_the_keywords() {
    assert_eq!(
        evaluate("NOW + 2 Days - 3 HOURS"),
        s("2026-03-12T02:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:32 "evaluateDateExpression > a bare m is ambiguous and rejected"
#[test]
fn evaluatedateexpression_a_bare_m_is_ambiguous_and_rejected() {
    assert_eq!(evaluate("now + 5 m"), None);
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:36 "evaluateDateExpression > an ISO base keeps its own - and +offset"
#[test]
fn evaluatedateexpression_an_iso_base_keeps_its_own_and_offset() {
    assert_eq!(
        evaluate("2026-05-01T10:00:00+07:00 + 1 day"),
        s("2026-05-02T03:00:00.000Z")
    );
    assert_eq!(
        evaluate("2026-05-01T10:00:00Z - 1 day"),
        s("2026-04-30T10:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:45 "evaluateDateExpression > a naive base is anchored in the clock zone"
#[test]
fn evaluatedateexpression_a_naive_base_is_anchored_in_the_clock_zone() {
    assert_eq!(
        evaluate_at("2026-05-01 + 1 day", "Asia/Ho_Chi_Minh", now()),
        s("2026-05-01T17:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:51 "evaluateDateExpression > rejects malformed expressions"
#[test]
fn evaluatedateexpression_rejects_malformed_expressions() {
    for expression in [
        "",
        "   ",
        "garbage",
        "now + ",
        "now + 1",
        "now + 1.5 days",
        "now + 1 fortnight",
        "now 1 day",
        "now ++ 1 day",
        "now+1day",
    ] {
        assert_eq!(evaluate(expression), None, "{expression}");
    }
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:68 "evaluateDateExpression > enforces term count and magnitude limits"
#[test]
fn evaluatedateexpression_enforces_term_count_and_magnitude_limits() {
    let terms = |count: usize| vec!["+ 1 min"; count].join(" ");
    assert!(evaluate(&format!("now {}", terms(DATE_EXPRESSION_LIMITS.max_terms))).is_some());
    assert!(
        evaluate(&format!(
            "now {}",
            terms(DATE_EXPRESSION_LIMITS.max_terms + 1)
        ))
        .is_none()
    );
    assert!(
        evaluate(&format!(
            "now + {} min",
            DATE_EXPRESSION_LIMITS.max_magnitude
        ))
        .is_some()
    );
    assert!(
        evaluate(&format!(
            "now + {} min",
            DATE_EXPRESSION_LIMITS.max_magnitude + 1.0
        ))
        .is_none()
    );
    assert!(evaluate("now + 100000 year").is_none());
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:87 "evaluateDateExpression > calendar arithmetic > clamps the month end and handles a leap day"
#[test]
fn evaluatedateexpression_calendar_arithmetic_clamps_the_month_end_and_handles_a_leap_day() {
    let jan31 = support::iso_ms("2026-01-31T10:00:00.000Z");
    assert_eq!(
        evaluate_at("now + 1 month", "UTC", jan31),
        s("2026-02-28T10:00:00.000Z")
    );
    let leap = support::iso_ms("2028-02-29T10:00:00.000Z");
    assert_eq!(
        evaluate_at("now + 1 year", "UTC", leap),
        s("2029-02-28T10:00:00.000Z")
    );
    assert_eq!(
        evaluate_at(
            "now - 1 month",
            "UTC",
            support::iso_ms("2026-03-31T10:00:00.000Z")
        ),
        s("2026-02-28T10:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:101 "evaluateDateExpression > calendar arithmetic > a day is a calendar day across the spring-forward gap"
#[test]
fn evaluatedateexpression_calendar_arithmetic_a_day_is_a_calendar_day_across_the_spring_forward_gap()
 {
    let before = support::iso_ms("2026-03-07T15:30:00.000Z");
    assert_eq!(
        evaluate_at("now + 1 day", "America/New_York", before),
        s("2026-03-08T14:30:00.000Z")
    );
    assert_eq!(
        evaluate_at("now + 24 hours", "America/New_York", before),
        s("2026-03-08T15:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:112 "evaluateDateExpression > calendar arithmetic > a nonexistent local time moves forward by the gap"
#[test]
fn evaluatedateexpression_calendar_arithmetic_a_nonexistent_local_time_moves_forward_by_the_gap() {
    let before = support::iso_ms("2026-03-07T07:30:00.000Z");
    assert_eq!(
        evaluate_at("now + 1 day", "America/New_York", before),
        s("2026-03-08T07:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:119 "evaluateDateExpression > calendar arithmetic > an ambiguous local time picks the earlier instant on the autumn fold"
#[test]
fn evaluatedateexpression_calendar_arithmetic_an_ambiguous_local_time_picks_the_earlier_instant_on_the_autumn_fold()
 {
    let before = support::iso_ms("2026-10-31T05:30:00.000Z");
    assert_eq!(
        evaluate_at("now + 1 day", "America/New_York", before),
        s("2026-11-01T05:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:126 "evaluateDateExpression > calendar arithmetic > handles a zone that skipped a whole day"
#[test]
fn evaluatedateexpression_calendar_arithmetic_handles_a_zone_that_skipped_a_whole_day() {
    let before = support::iso_ms("2011-12-29T10:00:00.000Z");
    let result = evaluate_date_expression(
        "now + 1 day",
        &DateClock {
            now_ms: before,
            time_zone: "Pacific/Apia".to_owned(),
        },
    );
    assert!(result.is_some());
    assert!(result.unwrap() > before);
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:137 "evaluateDateExpression > calendar arithmetic > keeps sub-minute precision"
#[test]
fn evaluatedateexpression_calendar_arithmetic_keeps_sub_minute_precision() {
    let precise = support::iso_ms("2026-03-10T05:30:45.123Z");
    assert_eq!(
        evaluate_at("now + 1 day", "UTC", precise),
        s("2026-03-11T05:30:45.123Z")
    );
}

/// Port of: packages/utils/__tests__/date-expression.test.ts:144 "evaluateDateExpression > calendar arithmetic > rejects results outside the representable years"
#[test]
fn evaluatedateexpression_calendar_arithmetic_rejects_results_outside_the_representable_years() {
    assert_eq!(add_date_unit(now(), DateUnit::Year, -3000.0, "UTC"), None);
    assert_eq!(add_date_unit(now(), DateUnit::Year, 9000.0, "UTC"), None);
}
