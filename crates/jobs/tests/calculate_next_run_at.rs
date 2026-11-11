//! Port of `packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts`
//! (spec 5.2). Times are unix ms; JavaScript `Date` identity (`toBe`) has no
//! Rust meaning, so those asserts compare values.

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use chatbotx_jobs::sequence::next_run::{
    NextRun, StepTiming, calculate_next_run_at, calculate_next_run_at_from_step,
    calculate_next_run_at_in,
};
use serde_json::json;

use sequence::{SEQ_ABC, STEP_TX, STEP_XYZ, STEP_ZERO, WS_1, id, iso_ms};
use support::TestClock;

const DAY_IN_MS: i64 = 24 * 60 * 60 * 1000;
const MINUTE_IN_MS: i64 = 60 * 1000;

fn step(days: i64, minutes: i64, unit: Option<&str>, specific: Option<i64>) -> StepTiming {
    StepTiming {
        delay_days: days,
        delay_minutes: minutes,
        delay_unit: unit.map(str::to_owned),
        specific_date_time_ms: specific,
    }
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:16 "calculateNextRunAtFromStep > returns a clone of specificDateTime when delayUnit is specificTime"
#[test]
fn returns_a_clone_of_specificdatetime_when_delayunit_is() {
    let specific = iso_ms("2024-06-01T10:00:00.000Z");
    // No base time: the default `new Date()` comes from the clock.
    let clock = TestClock::new(iso_ms("2030-01-01T00:00:00.000Z"));

    let result = calculate_next_run_at_from_step(
        &step(0, 0, Some("specificTime"), Some(specific)),
        None,
        clock.as_ref(),
    );

    assert_eq!(result, specific);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:34 "calculateNextRunAtFromStep > returns baseTime as same reference when delay is zero"
#[test]
fn returns_basetime_as_same_reference_when_delay_is_zero() {
    let base = iso_ms("2024-01-01T12:00:00.000Z");
    let clock = TestClock::new(0);

    let result =
        calculate_next_run_at_from_step(&step(0, 0, None, None), Some(base), clock.as_ref());

    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:51 "calculateNextRunAtFromStep > returns baseTime as same reference when both delayDays and delayMinutes are zero"
#[test]
fn returns_basetime_as_same_reference_when_both_delaydays() {
    let base = iso_ms("2024-03-15T09:30:00.000Z");
    let clock = TestClock::new(0);

    let result = calculate_next_run_at_from_step(
        &step(0, 0, Some("days"), None),
        Some(base),
        clock.as_ref(),
    );

    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:68 "calculateNextRunAtFromStep > adds days and minutes to baseTime when delay is positive"
#[test]
fn adds_days_and_minutes_to_basetime_when_delay_is_positive() {
    let base = iso_ms("2024-01-01T00:00:00.000Z");
    let clock = TestClock::new(0);

    let result =
        calculate_next_run_at_from_step(&step(2, 30, None, None), Some(base), clock.as_ref());

    assert_eq!(result, base + 2 * DAY_IN_MS + 30 * MINUTE_IN_MS);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:90 "calculateNextRunAtFromStep > adds only days when delayMinutes is zero"
#[test]
fn adds_only_days_when_delayminutes_is_zero() {
    let base = iso_ms("2024-01-10T06:00:00.000Z");
    let clock = TestClock::new(0);

    let result =
        calculate_next_run_at_from_step(&step(5, 0, None, None), Some(base), clock.as_ref());

    assert_eq!(result, base + 5 * 24 * 60 * 60 * 1000);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:108 "calculateNextRunAtFromStep > ignores specificDateTime when delayUnit is not specificTime"
#[test]
fn ignores_specificdatetime_when_delayunit_is_not_specifictime() {
    let base = iso_ms("2024-01-01T00:00:00.000Z");
    let specific = iso_ms("2099-12-31T00:00:00.000Z");
    let clock = TestClock::new(0);

    let result = calculate_next_run_at_from_step(
        &step(0, 0, Some("days"), Some(specific)),
        Some(base),
        clock.as_ref(),
    );

    // delay is zero → returns baseTime, not specificDateTime
    assert_eq!(result, base);
    assert_ne!(result, specific);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:133 "calculateNextRunAt > returns enrolledAt with null nextStepId when no first step is found"
#[tokio::test]
async fn returns_enrolledat_with_null_nextstepid_when_no_first_step() {
    let h = sequence::harness(Default::default()).await;
    let enrolled_at = iso_ms("2024-01-15T08:00:00.000Z");

    let result = calculate_next_run_at(&h.ctx, &id(SEQ_ABC), enrolled_at)
        .await
        .expect("next run");

    assert_eq!(
        result,
        NextRun {
            next_run_at_ms: enrolled_at,
            next_step_id: None,
        }
    );
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:143 "calculateNextRunAt > returns computed nextRunAt and step id when first step exists with a delay"
#[tokio::test]
async fn returns_computed_nextrunat_and_step_id_when_first_step() {
    let h = sequence::harness(Default::default()).await;
    sequence::seed_step(
        &h.db,
        json!({ "id": STEP_XYZ, "sequenceId": SEQ_ABC, "order": 0, "delayDays": 3 }),
    )
    .await;
    let enrolled_at = iso_ms("2024-01-01T00:00:00.000Z");

    let result = calculate_next_run_at(&h.ctx, &id(SEQ_ABC), enrolled_at)
        .await
        .expect("next run");

    assert_eq!(result.next_step_id, Some(id(STEP_XYZ)));
    assert_eq!(result.next_run_at_ms, enrolled_at + 3 * 24 * 60 * 60 * 1000);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:162 "calculateNextRunAt > returns enrolledAt as nextRunAt when first step has zero delay"
#[tokio::test]
async fn returns_enrolledat_as_nextrunat_when_first_step_has_zero() {
    let h = sequence::harness(Default::default()).await;
    sequence::seed_step(
        &h.db,
        json!({ "id": STEP_ZERO, "sequenceId": SEQ_ABC, "order": 0 }),
    )
    .await;
    let enrolled_at = iso_ms("2024-05-01T12:00:00.000Z");

    let result = calculate_next_run_at(&h.ctx, &id(SEQ_ABC), enrolled_at)
        .await
        .expect("next run");

    assert_eq!(result.next_step_id, Some(id(STEP_ZERO)));
    assert_eq!(result.next_run_at_ms, enrolled_at);
}

/// Port of: packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts:180 "calculateNextRunAt > uses provided transaction client instead of the default db"
///
/// The step exists only inside the caller's open transaction: the function
/// must read through that connection (the default db cannot see it).
#[tokio::test]
async fn uses_provided_transaction_client_instead_of_the_default_db() {
    let h = sequence::harness(Default::default()).await;
    let clock = TestClock::new(0);
    let enrolled_at = iso_ms("2024-01-01T00:00:00.000Z");
    sequence::ensure_sequence(&h.db, SEQ_ABC, WS_1).await;

    let seen =
        h.db.write(move |tx| {
            tx.execute(
                "INSERT INTO \"SequenceStep\" (\"id\", \"sequenceId\", \"order\", \
                 \"isActive\", \"delayDays\", \"delayMinutes\", \"anytime\") \
                 VALUES (?1, ?2, 0, 1, 1, 0, 1)",
                [STEP_TX, SEQ_ABC],
            )?;
            let result = calculate_next_run_at_in(tx, clock.as_ref(), &id(SEQ_ABC), enrolled_at)
                .expect("next run in tx");
            Ok(result)
        })
        .await
        .expect("write");

    assert_eq!(seen.next_step_id, Some(id(STEP_TX)));
    assert_eq!(seen.next_run_at_ms, enrolled_at + DAY_IN_MS);
}
