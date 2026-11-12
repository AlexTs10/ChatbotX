//! Port of `packages/database/__tests__/integration/thread-control-guarded-update.test.ts`
//! (P3.2). Runs against a migrated SQLite temp file (fresh file per test). Fixture:
//! workspace 2001 with a WhatsApp inbox 3100, contact 10 and contact inbox 11; another
//! workspace 2002.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::contact_inbox::{self as ci, ThreadControlTransition};
use chatbotx_db::repo::inbox::touch_thread_control_seen;
use chatbotx_db::repo::models::ContactInbox;
use chatbotx_db::repo::partials::thread_control::{
    THREAD_CONTROL_EVENT_PRECEDENCE, THREAD_CONTROL_SEEN_REFRESH_MS,
};
use chatbotx_db::repo::{self};
use repo_common::{ms, ok};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const CI: i64 = 11;

fn t0() -> i64 {
    ms("2026-09-29T10:00:00.000Z")
}
fn at(seconds: i64) -> i64 {
    t0() + seconds * 1000
}

async fn fixture() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, OTHER).await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_inbox(tx, 3100, WS, "whatsapp", "tc-phone")?;
        repo_common::insert_contact(tx, 10, WS)?;
        repo_common::insert_ci(tx, CI, 10, 3100, "whatsapp", "tc-wa-id")?;
        Ok(())
    })
    .await;
    db
}

async fn apply_in(
    db: &Db,
    ws: i64,
    event: &str,
    occurred_at: i64,
    role: Option<&str>,
    app: Option<&str>,
) -> Option<ContactInbox> {
    let input = ThreadControlTransition {
        id: CI,
        workspace_id: ws,
        event: event.to_owned(),
        owner_role: role.map(str::to_owned),
        owner_app_id: app.map(str::to_owned),
        occurred_at,
        ..Default::default()
    };
    ok(repo::write(db, move |tx| {
        ci::apply_thread_control_transition(tx, &input)
    })
    .await)
}

async fn apply(db: &Db, event: &str, occurred_at: i64, role: Option<&str>) -> Option<ContactInbox> {
    apply_in(db, WS, event, occurred_at, role, None).await
}

async fn reset(db: &Db) {
    repo_common::handover::exec(
        db,
        r#"UPDATE "ContactInbox" SET "threadControlState" = NULL, "threadOwnerRole" = NULL, "threadOwnerAppId" = NULL, "threadPreviousOwnerAppId" = NULL, "threadControlUpdatedAt" = NULL, "threadControlLastEvent" = NULL WHERE "id" = 11"#,
        vec![],
    )
    .await;
}

async fn row(db: &Db) -> ContactInbox {
    ok(repo::read(db, |c| ci::find_model_by_id_for_workspace(c, CI, WS)).await).expect("row")
}

