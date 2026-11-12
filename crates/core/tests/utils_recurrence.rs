//! Port of `packages/utils/__tests__/recurrence.test.ts`. Instants are
//! epoch ms; `Date | null` is `Option<i64>`; helpers that can hit the step
//! limit return `Result` (unwrapped).

mod support;

use chatbotx_core::utils::recurrence::{
    MonthDay, RECURRENCE_MAX_EXCLUDED_DATES, RecurrenceEnd, RecurrenceFrequency, RecurrenceRule,
    compute_latest_occurrence, compute_next_occurrence, compute_next_sendable, find_slot_on_date,
    is_excluded_slot, list_occurrences_between, list_upcoming_occurrences, list_upcoming_slots,
    parse_recurrence_rule, parse_recurrence_rule_shape, recurrence_max_interval,
    resolve_zoned_instant, to_local_slot,
};
use chatbotx_core::utils::timezone::{canonicalize_timezone, is_valid_timezone};
use serde_json::{Value, json};

fn base() -> RecurrenceRule {
    RecurrenceRule {
        frequency: RecurrenceFrequency::Daily,
        interval: 1,
        weekdays: None,
        month_day: None,
        time_of_day: "09:00".to_owned(),
        timezone: "Asia/Ho_Chi_Minh".to_owned(),
        starts_on: "2026-01-01".to_owned(),
        exclude_dates: None,
        end: RecurrenceEnd::Never,
    }
}

fn base_json() -> Value {
    json!({
        "frequency": "daily",
        "interval": 1,
        "timeOfDay": "09:00",
        "timezone": "Asia/Ho_Chi_Minh",
        "startsOn": "2026-01-01",
        "end": { "kind": "never" },
    })
}

fn with(mut value: Value, key: &str, field: Value) -> Value {
    value[key] = field;
    value
}

fn iso(ms: Option<i64>) -> Option<String> {
    ms.map(support::iso)
}

fn isos(list: Vec<i64>) -> Vec<String> {
    list.into_iter().map(support::iso).collect()
}

fn at(value: &str) -> i64 {
    support::iso_ms(value)
}

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

fn issues_of(rule: &Value) -> Vec<String> {
    match parse_recurrence_rule(rule) {
        Ok(_) => Vec::new(),
        Err(issues) => issues.into_iter().map(|i| i.path).collect(),
    }
}

fn weekly() -> RecurrenceRule {
    RecurrenceRule {
        frequency: RecurrenceFrequency::Weekly,
        weekdays: Some(vec![1, 3]),
        starts_on: "2026-10-12".to_owned(),
        ..base()
    }
}

fn monthly() -> RecurrenceRule {
    RecurrenceRule {
        frequency: RecurrenceFrequency::Monthly,
        month_day: Some(MonthDay::Day(31)),
        starts_on: "2026-01-01".to_owned(),
        ..base()
    }
}

fn skipping(exclude_dates: &[&str]) -> RecurrenceRule {
    RecurrenceRule {
        exclude_dates: Some(exclude_dates.iter().map(|d| (*d).to_owned()).collect()),
        ..base()
    }
}

