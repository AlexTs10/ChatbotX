//! Port of `packages/utils/__tests__/recurrence-oracle.test.ts`. The
//! independent oracle walks every calendar day; its wall clock -> instant
//! step uses `chrono-tz` directly (the original uses date-fns-tz
//! `fromZonedTime`; every rule runs at 12:00, which no zone here skips or
//! repeats).
#![allow(clippy::double_ended_iterator_last)] // P3.1 orchestrator: style lint in locked ported test

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chatbotx_core::utils::recurrence::{
    MonthDay, RecurrenceEnd, RecurrenceFrequency, RecurrenceRule, compute_latest_occurrence,
    compute_next_occurrence, list_upcoming_occurrences, local_date_of,
};
use chrono::TimeZone;

const DAY_MS: i64 = 86_400_000;

fn parse_day(value: &str) -> i64 {
    let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap();
    date.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

fn iso_day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string()
}

fn utc(ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(ms).unwrap()
}

fn last_day_of_month(ms: i64) -> u32 {
    use chrono::Datelike;
    let d = utc(ms);
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    chrono::NaiveDate::from_ymd_opt(y, m, 1)
        .unwrap()
        .pred_opt()
        .unwrap()
        .day()
}

/// Monday-based week number counted from a fixed Monday.
fn week_index(ms: i64) -> i64 {
    (ms - parse_day("2024-01-01")).div_euclid(7 * DAY_MS)
}

fn selects(rule: &RecurrenceRule, ms: i64) -> bool {
    use chrono::Datelike;
    let start = parse_day(&rule.starts_on);
    if ms < start {
        return false;
    }
    let date = utc(ms);
    match rule.frequency {
        RecurrenceFrequency::Daily => {
            ((ms - start) as f64 / DAY_MS as f64).round() as i64 % rule.interval == 0
        }
        RecurrenceFrequency::Weekly => {
            (week_index(ms) - week_index(start)) % rule.interval == 0
                && rule
                    .weekdays
                    .as_ref()
                    .unwrap()
                    .contains(&i64::from(date.weekday().num_days_from_sunday()))
        }
        RecurrenceFrequency::Monthly => {
            let start_date = utc(start);
            let months_apart = i64::from(date.year() - start_date.year()) * 12
                + i64::from(date.month())
                - i64::from(start_date.month());
            if months_apart % rule.interval != 0 {
                return false;
            }
            let target = match rule.month_day.unwrap() {
                MonthDay::Last => last_day_of_month(ms),
                MonthDay::Day(d) => (d as u32).min(last_day_of_month(ms)),
            };
            date.day() == target
        }
    }
}

fn wall_clock_instant(day: &str, time_of_day: &str, timezone: &str) -> i64 {
    let tz: chrono_tz::Tz = timezone.parse().unwrap();
    let naive = chrono::NaiveDateTime::parse_from_str(
        &format!("{day}T{time_of_day}:00"),
        "%Y-%m-%dT%H:%M:%S",
    )
    .unwrap();
    tz.from_local_datetime(&naive)
        .earliest()
        .unwrap()
        .timestamp_millis()
}

fn walk_oracle(rule: &RecurrenceRule) -> Vec<i64> {
    let start = parse_day(&rule.starts_on);
    let mut out = Vec::new();
    for i in 0..2500 {
        let ms = start + i * DAY_MS;
        if !selects(rule, ms) {
            continue;
        }
        let instant = wall_clock_instant(&iso_day(ms), &rule.time_of_day, &rule.timezone);
        if let RecurrenceEnd::OnDate(end) = &rule.end {
            if iso_day(ms).as_str() > end.as_str() {
                break;
            }
        }
        out.push(instant);
        if let RecurrenceEnd::AfterOccurrences(count) = rule.end {
            if out.len() as i64 >= count {
                break;
            }
        }
    }
    out
}

fn oracle(rule: &RecurrenceRule) -> Vec<i64> {
    static RUNS: OnceLock<Mutex<HashMap<String, Vec<i64>>>> = OnceLock::new();
    let key = format!("{rule:?}");
    let runs = RUNS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = runs.lock().unwrap().get(&key) {
        return cached.clone();
    }
    let computed = walk_oracle(rule);
    runs.lock().unwrap().insert(key, computed.clone());
    computed
}

struct NamedRule {
    name: String,
    rule: RecurrenceRule,
}

