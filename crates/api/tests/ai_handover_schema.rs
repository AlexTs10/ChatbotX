//! Ports of `apps/builder/__tests__/ai-handover-bulk-schema.test.ts` and
//! `ai-handover-settings-request-schema.test.ts` (Q.D6): the builder form
//! schemas (`parse_private_*`) and the resource mappers (`run_json`,
//! `apply_to_all_status_json`).
//! Not applicable: `applyToAllPollInterval` (the card's TanStack polling,
//! frontend only).

use chatbotx_api::handlers::ai_handover::{
    apply_to_all_status_json, parse_private_set_apply_to_all, parse_private_settings, run_json,
};
use chatbotx_api::handlers::input::Input;
use chatbotx_business::ai_handover::ApplyToAllStatus;
use chatbotx_db::repo::ai_handover_bulk_run::RequestedBy;
use chatbotx_db::repo::models::AiHandoverBulkRun;
use serde_json::{Value, json};

fn input(body: Value) -> Input {
    let Value::Object(map) = body else {
        unreachable!()
    };
    Input::from_object(map)
}

/// `Ok(())` or the issue paths.
fn set_apply(body: Value) -> Result<(), Vec<Value>> {
    let mut parsed = input(body);
    parse_private_set_apply_to_all(&mut parsed);
    issues(parsed)
}

