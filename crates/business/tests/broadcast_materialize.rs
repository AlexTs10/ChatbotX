//! Port of the `materializeOccurrence`, `skipOccurrence` and `excluded dates`
//! parts of `broadcast-recurrence-service.test.ts` (Q.D5 slice 5b). Adapted:
//! the mocked repository and transaction become stored rows, so each test
//! asserts on the run, the ledger and the series instead of the mock calls.
//! The series runs in 2030 (the database clock is the real one); an outage
//! puts the armed slot two days before the real latest slot.

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::BusinessError;
use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::broadcast::materialize::{
    self, Capacity, MaterializeInput, MaterializeResult,
};
use chatbotx_business::broadcast::plan_policy::{self, PlanContext, plan_limit_exception};
use chatbotx_business::broadcast::write;
use chatbotx_core::js::parse_date;
use chatbotx_core::utils::recurrence::{compute_latest_occurrence, parse_recurrence_rule};
use chatbotx_db::repo::models::Broadcast;
use chatbotx_db::repo::partials::broadcast_plan_policy::BroadcastPlanLimitReason;
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, scalar_i64, scalar_text, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const DAY: i64 = 86_400_000;

/// 09:00 in Ho Chi Minh.
fn slot() -> i64 {
    parse_date("2030-10-13T02:00:00.000Z").expect("date")
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ..AuditActor::default()
    }
}

