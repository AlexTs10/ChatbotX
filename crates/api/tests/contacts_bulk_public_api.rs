//! Ports for the `/v1/contacts/bulk/*` routes (P3.6-D2 slice contacts bulk):
//!
//! - `apps/builder/__tests__/contacts-bulk-public-api.test.ts` (10):
//!   adapted, the mocked services become the real ones over seeded rows; the
//!   asserted delegation becomes the stored rows and the JSON answer. The
//!   by-stats job runs in the request (no default-queue worker is ported), so
//!   "queues the job" is asserted through the tags it applies; "queues
//!   nothing" through the tag that is not created.
//! - `packages/business/__tests__/contact-delete-and-record.test.ts`, the
//!   three `deleteAndRecord` audit cases: adapted, the audit record is
//!   written by the route layer (P3.6-X1), so they run through
//!   `contacts.bulkDelete` with a direct audit writer. n-a: the four
//!   `analytics:dashboard` attribution cases (no dashboard events are ported,
//!   P3.6-X1 behaviour differences).

mod handlers_common;

use std::sync::Arc;

use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

/// Contacts 1, 2, 3 (`WS`) and 9 (`OTHER_WS`), with contact inboxes `40 +
/// id` and conversations `60 + id` on inbox 5 (`WS`) / 6 (`OTHER_WS`).
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            for (inbox, ws) in [(5_i64, WS), (6, OTHER_WS)] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'Page', 'messenger', ?3)"#,
                    params![inbox, ws, format!("page-{inbox}")],
                )?;
            }
            for (id, ws, inbox) in [(1_i64, WS, 5_i64), (2, WS, 5), (3, WS, 5), (9, OTHER_WS, 6)] {
                tx.execute(
                    r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (?1, ?2, 'Ada')"#,
                    params![id, ws],
                )?;
                tx.execute(
                    r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                       VALUES (?1, ?2, ?2, ?3, 'messenger', 'direct', ?4)"#,
                    params![40 + id, id, inbox, format!("psid-{id}")],
                )?;
                tx.execute(
                    r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (?1, ?2, ?3)"#,
                    params![60 + id, ws, id],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    f
}

async fn exec(f: &Fixture, sql: &'static str, values: Vec<i64>) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(sql, chatbotx_db::rusqlite::params_from_iter(values.iter()))?;
            Ok(())
        })
        .await
        .expect("exec");
}

/// The tag names of a contact, sorted.
async fn tag_names(f: &Fixture, contact_id: i64) -> Vec<String> {
    f.state
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT t."name" FROM "ContactToTag" ct JOIN "Tag" t ON t."id" = ct."tagId"
                   WHERE ct."contactId" = ?1 ORDER BY t."name""#,
            )?;
            let names = stmt
                .query_map([contact_id], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(names)
        })
        .await
        .expect("tag names")
}

async fn count(f: &Fixture, sql: &'static str) -> i64 {
    f.state
        .db
        .read(move |conn| Ok(conn.query_row(sql, [], |r| r.get(0))?))
        .await
        .expect("count")
}

async fn post(f: &Fixture, route: &str, body: Value) -> (u16, Value) {
    call(f, route, WS, &[], None, Some(body)).await
}

#[tokio::test]
async fn post_v1_contacts_bulk_tags_delegates_to_add_contact_tags_with_all_given_contact_ids() {
    let f = seeded().await;
    let (status, body) = post(
        &f,
        "contacts.bulkAddTags",
        json!({ "contactIds": ["1", "2", "3"], "tags": ["VIP"] }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "processed": 3, "skippedContactIds": [] }));
    for id in [1, 2, 3] {
        assert_eq!(tag_names(&f, id).await, vec!["VIP".to_string()]);
    }
}

