//! Port of `packages/database/__tests__/thread-control-repository.test.ts` (P3.2b).
//! The TS mocks the Drizzle update / select chains and renders the `set` and `where`
//! with the Postgres dialect; here the statements run on a migrated temp file (adapted)
//! and the rows show the written state, the stale / redelivery guard, the workspace
//! fence (the `EXISTS` on `Inbox`) and the throttles.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::contact_inbox::{self as ci, ThreadControlTransition};
use chatbotx_db::repo::models::ContactInbox;
use chatbotx_db::repo::partials::thread_control::THREAD_CONTROL_SEEN_REFRESH_MS;
use chatbotx_db::repo::{self, inbox, integration_messenger as im, integration_whatsapp as iw};
use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, WriteTx};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{insert_ci, insert_contact, insert_inbox, ms, ok, scalar_i64, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
/// Inbox 3001 (WS) holds contact 4001's row 81; inbox 3002 (OTHER) holds 4201's row 82.
const ROW: i64 = 81;
const FOREIGN_ROW: i64 = 82;

fn occurred_at() -> i64 {
    ms("2026-09-29T10:00:00.000Z")
}

fn expires_at() -> i64 {
    ms("2026-09-30T10:00:00.000Z")
}

async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        insert_inbox(tx, 3002, OTHER, "messenger", "page-other")?;
        insert_contact(tx, 4002, WS)?;
        insert_contact(tx, 4201, OTHER)?;
        insert_ci(tx, ROW, 4001, 3001, "messenger", "psid-81")?;
        insert_ci(tx, FOREIGN_ROW, 4201, 3002, "messenger", "psid-82")?;
        insert_ci(tx, 83, 4001, 3001, "messenger", "psid-83")?;
        insert_ci(tx, 84, 4002, 3001, "messenger", "psid-84")?;
        Ok(())
    })
    .await;
    db
}

/// Sets the thread-control columns of row `id`.
async fn state(
    db: &Db,
    id: i64,
    control: &str,
    role: Option<&str>,
    app: Option<&str>,
    updated_at: i64,
    last_event: &str,
) {
    let (control, role, app, last_event) = (
        control.to_owned(),
        role.map(str::to_owned),
        app.map(str::to_owned),
        last_event.to_owned(),
    );
    setup(db, move |tx| {
        tx.execute(
            "UPDATE \"ContactInbox\" SET \"threadControlState\" = ?1, \"threadOwnerRole\" = ?2, \
             \"threadOwnerAppId\" = ?3, \"threadControlUpdatedAt\" = ?4, \"threadControlLastEvent\" = ?5 \
             WHERE \"id\" = ?6",
            params![control, role, app, updated_at, last_event, id],
        )?;
        Ok(())
    })
    .await;
}

async fn set_expires(db: &Db, id: i64, value: Option<i64>) {
    setup(db, move |tx| {
        tx.execute(
            "UPDATE \"ContactInbox\" SET \"threadOwnerExpiresAt\" = ?1 WHERE \"id\" = ?2",
            params![value, id],
        )?;
        Ok(())
    })
    .await;
}

async fn row(db: &Db, id: i64) -> ContactInbox {
    ok(repo::read(db, move |c| {
        repo::find_one::<ContactInbox>(c, "\"id\" = ?1", &[&id])
    })
    .await)
    .expect("contact inbox row")
}

async fn write<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&mut WriteTx<'_>) -> repo::Result<T> + Send + 'static,
{
    ok(repo::write(db, f).await)
}

async fn run<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
{
    ok(repo::read(db, f).await)
}

fn transition(id: i64, event: &str, role: Option<&str>) -> ThreadControlTransition {
    ThreadControlTransition {
        id,
        workspace_id: WS,
        event: event.to_owned(),
        owner_role: role.map(str::to_owned),
        occurred_at: occurred_at(),
        ..ThreadControlTransition::default()
    }
}

async fn apply(db: &Db, input: ThreadControlTransition) -> Option<ContactInbox> {
    write(db, move |c| ci::apply_thread_control_transition(c, &input)).await
}

