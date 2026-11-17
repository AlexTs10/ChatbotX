//! Port of `broadcast-service-transitions.test.ts`, `broadcast-service-drafts.test.ts`,
//! `broadcast-service-clone.test.ts`, `broadcast-service-resend.test.ts` and
//! `broadcast-service-create.test.ts` (Q.D5). Adapted: the mocked drizzle
//! calls become stored rows; "updates with X" becomes the row after the call;
//! the audit spy is a recording job enqueuer.

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::broadcast::{self as service, audience, write};
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::Enqueuer;
use common::{OTHER_WS, TestDb, WS, scalar_i64, scalar_text, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const FUTURE: &str = "2030-01-07T09:00:00.000Z";

fn audit() -> (AuditService, Arc<RecordingEnqueuer>) {
    let jobs = RecordingEnqueuer::new();
    (AuditService::new(jobs.clone()), jobs)
}

/// The audited caller (the original reads it from the request context).
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

/// Flow 61 "Promo flow" (`WS`), flow 69 (`OTHER_WS`).
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Promo flow'), (69, ?2, 'Far')"#,
            params![WS, OTHER_WS],
        )?;
        Ok(())
    })
    .await;
    db
}

fn payload(overrides: Value) -> Value {
    let mut out = json!({
        "channel": "omnichannel",
        "subaction": "allContacts",
        "flowId": "61",
        "schedulesType": "future",
        "schedulesAt": FUTURE,
        "contactFilter": { "operator": "and", "conditions": [] },
    });
    for (k, v) in overrides.as_object().expect("object") {
        out[k] = v.clone();
    }
    out
}

/// The exception message.
fn message(error: &chatbotx_business::BusinessError) -> String {
    match error {
        chatbotx_business::BusinessError::Exception { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn rule() -> Value {
    json!({
        "frequency": "daily",
        "interval": 1,
        "timeOfDay": "09:00",
        "timezone": "UTC",
        "startsOn": "2030-01-01",
        "end": { "kind": "never" },
    })
}

async fn status(db: &TestDb, id: i64) -> String {
    scalar_text(
        db,
        r#"SELECT "status" FROM "Broadcast" WHERE "id" = ?1"#,
        id,
    )
    .await
    .expect("status")
}

// broadcast-service-create

#[tokio::test]
async fn create_names_a_flow_broadcast_after_its_flow_and_audits_the_create() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, jobs) = audit();
        let created = write::create(&db, &audit, WS, payload(json!({})), true)
            .await
            .expect("create");
        assert_eq!(
            (created.name.as_str(), created.status.as_str()),
            ("Promo flow", "scheduled")
        );
        assert_eq!(
            details(&jobs),
            vec![format!("created a new broadcast (#{})", created.id)]
        );
    })
    .await
}

#[tokio::test]
async fn create_now_also_audits_the_launch_and_a_draft_does_not() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, jobs) = audit();
        let now = write::create(
            &db,
            &audit,
            WS,
            payload(json!({ "schedulesType": "now" })),
            true,
        )
        .await
        .expect("now");
        let draft = write::create(
            &db,
            &audit,
            WS,
            payload(json!({ "schedulesType": "now", "saveAsDraft": true })),
            true,
        )
        .await
        .expect("draft");
        assert_eq!(
            details(&jobs),
            vec![
                format!("created a new broadcast (#{})", now.id),
                format!("launched a broadcast (#{})", now.id),
                format!("created a new broadcast (#{})", draft.id),
            ]
        );
    })
    .await
}

#[tokio::test]
async fn create_refuses_a_foreign_flow_and_an_unsupported_subaction() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let err = write::create(&db, &audit, WS, payload(json!({ "flowId": "69" })), true)
            .await
            .expect_err("foreign");
        assert_eq!(
            (err.code(), message(&err).contains("Flow not found")),
            (Some("validation"), true)
        );
        let err = write::create(
            &db,
            &audit,
            WS,
            payload(json!({ "subaction": "whatsappWithin24Hours" })),
            true,
        )
        .await
        .expect_err("subaction");
        assert!(message(&err).contains("Unsupported broadcast subaction"));
    })
    .await
}

