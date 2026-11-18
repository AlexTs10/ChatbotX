//! Port of `packages/business/__tests__/thread-control-service.test.ts` and
//! `thread-control-constants.test.ts` (Q.D12). Adapted: the mocked repository calls
//! become real `ContactInbox` / `Message` rows of a migrated SQLite file; the
//! realtime / cache spies are dropped (no cache; realtime `None`).

mod common;
mod x1_support;

use chatbotx_business::thread_control::{
    self, RecordEventInput, activity_text, classify_owner, is_unpromoted_standby_copy,
};
use chatbotx_db::Db;
use chatbotx_db::repo::contact_inbox as ci_repo;
use chatbotx_db::repo::models::ContactInbox;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::partials::thread_control::to_thread_control_timestamp;
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};
use serde_json::json;
use x1_support::RecordingEnqueuer;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
const CI: i64 = 50;
const CONV: i64 = 70;

fn now() -> i64 {
    to_thread_control_timestamp(now_ms())
}

async fn seed(db: &Db, channel: &'static str) {
    setup(db, move |tx| {
        common::inbox(tx, 5, WS, channel)?;
        common::contact(tx, 1, WS)?;
        common::contact_inbox(tx, CI, 1, 5, channel, "15550001")?;
        common::conversation(tx, CONV, WS, 1)?;
        Ok(())
    })
    .await;
}

async fn set_state(db: &Db, state: &'static str, role: Option<&'static str>, at: i64) {
    setup(db, move |tx| {
        tx.execute(
            r#"UPDATE "ContactInbox" SET "threadControlState" = ?1, "threadOwnerRole" = ?2,
               "threadControlUpdatedAt" = ?3, "lastIncomingMessageAt" = ?3 WHERE "id" = ?4"#,
            params![state, role, at, CI],
        )?;
        Ok(())
    })
    .await;
}

async fn row(db: &Db) -> ContactInbox {
    chatbotx_business::tx::read(db, |conn| {
        Ok(ci_repo::find_model_by_id_for_workspace(conn, CI, WS)?)
    })
    .await
    .expect("read")
    .expect("contact inbox")
}

async fn activity_count(db: &Db) -> i64 {
    db.read(|conn| {
        Ok(conn.query_row(
            r#"SELECT count(*) FROM "Message" WHERE "conversationId" = ?1 AND "messageType" = 'activity'"#,
            [CONV],
            |r| r.get(0),
        )?)
    })
    .await
    .expect("count")
}

fn event(ci: ContactInbox, event: &str, role: Option<&str>, at: i64) -> RecordEventInput {
    RecordEventInput {
        workspace_id: WS,
        inbox_id: ci.inbox_id,
        inbox_seen_at: Some(at),
        conversation_id: CONV,
        event: event.into(),
        owner_role: role.map(str::to_owned),
        occurred_at: at,
        contact_inbox: Some(ci),
        ..RecordEventInput::default()
    }
}

// describe("thread-control constants")

#[test]
fn keeps_the_whatsapp_passed_wording_escalation_partner_byte_for_byte() {
    assert_eq!(
        activity_text("passed", "whatsapp"),
        "Conversation passed to the escalation partner"
    );
}

#[test]
fn uses_neutral_wording_for_a_channel_with_no_override() {
    assert_eq!(
        activity_text("passed", "messenger"),
        "Conversation passed to another app"
    );
}

#[test]
fn falls_back_to_the_shared_text_for_events_a_channel_does_not_reword() {
    assert_eq!(
        activity_text("taken", "whatsapp"),
        "This app took over this conversation"
    );
}

// describe("threadControlService.recordEvent")

#[tokio::test]
async fn applies_a_guarded_transition_with_the_parsed_role_and_reports_a_state_change() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    set_state(&db, "owned", None, t - DAY / 2).await;
    let result = thread_control::record_event(
        &db,
        None,
        event(row(&db).await, "controlTaken", Some("ai_agent"), t - 1000),
    )
    .await
    .expect("record");
    assert!(result.event_applied && result.state_changed);
    let after = row(&db).await;
    assert_eq!(after.thread_control_state.as_deref(), Some("standby"));
    assert_eq!(after.thread_owner_role.as_deref(), Some("ai_agent"));
    assert_eq!(after.thread_owner_expires_at, None);
}

#[tokio::test]
async fn stores_an_unknown_meta_role_as_null() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    thread_control::record_event(
        &db,
        None,
        event(row(&db).await, "controlTaken", Some("not_a_role"), now()),
    )
    .await
    .expect("record");
    assert_eq!(row(&db).await.thread_owner_role, None);
}

