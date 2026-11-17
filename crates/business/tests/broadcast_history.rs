//! Port of the history and skip parts of `broadcast-recurrence-service.test.ts`
//! (`run groups`, `listOccurrences`, `setOccurrenceExcluded`, `nextSendAtOf`,
//! `previewUpcoming with skipped dates`) (Q.D5). Adapted: the mocked
//! repository becomes stored rows; the series runs in 2030 (the database clock
//! is the real one), so `SLOT` is 2030-10-13 09:00 in Ho Chi Minh.

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::BusinessError;
use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::broadcast::{history, recurrence, write};
use chatbotx_core::js::parse_date;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::broadcast_recurrence::LedgerBucketCell;
use chatbotx_db::repo::queries::date_bucket::Granularity;
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, scalar_i64, scalar_text, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const DAY: i64 = 86_400_000;

fn ms(iso: &str) -> i64 {
    parse_date(iso).expect("date")
}

/// 09:00 in Ho Chi Minh.
fn slot() -> i64 {
    ms("2030-10-13T02:00:00.000Z")
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ..AuditActor::default()
    }
}

fn audit() -> (AuditService, Arc<RecordingEnqueuer>) {
    let jobs = RecordingEnqueuer::new();
    (AuditService::new(jobs.clone()), jobs)
}

fn details(jobs: &RecordingEnqueuer) -> Vec<String> {
    jobs.calls()
        .iter()
        .filter_map(|c| c.data["data"]["detail"].as_str().map(str::to_owned))
        .collect()
}