fn rules() -> &'static Vec<NamedRule> {
    static RULES: OnceLock<Vec<NamedRule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let zones = [
            "UTC",
            "Asia/Ho_Chi_Minh",
            "America/New_York",
            "Pacific/Auckland",
        ];
        let starts = [
            "2026-01-31",
            "2026-02-28",
            "2026-03-01",
            "2026-10-11",
            "2026-10-12",
            "2026-12-30",
        ];
        let ends = [
            RecurrenceEnd::Never,
            RecurrenceEnd::OnDate("2027-02-14".to_owned()),
            RecurrenceEnd::AfterOccurrences(4),
        ];
        let end_kind = |end: &RecurrenceEnd| match end {
            RecurrenceEnd::Never => "never",
            RecurrenceEnd::OnDate(_) => "onDate",
            RecurrenceEnd::AfterOccurrences(_) => "afterOccurrences",
        };
        let base = |frequency, interval, timezone: &str, starts_on: &str, end: &RecurrenceEnd| {
            RecurrenceRule {
                frequency,
                interval,
                weekdays: None,
                month_day: None,
                time_of_day: "12:00".to_owned(),
                timezone: timezone.to_owned(),
                starts_on: starts_on.to_owned(),
                exclude_dates: None,
                end: end.clone(),
            }
        };
        let mut rules = Vec::new();
        for timezone in zones {
            for starts_on in starts {
                for end in &ends {
                    for interval in [1, 2, 3] {
                        rules.push(NamedRule {
                            name: format!(
                                "daily/{interval} {timezone} {starts_on} {}",
                                end_kind(end)
                            ),
                            rule: base(
                                RecurrenceFrequency::Daily,
                                interval,
                                timezone,
                                starts_on,
                                end,
                            ),
                        });
                        for weekdays in [
                            vec![1, 2, 3, 4, 5],
                            vec![0],
                            vec![6, 0],
                            vec![1, 4],
                            vec![3],
                        ] {
                            let label: Vec<String> = weekdays.iter().map(i64::to_string).collect();
                            rules.push(NamedRule {
                                name: format!(
                                    "weekly/{interval} {} {timezone} {starts_on} {}",
                                    label.join(","),
                                    end_kind(end)
                                ),
                                rule: RecurrenceRule {
                                    weekdays: Some(weekdays),
                                    ..base(
                                        RecurrenceFrequency::Weekly,
                                        interval,
                                        timezone,
                                        starts_on,
                                        end,
                                    )
                                },
                            });
                        }
                        for month_day in [
                            MonthDay::Day(1),
                            MonthDay::Day(15),
                            MonthDay::Day(29),
                            MonthDay::Day(30),
                            MonthDay::Day(31),
                            MonthDay::Last,
                        ] {
                            let label = match month_day {
                                MonthDay::Day(d) => d.to_string(),
                                MonthDay::Last => "last".to_owned(),
                            };
                            rules.push(NamedRule {
                                name: format!(
                                    "monthly/{interval} {label} {timezone} {starts_on} {}",
                                    end_kind(end)
                                ),
                                rule: RecurrenceRule {
                                    month_day: Some(month_day),
                                    ..base(
                                        RecurrenceFrequency::Monthly,
                                        interval,
                                        timezone,
                                        starts_on,
                                        end,
                                    )
                                },
                            });
                        }
                    }
                }
            }
        }
        rules
    })
}

fn uncapped(rule: &RecurrenceRule) -> RecurrenceRule {
    match rule.end {
        RecurrenceEnd::AfterOccurrences(_) => RecurrenceRule {
            end: RecurrenceEnd::Never,
            ..rule.clone()
        },
        _ => rule.clone(),
    }
}

/// Port of: packages/utils/__tests__/recurrence-oracle.test.ts:182 "recurrence math against an independent day-by-day oracle > covers 2592 combinations"
#[test]
fn recurrence_math_against_an_independent_day_by_day_oracle_covers_2592_combinations() {
    assert!(rules().len() > 1500);
}

/// Port of: packages/utils/__tests__/recurrence-oracle.test.ts:186 "recurrence math against an independent day-by-day oracle > listUpcomingOccurrences from before the start lists exactly the oracle's runs"
#[test]
fn recurrence_math_against_an_independent_day_by_day_oracle_listupcomingoccurrences_from_before_the_start_lists_exactly_the_oracle_s_runs()
 {
    let mut mismatches: Vec<String> = Vec::new();
    for NamedRule { name, rule } in rules() {
        let expected: Vec<i64> = oracle(rule).into_iter().take(12).collect();
        let actual =
            list_upcoming_occurrences(rule, parse_day(&rule.starts_on) - 3 * DAY_MS, 0, 12)
                .unwrap();
        if actual != expected {
            mismatches.push(format!(
                "{name}\n  oracle {:?}\n  actual {:?}",
                &expected[..expected.len().min(3)],
                &actual[..actual.len().min(3)]
            ));
        }
    }
    assert_eq!(
        mismatches.into_iter().take(5).collect::<Vec<_>>(),
        Vec::<String>::new()
    );
}