#[tokio::test]
async fn a_state_change_writes_one_deterministic_divider() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    thread_control::record_event(&db, None, event(row(&db).await, "controlTaken", None, t))
        .await
        .expect("record");
    let source: String = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "sourceId" FROM "Message" WHERE "conversationId" = ?1"#,
                [CONV],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("source");
    assert_eq!(source, format!("thread-control:{CI}:controlTaken:{t}"));
    assert_eq!(activity_count(&db).await, 1);
    let ci = row(&db).await;
    assert_eq!(ci.last_message_at, Some(t));
}

#[tokio::test]
async fn a_stale_event_writes_nothing() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    set_state(&db, "owned", None, t).await;
    let result = thread_control::record_event(
        &db,
        None,
        event(row(&db).await, "controlTaken", None, t - HOUR),
    )
    .await
    .expect("record");
    assert!(!result.event_applied);
    assert_eq!(activity_count(&db).await, 0);
    assert_eq!(
        row(&db).await.thread_control_state.as_deref(),
        Some("owned")
    );
}

#[tokio::test]
async fn an_exact_redelivery_is_applied_but_flagged() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    thread_control::record_event(&db, None, event(row(&db).await, "controlTaken", None, t))
        .await
        .expect("first");
    let again =
        thread_control::record_event(&db, None, event(row(&db).await, "controlTaken", None, t))
            .await
            .expect("again");
    assert!(again.event_applied && again.is_redelivery);
    assert_eq!(activity_count(&db).await, 1);
}

#[tokio::test]
async fn a_context_writes_one_idempotent_card_even_without_a_state_change() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    set_state(&db, "owned", None, t - 2000).await;
    let mut input = event(row(&db).await, "inboundReceived", None, t);
    input.context = Some(json!({ "summary": "hi" }));
    thread_control::record_event(&db, None, input.clone())
        .await
        .expect("record");
    thread_control::record_event(&db, None, input)
        .await
        .expect("record again");
    let cards: i64 = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT count(*) FROM "Message" WHERE "sourceId" LIKE 'thread-control-context:%'"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("count");
    assert_eq!(cards, 1);
}

// describe("threadControlService.recordInboundDelivery")

#[tokio::test]
async fn single_partner_a_null_thread_on_an_inbox_with_no_routing_traffic_writes_nothing() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let mut input = event(row(&db).await, "inboundReceived", None, now());
    input.inbox_seen_at = None;
    let out = thread_control::record_inbound_delivery(&db, None, input, "owner")
        .await
        .expect("deliver");
    assert!(out.is_none());
    assert_eq!(row(&db).await.thread_control_state, None);
}

#[tokio::test]
async fn an_active_inbox_marks_a_new_owner_delivery_as_owned() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    let input = event(row(&db).await, "inboundReceived", None, t);
    let out = thread_control::record_inbound_delivery(&db, None, input, "owner")
        .await
        .expect("deliver");
    assert!(out.is_some());
    assert_eq!(
        row(&db).await.thread_control_state.as_deref(),
        Some("owned")
    );
}

#[tokio::test]
async fn a_standby_delivery_records_standby_received_even_on_a_null_thread_and_quiet_inbox() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let mut input = event(row(&db).await, "inboundReceived", None, now());
    input.inbox_seen_at = None;
    thread_control::record_inbound_delivery(&db, None, input, "standby")
        .await
        .expect("deliver");
    let ci = row(&db).await;
    assert_eq!(ci.thread_control_state.as_deref(), Some("standby"));
    assert_eq!(
        ci.thread_control_last_event.as_deref(),
        Some("standbyReceived")
    );
}

#[tokio::test]
async fn a_standby_delivery_on_an_already_standby_thread_writes_nothing() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    set_state(&db, "standby", None, now() - 1000).await;
    let input = event(row(&db).await, "inboundReceived", None, now());
    let out = thread_control::record_inbound_delivery(&db, None, input, "standby")
        .await
        .expect("deliver");
    assert!(out.is_none());
}

// describe("threadControlService.requestAction")

#[tokio::test]
async fn a_contact_inbox_outside_the_workspace_is_not_found_and_never_reaches_the_channel() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let err = thread_control::request_action(
        &db,
        None,
        "http://127.0.0.1:9",
        thread_control::ActionRequest {
            workspace_id: common::OTHER_WS,
            contact_inbox_id: CI,
            conversation_id: CONV,
            action: "take",
            target_role: None,
            metadata: None,
            expected_updated_at: None,
        },
    )
    .await
    .expect_err("not found");
    assert!(err.is_permanent());
}

#[tokio::test]
async fn an_expected_version_that_no_longer_matches_skips_the_channel_call() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    set_state(&db, "owned", None, t).await;
    let out = thread_control::request_action(
        &db,
        None,
        "http://127.0.0.1:9",
        thread_control::ActionRequest {
            workspace_id: WS,
            contact_inbox_id: CI,
            conversation_id: CONV,
            action: "release",
            target_role: None,
            metadata: None,
            expected_updated_at: Some(Some(t - 5000)),
        },
    )
    .await
    .expect("snapshot");
    assert_eq!(out.thread_control_state.as_deref(), Some("owned"));
}