fn message(error: &BusinessError) -> String {
    match error {
        BusinessError::Exception { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn field(error: &BusinessError) -> Option<String> {
    match error {
        BusinessError::Exception { field, .. } => field.clone(),
        _ => None,
    }
}

fn rule() -> Value {
    json!({
        "frequency": "daily",
        "interval": 1,
        "timeOfDay": "09:00",
        "timezone": "Asia/Ho_Chi_Minh",
        "startsOn": "2030-10-01",
        "end": { "kind": "never" },
    })
}

fn rule_with(overrides: Value) -> Value {
    let mut out = rule();
    for (k, v) in overrides.as_object().expect("object") {
        out[k] = v.clone();
    }
    out
}

async fn seeded_db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Promo flow')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    db
}

/// A running series like `makeRecurrence()`: armed at `SLOT`, accounted through
/// the day before. Returns `(template id, recurrence id)`.
async fn series(db: &TestDb) -> (i64, i64) {
    let (audit, _) = audit();
    let draft = write::create(
        db,
        &audit,
        WS,
        json!({
            "channel": "omnichannel",
            "subaction": "allContacts",
            "flowId": "61",
            "schedulesType": "recurring",
            "recurrence": rule(),
            "saveAsDraft": true,
            "contactFilter": { "operator": "and", "conditions": [] },
        }),
        true,
    )
    .await
    .expect("draft");
    write::schedule_draft(
        db,
        &audit,
        None,
        WS,
        draft.id,
        "recurring".into(),
        0,
        None,
        None,
    )
    .await
    .expect("schedule");
    let id = draft.id;
    let (next, accounted) = (slot(), slot() - DAY);
    setup(db, move |tx| {
        tx.execute(
            r#"UPDATE "BroadcastRecurrence" SET "nextRunAt" = ?1, "accountedThroughAt" = ?2 WHERE "broadcastId" = ?3"#,
            params![next, accounted, id],
        )?;
        Ok(())
    })
    .await;
    let recurrence = scalar_i64(
        db,
        r#"SELECT "id" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
        id,
    )
    .await
    .expect("recurrence");
    (id, recurrence)
}

/// Sets the series' columns (`rule` as JSON) like `makeRecurrence(overrides)`.
async fn set_recurrence(db: &TestDb, template: i64, column: &'static str, value: Value) {
    setup(db, move |tx| {
        let sql =
            format!(r#"UPDATE "BroadcastRecurrence" SET "{column}" = ?1 WHERE "broadcastId" = ?2"#);
        let bound: chatbotx_db::rusqlite::types::Value = match value {
            Value::Null => chatbotx_db::rusqlite::types::Value::Null,
            Value::Number(n) => n.as_i64().expect("int").into(),
            other => other.to_string().into(),
        };
        tx.execute(&sql, params![bound, template])?;
        Ok(())
    })
    .await;
}

async fn set_template(db: &TestDb, template: i64, assignment: &'static str) {
    setup(db, move |tx| {
        let sql = format!(r#"UPDATE "Broadcast" SET {assignment} WHERE "id" = ?1"#);
        tx.execute(&sql, params![template])?;
        Ok(())
    })
    .await;
}

async fn stored_dates(db: &TestDb, template: i64) -> Value {
    let rule = scalar_text(
        db,
        r#"SELECT "rule" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
        template,
    )
    .await
    .expect("rule");
    serde_json::from_str::<Value>(&rule).expect("json")["excludeDates"].clone()
}

async fn next_run_at(db: &TestDb, template: i64) -> Option<i64> {
    scalar_i64(
        db,
        r#"SELECT "nextRunAt" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
        template,
    )
    .await
}

async fn skip(
    db: &TestDb,
    audit: &AuditService,
    template: i64,
    dates: &[&str],
    ats: &[&str],
    excluded: bool,
) -> chatbotx_business::Result<Value> {
    history::set_occurrence_excluded(
        db,
        audit,
        WS,
        template.to_string(),
        dates.iter().map(|d| (*d).to_owned()).collect(),
        ats.iter().map(|a| ms(a)).collect(),
        excluded,
    )
    .await
}

fn iso(ms: i64) -> Value {
    json!(chatbotx_core::js::to_iso_string(ms).expect("iso"))
}

// describe("setOccurrenceExcluded")

#[tokio::test]
async fn skips_a_future_run_only_the_rule_changes_never_the_cursor() {
    with_audit_context(actor(), async {
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        let (audit, jobs) = audit();
        let state = skip(&db, &audit, template, &["2030-10-15"], &[], true)
            .await
            .expect("skip");
        assert_eq!(stored_dates(&db, template).await, json!(["2030-10-15"]));
        assert_eq!(next_run_at(&db, template).await, Some(slot()));
        assert_eq!(state["excludeDates"], json!(["2030-10-15"]));
        assert_eq!(state["nextSendAt"], iso(slot()));
        assert!(details(&jobs).iter().any(|d| d.contains("2030-10-15")));
    })
    .await
}

#[tokio::test]
async fn the_next_real_send_moves_past_a_skipped_armed_slot() {
    with_audit_context(actor(), async {
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(
            &db,
            template,
            "rule",
            rule_with(json!({ "excludeDates": ["2030-10-13"] })),
        )
        .await;
        let (audit, _) = audit();
        let state = skip(&db, &audit, template, &["2030-10-14"], &[], true)
            .await
            .expect("skip");
        assert_eq!(
            stored_dates(&db, template).await,
            json!(["2030-10-13", "2030-10-14"])
        );
        assert_eq!(state["nextSendAt"], iso(slot() + 2 * DAY));
    })
    .await
}

#[tokio::test]
async fn restoring_removes_the_date_and_the_last_one_clears_the_list() {
    with_audit_context(actor(), async {
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        let (audit, _) = audit();
        set_recurrence(
            &db,
            template,
            "rule",
            rule_with(json!({ "excludeDates": ["2030-10-15", "2030-10-20"] })),
        )
        .await;
        let state = skip(&db, &audit, template, &["2030-10-15"], &[], false)
            .await
            .expect("restore");
        assert_eq!(stored_dates(&db, template).await, json!(["2030-10-20"]));
        assert_eq!(state["excludeDates"], json!(["2030-10-20"]));
        // restoring the last date clears the list
        skip(&db, &audit, template, &["2030-10-20"], &[], false)
            .await
            .expect("restore last");
        assert_eq!(stored_dates(&db, template).await, Value::Null);
    })
    .await
}

#[tokio::test]
async fn a_date_already_in_the_wanted_state_changes_nothing() {
    with_audit_context(actor(), async {
        for (excluded, dates) in [
            (true, json!(["2030-10-15"])),
            (false, json!(["2030-10-20"])),
        ] {
            let db = seeded_db().await;
            let (template, _) = series(&db).await;
            set_recurrence(
                &db,
                template,
                "rule",
                rule_with(json!({ "excludeDates": dates })),
            )
            .await;
            let (audit, jobs) = audit();
            let before = details(&jobs).len();
            let state = skip(&db, &audit, template, &["2030-10-15"], &[], excluded)
                .await
                .expect("no-op");
            assert_eq!(details(&jobs).len(), before);
            assert_eq!(state["excludeDates"], dates);
            assert_eq!(stored_dates(&db, template).await, dates);
        }
    })
    .await
}

#[tokio::test]
async fn rejects_a_date_the_schedule_has_no_run_on_but_a_leftover_can_be_removed() {
    with_audit_context(actor(), async {
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        let weekly_mondays =
            rule_with(json!({ "frequency": "weekly", "weekdays": [1], "startsOn": "2030-10-07" }));
        set_recurrence(&db, template, "rule", weekly_mondays.clone()).await;
        let (audit, _) = audit();
        let err = skip(&db, &audit, template, &["2030-10-15"], &[], true) // a Tuesday
            .await
            .expect_err("no run");
        assert_eq!(field(&err).as_deref(), Some("recurrence"));
        assert_eq!(stored_dates(&db, template).await, Value::Null);
        // a leftover date that no longer matches a run can still be removed
        let mut leftover = weekly_mondays;
        leftover["excludeDates"] = json!(["2030-10-15"]);
        set_recurrence(&db, template, "rule", leftover).await;
        skip(&db, &audit, template, &["2030-10-15"], &[], false)
            .await
            .expect("removed");
        assert_eq!(stored_dates(&db, template).await, Value::Null);
    })
    .await
}

#[tokio::test]
async fn rejects_a_run_that_is_due_and_a_run_the_series_already_settled() {
    with_audit_context(actor(), async {
        // a run that is due now: a series whose slots are in the past
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(
            &db,
            template,
            "rule",
            rule_with(json!({ "startsOn": "2026-10-01" })),
        )
        .await;
        set_recurrence(&db, template, "accountedThroughAt", Value::Null).await;
        let (audit, _) = audit();
        let err = skip(&db, &audit, template, &["2026-10-13"], &[], true)
            .await
            .expect_err("due");
        assert_eq!(
            message(&err),
            "That run has already started and can no longer change"
        );
        // a run the series already settled
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(&db, template, "accountedThroughAt", json!(slot() + 2 * DAY)).await;
        let err = skip(&db, &audit, template, &["2030-10-14"], &[], true)
            .await
            .expect_err("settled");
        assert_eq!(field(&err).as_deref(), Some("recurrence"));
        assert_eq!(stored_dates(&db, template).await, Value::Null);
    })
    .await
}

#[tokio::test]
async fn rejects_a_finished_series_a_draft_a_deleted_broadcast_and_a_run_of_a_series() {
    with_audit_context(actor(), async {
        let (audit, _) = audit();
        // a finished series
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(&db, template, "endedAt", json!(slot())).await;
        let err = skip(&db, &audit, template, &["2030-10-15"], &[], true)
            .await
            .expect_err("ended");
        assert_eq!(
            message(&err),
            "Only a repeating broadcast that is running or paused can skip a run"
        );
        for assignment in [
            r#""status" = 'draft'"#,
            r#""deletedAt" = 1"#,
            r#""occurrenceAt" = 1"#,
        ] {
            let db = seeded_db().await;
            let (template, _) = series(&db).await;
            set_template(&db, template, assignment).await;
            let err = skip(&db, &audit, template, &["2030-10-15"], &[], true)
                .await
                .expect_err(assignment);
            if assignment.contains("deletedAt") {
                // Changed: the real `findByIdOrName` skips a deleted row (404); the TS test mocks it.
                assert_eq!(err.code(), Some("notFound"));
            } else {
                assert_eq!(field(&err).as_deref(), Some("recurrence"), "{assignment}");
            }
        }
    })
    .await
}

#[tokio::test]
async fn works_on_a_paused_series() {
    with_audit_context(actor(), async {
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(&db, template, "nextRunAt", Value::Null).await;
        set_recurrence(&db, template, "pausedAt", json!(slot())).await;
        set_template(&db, template, r#""status" = 'cancelled'"#).await;
        let (audit, _) = audit();
        let state = skip(&db, &audit, template, &["2030-10-15"], &[], true)
            .await
            .expect("paused");
        assert_eq!(stored_dates(&db, template).await, json!(["2030-10-15"]));
        assert_eq!(state["nextSendAt"], Value::Null);
        assert_eq!(state["upcomingRuns"], json!([]));
    })
    .await
}

#[tokio::test]
async fn drops_dates_the_series_already_passed_and_refuses_to_grow_past_the_cap() {
    with_audit_context(actor(), async {
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(
            &db,
            template,
            "rule",
            rule_with(json!({ "excludeDates": ["2030-10-01", "2030-10-20"] })),
        )
        .await;
        let (audit, _) = audit();
        skip(&db, &audit, template, &["2030-10-25"], &[], true)
            .await
            .expect("skip");
        assert_eq!(
            stored_dates(&db, template).await,
            json!(["2030-10-20", "2030-10-25"])
        );
        // refuses to grow past the cap
        let full: Vec<String> = (0..366)
            .map(|i| {
                chatbotx_core::js::to_iso_string(ms("2031-01-01T00:00:00.000Z") + i * DAY)
                    .expect("iso")[..10]
                    .to_owned()
            })
            .collect();
        set_recurrence(
            &db,
            template,
            "rule",
            rule_with(json!({ "excludeDates": full })),
        )
        .await;
        let err = skip(&db, &audit, template, &["2030-10-15"], &[], true)
            .await
            .expect_err("cap");
        assert_eq!(message(&err), "Skip at most 366 dates");
    })
    .await
}

// describe("several runs at once")

#[tokio::test]
async fn several_runs_at_once_by_date_and_instant() {
    with_audit_context(actor(), async {
        let (audit, jobs) = audit();
        // skips every named date in one write
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        let state = skip(
            &db,
            &audit,
            template,
            &["2030-10-16", "2030-10-15", "2030-10-15"],
            &[],
            true,
        )
        .await
        .expect("batch");
        assert_eq!(state["excludeDates"], json!(["2030-10-15", "2030-10-16"]));
        assert!(
            details(&jobs)
                .iter()
                .any(|d| d.contains("2030-10-15, 2030-10-16"))
        );
        // names runs by their instant, read in the series' timezone
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        let state = skip(&db, &audit, template, &[], &["2030-10-15T02:00:00Z"], true)
            .await
            .expect("instant");
        assert_eq!(state["excludeDates"], json!(["2030-10-15"]));
        // an instant late in the UTC day can fall on the next local day
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        let state = skip(&db, &audit, template, &[], &["2030-10-15T18:00:00Z"], true)
            .await
            .expect("late instant");
        assert_eq!(state["excludeDates"], json!(["2030-10-16"]));
        // dates and instants combine and deduplicate
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        let state = skip(
            &db,
            &audit,
            template,
            &["2030-10-15"],
            &["2030-10-15T02:00:00Z"],
            true,
        )
        .await
        .expect("combined");
        assert_eq!(state["excludeDates"], json!(["2030-10-15"]));
    })
    .await
}

#[tokio::test]
async fn one_bad_date_rejects_the_whole_request_and_only_pending_dates_apply() {
    with_audit_context(actor(), async {
        let (audit, jobs) = audit();
        // one bad date rejects the whole request (the 13th is settled here)
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(&db, template, "accountedThroughAt", json!(slot())).await;
        let before = details(&jobs).len();
        skip(
            &db,
            &audit,
            template,
            &["2030-10-15", "2030-10-13"],
            &[],
            true,
        )
        .await
        .expect_err("bad date");
        assert_eq!(stored_dates(&db, template).await, Value::Null);
        assert_eq!(details(&jobs).len(), before);
        // only the dates not yet in the wanted state are applied
        let db = seeded_db().await;
        let (template, _) = series(&db).await;
        set_recurrence(
            &db,
            template,
            "rule",
            rule_with(json!({ "excludeDates": ["2030-10-15"] })),
        )
        .await;
        let state = skip(
            &db,
            &audit,
            template,
            &["2030-10-15", "2030-10-16"],
            &[],
            true,
        )
        .await
        .expect("pending");
        assert_eq!(state["excludeDates"], json!(["2030-10-15", "2030-10-16"]));
        assert!(
            details(&jobs)
                .last()
                .is_some_and(|d| d.contains("on 2030-10-16 ("))
        );
        // restores several dates together
        set_recurrence(
            &db,
            template,
            "rule",
            rule_with(json!({ "excludeDates": ["2030-10-15", "2030-10-16", "2030-10-20"] })),
        )
        .await;
        let state = skip(
            &db,
            &audit,
            template,
            &["2030-10-15", "2030-10-16"],
            &[],
            false,
        )
        .await
        .expect("restore");
        assert_eq!(state["excludeDates"], json!(["2030-10-20"]));
        // a request that names no run is rejected
        let err = skip(&db, &audit, template, &[], &[], true)
            .await
            .expect_err("nothing named");
        assert_eq!(message(&err), "Name at least one run to skip or restore");
    })
    .await
}

#[tokio::test]
async fn a_missing_broadcast_is_a_not_found() {
    let db = seeded_db().await;
    let (audit, _) = audit();
    let err = history::set_occurrence_excluded(
        &db,
        &audit,
        WS,
        "nope".into(),
        vec!["2030-10-15".into()],
        Vec::new(),
        true,
    )
    .await
    .expect_err("missing");
    assert_eq!(err.code(), Some("notFound"));
}

// describe("run groups")

/// Runs 501 (sent, 2030-10-14) with one delivered recipient, and ledger rows
/// for 2030-10-14 (created), 10-13 (missed), 10-12 (skipped).
async fn ledger(db: &TestDb, template: i64, recurrence: i64) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "parentBroadcastId", "occurrenceAt")
               VALUES (501, 'run', ?1, 'sent', 'now', 0, 'allContacts', 'omnichannel', ?2, ?3)"#,
            params![WS, template, ms("2030-10-14T02:00:00Z")],
        )?;
        common::inbox(tx, 81, WS, "webchat")?;
        common::contact(tx, 91, WS)?;
        common::contact_inbox(tx, 191, 91, 81, "webchat", "s91")?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "contactId", "workspaceId") VALUES (95, 91, ?1)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "sent", "deliveredAt") VALUES (501, 91, 191, 95, 1, 5)"#,
            [],
        )?;
        for (id, local, at, outcome, reason, child) in [
            (901, "2030-10-12 09:00", "2030-10-12T02:00:00Z", "skipped", Some("paused"), None),
            (902, "2030-10-13 09:00", "2030-10-13T02:00:00Z", "missed", Some("catchUpSuperseded"), None),
            (903, "2030-10-14 09:00", "2030-10-14T02:00:00Z", "created", None, Some(501)),
        ] {
            tx.execute(
                r#"INSERT INTO "BroadcastRecurrenceOccurrence" ("id", "workspaceId", "recurrenceId", "localSlot", "occurrenceAt", "outcome", "reason", "childBroadcastId", "materializedAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?5)"#,
                params![id, WS, recurrence, local, ms(at), outcome, reason, child],
            )?;
        }
        tx.execute(
            r#"UPDATE "BroadcastRecurrence" SET "occurrenceCount" = 4 WHERE "id" = ?1"#,
            params![recurrence],
        )?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn groups_the_ledger_so_a_day_without_a_send_is_still_a_row_with_its_reason() {
    let db = seeded_db().await;
    let (template, recurrence) = series(&db).await;
    ledger(&db, template, recurrence).await;
    let out = history::list_run_groups(
        &db,
        WS,
        template.to_string(),
        Granularity::Day,
        None,
        2,
        None,
    )
    .await
    .expect("groups");
    let data = out["data"].as_array().expect("data");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["periodStart"], "2030-10-14");
    assert_eq!(data[0]["periodEnd"], "2030-10-15");
    assert_eq!(data[0]["slotCount"], 1);
    assert_eq!(data[0]["createdCount"], 1);
    assert_eq!(data[0]["runStateCounts"], json!({ "sent": 1 }));
    assert_eq!(data[0]["stats"]["message:sent"], 1);
    assert_eq!(data[1]["createdCount"], 0);
    assert_eq!(data[1]["missedCount"], 1);
    assert_eq!(data[1]["reasonCounts"], json!({ "catchUpSuperseded": 1 }));
    assert_eq!(data[1]["stats"]["message:sent"], 0);
    assert_eq!(out["totalSlots"], 3);
    assert_eq!(out["totalRuns"], 4);
    assert_eq!(out["timezone"], "Asia/Ho_Chi_Minh");
    // A full page hands back a cursor.
    assert_eq!(out["nextCursor"], "2030-10-13");
}