#[tokio::test]
async fn create_prunes_email_and_phone_conditions_for_a_member_without_the_permission() {
    with_audit_context(actor(), async {
    let db = db().await;
    let (audit, _) = audit();
    let filter = json!({ "operator": "and", "conditions": [{ "field": "email", "operator": "isNotEmpty" }] });
    let created = write::create(
        &db,
        &audit,
        WS,
        payload(json!({ "contactFilter": filter })),
        false,
    )
    .await
    .expect("create");
    let stored = scalar_text(
        &db,
        r#"SELECT "contactFilter" FROM "Broadcast" WHERE "id" = ?1"#,
        created.id,
    )
    .await
    .expect("filter");
    assert!(!stored.contains("email"), "{stored}");
    })
    .await
}

#[tokio::test]
async fn create_truncates_the_schedule_to_the_minute() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let created = write::create(
            &db,
            &audit,
            WS,
            payload(json!({ "schedulesAt": "2030-01-07T09:00:42.123Z" })),
            true,
        )
        .await
        .expect("create");
        assert_eq!(created.schedules_at % 60_000, 0);
    })
    .await
}

// broadcast-service-drafts

#[tokio::test]
async fn update_draft_keeps_a_custom_name_and_schedules_with_save_as_draft_false() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let draft = write::create(
            &db,
            &audit,
            WS,
            payload(json!({ "saveAsDraft": true })),
            true,
        )
        .await
        .expect("draft");
        service::update(&db, &audit, WS, draft.id, "Kept".into())
            .await
            .expect("rename");
        let out = write::update_draft(&db, &audit, None, WS, draft.id, payload(json!({})), true)
            .await
            .expect("update");
        assert_eq!(
            out,
            json!({ "id": draft.id.to_string(), "status": "scheduled" })
        );
        assert_eq!(
            scalar_text(
                &db,
                r#"SELECT "name" FROM "Broadcast" WHERE "id" = ?1"#,
                draft.id
            )
            .await
            .as_deref(),
            Some("Kept")
        );
        let err = write::update_draft(
            &db,
            &audit,
            None,
            WS,
            draft.id,
            payload(json!({ "saveAsDraft": true })),
            true,
        )
        .await
        .expect_err("not a draft");
        assert_eq!(message(&err), "Broadcast is not a draft");
    })
    .await
}

#[tokio::test]
async fn scheduling_a_draft_waits_while_a_previous_prepare_may_still_write_recipients() {
    with_audit_context(actor(), async {
    let db = db().await;
    let (audit, _) = audit();
    let jobs: Arc<dyn Enqueuer> = RecordingEnqueuer::new();
    let recorder = jobs.clone();
    let draft = write::create(
        &db,
        &audit,
        WS,
        payload(json!({ "saveAsDraft": true })),
        true,
    )
    .await
    .expect("draft");
    let job_id = format!("schedule-prepare-broadcast-{}", draft.id);
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "jobs" ("queue", "name", "job_id", "data", "opts", "state", "ready_seq", "run_at_ms", "created_at_ms")
               VALUES ('schedule', 'prepareBroadcast', ?1, '{}', '{}', 'delayed', 1, 0, 0)"#,
            params![job_id],
        )?;
        Ok(())
    })
    .await;
    let err = write::schedule_draft(
        &db,
        &audit,
        Some(&jobs),
        WS,
        draft.id,
        "future".into(),
        1_893_920_400_000,
        None,
        None,
    )
    .await
    .expect_err("still preparing");
    assert_eq!(
        message(&err),
        "Broadcast is still clearing recipients from its previous schedule. Try again in a moment."
    );
    let _ = recorder;
    assert_eq!(status(&db, draft.id).await, "draft");
    })
    .await
}

#[tokio::test]
async fn schedule_draft_refuses_a_recurring_schedule_without_a_rule() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let draft = write::create(
            &db,
            &audit,
            WS,
            payload(json!({ "saveAsDraft": true })),
            true,
        )
        .await
        .expect("draft");
        let err = write::schedule_draft(
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
        .expect_err("no rule");
        assert_eq!(message(&err), "Repeat settings are required");
    })
    .await
}

// broadcast-service-transitions (repeating broadcasts)