fn slots_of(rule: &RecurrenceRule, timezone: &str, after: &str, limit: usize) -> Vec<String> {
    list_upcoming_occurrences(rule, at(after), 0, limit)
        .unwrap()
        .into_iter()
        .map(|date| to_local_slot(date, timezone))
        .collect()
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:34 "resolveZonedInstant (explicit DST policy) > a nonexistent New York time shifts FORWARD past the gap"
#[test]
fn resolvezonedinstant_explicit_dst_policy_a_nonexistent_new_york_time_shifts_forward_past_the_gap()
{
    assert_eq!(
        support::iso(resolve_zoned_instant(
            "2026-03-08",
            "02:30",
            "America/New_York"
        )),
        "2026-03-08T07:30:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:40 "resolveZonedInstant (explicit DST policy) > an ambiguous New York time takes the EARLIER instant"
#[test]
fn resolvezonedinstant_explicit_dst_policy_an_ambiguous_new_york_time_takes_the_earlier_instant() {
    assert_eq!(
        support::iso(resolve_zoned_instant(
            "2026-11-01",
            "01:30",
            "America/New_York"
        )),
        "2026-11-01T05:30:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:46 "resolveZonedInstant (explicit DST policy) > London gap and overlap"
#[test]
fn resolvezonedinstant_explicit_dst_policy_london_gap_and_overlap() {
    assert_eq!(
        support::iso(resolve_zoned_instant(
            "2026-03-29",
            "01:30",
            "Europe/London"
        )),
        "2026-03-29T01:30:00.000Z"
    );
    assert_eq!(
        support::iso(resolve_zoned_instant(
            "2026-10-25",
            "01:30",
            "Europe/London"
        )),
        "2026-10-25T00:30:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:59 "resolveZonedInstant (explicit DST policy) > Lord Howe's 30-minute shift"
#[test]
fn resolvezonedinstant_explicit_dst_policy_lord_howe_s_30_minute_shift() {
    assert_eq!(
        support::iso(resolve_zoned_instant(
            "2026-10-04",
            "02:15",
            "Australia/Lord_Howe"
        )),
        "2026-10-03T15:45:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:65 "resolveZonedInstant (explicit DST policy) > a skipped whole day (Apia, 2011-12-30) moves to the next day"
#[test]
fn resolvezonedinstant_explicit_dst_policy_a_skipped_whole_day_apia_2011_12_30_moves_to_the_next_day()
 {
    assert_eq!(
        support::iso(resolve_zoned_instant("2011-12-30", "12:00", "Pacific/Apia")),
        "2011-12-30T22:00:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:73 "resolveZonedInstant (explicit DST policy) > an ordinary time is unchanged"
#[test]
fn resolvezonedinstant_explicit_dst_policy_an_ordinary_time_is_unchanged() {
    assert_eq!(
        support::iso(resolve_zoned_instant(
            "2026-06-15",
            "09:00",
            "Asia/Ho_Chi_Minh"
        )),
        "2026-06-15T02:00:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:81 "daily > next slot is the first strictly after `after`"
#[test]
fn daily_next_slot_is_the_first_strictly_after_after() {
    assert_eq!(
        iso(compute_next_occurrence(&base(), at("2026-01-01T02:00:00.000Z"), 0).unwrap()),
        s("2026-01-02T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:92 "daily > a start date in the future is the first slot"
#[test]
fn daily_a_start_date_in_the_future_is_the_first_slot() {
    let rule = RecurrenceRule {
        starts_on: "2026-03-01".to_owned(),
        ..base()
    };
    assert_eq!(
        iso(compute_next_occurrence(&rule, at("2026-01-01T00:00:00.000Z"), 0).unwrap()),
        s("2026-03-01T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:103 "daily > interval N steps from the start date"
#[test]
fn daily_interval_n_steps_from_the_start_date() {
    let rule = RecurrenceRule {
        interval: 3,
        ..base()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-01-01T00:00:00.000Z"), 0, 3).unwrap()),
        vec![
            "2026-01-01T02:00:00.000Z",
            "2026-01-04T02:00:00.000Z",
            "2026-01-07T02:00:00.000Z"
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:118 "daily > a start years in the past still resolves in a single step"
#[test]
fn daily_a_start_years_in_the_past_still_resolves_in_a_single_step() {
    let rule = RecurrenceRule {
        starts_on: "2000-01-01".to_owned(),
        ..base()
    };
    assert_eq!(
        iso(compute_next_occurrence(&rule, at("2026-10-13T05:00:00.000Z"), 0).unwrap()),
        s("2026-10-14T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:139 "weekly > fires on every selected weekday"
#[test]
fn weekly_fires_on_every_selected_weekday() {
    assert_eq!(
        isos(list_upcoming_occurrences(&weekly(), at("2026-10-11T00:00:00.000Z"), 0, 4).unwrap()),
        vec![
            "2026-10-12T02:00:00.000Z",
            "2026-10-14T02:00:00.000Z",
            "2026-10-19T02:00:00.000Z",
            "2026-10-21T02:00:00.000Z",
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:154 "weekly > interval 2 skips alternate ISO weeks"
#[test]
fn weekly_interval_2_skips_alternate_iso_weeks() {
    let rule = RecurrenceRule {
        interval: 2,
        ..weekly()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-10-11T00:00:00.000Z"), 0, 4).unwrap()),
        vec![
            "2026-10-12T02:00:00.000Z",
            "2026-10-14T02:00:00.000Z",
            "2026-10-26T02:00:00.000Z",
            "2026-10-28T02:00:00.000Z",
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:168 "weekly > weekdays before startsOn in its own week are skipped"
#[test]
fn weekly_weekdays_before_startson_in_its_own_week_are_skipped() {
    let rule = RecurrenceRule {
        starts_on: "2026-10-14".to_owned(),
        ..weekly()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-10-01T00:00:00.000Z"), 0, 2).unwrap()),
        vec!["2026-10-14T02:00:00.000Z", "2026-10-19T02:00:00.000Z"]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:178 "weekly > Sunday belongs to the preceding ISO week"
#[test]
fn weekly_sunday_belongs_to_the_preceding_iso_week() {
    let rule = RecurrenceRule {
        weekdays: Some(vec![0]),
        interval: 2,
        starts_on: "2026-10-12".to_owned(),
        ..weekly()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-10-11T00:00:00.000Z"), 0, 2).unwrap()),
        vec!["2026-10-18T02:00:00.000Z", "2026-11-01T02:00:00.000Z"]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:204 "monthly > day 31 clamps to the last day of shorter months"
#[test]
fn monthly_day_31_clamps_to_the_last_day_of_shorter_months() {
    assert_eq!(
        slots_of(
            &monthly(),
            "Asia/Ho_Chi_Minh",
            "2026-01-01T00:00:00.000Z",
            4
        ),
        vec![
            "2026-01-31 09:00",
            "2026-02-28 09:00",
            "2026-03-31 09:00",
            "2026-04-30 09:00"
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:219 "monthly > "last" is always the last day, including leap February"
#[test]
fn monthly_last_is_always_the_last_day_including_leap_february() {
    let rule = RecurrenceRule {
        month_day: Some(MonthDay::Last),
        starts_on: "2028-02-01".to_owned(),
        ..monthly()
    };
    assert_eq!(
        slots_of(&rule, "Asia/Ho_Chi_Minh", "2028-02-01T00:00:00.000Z", 2),
        vec!["2028-02-29 09:00", "2028-03-31 09:00"]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:228 "monthly > interval 24 months is still found (no search horizon)"
#[test]
fn monthly_interval_24_months_is_still_found_no_search_horizon() {
    let rule = RecurrenceRule {
        interval: recurrence_max_interval(RecurrenceFrequency::Monthly),
        month_day: Some(MonthDay::Day(15)),
        ..monthly()
    };
    let next = compute_next_occurrence(&rule, at("2026-01-16T00:00:00.000Z"), 1).unwrap();
    assert_eq!(
        to_local_slot(next.unwrap(), "Asia/Ho_Chi_Minh"),
        "2028-01-15 09:00"
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:243 "end conditions > onDate is inclusive"
#[test]
fn end_conditions_ondate_is_inclusive() {
    let rule = RecurrenceRule {
        end: RecurrenceEnd::OnDate("2026-01-03".to_owned()),
        ..base()
    };
    assert_eq!(
        slots_of(&rule, "Asia/Ho_Chi_Minh", "2025-12-31T00:00:00.000Z", 10),
        vec!["2026-01-01 09:00", "2026-01-02 09:00", "2026-01-03 09:00"]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:260 "end conditions > afterOccurrences counts the lifetime materialized total"
#[test]
fn end_conditions_afteroccurrences_counts_the_lifetime_materialized_total() {
    let rule = RecurrenceRule {
        end: RecurrenceEnd::AfterOccurrences(2),
        ..base()
    };
    assert!(
        compute_next_occurrence(&rule, at("2026-01-01T00:00:00.000Z"), 1)
            .unwrap()
            .is_some()
    );
    assert!(
        compute_next_occurrence(&rule, at("2026-01-01T00:00:00.000Z"), 2)
            .unwrap()
            .is_none()
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:281 "computeLatestOccurrence (catch-up) > returns the latest slot at or before now, decades after the start"
#[test]
fn computelatestoccurrence_catch_up_returns_the_latest_slot_at_or_before_now_decades_after_the_start()
 {
    let rule = RecurrenceRule {
        starts_on: "1990-01-01".to_owned(),
        ..base()
    };
    assert_eq!(
        iso(compute_latest_occurrence(&rule, at("2026-10-13T05:00:00.000Z"), 0).unwrap()),
        s("2026-10-13T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:293 "computeLatestOccurrence (catch-up) > before the first slot there is nothing to catch up"
#[test]
fn computelatestoccurrence_catch_up_before_the_first_slot_there_is_nothing_to_catch_up() {
    let rule = RecurrenceRule {
        starts_on: "2027-01-01".to_owned(),
        ..base()
    };
    assert_eq!(
        compute_latest_occurrence(&rule, at("2026-10-13T00:00:00.000Z"), 0).unwrap(),
        None
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:302 "computeLatestOccurrence (catch-up) > never goes past an end date"
#[test]
fn computelatestoccurrence_catch_up_never_goes_past_an_end_date() {
    let rule = RecurrenceRule {
        end: RecurrenceEnd::OnDate("2026-01-05".to_owned()),
        ..base()
    };
    assert_eq!(
        iso(compute_latest_occurrence(&rule, at("2026-06-01T00:00:00.000Z"), 0).unwrap()),
        s("2026-01-05T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:317 "computeLatestOccurrence (catch-up) > an end date decades before now still resolves its last slot"
#[test]
fn computelatestoccurrence_catch_up_an_end_date_decades_before_now_still_resolves_its_last_slot() {
    let rule = RecurrenceRule {
        starts_on: "1990-01-01".to_owned(),
        end: RecurrenceEnd::OnDate("1995-01-01".to_owned()),
        ..base()
    };
    assert_eq!(
        iso(compute_latest_occurrence(&rule, at("2026-10-13T05:00:00.000Z"), 0).unwrap()),
        s("1995-01-01T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:333 "computeLatestOccurrence (catch-up) > weekly picks the latest selected weekday"
#[test]
fn computelatestoccurrence_catch_up_weekly_picks_the_latest_selected_weekday() {
    let rule = RecurrenceRule {
        frequency: RecurrenceFrequency::Weekly,
        weekdays: Some(vec![1, 3]),
        starts_on: "2026-10-12".to_owned(),
        ..base()
    };
    assert_eq!(
        iso(compute_latest_occurrence(&rule, at("2026-10-17T00:00:00.000Z"), 0).unwrap()),
        s("2026-10-14T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:352 "listOccurrencesBetween > lists the slots in (after, through], oldest first"
#[test]
fn listoccurrencesbetween_lists_the_slots_in_after_through_oldest_first() {
    assert_eq!(
        isos(
            list_occurrences_between(
                &base(),
                at("2026-01-01T02:00:00.000Z"),
                at("2026-01-04T02:00:00.000Z"),
                0,
                10
            )
            .unwrap()
        ),
        vec![
            "2026-01-02T02:00:00.000Z",
            "2026-01-03T02:00:00.000Z",
            "2026-01-04T02:00:00.000Z"
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:367 "listOccurrencesBetween > honours the limit and an empty range"
#[test]
fn listoccurrencesbetween_honours_the_limit_and_an_empty_range() {
    assert_eq!(
        list_occurrences_between(
            &base(),
            at("2026-01-01T00:00:00.000Z"),
            at("2026-12-31T00:00:00.000Z"),
            0,
            2
        )
        .unwrap()
        .len(),
        2
    );
    assert_eq!(
        list_occurrences_between(
            &base(),
            at("2026-01-02T02:00:00.000Z"),
            at("2026-01-02T03:00:00.000Z"),
            0,
            5
        )
        .unwrap(),
        Vec::<i64>::new()
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:388 "DST + series > a daily 02:30 New York series moves forward on the gap day only"
#[test]
fn dst_series_a_daily_02_30_new_york_series_moves_forward_on_the_gap_day_only() {
    let rule = RecurrenceRule {
        timezone: "America/New_York".to_owned(),
        time_of_day: "02:30".to_owned(),
        starts_on: "2026-03-07".to_owned(),
        ..base()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-03-06T00:00:00.000Z"), 0, 3).unwrap()),
        vec![
            "2026-03-07T07:30:00.000Z",
            "2026-03-08T07:30:00.000Z",
            "2026-03-09T06:30:00.000Z"
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:417 "DST inside a series > an overlap day yields one slot at the earlier instant, no duplicate"
#[test]
fn dst_inside_a_series_an_overlap_day_yields_one_slot_at_the_earlier_instant_no_duplicate() {
    let rule = RecurrenceRule {
        timezone: "America/New_York".to_owned(),
        time_of_day: "01:30".to_owned(),
        starts_on: "2026-10-30".to_owned(),
        ..base()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-10-29T00:00:00.000Z"), 0, 4).unwrap()),
        vec![
            "2026-10-30T05:30:00.000Z",
            "2026-10-31T05:30:00.000Z",
            "2026-11-01T05:30:00.000Z",
            "2026-11-02T06:30:00.000Z",
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:437 "DST inside a series > a skipped whole day (Apia) shifts that slot to the next day, nothing is lost"
#[test]
fn dst_inside_a_series_a_skipped_whole_day_apia_shifts_that_slot_to_the_next_day_nothing_is_lost() {
    let rule = RecurrenceRule {
        timezone: "Pacific/Apia".to_owned(),
        time_of_day: "12:00".to_owned(),
        starts_on: "2011-12-28".to_owned(),
        ..base()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2011-12-27T00:00:00.000Z"), 0, 4).unwrap()),
        vec![
            "2011-12-28T22:00:00.000Z",
            "2011-12-29T22:00:00.000Z",
            "2011-12-30T22:00:00.000Z",
            "2011-12-31T22:00:00.000Z",
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:459 "maximum intervals > daily at its cap steps exactly that many days"
#[test]
fn maximum_intervals_daily_at_its_cap_steps_exactly_that_many_days() {
    let rule = RecurrenceRule {
        interval: recurrence_max_interval(RecurrenceFrequency::Daily),
        ..base()
    };
    let slots = list_upcoming_occurrences(&rule, at("2025-12-31T00:00:00.000Z"), 0, 2).unwrap();
    assert_eq!((slots[1] - slots[0]) as f64 / 86_400_000.0, 365.0);
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:469 "maximum intervals > weekly at its cap skips 51 weeks between active weeks"
#[test]
fn maximum_intervals_weekly_at_its_cap_skips_51_weeks_between_active_weeks() {
    let rule = RecurrenceRule {
        frequency: RecurrenceFrequency::Weekly,
        weekdays: Some(vec![6, 0]),
        interval: recurrence_max_interval(RecurrenceFrequency::Weekly),
        starts_on: "2026-10-12".to_owned(),
        ..base()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-10-11T00:00:00.000Z"), 0, 4).unwrap()),
        vec![
            "2026-10-17T02:00:00.000Z",
            "2026-10-18T02:00:00.000Z",
            "2027-10-16T02:00:00.000Z",
            "2027-10-17T02:00:00.000Z",
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:500 "recurrenceRuleSchema > accepts a valid rule"
#[test]
fn recurrenceruleschema_accepts_a_valid_rule() {
    assert!(parse_recurrence_rule(&base_json()).is_ok());
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:504 "recurrenceRuleSchema > rejects an offset timezone, an unknown zone and a fake date"
#[test]
fn recurrenceruleschema_rejects_an_offset_timezone_an_unknown_zone_and_a_fake_date() {
    assert!(
        issues_of(&with(base_json(), "timezone", json!("+07:00"))).contains(&"timezone".to_owned())
    );
    assert!(
        issues_of(&with(base_json(), "timezone", json!("Mars/Olympus")))
            .contains(&"timezone".to_owned())
    );
    assert!(
        issues_of(&with(base_json(), "startsOn", json!("2026-02-31")))
            .contains(&"startsOn".to_owned())
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:512 "recurrenceRuleSchema > weekly requires weekdays and monthly requires monthDay"
#[test]
fn recurrenceruleschema_weekly_requires_weekdays_and_monthly_requires_monthday() {
    assert!(
        issues_of(&with(base_json(), "frequency", json!("weekly")))
            .contains(&"weekdays".to_owned())
    );
    assert!(
        issues_of(&with(base_json(), "frequency", json!("monthly")))
            .contains(&"monthDay".to_owned())
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:517 "recurrenceRuleSchema > rejects an end date before the start and an over-cap interval"
#[test]
fn recurrenceruleschema_rejects_an_end_date_before_the_start_and_an_over_cap_interval() {
    assert!(
        issues_of(&with(
            base_json(),
            "end",
            json!({ "kind": "onDate", "date": "2025-12-31" })
        ))
        .contains(&"end.date".to_owned())
    );
    let over = recurrence_max_interval(RecurrenceFrequency::Daily) + 1;
    assert!(
        issues_of(&with(base_json(), "interval", json!(over))).contains(&"interval".to_owned())
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:526 "recurrenceRuleSchema > the unrefined shape still parses a rule the schema rejects"
#[test]
fn recurrenceruleschema_the_unrefined_shape_still_parses_a_rule_the_schema_rejects() {
    let stale = with(base_json(), "timezone", json!("Mars/Olympus"));
    assert!(parse_recurrence_rule(&stale).is_err());
    assert!(parse_recurrence_rule_shape(&stale).is_ok());
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:534 "timezone helpers > canonicalizes legacy names and validates strictly"
#[test]
fn timezone_helpers_canonicalizes_legacy_names_and_validates_strictly() {
    assert_eq!(canonicalize_timezone("Asia/Saigon"), "Asia/Ho_Chi_Minh");
    assert_eq!(canonicalize_timezone("Europe/London"), "Europe/London");
    assert!(is_valid_timezone("Asia/Saigon"));
    assert!(is_valid_timezone("UTC"));
    assert!(!is_valid_timezone("+07:00"));
    // Listed as an alias but not a zone the runtime can format with.
    assert!(!is_valid_timezone("US"));
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:551 "excludeDates > the slot of an excluded date is flagged but still generated"
#[test]
fn excludedates_the_slot_of_an_excluded_date_is_flagged_but_still_generated() {
    let rule = skipping(&["2026-01-02"]);
    assert_eq!(
        iso(compute_next_occurrence(&rule, at("2026-01-01T03:00:00Z"), 0).unwrap()),
        s("2026-01-02T02:00:00.000Z")
    );
    assert!(is_excluded_slot(&rule, at("2026-01-02T02:00:00Z")));
    assert!(!is_excluded_slot(&rule, at("2026-01-03T02:00:00Z")));
    assert!(!is_excluded_slot(&base(), at("2026-01-02T02:00:00Z")));
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:567 "excludeDates > sendable previews skip the date; slot previews flag it"
#[test]
fn excludedates_sendable_previews_skip_the_date_slot_previews_flag_it() {
    let rule = skipping(&["2026-01-02"]);
    let after = at("2026-01-01T00:00:00Z");
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, after, 0, 3).unwrap()),
        vec![
            "2026-01-01T02:00:00.000Z",
            "2026-01-03T02:00:00.000Z",
            "2026-01-04T02:00:00.000Z"
        ]
    );
    let flagged: Vec<(String, bool)> = list_upcoming_slots(&rule, after, 0, 3)
        .unwrap()
        .into_iter()
        .map(|slot| (slot.date, slot.excluded))
        .collect();
    assert_eq!(
        flagged,
        vec![
            ("2026-01-01".to_owned(), false),
            ("2026-01-02".to_owned(), true),
            ("2026-01-03".to_owned(), false),
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:591 "excludeDates > skipped slots do not consume afterOccurrences"
#[test]
fn excludedates_skipped_slots_do_not_consume_afteroccurrences() {
    let rule = RecurrenceRule {
        end: RecurrenceEnd::AfterOccurrences(2),
        ..skipping(&["2026-01-02"])
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-01-01T00:00:00Z"), 0, 5).unwrap()),
        vec!["2026-01-01T02:00:00.000Z", "2026-01-03T02:00:00.000Z"]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:605 "excludeDates > a weekly skip leaves the other weekdays alone"
#[test]
fn excludedates_a_weekly_skip_leaves_the_other_weekdays_alone() {
    let rule = RecurrenceRule {
        frequency: RecurrenceFrequency::Weekly,
        weekdays: Some(vec![1, 3]),
        starts_on: "2026-01-05".to_owned(),
        exclude_dates: Some(vec!["2026-01-07".to_owned()]),
        ..base()
    };
    assert_eq!(
        isos(list_upcoming_occurrences(&rule, at("2026-01-04T00:00:00Z"), 0, 3).unwrap()),
        vec![
            "2026-01-05T02:00:00.000Z",
            "2026-01-12T02:00:00.000Z",
            "2026-01-14T02:00:00.000Z"
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:626 "excludeDates > a clamped monthly slot is excluded by the date it lands on"
#[test]
fn excludedates_a_clamped_monthly_slot_is_excluded_by_the_date_it_lands_on() {
    let rule = RecurrenceRule {
        frequency: RecurrenceFrequency::Monthly,
        month_day: Some(MonthDay::Day(31)),
        starts_on: "2026-01-31".to_owned(),
        exclude_dates: Some(vec!["2026-02-28".to_owned()]),
        ..base()
    };
    let slots: Vec<(String, bool)> = list_upcoming_slots(&rule, at("2026-01-30T00:00:00Z"), 0, 3)
        .unwrap()
        .into_iter()
        .map(|slot| (slot.date, slot.excluded))
        .collect();
    assert_eq!(
        slots,
        vec![
            ("2026-01-31".to_owned(), false),
            ("2026-02-28".to_owned(), true),
            ("2026-03-31".to_owned(), false),
        ]
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:646 "excludeDates > a DST-shifted slot is excluded by its resolved local date"
#[test]
fn excludedates_a_dst_shifted_slot_is_excluded_by_its_resolved_local_date() {
    let rule = RecurrenceRule {
        timezone: "America/New_York".to_owned(),
        time_of_day: "02:30".to_owned(),
        starts_on: "2026-03-07".to_owned(),
        exclude_dates: Some(vec!["2026-03-08".to_owned()]),
        ..base()
    };
    let slots = list_upcoming_slots(&rule, at("2026-03-07T00:00:00Z"), 0, 2).unwrap();
    let shifted = &slots[1];
    assert_eq!(support::iso(shifted.at), "2026-03-08T07:30:00.000Z");
    assert!(shifted.excluded);
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:663 "excludeDates > computeNextSendable walks past consecutive skipped slots"
#[test]
fn excludedates_computenextsendable_walks_past_consecutive_skipped_slots() {
    let rule = skipping(&["2026-01-02", "2026-01-03"]);
    assert_eq!(
        iso(compute_next_sendable(&rule, at("2026-01-02T02:00:00Z"), 0).unwrap()),
        s("2026-01-04T02:00:00.000Z")
    );
    assert_eq!(
        iso(compute_next_sendable(&rule, at("2026-01-01T02:00:00Z"), 0).unwrap()),
        s("2026-01-01T02:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:683 "excludeDates > computeNextSendable is null when only skipped slots remain"
#[test]
fn excludedates_computenextsendable_is_null_when_only_skipped_slots_remain() {
    let rule = RecurrenceRule {
        end: RecurrenceEnd::OnDate("2026-01-03".to_owned()),
        ..skipping(&["2026-01-02", "2026-01-03"])
    };
    assert_eq!(
        compute_next_sendable(&rule, at("2026-01-02T02:00:00Z"), 0).unwrap(),
        None
    );
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:696 "excludeDates > findSlotOnDate returns the slot only on dates the rule runs"
#[test]
fn excludedates_findslotondate_returns_the_slot_only_on_dates_the_rule_runs() {
    let rule = RecurrenceRule {
        frequency: RecurrenceFrequency::Weekly,
        weekdays: Some(vec![1]),
        starts_on: "2026-01-05".to_owned(),
        ..base()
    };
    assert_eq!(
        iso(find_slot_on_date(&rule, "2026-01-12", 0).unwrap()),
        s("2026-01-12T02:00:00.000Z")
    );
    assert_eq!(find_slot_on_date(&rule, "2026-01-13", 0).unwrap(), None);
    assert_eq!(find_slot_on_date(&rule, "2025-12-29", 0).unwrap(), None);
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:714 "excludeDates > findSlotOnDate respects the end conditions"
#[test]
fn excludedates_findslotondate_respects_the_end_conditions() {
    let ended = RecurrenceRule {
        end: RecurrenceEnd::OnDate("2026-01-10".to_owned()),
        ..base()
    };
    assert_eq!(find_slot_on_date(&ended, "2026-01-11", 0).unwrap(), None);
    let counted = RecurrenceRule {
        end: RecurrenceEnd::AfterOccurrences(3),
        ..base()
    };
    assert_eq!(find_slot_on_date(&counted, "2026-01-05", 3).unwrap(), None);
}

/// Port of: packages/utils/__tests__/recurrence.test.ts:729 "excludeDates > schema rejects non-real dates and more than the cap"
#[test]
fn excludedates_schema_rejects_non_real_dates_and_more_than_the_cap() {
    assert!(
        parse_recurrence_rule(&with(base_json(), "excludeDates", json!(["2026-02-30"]))).is_err()
    );
    let too_many: Vec<String> = (0..=RECURRENCE_MAX_EXCLUDED_DATES as i64)
        .map(|index| {
            support::iso(support::iso_ms("2026-01-01T00:00:00Z") + index * 86_400_000)[..10]
                .to_owned()
        })
        .collect();
    assert!(parse_recurrence_rule(&with(base_json(), "excludeDates", json!(too_many))).is_err());
    assert!(
        parse_recurrence_rule(&with(base_json(), "excludeDates", json!(["2026-12-25"]))).is_ok()
    );
    assert!(parse_recurrence_rule(&base_json()).is_ok());
}