#[tokio::test]
async fn falls_back_to_the_rules_zone_and_a_single_page_offers_no_further_page() {
    let db = seeded_db().await;
    let (template, recurrence) = series(&db).await;
    ledger(&db, template, recurrence).await;
    // Adapted: the capped page size (31 + 1) is internal; 2 ISO weeks fit one page.
    let out = history::list_run_groups(
        &db,
        WS,
        template.to_string(),
        Granularity::Week,
        Some("Not/AZone".into()),
        500,
        None,
    )
    .await
    .expect("weeks");
    assert_eq!(out["timezone"], "Asia/Ho_Chi_Minh");
    assert_eq!(out["data"].as_array().map(Vec::len), Some(2));
    assert_eq!(out["nextCursor"], Value::Null);
    // a series with exactly one page of groups offers no further page
    let out = history::list_run_groups(
        &db,
        WS,
        template.to_string(),
        Granularity::Day,
        None,
        3,
        None,
    )
    .await
    .expect("days");
    assert_eq!(out["data"].as_array().map(Vec::len), Some(3));
    assert_eq!(out["nextCursor"], Value::Null);
}

#[tokio::test]
async fn run_groups_404_a_broadcast_that_does_not_repeat() {
    let db = seeded_db().await;
    let (audit, _) = audit();
    let one_off = write::create(
        &db,
        &audit,
        WS,
        json!({
            "channel": "omnichannel",
            "subaction": "allContacts",
            "flowId": "61",
            "schedulesType": "future",
            "schedulesAt": "2030-01-07T09:00:00.000Z",
            "contactFilter": { "operator": "and", "conditions": [] },
        }),
        true,
    )
    .await
    .expect("one-off");
    let err = history::list_run_groups(
        &db,
        WS,
        one_off.id.to_string(),
        Granularity::Day,
        None,
        5,
        None,
    )
    .await
    .expect_err("not repeating");
    assert!(message(&err).contains("does not repeat"));
    let err = history::list_run_groups(&db, WS, "x".into(), Granularity::Day, None, 5, None)
        .await
        .expect_err("missing");
    assert_eq!(message(&err), "Broadcast not found");
}

