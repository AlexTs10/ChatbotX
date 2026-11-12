//! Port of `packages/flow-config/__tests__/follow-up.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use chatbotx_core::TestClock;
use chrono::{DateTime, Utc};

#[test]
fn follow_up_step_schema_creates_the_default_one_hour_follow_up_step() {
    assert_match_object(
        &fc::follow_up_step_default_fn(),
        &json!({ "stepType": "followUp", "duration": 1, "unit": "hours" }),
    );
}

#[test]
fn follow_up_step_schema_accepts_the_366_day_boundary_for_duration_unit() {
    for delay in [
        json!({ "duration": 366, "unit": "days" }),
        json!({ "duration": 8784, "unit": "hours" }),
    ] {
        let value = spread(&json!({ "id": "1", "stepType": "followUp" }), delay);
        assert!(
            fc::follow_up_step_schema().safe_parse(&value).success,
            "{value}"
        );
    }
}

#[test]
fn follow_up_step_schema_rejects_delays_longer_than_366_days_for_duration_unit() {
    for delay in [
        json!({ "duration": 367, "unit": "days" }),
        json!({ "duration": 8785, "unit": "hours" }),
    ] {
        let result = fc::follow_up_step_schema().safe_parse(&spread(
            &json!({ "id": "1", "stepType": "followUp" }),
            delay,
        ));

        assert!(!result.success);
        assert_eq!(result.issues()[0].path_json(), json!(["duration"]));
    }
}

#[test]
fn follow_up_timing_helpers_maps_s_to_milliseconds() {
    for (unit, expected_ms) in [
        ("seconds", 1000),
        ("minutes", 60_000),
        ("hours", 3_600_000),
        ("days", 86_400_000),
    ] {
        assert_eq!(fc::delay_unit_to_ms(unit), expected_ms);
    }
}

fn at(iso: &str) -> DateTime<Utc> {
    iso.parse().unwrap()
}

#[test]
fn follow_up_timing_helpers_computes_triggerat_using_the_selected_delay_unit() {
    let clock = TestClock::new(at("2026-07-16T00:00:00.000Z").timestamp_millis());

    let trigger_at = fc::compute_follow_up_trigger_at(
        &json!({ "id": "1", "stepType": "followUp", "duration": 2, "unit": "minutes" }),
        &clock,
    );

    assert_eq!(trigger_at, at("2026-07-16T00:02:00.000Z"));
}

#[test]
fn follow_up_timing_helpers_wait_duration_schema_still_parses_after_delay_unit_refactor() {
    assert!(
        fc::wait_step_schema()
            .safe_parse(&json!({
                "id": "1",
                "stepType": "wait",
                "delayType": "duration",
                "duration": 1,
                "unit": "hours",
                "interval": false,
                "startTime": null,
                "endTime": null,
            }))
            .success
    );
}

#[test]
fn follow_up_timing_helpers_buildjobid_includes_triggerat_so_rescheduled_rows_arm_new_wake_ups() {
    assert_eq!(
        fc::build_job_id("row-1", at("2026-07-16T00:01:00.000Z")),
        "smart-delay-row-1-1784160060000"
    );
}
