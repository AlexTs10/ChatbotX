//! P3.6-X1 own tests (not ported, not locked): the side effects the handlers
//! now send through `ApiState` (audit rows, bus events, realtime frames).

mod handlers_common;

use std::sync::Arc;

use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::events::{EventBus, RecordingSubscriber, TriggerEvent};
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

#[allow(dead_code)]
fn trigger_types(recorder: &RecordingSubscriber) -> Vec<String> {
    recorder
        .trigger_events()
        .into_iter()
        .map(|event: TriggerEvent| event.event_type)
        .collect()
}

#[tokio::test]
async fn flows_write_the_audit_rows_of_the_flow_service() {
    let (f, _) = wired().await;
    let (status, body) =
        as_token(&f, "flows.create", &[], None, Some(json!({ "name": "F", "folderId": null }))).await;
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