// ---------------------------------------------------------------------------
// contactInboxRepository.applyThreadControlTransition
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_writes_the_state_derived_from_the_event_plus_role_time_and_event()
 {
    let db = fixture().await;
    let written = apply(&db, transition(ROW, "standbyReceived", Some("ai_agent")))
        .await
        .expect("applied");
    assert_eq!(written.id, ROW);
    assert_eq!(written.thread_control_state.as_deref(), Some("standby"));
    assert_eq!(written.thread_owner_role.as_deref(), Some("ai_agent"));
    // Role-based channels never name an app: both columns are written null.
    assert_eq!(written.thread_owner_app_id, None);
    assert_eq!(written.thread_previous_owner_app_id, None);
    assert_eq!(written.thread_control_updated_at, Some(occurred_at()));
    assert_eq!(
        written.thread_control_last_event.as_deref(),
        Some("standbyReceived")
    );
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_thread_owner_expires_at_a_date_sets_the_column()
 {
    let db = fixture().await;
    let input = ThreadControlTransition {
        thread_owner_expires_at: Some(Some(expires_at())),
        ..transition(ROW, "standbyReceived", None)
    };
    apply(&db, input).await.expect("applied");
    assert_eq!(
        row(&db, ROW).await.thread_owner_expires_at,
        Some(expires_at())
    );
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_thread_owner_expires_at_null_clears_the_column()
 {
    let db = fixture().await;
    set_expires(&db, ROW, Some(expires_at())).await;
    let input = ThreadControlTransition {
        thread_owner_expires_at: Some(None),
        ..transition(ROW, "standbyReceived", None)
    };
    apply(&db, input).await.expect("applied");
    assert_eq!(row(&db, ROW).await.thread_owner_expires_at, None);
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_thread_owner_expires_at_undefined_and_the_whatsapp_path_which_never_provides_it_leaves_the_column_out_of_the_set()
 {
    let db = fixture().await;
    set_expires(&db, ROW, Some(expires_at())).await;
    apply(&db, transition(ROW, "standbyReceived", None))
        .await
        .expect("applied");
    assert_eq!(
        row(&db, ROW).await.thread_owner_expires_at,
        Some(expires_at())
    );
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_thread_owner_expires_at_the_expiry_is_not_part_of_the_transition_guard()
 {
    let db = fixture().await;
    let with_expiry = |id: i64| ThreadControlTransition {
        thread_owner_expires_at: Some(Some(expires_at())),
        ..transition(id, "standbyReceived", None)
    };
    // Same row state, with and without the expiry: the same guard result.
    assert!(apply(&db, with_expiry(ROW)).await.is_some());
    assert!(
        apply(&db, transition(83, "standbyReceived", None))
            .await
            .is_some()
    );
    for id in [ROW, 83] {
        state(
            &db,
            id,
            "owned",
            None,
            None,
            occurred_at() + 1,
            "inboundReceived",
        )
        .await;
    }
    assert!(apply(&db, with_expiry(ROW)).await.is_none());
    assert!(
        apply(&db, transition(83, "standbyReceived", None))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_writes_the_owner_and_previous_owner_app_ids_of_an_app_id_channel()
 {
    let db = fixture().await;
    let input = ThreadControlTransition {
        owner_app_id: Some("app-target".to_owned()),
        previous_owner_app_id: Some("app-us".to_owned()),
        ..transition(ROW, "passed", None)
    };
    let written = apply(&db, input).await.expect("applied");
    assert_eq!(written.thread_owner_app_id.as_deref(), Some("app-target"));
    assert_eq!(
        written.thread_previous_owner_app_id.as_deref(),
        Some("app-us")
    );
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_the_idempotent_branch_compares_the_owner_app_id_null_vs_null_for_role_based_channels()
 {
    let db = fixture().await;
    // Role-based: a redelivery of the same event at the same time (app id null = null).
    state(
        &db,
        ROW,
        "owned",
        Some("escalation"),
        None,
        occurred_at(),
        "controlPassed",
    )
    .await;
    assert!(
        apply(&db, transition(ROW, "controlPassed", Some("escalation")))
            .await
            .is_some()
    );
    // App-based: the redelivery must name the same owner app.
    state(
        &db,
        83,
        "standby",
        None,
        Some("app-b"),
        occurred_at(),
        "passed",
    )
    .await;
    let passed = |app: &str| ThreadControlTransition {
        owner_app_id: Some(app.to_owned()),
        ..transition(83, "passed", None)
    };
    assert!(apply(&db, passed("app-c")).await.is_none());
    assert!(apply(&db, passed("app-b")).await.is_some());
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_returns_null_when_the_guard_rejects_the_event_as_stale()
 {
    let db = fixture().await;
    state(
        &db,
        ROW,
        "owned",
        None,
        None,
        occurred_at() + 1000,
        "inboundReceived",
    )
    .await;
    assert_eq!(apply(&db, transition(ROW, "released", None)).await, None);
    assert_eq!(
        row(&db, ROW).await.thread_control_state.as_deref(),
        Some("owned")
    );
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_guard_scopes_by_id_and_workspace_and_orders_by_timestamp_then_precedence()
 {
    let db = fixture().await;
    // Another workspace's row is fenced out by the EXISTS on Inbox.
    assert!(
        apply(&db, transition(FOREIGN_ROW, "taken", Some("escalation")))
            .await
            .is_none()
    );
    // An older row always loses.
    state(
        &db,
        ROW,
        "standby",
        None,
        None,
        occurred_at() - 1,
        "controlTaken",
    )
    .await;
    assert!(
        apply(&db, transition(ROW, "taken", Some("escalation")))
            .await
            .is_some()
    );
    // On a tie, `taken` outranks the inferred and outbound-call events ...
    for (id, last) in [(ROW, "released"), (83, "inboundReceived")] {
        state(&db, id, "idle", None, None, occurred_at(), last).await;
        assert!(
            apply(&db, transition(id, "taken", Some("escalation")))
                .await
                .is_some(),
            "{last}"
        );
    }
    // ... never Meta's own control events.
    for last in ["controlPassed", "controlTaken"] {
        state(&db, ROW, "standby", None, None, occurred_at(), last).await;
        assert!(
            apply(&db, transition(ROW, "taken", Some("escalation")))
                .await
                .is_none(),
            "{last}"
        );
    }
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_the_standby_copy_promotion_is_guarded_to_the_exact_standby_copy_in_the_workspace()
 {
    let db = fixture().await;
    let copy_at = ms("2026-09-29T09:59:59.000Z");
    let promote = |id: i64, workspace_id: i64| {
        write(&db, move |c| {
            ci::promote_standby_to_owner_delivery(c, id, workspace_id, None, None, None, copy_at)
        })
    };
    state(&db, ROW, "standby", None, None, copy_at, "standbyReceived").await;
    state(
        &db,
        83,
        "standby",
        None,
        None,
        copy_at + 1,
        "standbyReceived",
    )
    .await;
    state(
        &db,
        FOREIGN_ROW,
        "standby",
        None,
        None,
        copy_at,
        "standbyReceived",
    )
    .await;
    assert!(promote(83, WS).await.is_none());
    assert!(promote(FOREIGN_ROW, WS).await.is_none());
    let promoted = promote(ROW, WS).await.expect("the exact standby copy");
    assert_eq!(promoted.thread_control_state.as_deref(), Some("owned"));
    assert_eq!(
        promoted.thread_control_last_event.as_deref(),
        Some("inboundReceived")
    );
    assert_eq!(promoted.thread_control_updated_at, Some(copy_at));
}

#[tokio::test]
async fn contact_inbox_repository_apply_thread_control_transition_the_lowest_precedence_event_has_no_outranked_list_only_redelivery_on_a_tie()
 {
    let db = fixture().await;
    state(
        &db,
        ROW,
        "standby",
        None,
        None,
        occurred_at(),
        "standbyReceived",
    )
    .await;
    assert!(
        apply(&db, transition(ROW, "inboundReceived", None))
            .await
            .is_none()
    );
    state(
        &db,
        83,
        "owned",
        None,
        None,
        occurred_at(),
        "inboundReceived",
    )
    .await;
    assert!(
        apply(&db, transition(83, "inboundReceived", None))
            .await
            .is_some()
    );
}

// ---------------------------------------------------------------------------
// contactInboxRepository.setStandbyThreadOwnerExpiresAt
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contact_inbox_repository_set_standby_thread_owner_expires_at_writes_only_the_expiry_guarded_to_the_same_standby_owner_in_the_workspace()
 {
    let db = fixture().await;
    let observed = occurred_at();
    state(
        &db,
        ROW,
        "standby",
        None,
        Some("app-partner"),
        observed,
        "passed",
    )
    .await;
    let set = |app: &'static str, workspace_id: i64| {
        write(&db, move |c| {
            ci::set_standby_thread_owner_expires_at(
                c,
                ROW,
                workspace_id,
                Some(app),
                Some(observed),
                Some(expires_at()),
            )
        })
    };
    assert!(set("app-other", WS).await.is_none());
    assert!(set("app-partner", OTHER).await.is_none());
    let written = set("app-partner", WS).await.expect("same standby owner");
    assert_eq!(written.thread_owner_expires_at, Some(expires_at()));
    assert_eq!(written.thread_control_state.as_deref(), Some("standby"));
    assert_eq!(written.thread_control_updated_at, Some(observed));
    assert_eq!(written.thread_control_last_event.as_deref(), Some("passed"));
}

#[tokio::test]
async fn contact_inbox_repository_set_standby_thread_owner_expires_at_returns_null_when_the_thread_moved_on()
 {
    let db = fixture().await;
    state(
        &db,
        ROW,
        "owned",
        None,
        None,
        occurred_at(),
        "inboundReceived",
    )
    .await;
    let written = write(&db, |c| {
        ci::set_standby_thread_owner_expires_at(c, ROW, WS, None, None, None)
    })
    .await;
    assert_eq!(written, None);
}

// ---------------------------------------------------------------------------
// contactInboxRepository.listThreadControlledByContactIds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contact_inbox_repository_list_thread_controlled_by_contact_ids_returns_without_querying_for_an_empty_id_list()
 {
    let db = fixture().await;
    assert!(
        run(&db, |c| ci::list_thread_controlled_by_contact_ids(
            c,
            WS,
            &[]
        ))
        .await
        .is_empty()
    );
}

#[tokio::test]
async fn contact_inbox_repository_list_thread_controlled_by_contact_ids_joins_inbox_scoped_to_the_workspace_and_filters_owned_rows_in_one_query()
 {
    let db = fixture().await;
    state(
        &db,
        ROW,
        "owned",
        None,
        None,
        occurred_at(),
        "inboundReceived",
    )
    .await;
    state(
        &db,
        84,
        "standby",
        None,
        None,
        occurred_at(),
        "standbyReceived",
    )
    .await;
    state(
        &db,
        FOREIGN_ROW,
        "owned",
        None,
        None,
        occurred_at(),
        "inboundReceived",
    )
    .await;
    let rows = run(&db, |c| {
        ci::list_thread_controlled_by_contact_ids(c, WS, &[4001, 4002, 4201])
    })
    .await;
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ROW]);
    assert_eq!((rows[0].contact_id, rows[0].inbox_id), (4001, 3001));
}

// ---------------------------------------------------------------------------
// inboxRepository.touchThreadControlSeen
// ---------------------------------------------------------------------------

async fn seen_at(db: &Db, inbox_id: i64) -> Option<i64> {
    scalar_i64(
        db,
        "SELECT \"threadControlSeenAt\" FROM \"Inbox\" WHERE \"id\" = ?1",
        vec![Value::Integer(inbox_id)],
    )
    .await
}

async fn touch(db: &Db, workspace_id: i64, inbox_id: i64, at: i64) -> bool {
    write(db, move |c| {
        inbox::touch_thread_control_seen(c, workspace_id, inbox_id, at)
    })
    .await
}

#[tokio::test]
async fn inbox_repository_touch_thread_control_seen_sets_the_timestamp_guarded_by_workspace_and_a_sql_side_throttle()
 {
    let db = fixture().await;
    let seen = occurred_at();
    assert!(!touch(&db, WS, 3002, seen).await);
    assert_eq!(seen_at(&db, 3002).await, None);
    assert!(touch(&db, WS, 3001, seen - THREAD_CONTROL_SEEN_REFRESH_MS - 1).await);
    assert!(touch(&db, WS, 3001, seen).await);
    assert_eq!(seen_at(&db, 3001).await, Some(seen));
}

#[tokio::test]
async fn inbox_repository_touch_thread_control_seen_reports_false_when_the_throttle_skipped_the_write()
 {
    let db = fixture().await;
    let seen = occurred_at();
    assert!(touch(&db, WS, 3001, seen - 3_600_000).await);
    assert!(!touch(&db, WS, 3001, seen).await);
    assert_eq!(seen_at(&db, 3001).await, Some(seen - 3_600_000));
}

// ---------------------------------------------------------------------------
// integration{Whatsapp,Messenger}Repository.updateHandoverResumeFlow
// ---------------------------------------------------------------------------

/// Flow 61 (WS). WhatsApp integrations: 5001 (WS, inbox 3003 connected), 5002 (WS, inbox
/// 3005 disconnected), 5003 (OTHER, inbox 3006 connected). Messenger 6001 (WS, inbox 3004).
async fn integration_fixture() -> common::TestDb {
    let db = fixture().await;
    setup(&db, |tx| {
        ins!(tx, "Flow", { "id" => 61_i64, "workspaceId" => WS, "name" => "Resume".to_owned() })?;
        for (inbox, ws, channel) in [
            (3003_i64, WS, "whatsapp"),
            (3004, WS, "messenger"),
            (3005, WS, "whatsapp"),
            (3006, OTHER, "whatsapp"),
        ] {
            insert_inbox(tx, inbox, ws, channel, &format!("src-{inbox}"))?;
        }
        tx.execute(
            "UPDATE \"Inbox\" SET \"status\" = 'disconnected', \"disconnectReason\" = 'manual', \
             \"disconnectedAt\" = 1 WHERE \"id\" = 3005",
            [],
        )?;
        for (id, ws, inbox) in [
            (5001_i64, WS, 3003_i64),
            (5002, WS, 3005),
            (5003, OTHER, 3006),
        ] {
            ins!(tx, "IntegrationWhatsapp", {
                "id" => id, "auth" => "{\"t\":1}".to_owned(), "phoneNumberId" => format!("pn-{id}"),
                "wabaId" => format!("waba-{id}"), "businessId" => "biz".to_owned(),
                "name" => format!("WA {id}"), "workspaceId" => ws, "inboxId" => inbox,
            })?;
        }
        ins!(tx, "IntegrationMessenger", {
            "id" => 6001_i64, "auth" => "{}".to_owned(), "pageId" => "page-6001".to_owned(),
            "name" => "Page".to_owned(), "conversationStarters" => "[]".to_owned(),
            "persistentMenus" => "[]".to_owned(), "personas" => "[]".to_owned(),
            "workspaceId" => WS, "inboxId" => 3004_i64,
        })?;
        Ok(())
    })
    .await;
    db
}

async fn whatsapp_flow(db: &Db, workspace_id: i64, flow: Option<i64>) -> Option<Option<i64>> {
    write(db, move |c| {
        iw::update_handover_resume_flow(c, 5001, workspace_id, flow)
    })
    .await
    .map(|r| r.handover_resume_flow_id)
}

async fn messenger_flow(db: &Db, workspace_id: i64, flow: Option<i64>) -> Option<Option<i64>> {
    write(db, move |c| {
        im::update_handover_resume_flow(c, 6001, workspace_id, flow)
    })
    .await
    .map(|r| r.handover_resume_flow_id)
}

#[tokio::test]
async fn integration_whatsapp_repository_update_handover_resume_flow_sets_the_flow_scoped_by_workspace_and_integration()
 {
    let db = integration_fixture().await;
    assert_eq!(whatsapp_flow(&db, WS, Some(61)).await, Some(Some(61)));
}

#[tokio::test]
async fn integration_whatsapp_repository_update_handover_resume_flow_clears_the_flow_with_null() {
    let db = integration_fixture().await;
    whatsapp_flow(&db, WS, Some(61)).await;
    assert_eq!(whatsapp_flow(&db, WS, None).await, Some(None));
}

#[tokio::test]
async fn integration_whatsapp_repository_update_handover_resume_flow_returns_null_when_the_integration_is_not_in_the_workspace()
 {
    let db = integration_fixture().await;
    assert_eq!(whatsapp_flow(&db, OTHER, Some(61)).await, None);
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT \"handoverResumeFlowId\" FROM \"IntegrationWhatsapp\" WHERE \"id\" = 5001",
            vec![]
        )
        .await,
        None
    );
}

#[tokio::test]
async fn integration_messenger_repository_update_handover_resume_flow_sets_the_flow_scoped_by_workspace_and_integration()
 {
    let db = integration_fixture().await;
    assert_eq!(messenger_flow(&db, WS, Some(61)).await, Some(Some(61)));
}

#[tokio::test]
async fn integration_messenger_repository_update_handover_resume_flow_clears_the_flow_with_null() {
    let db = integration_fixture().await;
    messenger_flow(&db, WS, Some(61)).await;
    assert_eq!(messenger_flow(&db, WS, None).await, Some(None));
}

#[tokio::test]
async fn integration_messenger_repository_update_handover_resume_flow_returns_null_when_the_integration_is_not_in_the_workspace()
 {
    let db = integration_fixture().await;
    assert_eq!(messenger_flow(&db, OTHER, Some(61)).await, None);
}

#[tokio::test]
async fn integration_whatsapp_repository_find_all_connected_for_webhook_subscription_joins_the_inbox_and_keeps_only_connected_inboxes()
 {
    let db = integration_fixture().await;
    let rows = run(&db, iw::find_all_connected_for_webhook_subscription).await;
    assert_eq!(
        rows.iter()
            .map(|r| (r.id, r.workspace_id, r.waba_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(5001, WS, "waba-5001"), (5003, OTHER, "waba-5003")]
    );
    assert_eq!(rows[0].auth.0, serde_json::json!({ "t": 1 }));
}

// ---------------------------------------------------------------------------
// contactInboxRepository.findModelByIdForWorkspace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contact_inbox_repository_find_model_by_id_for_workspace_returns_the_full_contact_inbox_row_joined_to_the_workspace_s_inbox()
 {
    let db = fixture().await;
    state(
        &db,
        ROW,
        "owned",
        None,
        None,
        occurred_at(),
        "inboundReceived",
    )
    .await;
    let found = run(&db, |c| ci::find_model_by_id_for_workspace(c, ROW, WS)).await;
    assert_eq!(found, Some(row(&db, ROW).await));
    assert_eq!(
        found.and_then(|r| r.thread_control_state).as_deref(),
        Some("owned")
    );
}

#[tokio::test]
async fn contact_inbox_repository_find_model_by_id_for_workspace_returns_null_for_an_unknown_id_or_another_workspace()
 {
    let db = fixture().await;
    assert_eq!(
        run(&db, |c| ci::find_model_by_id_for_workspace(c, ROW, OTHER)).await,
        None
    );
    assert_eq!(
        run(&db, |c| ci::find_model_by_id_for_workspace(c, 999, WS)).await,
        None
    );
}