#[tokio::test]
async fn post_v1_contacts_bulk_tags_remove_detaches_the_named_tags_and_reports_skipped_contact_ids()
{
    let f = seeded().await;
    post(
        &f,
        "contacts.bulkAddTags",
        json!({ "contactIds": ["1", "2"], "tags": ["VIP", "Lead", "Keep"] }),
    )
    .await;
    let (status, body) = post(
        &f,
        "contacts.bulkRemoveTags",
        json!({ "contactIds": ["1", "2", "9"], "tags": ["VIP", "Lead"] }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "processed": 2, "skippedContactIds": ["9"] }));
    assert_eq!(tag_names(&f, 1).await, vec!["Keep".to_string()]);
    assert_eq!(tag_names(&f, 2).await, vec!["Keep".to_string()]);
}

#[tokio::test]
async fn post_v1_contacts_bulk_delete_delegates_to_delete_contact_with_all_given_contact_ids() {
    let f = seeded().await;
    let (status, body) = post(
        &f,
        "contacts.bulkDelete",
        json!({ "contactIds": ["1", "2"] }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "processed": 2, "skippedContactIds": [] }));
    assert_eq!(
        count(&f, r#"SELECT count(*) FROM "Contact" WHERE "id" IN (1, 2)"#).await,
        0
    );
    assert_eq!(
        count(&f, r#"SELECT count(*) FROM "Contact" WHERE "id" = 3"#).await,
        1
    );
}

#[tokio::test]
async fn post_v1_contacts_bulk_sequences_delegates_to_subscribe_contacts_to_sequences_with_all_given_contact_ids()
 {
    let f = seeded().await;
    exec(
        &f,
        r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, 'seq', ?2)"#,
        vec![31, WS],
    )
    .await;
    let (status, body) = post(
        &f,
        "contacts.bulkSubscribeSequences",
        json!({ "contactIds": ["1", "2"], "sequenceIds": ["31"] }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "processed": 2, "skippedContactIds": [] }));
    assert_eq!(
        count(
            &f,
            r#"SELECT count(*) FROM "ContactOnSequence" WHERE "sequenceId" = 31"#
        )
        .await,
        2
    );
}

#[tokio::test]
async fn post_v1_contacts_bulk_sequences_remove_unsubscribes_the_contacts_and_reports_skipped_ids()
{
    let f = seeded().await;
    exec(
        &f,
        r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, 'seq', ?2)"#,
        vec![31, WS],
    )
    .await;
    exec(
        &f,
        r#"INSERT INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId", "status") VALUES (500, 1, 31, ?1, 'active')"#,
        vec![WS],
    )
    .await;
    let (status, body) = post(
        &f,
        "contacts.bulkUnsubscribeSequences",
        json!({ "contactIds": ["1", "8"], "sequenceIds": ["31"] }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "processed": 1, "skippedContactIds": ["8"] }));
    assert_eq!(
        count(&f, r#"SELECT count(*) FROM "ContactOnSequence""#).await,
        0
    );
}

/// Broadcast 70 (`WS`): contact 1 has seen it, contact 2 only got it.
async fn seen_broadcast(f: &Fixture) {
    exec(
        f,
        r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
           VALUES (70, 'b', ?1, 'sent', 'now', 0, 'message', 'messenger')"#,
        vec![WS],
    )
    .await;
    exec(
        f,
        r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "deliveredAt", "seenAt")
           VALUES (70, 1, 41, 61, 1000, 2000), (70, 2, 42, 62, 1000, NULL)"#,
        vec![],
    )
    .await;
}

#[tokio::test]
async fn post_v1_contacts_bulk_tags_by_stats_checks_the_broadcast_belongs_to_the_workspace_then_queues_the_job_as_the_owner()
 {
    let f = seeded().await;
    seen_broadcast(&f).await;
    let (status, body) = post(
        &f,
        "contacts.bulkTagByStats",
        json!({
            "source": "broadcast",
            "broadcastId": "70",
            "eventType": "message:seen",
            "tags": ["Engaged"],
            "excludedContactIds": [],
        }),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body, json!({ "queued": true }));
    assert_eq!(tag_names(&f, 1).await, vec!["Engaged".to_string()]);
    assert!(tag_names(&f, 2).await.is_empty());
}