#[test]
fn period_boundaries_follow_the_grouping() {
    assert_eq!(
        history::next_bucket_start("2026-10-13", Granularity::Day),
        "2026-10-14"
    );
    assert_eq!(
        history::next_bucket_start("2026-10-12", Granularity::Week),
        "2026-10-19"
    );
    assert_eq!(
        history::next_bucket_start("2026-12-01", Granularity::Month),
        "2027-01-01"
    );
    let groups = history::group_ledger_cells(
        &["2026-10-13".to_owned()],
        &[],
        Granularity::Day,
        "Asia/Ho_Chi_Minh",
    );
    assert_eq!(groups.len(), 1);
    assert_eq!(
        (
            groups[0]["slotCount"].clone(),
            groups[0]["periodEnd"].clone()
        ),
        (json!(0), json!("2026-10-14"))
    );
}

#[test]
fn a_group_carries_its_boundaries_as_instants_in_the_groups_zone() {
    let cells: [LedgerBucketCell; 0] = [];
    let group = &history::group_ledger_cells(
        &["2026-10-13".to_owned()],
        &cells,
        Granularity::Day,
        "Asia/Ho_Chi_Minh",
    )[0];
    assert_eq!(group["periodStartAt"], "2026-10-12T17:00:00.000Z");
    assert_eq!(group["periodEndAt"], "2026-10-13T17:00:00.000Z");
    let slot = ms("2026-10-12T23:00:00.000Z"); // 2026-10-13 06:00 local
    assert!(slot >= ms(group["periodStartAt"].as_str().unwrap()));
    assert!(slot < ms(group["periodEndAt"].as_str().unwrap()));
    let week =
        &history::group_ledger_cells(&["2026-10-12".to_owned()], &cells, Granularity::Week, "UTC")
            [0];
    assert_eq!(week["periodEndAt"], "2026-10-19T00:00:00.000Z");
    let month = &history::group_ledger_cells(
        &["2026-10-01".to_owned()],
        &cells,
        Granularity::Month,
        "America/New_York",
    )[0];
    assert_eq!(month["periodStartAt"], "2026-10-01T04:00:00.000Z");
    assert_eq!(month["periodEndAt"], "2026-11-01T04:00:00.000Z");
}