#[tokio::test]
async fn a_channel_without_update_thread_control_is_unsupported() {
    let db = two_workspaces().await;
    seed(&db, "webchat").await;
    let err = thread_control::request_action(
        &db,
        None,
        "http://127.0.0.1:9",
        thread_control::ActionRequest {
            workspace_id: WS,
            contact_inbox_id: CI,
            conversation_id: CONV,
            action: "take",
            target_role: None,
            metadata: None,
            expected_updated_at: None,
        },
    )
    .await
    .expect_err("unsupported");
    assert!(matches!(
        err,
        thread_control::ThreadControlError::Unsupported(_)
    ));
}

// describe("threadControlService.refreshForRouting")

#[tokio::test]
async fn a_thread_that_never_observed_routing_is_returned_as_is() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let ci = row(&db).await;
    set_state(&db, "owned", None, now()).await;
    let out = thread_control::refresh_for_routing(&db, WS, ci)
        .await
        .expect("refresh");
    assert_eq!(out.thread_control_state, None);
}

#[tokio::test]
async fn a_routed_thread_is_re_read() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    set_state(&db, "standby", None, now() - 1000).await;
    let ci = row(&db).await;
    set_state(&db, "owned", None, now()).await;
    let out = thread_control::refresh_for_routing(&db, WS, ci)
        .await
        .expect("refresh");
    assert_eq!(out.thread_control_state.as_deref(), Some("owned"));
}

// describe("threadControlService.resolveCurrentState")

#[tokio::test]
async fn resolve_current_state_re_reads_the_row_and_is_null_when_gone() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    let t = now();
    set_state(&db, "owned", None, t).await;
    let state = thread_control::resolve_current_state(&db, WS, CI)
        .await
        .expect("state");
    assert_eq!(state, Some(("owned".to_owned(), Some(t))));
    let gone = thread_control::resolve_current_state(&db, WS, 999)
        .await
        .expect("state");
    assert_eq!(gone, None);
}

// describe("threadControlService.promoteStandbyDelivery")

#[test]
fn a_message_not_stored_from_a_standby_copy_is_never_promoted() {
    assert!(!is_unpromoted_standby_copy(Some(&json!({}))));
    assert!(is_unpromoted_standby_copy(Some(
        &json!({ "threadControlDelivery": "standby" })
    )));
    assert!(!is_unpromoted_standby_copy(Some(
        &json!({ "threadControlDelivery": "standby", "threadControlPromoted": true })
    )));
}

// describe("threadControlService.releaseOwnedThreadsForContacts")

#[tokio::test]
async fn a_channel_without_archive_release_is_skipped_whatsapp_still_enqueues() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    setup(&db, |tx| {
        common::inbox(tx, 6, WS, "messenger")?;
        common::contact_inbox(tx, 60, 1, 6, "messenger", "psid")?;
        Ok(())
    })
    .await;
    let t = now();
    set_state(&db, "owned", None, t).await;
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "ContactInbox" SET "threadControlState" = 'owned', "threadControlUpdatedAt" = ?1, "lastIncomingMessageAt" = ?1 WHERE "id" = 60"#,
            [t],
        )?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();
    thread_control::release_owned_threads_for_contacts(&db, &*jobs, WS, &[(CONV, 1)], 42)
        .await
        .expect("release");
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].opts.job_id.as_deref(),
        Some(format!("thread-release-{CI}-{t}-42").as_str())
    );
    assert_eq!(
        calls[0].data["data"]["contactInboxId"],
        json!(CI.to_string())
    );
}

#[tokio::test]
async fn archive_of_an_expired_owned_thread_enqueues_nothing() {
    let db = two_workspaces().await;
    seed(&db, "whatsapp").await;
    set_state(&db, "owned", None, now() - 2 * DAY).await;
    let jobs = RecordingEnqueuer::new();
    thread_control::release_owned_threads_for_contacts(&db, &*jobs, WS, &[(CONV, 1)], 1)
        .await
        .expect("release");
    assert!(jobs.calls().is_empty());
}

// describe("classifyThreadOwner")

#[test]
fn classify_thread_owner_names_self_ai_partner_none_unknown() {
    assert_eq!(classify_owner(None, Some("a"), Some("ai")), "none");
    assert_eq!(classify_owner(Some("x"), None, Some("ai")), "unknown");
    assert_eq!(classify_owner(Some("a"), Some("a"), Some("ai")), "self");
    assert_eq!(
        classify_owner(Some("ai"), Some("a"), Some("ai")),
        "aiHandover"
    );
    assert_eq!(classify_owner(Some("p"), Some("a"), Some("ai")), "partner");
}