#[tokio::test]
async fn a_repeating_broadcast_starts_pauses_resumes_moves_to_draft_and_is_retired_on_delete() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, jobs) = audit();
        let draft = write::create(
            &db,
            &audit,
            WS,
            payload(
                json!({ "schedulesType": "recurring", "recurrence": rule(), "saveAsDraft": true }),
            ),
            true,
        )
        .await
        .expect("draft");
        assert_eq!(draft.status, "draft");
        let out = write::schedule_draft(
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
        assert_eq!(out, json!({ "id": draft.id.to_string() }));
        assert_eq!(status(&db, draft.id).await, "recurring");
        let next = scalar_i64(
            &db,
            r#"SELECT "nextRunAt" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
            draft.id,
        )
        .await;
        assert!(next.is_some());

        service::stop_sending(&db, &audit, WS, draft.id)
            .await
            .expect("pause");
        assert_eq!(status(&db, draft.id).await, "cancelled");
        let paused = scalar_i64(
            &db,
            r#"SELECT "pausedAt" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
            draft.id,
        )
        .await;
        assert!(paused.is_some());

        let resumed = service::resume_sending(&db, &audit, WS, draft.id, None)
            .await
            .expect("resume");
        assert!(resumed["nextRunAt"].is_string(), "{resumed}");
        assert_eq!(status(&db, draft.id).await, "recurring");

        service::move_to_draft(&db, &audit, WS, draft.id)
            .await
            .expect("draft");
        assert_eq!(status(&db, draft.id).await, "draft");
        assert_eq!(
            scalar_i64(
                &db,
                r#"SELECT "nextRunAt" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
                draft.id
            )
            .await,
            None
        );

        let (deleted, requested) =
            service::soft_delete_broadcasts(&db, &audit, WS, vec![draft.id, 1])
                .await
                .expect("delete");
        assert_eq!((deleted, requested), (1, 2));
        assert!(
            details(&jobs)
                .iter()
                .any(|d| d == &format!("started a repeating broadcast (#{})", draft.id))
        );
    })
    .await
}

#[tokio::test]
async fn move_to_draft_resets_the_contact_count_and_bumps_the_epoch() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let created = write::create(&db, &audit, WS, payload(json!({})), true)
            .await
            .expect("create");
        service::move_to_draft(&db, &audit, WS, created.id)
            .await
            .expect("draft");
        assert_eq!(
            scalar_i64(
                &db,
                r#"SELECT "resumeCount" FROM "Broadcast" WHERE "id" = ?1"#,
                created.id
            )
            .await,
            Some(1)
        );
        assert!(
            service::move_to_draft(&db, &audit, WS, created.id)
                .await
                .is_err()
        );
    })
    .await
}

// broadcast-service-clone / resend

#[tokio::test]
async fn clone_numbers_copies_and_copies_the_repeat_rule() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, jobs) = audit();
        let source = write::create(
            &db,
            &audit,
            WS,
            payload(
                json!({ "schedulesType": "recurring", "recurrence": rule(), "saveAsDraft": true }),
            ),
            true,
        )
        .await
        .expect("source");
        let first = write::clone_broadcast(&db, &audit, WS, source.id, true)
            .await
            .expect("clone");
        assert_eq!(
            (first.name.as_str(), first.status.as_str()),
            ("Promo flow (Copy 1)", "draft")
        );
        let second = write::clone_broadcast(&db, &audit, WS, first.id, true)
            .await
            .expect("clone");
        assert_eq!(second.name, "Promo flow (Copy 2)");
        assert!(
            scalar_text(
                &db,
                r#"SELECT "rule" FROM "BroadcastRecurrence" WHERE "broadcastId" = ?1"#,
                first.id
            )
            .await
            .is_some()
        );
        assert!(details(&jobs).contains(&format!("cloned a broadcast (#{})", first.id)));
        assert!(
            write::clone_broadcast(&db, &audit, OTHER_WS, source.id, true)
                .await
                .is_err()
        );
    })
    .await
}