// describe("listOccurrences")

#[tokio::test]
async fn occurrences_report_lateness_flag_a_stalled_run_and_page_with_a_cursor() {
    let db = seeded_db().await;
    let (template, recurrence) = series(&db).await;
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "parentBroadcastId", "occurrenceAt")
               VALUES (503, 'run-3', ?1, 'scheduled', 'now', 0, 'allContacts', 'omnichannel', ?2, ?3)"#,
            params![WS, template, ms("2020-01-03T02:00:00Z")],
        )?;
        for (id, local, at, outcome, reason, child) in [
            (901, "2020-01-01 02:00", "2020-01-01T02:00:00Z", "created", None, None),
            (902, "2020-01-02 02:00", "2020-01-02T02:00:00Z", "missed", Some("catchUpSuperseded"), None),
            (903, "2020-01-03 02:00", "2020-01-03T02:00:00Z", "created", None, Some(503)),
        ] {
            tx.execute(
                r#"INSERT INTO "BroadcastRecurrenceOccurrence" ("id", "workspaceId", "recurrenceId", "localSlot", "occurrenceAt", "outcome", "reason", "childBroadcastId", "materializedAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?5 + 5000)"#,
                params![id, WS, recurrence, local, ms(at), outcome, reason, child],
            )?;
        }
        Ok(())
    })
    .await;
    let out = history::list_occurrences(&db, WS, template.to_string(), None, None, None, 2)
        .await
        .expect("occurrences");
    let data = out["data"].as_array().expect("data");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["latenessMs"], 5000);
    assert_eq!(data[0]["run"]["id"], "503");
    assert_eq!(data[0]["run"]["isStalled"], true);
    assert_eq!(data[0]["run"]["stats"]["message:sent"], 0);
    assert_eq!(data[1]["outcome"], "missed");
    assert_eq!(data[1]["run"], Value::Null);
    let cursor = out["nextCursor"].as_str().expect("cursor");
    assert!(
        cursor.ends_with(":902")
            && cursor
                .split(':')
                .next()
                .is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit()))
    );
    // the cursor continues with the older slot
    let out = history::list_occurrences(
        &db,
        WS,
        template.to_string(),
        None,
        None,
        Some(cursor.to_owned()),
        2,
    )
    .await
    .expect("next page");
    assert_eq!(out["data"][0]["localSlot"], "2020-01-01 02:00");
}