fn issues(parsed: Input) -> Result<(), Vec<Value>> {
    parsed.finish().map_err(|e| {
        e.data.expect("issues")["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|issue| issue["path"].clone())
            .collect()
    })
}

#[test]
fn an_on_needs_no_message() {
    assert!(set_apply(json!({ "applyToAllCustomers": true, "message": "" })).is_ok());
}

#[test]
fn an_off_with_a_blank_message_is_rejected_on_the_message_field() {
    for message in ["", "   ", "\n\t"] {
        let paths = set_apply(json!({ "applyToAllCustomers": false, "message": message }))
            .expect_err("blank");
        assert_eq!(paths[0], json!(["message"]));
    }
}

#[test]
fn an_off_accepts_up_to_2000_characters_and_rejects_more() {
    let off =
        |message: String| set_apply(json!({ "applyToAllCustomers": false, "message": message }));
    assert!(off("a".repeat(2000)).is_ok());
    assert!(off("a".repeat(2001)).is_err());
}

const NOW: i64 = 1_790_942_400_000; // 2026-10-02T12:00:00Z

fn run() -> AiHandoverBulkRun {
    AiHandoverBulkRun {
        id: 11,
        created_at: 0,
        updated_at: 0,
        workspace_id: 1,
        inbox_id: 2,
        channel: "messenger".into(),
        revision: 1,
        action: "disable".into(),
        status: "completed".into(),
        message: Some("hi".into()),
        requested_by_user_id: None,
        requested_at: NOW - 7_200_000,
        started_at: None,
        finished_at: None,
        last_heartbeat_at: None,
        paused_until: None,
        claim_token: None,
        attempts: 0,
        chunk_seq: 0,
        total_count: Some(6),
        processed_count: 3,
        skipped_count: 1,
        failed_count: 2,
        cursor_contact_inbox_id: None,
        in_flight_from_id: None,
        in_flight_to_id: None,
        current_error: None,
    }
}

fn user(name: Option<&str>) -> RequestedBy {
    RequestedBy {
        id: 5,
        name: name.map(str::to_owned),
        email: "l@x.io".into(),
    }
}

#[test]
fn shows_the_requesters_name_falling_back_to_the_email() {
    assert_eq!(
        run_json(&run(), Some(&user(Some("Linh"))), NOW)["requestedByName"],
        json!("Linh")
    );
    assert_eq!(
        run_json(&run(), Some(&user(None)), NOW)["requestedByName"],
        json!("l@x.io")
    );
}

#[test]
fn a_deleted_requester_has_no_name() {
    assert_eq!(run_json(&run(), None, NOW)["requestedByName"], Value::Null);
}

#[test]
fn a_live_run_with_a_future_paused_until_is_paused_until_then() {
    let live = AiHandoverBulkRun {
        status: "running".into(),
        paused_until: Some(NOW + 3_600_000),
        ..run()
    };
    assert_eq!(
        run_json(&live, None, NOW)["pausedUntil"],
        json!("2026-10-02T13:00:00.000Z")
    );
}

#[test]
fn an_elapsed_pause_or_a_finished_run_is_not_paused() {
    let elapsed = AiHandoverBulkRun {
        status: "running".into(),
        paused_until: Some(NOW - 60_000),
        ..run()
    };
    let finished = AiHandoverBulkRun {
        paused_until: Some(NOW + 1_800_000),
        ..run()
    };
    assert_eq!(run_json(&elapsed, None, NOW)["pausedUntil"], Value::Null);
    assert_eq!(run_json(&finished, None, NOW)["pausedUntil"], Value::Null);
}

#[test]
fn carries_the_counters_through_unchanged() {
    let out = run_json(&run(), None, NOW);
    assert_eq!(
        (
            out["processedCount"].clone(),
            out["skippedCount"].clone(),
            out["failedCount"].clone(),
            out["totalCount"].clone()
        ),
        (json!(3), json!(1), json!(2), json!(6))
    );
}

#[test]
fn a_page_never_switched_is_idle_with_no_run() {
    let state = ApplyToAllStatus {
        apply_to_all_customers: false,
        revision: 0,
        run: None,
    };
    assert_eq!(
        apply_to_all_status_json(&state, false, NOW),
        json!({ "status": "idle", "applyToAllCustomers": false, "isAutomationActive": false, "run": null })
    );
}

#[test]
fn a_stored_change_without_its_run_yet_is_reconciling() {
    let state = ApplyToAllStatus {
        apply_to_all_customers: true,
        revision: 2,
        run: None,
    };
    assert_eq!(
        apply_to_all_status_json(&state, true, NOW)["status"],
        json!("reconciling")
    );
}

#[test]
fn otherwise_the_status_is_the_runs_and_the_switch_is_the_desired_state() {
    let state = ApplyToAllStatus {
        apply_to_all_customers: true,
        revision: 2,
        run: Some(AiHandoverBulkRun {
            status: "running".into(),
            ..run()
        }),
    };
    let out = apply_to_all_status_json(&state, true, NOW);
    assert_eq!(
        (
            out["status"].clone(),
            out["applyToAllCustomers"].clone(),
            out["isAutomationActive"].clone()
        ),
        (json!("running"), json!(true), json!(true))
    );
    assert_eq!(out["run"]["id"], json!("11"));
}

// describe("saveAiHandoverSettingsRequest")

fn valid() -> Value {
    json!({
        "enabled": true,
        "scheduleEnabled": true,
        "timeRanges": [{ "from": 8, "to": 17 }],
        "gotoFlowId": null,
        "returnMessage": "",
        "pauseBotWaitingForStaff": false,
    })
}

fn parse(overrides: Value) -> Result<(), Vec<Value>> {
    let mut body = valid();
    for (k, v) in overrides.as_object().expect("object") {
        body[k] = v.clone();
    }
    let mut parsed = input(body);
    parse_private_settings(&mut parsed);
    issues(parsed)
}

#[test]
fn accepts_a_complete_valid_payload() {
    assert!(parse(json!({})).is_ok());
}

#[test]
fn accepts_a_midnight_wrapping_and_an_all_day_range() {
    assert!(
        parse(json!({ "timeRanges": [{ "from": 22, "to": 6 }, { "from": 0, "to": 24 }] })).is_ok()
    );
}

#[test]
fn accepts_no_ranges_when_the_schedule_is_off() {
    assert!(parse(json!({ "scheduleEnabled": false, "timeRanges": [] })).is_ok());
}

#[test]
fn rejects_an_enabled_schedule_with_no_range_on_the_time_ranges_path() {
    let paths = parse(json!({ "scheduleEnabled": true, "timeRanges": [] })).expect_err("no range");
    assert_eq!(paths[0], json!(["timeRanges"]));
}

#[test]
fn rejects_a_range_with_equal_bounds_above_24_negative_or_fractional() {
    for range in [
        json!({ "from": 9, "to": 9 }),
        json!({ "from": 0, "to": 25 }),
        json!({ "from": -1, "to": 5 }),
        json!({ "from": 8.5, "to": 17 }),
    ] {
        assert!(parse(json!({ "timeRanges": [range] })).is_err(), "{range}");
    }
}

#[test]
fn rejects_a_return_message_over_2000_characters() {
    assert!(parse(json!({ "returnMessage": "x".repeat(2001) })).is_err());
    assert!(parse(json!({ "returnMessage": "x".repeat(2000) })).is_ok());
}

#[test]
fn accepts_a_flow_id_string_or_null_rejects_a_non_id() {
    assert!(parse(json!({ "gotoFlowId": "123456789" })).is_ok());
    assert!(parse(json!({ "gotoFlowId": null })).is_ok());
    assert!(parse(json!({ "gotoFlowId": "not-an-id" })).is_err());
}