#[tokio::test]
async fn resend_clones_a_sent_broadcast_as_a_send_now_one() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let source = write::create(&db, &audit, WS, payload(json!({})), true)
            .await
            .expect("source");
        let err = write::resend_with_pruning(&db, &audit, WS, source.id, true)
            .await
            .expect_err("not sent");
        assert_eq!(message(&err), "Broadcast is not sent");
        setup(&db, |tx| {
            tx.execute(r#"UPDATE "Broadcast" SET "status" = 'failed'"#, [])?;
            Ok(())
        })
        .await;
        let copy = write::resend_with_pruning(&db, &audit, WS, source.id, true)
            .await
            .expect("resend");
        assert_eq!(
            (
                copy.name.as_str(),
                copy.status.as_str(),
                copy.schedules_type.as_str()
            ),
            ("Promo flow (Resend)", "scheduled", "now")
        );
    })
    .await
}

// broadcast-service-audience

#[tokio::test]
async fn the_audience_is_empty_without_inboxes() {
    with_audit_context(actor(), async {
        let db = db().await;
        let input = audience::AudienceInput {
            workspace_id: WS,
            can_view_email_and_phone: true,
            ..Default::default()
        };
        let counted = audience::count_audience(&db, input.clone(), None)
            .await
            .expect("count");
        assert_eq!((counted.total, counted.total_count_capped), (0, false));
        assert!(
            audience::list_audience_preview(&db, input, None, None)
                .await
                .expect("preview")
                .is_empty()
        );
    })
    .await
}

#[tokio::test]
async fn the_audience_counts_contact_inboxes_with_a_dm() {
    with_audit_context(actor(), async {
    let db = db().await;
    setup(&db, |tx| {
        common::inbox(tx, 81, WS, "webchat")?;
        for contact in [91, 92] {
            common::contact(tx, contact, WS)?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "sourceId", "channel", "source") VALUES (?1, ?2, ?2, 81, ?3, 'webchat', 'webchat')"#,
                params![contact + 100, contact, format!("s{contact}")],
            )?;
        }
        // Only contact 91 has a DM.
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "contactId", "workspaceId") VALUES (95, 91, ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let input = audience::AudienceInput {
        workspace_id: WS,
        channels: Some(vec!["omnichannel".into()]),
        can_view_email_and_phone: true,
        ..Default::default()
    };
    assert_eq!(
        audience::count_audience(&db, input.clone(), None)
            .await
            .expect("count")
            .total,
        1
    );
    let rows = audience::list_audience_preview(&db, input, None, None)
        .await
        .expect("preview");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0]["contactId"].clone(),
            rows[0]["conversationId"].clone()
        ),
        (json!("91"), json!("95"))
    );
    })
    .await
}

#[tokio::test]
async fn the_calendar_lists_the_window_and_hides_a_paused_series() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let one_off = write::create(&db, &audit, WS, payload(json!({})), true)
            .await
            .expect("one-off");
        let rows = audience::list_for_calendar(
            &db,
            audience::CalendarInput {
                workspace_id: WS,
                from: 0,
                to: i64::MAX,
                ..Default::default()
            },
        )
        .await
        .expect("calendar");
        assert_eq!(
            rows.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(),
            vec![json!(one_off.id.to_string())]
        );
        let none = audience::list_for_calendar(
            &db,
            audience::CalendarInput {
                workspace_id: WS,
                from: 0,
                to: 1,
                ..Default::default()
            },
        )
        .await
        .expect("calendar");
        assert!(none.is_empty());
    })
    .await
}

// broadcast-service-update

#[tokio::test]
async fn update_renames_the_broadcast_and_audits() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, jobs) = audit();
        let created = write::create(&db, &audit, WS, payload(json!({})), true)
            .await
            .expect("create");
        service::update(&db, &audit, WS, created.id, "New Name".into())
            .await
            .expect("update");
        let name = scalar_text(
            &db,
            r#"SELECT "name" FROM "Broadcast" WHERE "id" = ?1"#,
            created.id,
        )
        .await
        .expect("name");
        assert_eq!(name, "New Name");
        assert_eq!(
            details(&jobs).last(),
            Some(&format!("updated a broadcast (#{})", created.id))
        );
    })
    .await
}