#[tokio::test]
async fn occurrences_of_an_empty_series_and_a_bad_cursor() {
    let db = seeded_db().await;
    let (template, _) = series(&db).await;
    let out = history::list_occurrences(&db, WS, template.to_string(), None, None, None, 10)
        .await
        .expect("empty");
    assert_eq!(out, json!({ "data": [], "nextCursor": null }));
    for cursor in [
        "garbage",
        "1700000000000:abc",
        "abc:12",
        ":5",
        "1700000000000:",
    ] {
        let err = history::list_occurrences(
            &db,
            WS,
            template.to_string(),
            None,
            None,
            Some(cursor.into()),
            10,
        )
        .await
        .expect_err(cursor);
        assert_eq!(field(&err).as_deref(), Some("cursor"), "{cursor}");
    }
}

// describe("nextSendAtOf") / describe("previewUpcoming with skipped dates")

fn ts_rule(overrides: Value) -> Value {
    let mut out = rule_with(json!({ "startsOn": "2026-10-01" }));
    for (k, v) in overrides.as_object().expect("object") {
        out[k] = v.clone();
    }
    out
}

fn ts_slot() -> i64 {
    ms("2026-10-13T02:00:00.000Z")
}

#[test]
fn next_send_at_is_the_armed_slot_or_the_first_slot_after_skipped_dates() {
    let plain = Json(ts_rule(json!({})));
    assert_eq!(
        recurrence::next_send_at_of(&plain, Some(ts_slot()), 0),
        Some(ts_slot())
    );
    let skipped = Json(ts_rule(
        json!({ "excludeDates": ["2026-10-13", "2026-10-14"] }),
    ));
    assert_eq!(
        recurrence::next_send_at_of(&skipped, Some(ts_slot()), 0),
        Some(ts_slot() + 2 * DAY)
    );
    // is null for a paused series and when only skipped dates remain
    assert_eq!(recurrence::next_send_at_of(&plain, None, 0), None);
    let ended = Json(ts_rule(json!({
        "excludeDates": ["2026-10-13"],
        "end": { "kind": "onDate", "date": "2026-10-13" },
    })));
    assert_eq!(
        recurrence::next_send_at_of(&ended, Some(ts_slot()), 0),
        None
    );
}

#[test]
fn preview_lists_only_the_runs_that_send_and_tolerates_duplicate_dates() {
    let preview = recurrence::preview_upcoming(
        &ts_rule(json!({ "excludeDates": ["2026-10-14", "2026-10-14"] })),
        3,
        ts_slot() - 1000,
    )
    .expect("preview");
    assert_eq!(
        preview["upcomingRuns"],
        json!([
            iso(ts_slot()),
            iso(ts_slot() + 2 * DAY),
            iso(ts_slot() + 3 * DAY)
        ])
    );
}