#[tokio::test]
async fn post_v1_contacts_bulk_tags_by_stats_queues_nothing_when_the_broadcast_is_not_in_the_workspace()
 {
    let f = seeded().await;
    let (status, body) = post(
        &f,
        "contacts.bulkTagByStats",
        json!({
            "source": "broadcast",
            "broadcastId": "71",
            "eventType": "message:seen",
            "tags": ["x"],
            "excludedContactIds": [],
        }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Broadcast not found"));
    assert_eq!(count(&f, r#"SELECT count(*) FROM "Tag""#).await, 0);
}

#[tokio::test]
async fn post_v1_contacts_bulk_tags_by_stats_checks_sequence_ownership_for_a_sequence_step_source()
{
    let f = seeded().await;
    exec(
        &f,
        r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (39, 'foreign', ?1)"#,
        vec![OTHER_WS],
    )
    .await;
    let (status, body) = post(
        &f,
        "contacts.bulkTagByStats",
        json!({
            "source": "sequenceStep",
            "sequenceId": "39",
            "stepId": "1",
            "eventType": "message:sent",
            "tags": ["x"],
            "excludedContactIds": [],
        }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Sequence not found"));
    assert_eq!(count(&f, r#"SELECT count(*) FROM "Tag""#).await, 0);
}

#[tokio::test]
async fn post_v1_contacts_bulk_tags_by_stats_queues_a_comment_automation_source_without_a_pre_check_scoped_in_the_job()
 {
    let f = seeded().await;
    let (status, body) = post(
        &f,
        "contacts.bulkTagByStats",
        json!({
            "source": "commentAutomation",
            "automationId": "1",
            "eventType": "message:sent",
            "tags": ["x"],
            "excludedContactIds": [],
        }),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body, json!({ "queued": true }));
    assert_eq!(
        count(&f, r#"SELECT count(*) FROM "Tag" WHERE "name" = 'x'"#).await,
        1
    );
}

// --- contact-delete-and-record.test.ts (audit) -------------------------------

/// The fixture with a direct audit writer (rows on).
async fn audited() -> Fixture {
    let mut f = seeded().await;
    f.state = f
        .state
        .clone()
        .with_audit(AuditService::direct(Arc::clone(&f.state.db), true));
    f
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ip_address: None,
        user_agent: None,
        source: Some("api-token:77".into()),
    }
}

async fn bulk_delete_as_token(f: &Fixture, ids: &[&str]) -> (u16, Value) {
    with_audit_context(
        actor(),
        post(f, "contacts.bulkDelete", json!({ "contactIds": ids })),
    )
    .await
}

/// `(action, detail)` of every audit row of [`WS`], oldest first.
async fn audit_rows(f: &Fixture) -> Vec<(String, String)> {
    f.state
        .db
        .read(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "action", "detail" FROM "AuditLog" WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#,
            )?;
            let rows = stmt
                .query_map([WS], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("audit rows")
}

#[tokio::test]
async fn contact_service_delete_and_record_emits_one_delete_audit_row_listing_every_deleted_contact()
 {
    let f = audited().await;
    let (status, body) = bulk_delete_as_token(&f, &["1", "2"]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        audit_rows(&f).await,
        vec![(
            "delete".to_string(),
            "deleted contacts (#1, #2)".to_string()
        )]
    );
}

#[tokio::test]
async fn contact_service_delete_and_record_uses_singular_wording_for_a_single_deleted_contact() {
    let f = audited().await;
    let (status, body) = bulk_delete_as_token(&f, &["1"]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        audit_rows(&f).await,
        vec![("delete".to_string(), "deleted contact (#1)".to_string())]
    );
}

#[tokio::test]
async fn contact_service_delete_and_record_emits_no_audit_row_when_nothing_was_deleted() {
    let f = audited().await;
    let (status, body) = bulk_delete_as_token(&f, &["8"]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "processed": 0, "skippedContactIds": ["8"] }));
    assert!(audit_rows(&f).await.is_empty());
}