#[tokio::test]
async fn update_propagates_the_not_found_error_and_never_updates() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, jobs) = audit();
        let created = write::create(
            &db,
            &audit,
            OTHER_WS,
            payload(json!({ "flowId": "69" })),
            true,
        )
        .await
        .expect("create");
        let err = service::update(&db, &audit, WS, created.id, "New Name".into())
            .await
            .expect_err("foreign");
        assert_eq!(err.code(), Some("notFound"));
        let name = scalar_text(
            &db,
            r#"SELECT "name" FROM "Broadcast" WHERE "id" = ?1"#,
            created.id,
        )
        .await
        .expect("name");
        assert_eq!(name, "Far");
        assert!(!details(&jobs).iter().any(|d| d.starts_with("updated")));
    })
    .await
}

// broadcast-service-audience (listAudience deletedAt gate)

#[tokio::test]
async fn list_audience_is_scoped_to_the_workspace_and_skips_a_deleted_broadcast() {
    with_audit_context(actor(), async {
        let db = db().await;
        let (audit, _) = audit();
        let live = write::create(&db, &audit, WS, payload(json!({})), true)
            .await
            .expect("create");
        let out = service::list_audience(&db, WS, live.id.to_string(), Some(1), Some(10))
            .await
            .expect("listed");
        assert!(out.is_object());
        // throws not-found when the broadcast belongs to a different workspace
        let err = service::list_audience(&db, OTHER_WS, live.id.to_string(), Some(1), Some(10))
            .await
            .expect_err("foreign");
        assert_eq!(message(&err), "Broadcast not found");
        // throws not-found for a soft-deleted broadcast
        service::soft_delete_broadcasts(&db, &audit, WS, vec![live.id])
            .await
            .expect("deleted");
        let err = service::list_audience(&db, WS, live.id.to_string(), Some(1), Some(10))
            .await
            .expect_err("deleted");
        assert_eq!(message(&err), "Broadcast not found");
    })
    .await
}

// broadcast-service-payment-request (requireTemplateName via create)

fn exception_field(error: &chatbotx_business::BusinessError) -> Option<String> {
    match error {
        chatbotx_business::BusinessError::Exception { field, .. } => field.clone(),
        _ => None,
    }
}

/// WhatsApp inbox 71 / integration 72 "Page A" with template 73 "pay" (and an
/// unapproved 74); messenger inbox 75 / integration 76 "Page A" with template 77 "pay".
async fn payment_db(buttons: Value) -> TestDb {
    let db = db().await;
    let components = json!([{ "type": "BUTTONS", "buttons": buttons }]).to_string();
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (71, 'Page A', 'whatsapp', 's71', ?1), (75, 'Page A', 'messenger', 's75', ?1)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
               VALUES (72, '{}', 'p', 'w', 'b', 'Page A', ?1, 71)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
               VALUES (76, '{}', 'pg', 'Page A', '[]', '[]', '[]', ?1, 75)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "WhatsappMessageTemplate" ("id", "name", "integrationWhatsappId", "sourceId", "language", "category", "status", "components")
               VALUES (73, 'pay', 72, 't', 'pt_BR', 'UTILITY', 'APPROVED', ?1), (74, 'pay', 72, 't2', 'pt_BR', 'UTILITY', 'PENDING', ?1)"#,
            params![components],
        )?;
        tx.execute(
            r#"INSERT INTO "MessengerMessageTemplate" ("id", "name", "integrationMessengerId", "sourceId", "language", "category", "status", "components")
               VALUES (77, 'pay', 76, 't', 'pt_BR', 'UTILITY', 'APPROVED', ?1)"#,
            params![components],
        )?;
        Ok(())
    })
    .await;
    db
}

fn pix_button() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Copy Pix code", "payment_setting": { "type": "pix_dynamic_code" } })
}

fn payment_params(request: Value) -> Value {
    json!({ "button": [{ "sub_type": "payment_request", "index": 0, "payment_request": request }] })
}