async fn promote(db: &Db, ws: i64, occurred_at: i64) -> Option<ContactInbox> {
    ok(repo::write(db, move |tx| {
        ci::promote_standby_to_owner_delivery(tx, CI, ws, None, None, None, occurred_at)
    })
    .await)
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:186 "contactInboxRepository.applyThreadControlTransition against Postgres > a fresh row accepts the first event and records state, role, time and event"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_a_fresh_row_accepts_the_first_event_and_records_state_role_time_and_event()
 {
    let db = fixture().await;
    let r = apply(&db, "standbyReceived", t0(), Some("ai_agent"))
        .await
        .expect("row");
    assert_eq!(r.thread_control_state.as_deref(), Some("standby"));
    assert_eq!(r.thread_owner_role.as_deref(), Some("ai_agent"));
    assert_eq!(
        r.thread_control_last_event.as_deref(),
        Some("standbyReceived")
    );
    assert_eq!(r.thread_control_updated_at, Some(t0()));
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:198 "contactInboxRepository.applyThreadControlTransition against Postgres > a stale event (older timestamp) is rejected and leaves the row untouched"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_a_stale_event_older_timestamp_is_rejected_and_leaves_the_row_untouched()
 {
    let db = fixture().await;
    apply(&db, "controlTaken", at(10), Some("ai_agent")).await;
    assert!(apply(&db, "inboundReceived", at(5), None).await.is_none());
    let r = row(&db).await;
    assert_eq!(
        (
            r.thread_control_state.as_deref(),
            r.thread_owner_role.as_deref(),
            r.thread_control_last_event.as_deref()
        ),
        (Some("standby"), Some("ai_agent"), Some("controlTaken"))
    );
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:212 "contactInboxRepository.applyThreadControlTransition against Postgres > a same-state event advances the timestamp so an older transition stays stale"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_a_same_state_event_advances_the_timestamp_so_an_older_transition_stays_stale()
 {
    let db = fixture().await;
    apply(&db, "standbyReceived", at(1), Some("ai_agent")).await;
    let no_op = apply(&db, "standbyReceived", at(20), Some("ai_agent"))
        .await
        .expect("row");
    assert_eq!(no_op.thread_control_updated_at, Some(at(20)));
    assert!(apply(&db, "inboundReceived", at(10), None).await.is_none());
    assert_eq!(
        row(&db).await.thread_control_state.as_deref(),
        Some("standby")
    );
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:231 "contactInboxRepository.applyThreadControlTransition against Postgres > an exact redelivery on the same second is accepted as idempotent"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_an_exact_redelivery_on_the_same_second_is_accepted_as_idempotent()
 {
    let db = fixture().await;
    apply(&db, "controlPassed", t0(), Some("customer_service")).await;
    assert!(
        apply(&db, "controlPassed", t0(), Some("customer_service"))
            .await
            .is_some()
    );
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:246 "contactInboxRepository.applyThreadControlTransition against Postgres > same second, same event and state but a different role is rejected as stale"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_same_second_same_event_and_state_but_a_different_role_is_rejected_as_stale()
 {
    let db = fixture().await;
    apply(&db, "controlPassed", t0(), Some("customer_service")).await;
    assert!(
        apply(&db, "controlPassed", t0(), Some("escalation"))
            .await
            .is_none()
    );
    assert_eq!(
        row(&db).await.thread_owner_role.as_deref(),
        Some("customer_service")
    );
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:264 "contactInboxRepository.applyThreadControlTransition against Postgres > role-based (WhatsApp) writes leave the owner app id null and an exact redelivery is still idempotent"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_role_based_whats_app_writes_leave_the_owner_app_id_null_and_an_exact_redelivery_is_still_idempotent()
 {
    let db = fixture().await;
    let first = apply(&db, "controlPassed", t0(), Some("escalation"))
        .await
        .expect("row");
    assert_eq!(
        (
            first.thread_owner_app_id,
            first.thread_previous_owner_app_id
        ),
        (None, None)
    );
    assert!(
        apply(&db, "controlPassed", t0(), Some("escalation"))
            .await
            .is_some()
    );
    assert_eq!(row(&db).await.thread_owner_app_id, None);
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:282 "contactInboxRepository.applyThreadControlTransition against Postgres > same-second passes to DIFFERENT app ids are not one redelivery"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_same_second_passes_to_different_app_ids_are_not_one_redelivery()
 {
    let db = fixture().await;
    apply_in(&db, WS, "passed", t0(), None, Some("app-a")).await;
    assert!(
        apply_in(&db, WS, "passed", t0(), None, Some("app-b"))
            .await
            .is_none()
    );
    assert_eq!(row(&db).await.thread_owner_app_id.as_deref(), Some("app-a"));
    assert!(
        apply_in(&db, WS, "passed", t0(), None, Some("app-a"))
            .await
            .is_some()
    );
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:296 "contactInboxRepository.applyThreadControlTransition against Postgres > same-second ties resolve by precedence regardless of processing order"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_same_second_ties_resolve_by_precedence_regardless_of_processing_order()
 {
    let db = fixture().await;
    let events = THREAD_CONTROL_EVENT_PRECEDENCE;
    for low in 0..events.len() {
        for high in low + 1..events.len() {
            let mut finals = Vec::new();
            for (first, second) in [(events[low], events[high]), (events[high], events[low])] {
                reset(&db).await;
                apply(&db, first, at(100), None).await;
                apply(&db, second, at(100), None).await;
                finals.push(row(&db).await.thread_control_last_event);
            }
            assert_eq!(
                finals[0].as_deref(),
                Some(events[high]),
                "{} vs {}",
                events[low],
                events[high]
            );
            assert_eq!(
                finals[1].as_deref(),
                Some(events[high]),
                "{} vs {}",
                events[high],
                events[low]
            );
        }
    }
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:319 "contactInboxRepository.applyThreadControlTransition against Postgres > the owner copy of a standby message is promoted at the copy's own time, in either order against a handover"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_the_owner_copy_of_a_standby_message_is_promoted_at_the_copy_s_own_time_in_either_order_against_a_handover()
 {
    let db = fixture().await;
    apply(&db, "standbyReceived", t0(), Some("ai_agent")).await;
    let promoted = promote(&db, WS, t0()).await.expect("promoted");
    assert_eq!(promoted.thread_control_state.as_deref(), Some("owned"));
    assert_eq!(promoted.thread_owner_app_id, None);
    assert_eq!(
        promoted.thread_control_last_event.as_deref(),
        Some("inboundReceived")
    );
    assert_eq!(promoted.thread_control_updated_at, Some(t0()));
    assert!(promote(&db, WS, t0()).await.is_none());
    assert!(apply(&db, "inboundReceived", t0(), None).await.is_some());
    assert_eq!(
        row(&db).await.thread_control_state.as_deref(),
        Some("owned")
    );
    reset(&db).await;
    apply(&db, "standbyReceived", t0(), Some("ai_agent")).await;
    apply(&db, "controlTaken", t0(), Some("escalation")).await;
    assert!(promote(&db, WS, t0()).await.is_none());
    assert!(apply(&db, "inboundReceived", t0(), None).await.is_none());
    let r = row(&db).await;
    assert_eq!(
        (
            r.thread_control_state.as_deref(),
            r.thread_control_last_event.as_deref()
        ),
        (Some("standby"), Some("controlTaken"))
    );
    reset(&db).await;
    apply(&db, "standbyReceived", t0(), Some("ai_agent")).await;
    assert!(promote(&db, WS, t0()).await.is_some());
    let handover = apply(&db, "controlTaken", t0(), Some("escalation"))
        .await
        .expect("handover");
    assert_eq!(
        (
            handover.thread_control_state.as_deref(),
            handover.thread_control_last_event.as_deref()
        ),
        (Some("standby"), Some("controlTaken"))
    );
    assert_eq!(
        row(&db).await.thread_control_state.as_deref(),
        Some("standby")
    );
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:385 "contactInboxRepository.applyThreadControlTransition against Postgres > the promotion is workspace-scoped and never touches a row that is not the standby copy"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_the_promotion_is_workspace_scoped_and_never_touches_a_row_that_is_not_the_standby_copy()
 {
    let db = fixture().await;
    apply(&db, "standbyReceived", t0(), Some("ai_agent")).await;
    assert!(promote(&db, OTHER, t0()).await.is_none());
    assert!(promote(&db, WS, at(1)).await.is_none());
    assert_eq!(
        row(&db).await.thread_control_state.as_deref(),
        Some("standby")
    );
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:415 "contactInboxRepository.applyThreadControlTransition against Postgres > the standby expiry write lands only against the observed ownership version"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_the_standby_expiry_write_lands_only_against_the_observed_ownership_version()
 {
    let db = fixture().await;
    let expires = at(3600);
    let set_expiry = |observed: Option<i64>| {
        repo::write(&db, move |tx| {
            ci::set_standby_thread_owner_expires_at(
                tx,
                CI,
                WS,
                Some("app-a"),
                observed,
                Some(expires),
            )
        })
    };
    apply_in(&db, WS, "standbyReceived", t0(), None, Some("app-a")).await;
    apply(&db, "inboundReceived", at(5), None).await;
    apply_in(&db, WS, "standbyReceived", at(10), None, Some("app-a")).await;
    assert!(ok(set_expiry(Some(t0())).await).is_none());
    let landed = ok(set_expiry(Some(at(10))).await).expect("landed");
    assert_eq!(landed.thread_owner_expires_at, Some(expires));
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:445 "contactInboxRepository.applyThreadControlTransition against Postgres > a contact inbox of another workspace is never updated"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_a_contact_inbox_of_another_workspace_is_never_updated()
 {
    let db = fixture().await;
    assert!(
        apply_in(&db, OTHER, "inboundReceived", t0(), None, None)
            .await
            .is_none()
    );
    assert_eq!(row(&db).await.thread_control_state, None);
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:463 "contactInboxRepository.applyThreadControlTransition against Postgres > listThreadControlledByContactIds returns only owned rows of the workspace"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_list_thread_controlled_by_contact_ids_returns_only_owned_rows_of_the_workspace()
 {
    let db = fixture().await;
    apply(&db, "inboundReceived", t0(), None).await;
    let owned = ok(repo::read(&db, |c| {
        ci::list_thread_controlled_by_contact_ids(c, WS, &[10])
    })
    .await);
    let foreign = ok(repo::read(&db, |c| {
        ci::list_thread_controlled_by_contact_ids(c, OTHER, &[10])
    })
    .await);
    apply(&db, "controlTaken", at(5), Some("ai_agent")).await;
    let after = ok(repo::read(&db, |c| {
        ci::list_thread_controlled_by_contact_ids(c, WS, &[10])
    })
    .await);
    assert_eq!(owned.iter().map(|r| r.id).collect::<Vec<_>>(), vec![CI]);
    assert!(foreign.is_empty());
    assert!(after.is_empty());
}

/// Port of: packages/database/__tests__/integration/thread-control-guarded-update.test.ts:498 "contactInboxRepository.applyThreadControlTransition against Postgres > touchThreadControlSeen writes once, then is throttled for the refresh interval"
#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_against_postgres_touch_thread_control_seen_writes_once_then_is_throttled_for_the_refresh_interval()
 {
    let db = fixture().await;
    let touch = |ws: i64, seen: i64| {
        repo::write(&db, move |tx| touch_thread_control_seen(tx, ws, 3100, seen))
    };
    let first = ok(touch(WS, t0()).await);
    let throttled = ok(touch(WS, at(60)).await);
    let refreshed = ok(touch(WS, t0() + THREAD_CONTROL_SEEN_REFRESH_MS + 1000).await);
    let foreign = ok(touch(OTHER, t0()).await);
    assert_eq!(
        [first, throttled, refreshed, foreign],
        [true, false, true, false]
    );
}
