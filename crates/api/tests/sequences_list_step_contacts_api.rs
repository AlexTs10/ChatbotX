//! Port of `apps/builder/__tests__/sequences-list-step-contacts-api.test.ts`
//! (Q.P6-U5). Adapted: the mocked `sequenceService.listStepContactsPage`
//! becomes real dispatch rows, read through the public
//! `sequences.listStepContacts` route (the `/rpc` twin is covered in
//! `rpc_automation_pages.rs`).

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::json;

/// Sequence 31 (step 41) with one seen dispatch for contact 101; sequence 39
/// of `OTHER_WS`.
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (11, ?1, 'Inbox', 'whatsapp', 'src-11')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "lastName", "workspaceId") VALUES (101, 'Ada', 'Lovelace', ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (201, 101, 101, 11, 'whatsapp', 'direct', 'source-1')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (31, 'Drip', ?1), (39, 'Far', ?2)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "SequenceStep" ("id", "order", "delayDays", "sequenceId") VALUES (41, 0, 0, 31), (49, 0, 0, 39)"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId") VALUES (401, 101, 31, ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "SequenceDispatch" ("runAtMs", "idempotencyKey", "workspaceId", "sequenceId", "contactId", "contactInboxId", "stepId", "enrollmentId", "deliveredAt", "seenAt")
                   VALUES (0, 'k1', ?1, 31, 101, 201, 41, 401, 1767225600000, 1767225600000)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

#[test]
fn registers_the_expected_route() {
    let route = route_by_id("sequences.listStepContacts").expect("route");
    assert_eq!(route.path, "/v1/sequences/{id}/steps/{stepId}/contacts");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["broadcasts"]));
}

#[tokio::test]
async fn calls_the_service_with_the_request_params_and_returns_its_result() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.listStepContacts",
        WS,
        &[("id", "31"), ("stepId", "41")],
        Some("eventType=message:seen&page=1&perPage=20"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "data": [{
                "contactId": "101",
                "contactInboxId": "201",
                "firstName": "Ada",
                "lastName": "Lovelace",
                "fullName": "Ada Lovelace",
                "sourceId": "source-1",
                "avatar": null,
                "channel": "whatsapp",
                "errorContent": null,
                "occurredAt": "2026-01-01T00:00:00.000Z",
                "conversationId": "",
            }],
            "total": 1,
            "pageCount": 1,
        })
    );
}

#[tokio::test]
async fn does_not_forward_a_caller_supplied_total_reports_the_services_own() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.listStepContacts",
        WS,
        &[("id", "31"), ("stepId", "41")],
        Some("eventType=message:failed&total=999"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "data": [], "total": 0, "pageCount": 0 }));
}

#[tokio::test]
async fn a_sequence_of_another_workspace_is_not_found_and_the_event_type_is_validated() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.listStepContacts",
        WS,
        &[("id", "39"), ("stepId", "49")],
        Some("eventType=message:sent"),
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    let (status, body) = call(
        &f,
        "sequences.listStepContacts",
        WS,
        &[("id", "31"), ("stepId", "41")],
        Some("eventType=nope"),
        None,
    )
    .await;
    assert_eq!(status, 422, "{body}");
}