/// Port of: packages/utils/__tests__/recurrence-oracle.test.ts:204 "recurrence math against an independent day-by-day oracle > excludeDates removes exactly the oracle runs on those local dates"
#[test]
fn recurrence_math_against_an_independent_day_by_day_oracle_excludedates_removes_exactly_the_oracle_runs_on_those_local_dates()
 {
    let mut mismatches: Vec<String> = Vec::new();
    for NamedRule { name, rule } in rules() {
        let all: Vec<i64> = oracle(&uncapped(rule)).into_iter().take(14).collect();
        let exclude_dates: Vec<String> = [all.get(1), all.get(3), all.get(4)]
            .into_iter()
            .flatten()
            .map(|instant| local_date_of(*instant, &rule.timezone))
            .collect();
        let with_skips = RecurrenceRule {
            exclude_dates: Some(exclude_dates.clone()),
            ..rule.clone()
        };
        let survivors: Vec<i64> = all
            .iter()
            .copied()
            .filter(|instant| !exclude_dates.contains(&local_date_of(*instant, &rule.timezone)))
            .collect();
        let budget = match rule.end {
            RecurrenceEnd::AfterOccurrences(count) => count as usize,
            _ => survivors.len(),
        };
        let expected: Vec<i64> = survivors.into_iter().take(budget).collect();
        let actual = list_upcoming_occurrences(
            &with_skips,
            parse_day(&rule.starts_on) - 3 * DAY_MS,
            0,
            expected.len(),
        )
        .unwrap();
        if actual != expected {
            mismatches.push(name.clone());
        }
    }
    assert_eq!(
        mismatches.into_iter().take(5).collect::<Vec<_>>(),
        Vec::<String>::new()
    );
}

/// Port of: packages/utils/__tests__/recurrence-oracle.test.ts:237 "recurrence math against an independent day-by-day oracle > stepping with computeNextOccurrence walks the same sequence"
#[test]
fn recurrence_math_against_an_independent_day_by_day_oracle_stepping_with_computenextoccurrence_walks_the_same_sequence()
 {
    let mut mismatches: Vec<String> = Vec::new();
    for NamedRule { name, rule } in rules() {
        let expected: Vec<i64> = oracle(rule).into_iter().take(8).collect();
        let mut walked = Vec::new();
        let mut after = parse_day(&rule.starts_on) - DAY_MS;
        let mut count = 0;
        while walked.len() < expected.len() {
            let Some(next) = compute_next_occurrence(rule, after, count).unwrap() else {
                break;
            };
            walked.push(next);
            after = next;
            count += 1;
        }
        if walked != expected {
            mismatches.push(name.clone());
        }
    }
    assert_eq!(
        mismatches.into_iter().take(5).collect::<Vec<_>>(),
        Vec::<String>::new()
    );
}

/// Port of: packages/utils/__tests__/recurrence-oracle.test.ts:261 "recurrence math against an independent day-by-day oracle > computeLatestOccurrence is the last oracle run at or before any instant"
#[test]
fn recurrence_math_against_an_independent_day_by_day_oracle_computelatestoccurrence_is_the_last_oracle_run_at_or_before_any_instant()
 {
    let mut mismatches: Vec<String> = Vec::new();
    for NamedRule { name, rule } in rules() {
        let all = oracle(&uncapped(rule));
        for probe in [20, 100, 333] {
            let at = parse_day(&rule.starts_on) + probe * DAY_MS + 7 * 3_600_000;
            let expected = all.iter().copied().filter(|d| *d <= at).last();
            for occurrence_count in [0, 3, 4] {
                let exhausted = matches!(rule.end, RecurrenceEnd::AfterOccurrences(count) if occurrence_count >= count);
                let actual = compute_latest_occurrence(rule, at, occurrence_count).unwrap();
                let want = if exhausted { None } else { expected };
                if actual != want {
                    mismatches.push(format!("{name} @+{probe}d count {occurrence_count}"));
                }
            }
        }
    }
    assert_eq!(
        mismatches.into_iter().take(5).collect::<Vec<_>>(),
        Vec::<String>::new()
    );
}
