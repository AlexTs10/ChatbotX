//! P3.6-X1 own tests (not ported, not locked): the side effects the handlers
//! now send through `ApiState` (audit rows, bus events, realtime frames).

mod handlers_common;

use std::sync::Arc;

use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::events::{EventBus, RecordingSubscriber};
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

/// The fixture with a direct audit writer (rows on) and a recording bus.
async fn wired() -> (Fixture, Arc<RecordingSubscriber>) {
    let mut f = fixture().await;
    let bus = EventBus::new();
    let recorder = RecordingSubscriber::new();
    bus.subscribe(recorder.clone());
    f.state = f
        .state
        .clone()
        .with_audit(AuditService::direct(Arc::clone(&f.state.db), true))
        .with_events(bus);
    (f, recorder)
}

/// The token middleware actor: the workspace owner and the token source.
fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ip_address: Some("203.0.113.9".into()),
        user_agent: Some("x1-test".into()),
        source: Some("api-token:77".into()),
    }
}

/// One handler call as the token middleware runs it.
async fn as_token(
    f: &Fixture,
    route: &str,
    params: &[(&'static str, &str)],
    query: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    with_audit_context(actor(), call(f, route, WS, params, query, body)).await
}

/// `(action, detail, source)` of every audit row of [`WS`], oldest first.
async fn audit_rows(f: &Fixture) -> Vec<(String, String, Option<String>)> {
    f.state
        .db
        .read(|conn| {
            let mut statement = conn.prepare(
                r#"SELECT "action", "detail", "source" FROM "AuditLog" WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#,
            )?;
            let rows = statement
                .query_map([WS], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("audit rows")
}

fn details(rows: &[(String, String, Option<String>)]) -> Vec<String> {
    rows.iter()
        .map(|(action, detail, _)| format!("{action}: {detail}"))
        .collect()
}

#[tokio::test]
async fn flows_write_the_audit_rows_of_the_flow_service() {
    let (f, _) = wired().await;
    let (status, body) = as_token(
        &f,
        "flows.create",
        &[],
        None,
        Some(json!({ "name": "F", "folderId": null })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().expect("id").to_string();
    let (status, _) = as_token(
        &f,
        "flows.update",
        &[("id", &id)],
        None,
        Some(json!({ "name": "G" })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, copy) = as_token(&f, "flows.duplicate", &[("id", &id)], None, None).await;
    assert_eq!(status, 201, "{copy}");
    let copy = copy["id"].as_str().expect("copy id").to_string();
    let (status, _) = as_token(&f, "flows.delete", &[("id", &copy)], None, None).await;
    assert_eq!(status, 204);
    let rows = audit_rows(&f).await;
    assert_eq!(
        details(&rows),
        [
            format!("create: created a new flow (#{id})"),
            format!("update: updated a flow (#{id})"),
            format!("create: duplicated a flow (#{id} \u{2192} #{copy})"),
            format!("delete: deleted flow (#{copy})"),
        ]
    );
    assert!(
        rows.iter()
            .all(|(_, _, source)| source.as_deref() == Some("api-token:77"))
    );
}

#[tokio::test]
async fn no_audit_row_without_the_token_actor() {
    let (f, _) = wired().await;
    let (status, _) = call(
        &f,
        "flows.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "F", "folderId": null })),
    )
    .await;
    assert_eq!(status, 201);
    assert!(audit_rows(&f).await.is_empty());
}

/// Contact 10 with tag 1; tags 2 and 3; text custom field 50 (`city`).
async fn seed_contact(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            for (id, name) in [(1_i64, "vip"), (2, "gold"), (3, "basic")] {
                tx.execute(
                    r#"INSERT INTO "Tag" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
                    chatbotx_db::rusqlite::params![id, WS, name],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (10, 'Ada', ?1)"#,
                [WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (10, 1)"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "CustomField" ("id", "workspaceId", "name", "type") VALUES (50, ?1, 'city', 'shortText')"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

const CONTACT: (&str, &str) = ("identifier", "id:10");

fn tag_events(recorder: &RecordingSubscriber) -> Vec<String> {
    recorder
        .trigger_events()
        .into_iter()
        .map(|event| format!("{} {}", event.event_type, event.metadata["tagId"]))
        .collect()
}

#[tokio::test]
async fn tag_routes_emit_tag_applied_and_tag_removed() {
    let (f, recorder) = wired().await;
    seed_contact(&f).await;
    let (status, _) = as_token(
        &f,
        "contacts.addTags",
        &[CONTACT],
        None,
        Some(json!({ "tagIds": ["2", "1"] })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = as_token(
        &f,
        "contacts.removeTags",
        &[CONTACT],
        None,
        Some(json!({ "tagIds": ["1"] })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = as_token(
        &f,
        "contacts.setTags",
        &[CONTACT],
        None,
        Some(json!({ "tags": ["basic"] })),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(
        tag_events(&recorder),
        [
            "tagApplied \"2\"",
            "tagRemoved \"1\"",
            "tagApplied \"3\"",
            "tagRemoved \"2\"",
        ]
    );
    let first = &recorder.trigger_events()[0];
    assert_eq!(first.workspace_id, WS.to_string());
    assert_eq!(first.contact_id, "10");
    assert_eq!(first.source_id(), Some("2"));
}

#[tokio::test]
async fn custom_field_writes_emit_custom_field_value_changed() {
    let (f, recorder) = wired().await;
    seed_contact(&f).await;
    let field = ("idOrName", "city");
    let (status, body) = as_token(
        &f,
        "contacts.setCustomField",
        &[CONTACT, field],
        None,
        Some(json!({ "value": "Oslo" })),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    // The same value again is no change.
    let (status, _) = as_token(
        &f,
        "contacts.setCustomField",
        &[CONTACT, field],
        None,
        Some(json!({ "value": "Oslo" })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = as_token(
        &f,
        "contacts.clearCustomField",
        &[CONTACT, field],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let events = recorder.trigger_events();
    let seen: Vec<(String, Value, Value)> = events
        .iter()
        .map(|e| {
            (
                e.event_type.clone(),
                e.metadata["oldValue"].clone(),
                e.metadata["newValue"].clone(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (
                "customFieldValueChanged".to_string(),
                Value::Null,
                json!("Oslo")
            ),
            (
                "customFieldValueChanged".to_string(),
                json!("Oslo"),
                Value::Null
            ),
        ]
    );
    assert_eq!(events[0].metadata["customFieldId"], "50");
    assert_eq!(events[0].metadata["customFieldName"], "city");
    assert_eq!(events[0].source_id(), Some("50"));
}

#[tokio::test]
async fn contact_writes_record_audit_rows_and_trigger_events() {
    let (f, recorder) = wired().await;
    seed_contact(&f).await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (60, ?1, 'Chat', 'webchat', 'src-60')"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("inbox");
    let (status, body) = as_token(
        &f,
        "contacts.create",
        &[],
        None,
        Some(json!({
            "firstName": "Grace",
            "inboxId": "60",
            "channel": "webchat",
            "email": "",
            "gender": "unknown",
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let created = body["id"].as_str().expect("id").to_string();
    let (status, body) = as_token(
        &f,
        "contacts.update",
        &[CONTACT],
        None,
        Some(json!({ "email": "ada@example.com", "50": "Bergen" })),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    let identifier = format!("id:{created}");
    let (status, _) = as_token(
        &f,
        "contacts.delete",
        &[("identifier", &identifier)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(
        details(&audit_rows(&f).await),
        [
            format!("create: created a new contact (#{created})"),
            "update: updated a contact (#10)".to_string(),
            format!("delete: deleted contact (#{created})"),
        ]
    );
    let events = recorder.trigger_events();
    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert_eq!(
        types,
        [
            "newContact",
            "contactInfoUpdated",
            "customFieldValueChanged"
        ]
    );
    assert_eq!(events[0].metadata["name"], "Grace");
    assert!(events[0].metadata.get("email").is_none());
    assert!(events[0].metadata.get("contactInboxId").is_some());
    assert_eq!(events[1].metadata["infoType"], "email");
    assert_eq!(events[1].metadata["newValue"], "ada@example.com");
    assert_eq!(events[2].metadata["newValue"], "Bergen");
}

/// [`as_token`] through `dispatch_as` (the connections routes need it).
async fn as_token_principal(f: &Fixture, route: &str, body: Value) -> (u16, Value) {
    with_audit_context(
        actor(),
        handlers_common::call_as(f, route, WS, None, &[], None, Some(body)),
    )
    .await
    .expect("handled")
}

#[tokio::test]
async fn channel_writes_record_audit_rows() {
    let (f, _) = wired().await;
    let (status, body) = as_token_principal(
        &f,
        "connections.create",
        json!({ "provider": "api", "config": { "name": "Bot" } }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let inbox = body["connection"]["inboxId"]
        .as_str()
        .expect("inbox")
        .to_string();
    let webchat = json!({
        "name": "My Webchat",
        "persistentMenus": [],
        "authorizedDomains": ["example.com"],
        "conversationStarters": [],
        "brandColor": "#112233",
        "hideHeader": false,
        "showLogo": true,
        "hideMessageInput": false,
        "enable": true,
    });
    let (status, body) = as_token(&f, "webchats.create", &[], None, Some(webchat)).await;
    assert_eq!(status, 201, "{body}");
    let webchat_id = body["id"].as_str().expect("id").to_string();
    let (status, _) = as_token(&f, "webchats.delete", &[("id", &webchat_id)], None, None).await;
    assert_eq!(status, 204);
    let (status, body) = as_token(
        &f,
        "sequences.create",
        &[],
        None,
        Some(json!({ "name": "Welcome" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let sequence = body["sequenceId"].as_str().expect("sequence").to_string();
    assert_eq!(
        details(&audit_rows(&f).await),
        [
            format!("create: created a new API key (#{inbox})"),
            format!("connect: connected a new Webchat channel (#{webchat_id})"),
            format!("disconnect: disconnected the Webchat channel (#{webchat_id})"),
            format!("create: created a new sequence (#{sequence})"),
        ]
    );
}

#[tokio::test]
async fn members_list_and_get_the_workspace_members() {
    let (f, _) = wired().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "User" SET "name" = 'Owner One' WHERE "id" = 1001"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions", "createdAt", "updatedAt") VALUES (5001, ?1, 1001, 'owner', '{"superAdmin":true,"flows":true,"extra":1}', 1000, 1000)"#,
                [WS],
            )?;
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (5002, ?1, 1002, 'agent')"#,
                [handlers_common::OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("members");
    let (status, body) = call(&f, "workspaceMembers.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], 1);
    let data = body["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], "5001");
    assert_eq!(data[0]["role"], "owner");
    assert_eq!(data[0]["createdAt"], "1970-01-01T00:00:01.000Z");
    assert_eq!(
        data[0]["permissions"],
        json!({ "superAdmin": true, "flows": true })
    );
    assert_eq!(
        data[0]["user"],
        json!({ "id": "1001", "name": "Owner One", "image": null })
    );
    let (status, body) = call(
        &f,
        "workspaceMembers.list",
        WS,
        &[],
        Some("keyword=nobody"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({ "data": [], "pageCount": 0 }));
    let (status, body) = call(
        &f,
        "workspaceMembers.get",
        WS,
        &[("memberId", "5001")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["userId"], "1001");
    let (status, body) = call(
        &f,
        "workspaceMembers.get",
        WS,
        &[("memberId", "5002")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], "Member not found");
}