/// `requireTemplateName` through a draft create; `None` leaves `templateData` out.
async fn template_name(
    db: &TestDb,
    channel: &str,
    template_id: &str,
    template_data: Option<Value>,
) -> chatbotx_business::Result<String> {
    let (inbox, subaction) = match channel {
        "whatsapp" => ("71", "whatsappTemplateMessage"),
        _ => ("75", "messengerTemplateMessage"),
    };
    let mut target = json!({ "inboxId": inbox, "templateId": template_id });
    if let Some(data) = template_data {
        target["templateData"] = data;
    }
    let (audit, _) = audit();
    write::create(
        db,
        &audit,
        WS,
        payload(json!({
            "channel": channel,
            "subaction": subaction,
            "flowId": null,
            "saveAsDraft": true,
            "targets": [target],
        })),
        true,
    )
    .await
    .map(|b| b.name)
}

#[tokio::test]
async fn payment_request_matching_its_template_button_is_accepted() {
    with_audit_context(actor(), async {
        let db = payment_db(json!([pix_button()])).await;
        let name = template_name(
            &db,
            "whatsapp",
            "73",
            Some(payment_params(
                json!({ "method": "pix", "code": "{{pix}}" }),
            )),
        )
        .await
        .expect("accepted");
        assert_eq!(name, "Page A - pay");
    })
    .await
}

#[tokio::test]
async fn payment_request_mismatches_are_rejected_on_the_template_id() {
    with_audit_context(actor(), async {
        // rejects a request for another payment method
        let db =
            payment_db(json!([{ "type": "PAYMENT_REQUEST", "text": "Copy Boleto code" }])).await;
        let err = template_name(
            &db,
            "whatsapp",
            "73",
            Some(payment_params(json!({ "method": "pix", "code": "x" }))),
        )
        .await
        .expect_err("other method");
        assert_eq!(exception_field(&err).as_deref(), Some("templateId"));
        // rejects a missing request for a template payment button
        let db = payment_db(json!([pix_button()])).await;
        let err = template_name(&db, "whatsapp", "73", Some(json!({ "button": [] })))
            .await
            .expect_err("missing request");
        assert_eq!(exception_field(&err).as_deref(), Some("templateId"));
        // params left out for a payment template are rejected, not failed per recipient
        for data in [None, Some(Value::Null)] {
            let err = template_name(&db, "whatsapp", "73", data)
                .await
                .expect_err("left out");
            assert_eq!(exception_field(&err).as_deref(), Some("templateId"));
        }
        // rejects payment data for a template without payment buttons
        let db = payment_db(json!([{ "type": "QUICK_REPLY", "text": "Later" }])).await;
        let err = template_name(
            &db,
            "whatsapp",
            "73",
            Some(payment_params(json!({ "method": "pix", "code": "x" }))),
        )
        .await
        .expect_err("no payment button");
        assert_eq!(exception_field(&err).as_deref(), Some("templateId"));
    })
    .await
}

#[tokio::test]
async fn templates_without_payment_buttons_and_unparsed_params_pass() {
    with_audit_context(actor(), async {
        // a template without payment buttons needs no params
        let db = payment_db(json!([{ "type": "QUICK_REPLY", "text": "Later" }])).await;
        for data in [None, Some(Value::Null)] {
            let name = template_name(&db, "whatsapp", "73", data)
                .await
                .expect("no params");
            assert_eq!(name, "Page A - pay");
        }
        // params that do not parse are left to the other validators
        let db = payment_db(json!([pix_button()])).await;
        let name = template_name(
            &db,
            "whatsapp",
            "73",
            Some(json!({ "button": "not-a-list" })),
        )
        .await
        .expect("unparsed");
        assert_eq!(name, "Page A - pay");
        // only WhatsApp broadcasts are checked
        let name = template_name(
            &db,
            "messenger",
            "77",
            Some(payment_params(
                json!({ "method": "boleto", "digitableLine": "1" }),
            )),
        )
        .await
        .expect("messenger");
        assert_eq!(name, "Page A - pay");
    })
    .await
}

#[tokio::test]
async fn an_unapproved_template_is_still_refused_first() {
    with_audit_context(actor(), async {
        let db = payment_db(json!([pix_button()])).await;
        let err = template_name(
            &db,
            "whatsapp",
            "74",
            Some(payment_params(json!({ "method": "pix", "code": "x" }))),
        )
        .await
        .expect_err("pending");
        assert_eq!(message(&err), "Template is not approved");
    })
    .await
}