fn details(jobs: &RecordingEnqueuer) -> Vec<String> {
    jobs.calls()
        .iter()
        .filter_map(|c| c.data["data"]["detail"].as_str().map(str::to_owned))
        .collect()
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

struct Series {
    db: TestDb,
    template: i64,
    recurrence: i64,
    jobs: Arc<RecordingEnqueuer>,
}

impl Series {
    fn input(&self, occurrence_at: i64) -> MaterializeInput {
        MaterializeInput {
            recurrence_id: self.recurrence,
            workspace_id: WS,
            occurrence_at,
        }
    }

    async fn materialize(&self, occurrence_at: i64) -> Result<MaterializeResult, BusinessError> {
        let audit = AuditService::new(self.jobs.clone());
        with_audit_context(
            actor(),
            materialize::materialize_occurrence(&self.db, &audit, self.input(occurrence_at)),
        )
        .await
    }

    async fn set(&self, column: &'static str, value: Value) {
        let id = self.recurrence;
        setup(&self.db, move |tx| {
            let sql =
                format!(r#"UPDATE "BroadcastRecurrence" SET "{column}" = ?1 WHERE "id" = ?2"#);
            let bound: chatbotx_db::rusqlite::types::Value = match value {
                Value::Null => chatbotx_db::rusqlite::types::Value::Null,
                Value::Number(n) => n.as_i64().expect("int").into(),
                other => other.to_string().into(),
            };
            tx.execute(&sql, params![bound, id])?;
            Ok(())
        })
        .await;
    }

    async fn set_template(&self, assignment: &'static str) {
        let id = self.template;
        setup(&self.db, move |tx| {
            tx.execute(
                &format!(r#"UPDATE "Broadcast" SET {assignment} WHERE "id" = ?1"#),
                params![id],
            )?;
            Ok(())
        })
        .await;
    }

    async fn recurrence_int(&self, column: &'static str) -> Option<i64> {
        let sql: &'static str = Box::leak(
            format!(r#"SELECT "{column}" FROM "BroadcastRecurrence" WHERE "id" = ?1"#)
                .into_boxed_str(),
        );
        scalar_i64(&self.db, sql, self.recurrence).await
    }

    async fn template_int(&self, column: &'static str) -> Option<i64> {
        let sql: &'static str = Box::leak(
            format!(r#"SELECT "{column}" FROM "Broadcast" WHERE "id" = ?1"#).into_boxed_str(),
        );
        scalar_i64(&self.db, sql, self.template).await
    }

    async fn runs(&self) -> Vec<Broadcast> {
        let id = self.template;
        setup(&self.db, move |tx| {
            Ok(repo::find_many::<Broadcast>(
                tx,
                r#"WHERE "parentBroadcastId" = ?1 ORDER BY "occurrenceAt""#,
                &[&id],
            )
            .expect("runs"))
        })
        .await
    }

    /// `(localSlot, outcome, reason, childBroadcastId)` by slot.
    async fn ledger(&self) -> Vec<(String, String, Option<String>, Option<i64>)> {
        let id = self.recurrence;
        setup(&self.db, move |tx| {
            let mut stmt = tx.prepare(
                r#"SELECT "localSlot", "outcome", "reason", "childBroadcastId" FROM "BroadcastRecurrenceOccurrence"
                   WHERE "recurrenceId" = ?1 ORDER BY "occurrenceAt""#,
            )?;
            let rows = stmt
                .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
    }
}

/// A running series like `makeRecurrence()`: armed at `SLOT`, accounted
/// through the day before, flow 61 in channel mode.
async fn series() -> Series {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Promo flow')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (81, ?1, 'Inbox', 'webchat', 'src')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();
    let audit = AuditService::new(jobs.clone());
    let draft = write::create(
        &db,
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
        &db,
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
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "BroadcastRecurrence" SET "nextRunAt" = ?1, "accountedThroughAt" = ?2 WHERE "broadcastId" = ?3"#,
            params![next, accounted, id],
        )?;
        tx.execute(r#"UPDATE "Broadcast" SET "name" = 'Daily promo' WHERE "id" = ?1"#, params![id])?;
        Ok(())
    })
    .await;
    let recurrence = scalar_i64(
        &db,
        r#"SELECT "id" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
        id,
    )
    .await
    .expect("recurrence");
    Series {
        db,
        template: id,
        recurrence,
        jobs: RecordingEnqueuer::new(),
    }
}

/// The real latest 09:00 slot two days back, armed as after an outage. Returns
/// `(armed, latest)`.
async fn outage(s: &Series, rule: Value) -> (i64, i64) {
    let parsed = parse_recurrence_rule(&rule).expect("rule");
    let latest = compute_latest_occurrence(&parsed, now_ms(), 0)
        .expect("steps")
        .expect("latest");
    let armed = latest - 2 * DAY;
    s.set("rule", rule).await;
    s.set("nextRunAt", json!(armed)).await;
    s.set("accountedThroughAt", json!(armed - DAY)).await;
    (armed, latest)
}

#[tokio::test]
async fn creates_a_run_for_the_due_slot_and_advances_the_series_in_one_transaction() {
    let s = series().await;

    let result = s.materialize(slot()).await.expect("materialize");

    let runs = s.runs().await;
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(
        result,
        MaterializeResult {
            created_broadcast_id: Some(run.id),
            ..MaterializeResult::default()
        }
    );
    assert_eq!(run.parent_broadcast_id, Some(s.template));
    assert_eq!(run.status, "scheduled");
    assert_eq!(run.schedules_type, "future");
    assert_eq!(run.schedules_at, slot());
    assert_eq!(run.occurrence_at, Some(slot()));
    assert_eq!(run.flow_id, Some(61));
    assert_eq!(run.name, "Daily promo · 2030-10-13 09:00");
    assert_eq!(
        s.ledger().await.last().cloned(),
        Some((
            "2030-10-13 09:00".into(),
            "created".into(),
            None,
            Some(run.id)
        ))
    );
    assert_eq!(s.recurrence_int("nextRunAt").await, Some(slot() + DAY));
    assert_eq!(s.recurrence_int("occurrenceCount").await, Some(1));
    assert_eq!(s.recurrence_int("accountedThroughAt").await, Some(slot()));
    assert_eq!(s.template_int("schedulesAt").await, Some(slot() + DAY));
    assert_eq!(
        details(&s.jobs),
        vec![format!("launched a recurring broadcast run (#{})", run.id)]
    );
}

#[tokio::test]
async fn after_an_outage_only_the_latest_slot_is_sent_every_earlier_one_is_accounted_as_missed() {
    let s = series().await;
    let (armed, latest) = outage(&s, rule_with(json!({ "startsOn": "2026-01-01" }))).await;

    let result = s.materialize(armed).await.expect("materialize");

    let runs = s.runs().await;
    assert_eq!(result.created_broadcast_id, Some(runs[0].id));
    assert_eq!(runs[0].occurrence_at, Some(latest));
    let ledger = s.ledger().await;
    assert_eq!(ledger.len(), 3);
    for missed in &ledger[..2] {
        assert_eq!(missed.1, "missed");
        assert_eq!(missed.2.as_deref(), Some("catchUpSuperseded"));
    }
    let local = |at: i64| chatbotx_core::utils::recurrence::to_local_slot(at, "Asia/Ho_Chi_Minh");
    assert_eq!(
        ledger.iter().map(|r| r.0.clone()).collect::<Vec<_>>(),
        vec![local(armed), local(armed + DAY), local(latest)]
    );
    assert_eq!(ledger[2].1, "created");
}

#[tokio::test]
async fn a_slot_that_is_no_longer_the_armed_one_is_stale_nothing_is_created_and_a_settled_run_is_reported()
 {
    // Adapted: the first attempt settles the slot (the mock returned its row).
    let s = series().await;
    let first = s.materialize(slot()).await.expect("first");

    let result = s.materialize(slot()).await.expect("replay");

    assert_eq!(
        result,
        MaterializeResult {
            created_broadcast_id: first.created_broadcast_id,
            series_ended: false,
            skipped: None,
            replayed: true,
        }
    );
    assert_eq!(s.runs().await.len(), 1);
    assert_eq!(s.recurrence_int("nextRunAt").await, Some(slot() + DAY));
    // A replay is not a second launch.
    assert_eq!(details(&s.jobs).len(), 1);
}

#[tokio::test]
async fn a_replay_after_a_catch_up_finds_the_run_that_belongs_to_the_later_slot() {
    let s = series().await;
    let (armed, _) = outage(&s, rule_with(json!({ "startsOn": "2026-01-01" }))).await;
    let first = s.materialize(armed).await.expect("first");

    let result = s.materialize(armed).await.expect("replay");

    assert!(first.created_broadcast_id.is_some());
    assert_eq!(result.created_broadcast_id, first.created_broadcast_id);
    assert!(result.replayed);
    assert_eq!(details(&s.jobs).len(), 1);
}

#[tokio::test]
async fn a_replay_of_the_last_run_reports_that_the_series_ended() {
    let s = series().await;
    s.set(
        "rule",
        rule_with(json!({ "end": { "kind": "afterOccurrences", "count": 1 } })),
    )
    .await;
    let first = s.materialize(slot()).await.expect("first");

    let result = s.materialize(slot()).await.expect("replay");

    assert_eq!(result.created_broadcast_id, first.created_broadcast_id);
    assert!(result.series_ended);
}

#[tokio::test]
async fn a_paused_series_or_a_draft_template_is_disarmed_never_sent() {
    let s = series().await;
    s.set_template(r#""status" = 'cancelled'"#).await;

    let result = s.materialize(slot()).await.expect("materialize");

    assert_eq!(result.created_broadcast_id, None);
    assert!(s.runs().await.is_empty());
    assert_eq!(s.recurrence_int("nextRunAt").await, None);
}

#[tokio::test]
async fn a_missing_series_row_creates_nothing() {
    let s = series().await;
    let audit = AuditService::new(s.jobs.clone());
    let input = MaterializeInput {
        recurrence_id: 999_999,
        ..s.input(slot())
    };
    assert_eq!(
        materialize::materialize_occurrence(&s.db, &audit, input)
            .await
            .expect("ok"),
        MaterializeResult::default()
    );
}

/// `%s is skipped with its reason and the series moves on`.
async fn assert_skipped(s: &Series, reason: &'static str) {
    let result = s.materialize(slot()).await.expect("materialize");
    assert_eq!(
        result,
        MaterializeResult {
            skipped: Some(reason),
            ..MaterializeResult::default()
        }
    );
    assert!(s.runs().await.is_empty());
    let last = s.ledger().await.last().cloned().expect("row");
    assert_eq!(
        (last.1.as_str(), last.2.as_deref()),
        ("skipped", Some(reason))
    );
    // A skip does not use up the afterOccurrences budget but still advances.
    assert_eq!(s.recurrence_int("occurrenceCount").await, Some(0));
    assert_eq!(s.recurrence_int("nextRunAt").await, Some(slot() + DAY));
    assert!(details(&s.jobs).is_empty());
}

#[tokio::test]
async fn targets_mode_with_no_page_left_is_skipped_with_its_reason_and_the_series_moves_on() {
    let s = series().await;
    s.set_template(r#""targetMode" = 'targets', "flowId" = NULL"#)
        .await;
    assert_skipped(&s, "noTargets").await;
}

#[tokio::test]
async fn targets_mode_where_no_page_can_deliver_is_skipped_with_its_reason_and_the_series_moves_on()
{
    let s = series().await;
    s.set_template(r#""targetMode" = 'targets', "flowId" = NULL"#)
        .await;
    let id = s.template;
    setup(&s.db, move |tx| {
        tx.execute(
            r#"INSERT INTO "BroadcastTarget" ("broadcastId", "inboxId") VALUES (?1, 81)"#,
            params![id],
        )?;
        Ok(())
    })
    .await;
    assert_skipped(&s, "undeliverable").await;
}

#[tokio::test]
async fn channel_mode_without_a_flow_or_template_is_skipped_with_its_reason_and_the_series_moves_on()
 {
    let s = series().await;
    s.set_template(r#""flowId" = NULL, "templateId" = NULL"#)
        .await;
    assert_skipped(&s, "undeliverable").await;
}

#[tokio::test]
async fn a_template_whose_page_was_disconnected_is_skipped_with_its_reason_and_the_series_moves_on()
{
    let s = series().await;
    s.set_template(
        r#""flowId" = NULL, "templateId" = 5, "integrationWhatsappId" = NULL, "integrationMessengerId" = NULL"#,
    )
    .await;
    assert_skipped(&s, "undeliverable").await;
}

/// A messenger template (the restricted channel) plus one active broadcast when `full`.
async fn messenger_template(s: &Series, send_rate: Option<i64>, full: bool) -> Broadcast {
    let id = s.template;
    setup(&s.db, move |tx| {
        tx.execute(
            r#"UPDATE "Broadcast" SET "channel" = 'messenger', "sendRatePerMinute" = ?2 WHERE "id" = ?1"#,
            params![id, send_rate],
        )?;
        if full {
            tx.execute(
                r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                   VALUES (777, 'Busy', ?1, 'sending', 'now', 0, 'allContacts', 'messenger')"#,
                params![WS],
            )?;
        }
        Ok(repo::find_one::<Broadcast>(tx, r#""id" = ?1"#, &[&id]).expect("find").expect("row"))
    })
    .await
}

// Adapted (the three "on a restricted plan" tests): every edition resolves to
// the unrestricted plan (`resolveForWorkspace`), so `reserveCapacity` is called
// with the trial plan directly instead of through a mocked policy service.
#[tokio::test]
async fn a_full_active_slot_is_a_plan_limit_skip_decided_before_anything_is_inserted() {
    let s = series().await;
    let template = messenger_template(&s, None, true).await;
    let capacity = setup(&s.db, move |tx| {
        Ok(
            materialize::reserve_capacity(tx, &template, &PlanContext::trial(Some("Trial".into())))
                .expect("capacity"),
        )
    })
    .await;
    assert_eq!(capacity, Capacity::PlanLimit);
    assert!(s.runs().await.is_empty());
}

#[tokio::test]
async fn the_template_rate_is_clamped_to_the_plans_current_cap_and_checks_the_slot() {
    let s = series().await;
    let template = messenger_template(&s, Some(500), false).await;
    let capacity = setup(&s.db, move |tx| {
        Ok(
            materialize::reserve_capacity(tx, &template, &PlanContext::trial(None))
                .expect("capacity"),
        )
    })
    .await;
    assert_eq!(capacity, Capacity::Rate(60));
}

#[tokio::test]
async fn any_other_plan_error_is_not_swallowed() {
    let ctx =
        plan_policy::restriction_for(&PlanContext::trial(None), "messenger").expect("restricted");
    assert!(materialize::is_active_broadcasts_limit(
        &plan_limit_exception(BroadcastPlanLimitReason::ActiveBroadcasts, &ctx)
    ));
    assert!(!materialize::is_active_broadcasts_limit(
        &plan_limit_exception(BroadcastPlanLimitReason::SendRate, &ctx)
    ));
    assert!(!materialize::is_active_broadcasts_limit(
        &BusinessError::exception("db down", "systemError", 500)
    ));
}

#[tokio::test]
async fn an_existing_run_for_the_same_instant_is_reused_instead_of_inserting_a_second_one() {
    let s = series().await;
    let (template, at) = (s.template, slot());
    setup(&s.db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "parentBroadcastId", "occurrenceAt")
               VALUES (555, 'Existing', ?1, 'scheduled', 'future', ?2, 'allContacts', 'omnichannel', ?3, ?2)"#,
            params![WS, at, template],
        )?;
        Ok(())
    })
    .await;

    let result = s.materialize(slot()).await.expect("materialize");

    assert_eq!(result.created_broadcast_id, Some(555));
    assert_eq!(s.runs().await.len(), 1);
}

#[tokio::test]
async fn the_last_allowed_run_ends_the_series_and_marks_the_template_sent() {
    let s = series().await;
    s.set(
        "rule",
        rule_with(json!({ "end": { "kind": "afterOccurrences", "count": 1 } })),
    )
    .await;

    let result = s.materialize(slot()).await.expect("materialize");

    assert_eq!(
        result,
        MaterializeResult {
            created_broadcast_id: Some(s.runs().await[0].id),
            series_ended: true,
            ..MaterializeResult::default()
        }
    );
    assert_eq!(s.recurrence_int("nextRunAt").await, None);
    assert_eq!(s.recurrence_int("occurrenceCount").await, Some(1));
    assert!(s.recurrence_int("endedAt").await.is_some());
    assert_eq!(s.template_int("schedulesAt").await, Some(slot()));
    assert_eq!(
        scalar_text(
            &s.db,
            r#"SELECT "status" FROM "Broadcast" WHERE "id" = ?1"#,
            s.template
        )
        .await
        .as_deref(),
        Some("sent")
    );
}

#[tokio::test]
async fn a_lost_compare_and_set_fails_loudly_so_the_transaction_rolls_back() {
    // Adapted: a trigger moves the series between the read and the update.
    let s = series().await;
    setup(&s.db, |tx| {
        tx.execute_batch(
            r#"CREATE TRIGGER "moves_series" AFTER INSERT ON "BroadcastRecurrenceOccurrence"
               BEGIN UPDATE "BroadcastRecurrence" SET "nextRunAt" = "nextRunAt" + 1; END;"#,
        )?;
        Ok(())
    })
    .await;

    let error = s.materialize(slot()).await.expect_err("moved");

    assert!(
        error
            .to_string()
            .contains("Recurrence moved while materializing"),
        "{error}"
    );
    assert!(s.runs().await.is_empty());
    assert!(s.ledger().await.is_empty());
    assert_eq!(s.recurrence_int("nextRunAt").await, Some(slot()));
}

#[tokio::test]
async fn records_a_blocked_owners_slot_as_skipped_and_advances_without_a_run() {
    let s = series().await;

    materialize::skip_occurrence(&s.db, s.input(slot()), "blockedOwner")
        .await
        .expect("skip");

    assert!(s.runs().await.is_empty());
    let last = s.ledger().await.last().cloned().expect("row");
    assert_eq!(
        (last.1.as_str(), last.2.as_deref()),
        ("skipped", Some("blockedOwner"))
    );
    assert_eq!(s.recurrence_int("nextRunAt").await, Some(slot() + DAY));
}

#[tokio::test]
async fn ignores_a_slot_that_is_no_longer_armed() {
    let s = series().await;
    s.set("nextRunAt", Value::Null).await;

    materialize::skip_occurrence(&s.db, s.input(slot()), "blockedOwner")
        .await
        .expect("skip");

    assert!(s.ledger().await.is_empty());
    assert_eq!(
        s.recurrence_int("accountedThroughAt").await,
        Some(slot() - DAY)
    );
}

#[tokio::test]
async fn a_due_slot_on_an_excluded_date_is_settled_as_skipped_without_touching_capacity_or_targets()
{
    let s = series().await;
    s.set("rule", rule_with(json!({ "excludeDates": ["2030-10-13"] })))
        .await;

    let result = s.materialize(slot()).await.expect("materialize");

    assert_eq!(result, MaterializeResult::default());
    assert!(s.runs().await.is_empty());
    assert_eq!(
        s.ledger().await.last().cloned(),
        Some((
            "2030-10-13 09:00".into(),
            "skipped".into(),
            Some("excludedDate".into()),
            None
        ))
    );
    // The series moves on to the next slot and no run is counted.
    assert_eq!(s.recurrence_int("occurrenceCount").await, Some(0));
    assert_eq!(s.recurrence_int("nextRunAt").await, Some(slot() + DAY));
}

#[tokio::test]
async fn a_skipped_slot_does_not_use_up_an_after_occurrences_budget() {
    let s = series().await;
    s.set(
        "rule",
        rule_with(json!({ "excludeDates": ["2030-10-13"], "end": { "kind": "afterOccurrences", "count": 1 } })),
    )
    .await;

    let result = s.materialize(slot()).await.expect("materialize");

    assert!(!result.series_ended);
    assert_eq!(s.recurrence_int("occurrenceCount").await, Some(0));
    assert_eq!(s.recurrence_int("nextRunAt").await, Some(slot() + DAY));
}

#[tokio::test]
async fn a_series_whose_last_slot_is_skipped_ends_after_recording_it() {
    let s = series().await;
    s.set(
        "rule",
        rule_with(json!({ "excludeDates": ["2030-10-13"], "end": { "kind": "onDate", "date": "2030-10-13" } })),
    )
    .await;

    let result = s.materialize(slot()).await.expect("materialize");

    assert_eq!(
        result,
        MaterializeResult {
            series_ended: true,
            ..MaterializeResult::default()
        }
    );
    assert_eq!(
        s.ledger().await.last().and_then(|r| r.2.clone()).as_deref(),
        Some("excludedDate")
    );
    assert_eq!(
        scalar_text(
            &s.db,
            r#"SELECT "status" FROM "Broadcast" WHERE "id" = ?1"#,
            s.template
        )
        .await
        .as_deref(),
        Some("sent")
    );
}

#[tokio::test]
async fn after_an_outage_a_skipped_latest_slot_sends_nothing_and_earlier_slots_are_superseded() {
    let s = series().await;
    let base = rule_with(json!({ "startsOn": "2026-01-01" }));
    let parsed = parse_recurrence_rule(&base).expect("rule");
    let latest = compute_latest_occurrence(&parsed, now_ms(), 0)
        .expect("steps")
        .expect("latest");
    let latest_date = chatbotx_core::utils::recurrence::local_date_of(latest, "Asia/Ho_Chi_Minh");
    let (armed, _) = outage(
        &s,
        rule_with(json!({ "startsOn": "2026-01-01", "excludeDates": [latest_date] })),
    )
    .await;

    let result = s.materialize(armed).await.expect("materialize");

    assert_eq!(result.created_broadcast_id, None);
    assert!(s.runs().await.is_empty());
    let ledger = s.ledger().await;
    assert_eq!(
        ledger[..2]
            .iter()
            .map(|r| r.2.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["catchUpSuperseded", "catchUpSuperseded"]
    );
    assert_eq!(
        (
            ledger[2].0.clone(),
            ledger[2].1.as_str(),
            ledger[2].2.as_deref()
        ),
        (
            chatbotx_core::utils::recurrence::to_local_slot(latest, "Asia/Ho_Chi_Minh"),
            "skipped",
            Some("excludedDate")
        )
    );
}
