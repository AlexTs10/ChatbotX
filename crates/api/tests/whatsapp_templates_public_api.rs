//! Port of `apps/builder/__tests__/whatsapp-templates-more-public-api.test.ts` and
//! `whatsapp-template-status-forwarding.test.ts` (Q.D12). Adapted: the mocked
//! `whatsappMessageTemplateService` / sync become real rows and a local Graph server
//! (`ApiState.graph_url`); "called with" becomes the returned rows.

mod handlers_common;

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// WhatsApp integration 101 (inbox 6) with templates 9 (APPROVED) and 10 (REJECTED);
/// integration 201 of the other workspace with template 20.
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId")
                   VALUES (6, ?1, 'Wa', 'whatsapp', 'wa-6'), (7, ?2, 'Wa2', 'whatsapp', 'wa-7')"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (101, '{"tokens":{"accessToken":"t"},"metadata":{"wabaId":"waba-1"}}', 'phone-1', 'waba-1', 'b', 'Wa', ?1, 6),
                          (201, '{}', 'phone-2', 'waba-2', 'b', 'Wa2', ?2, 7)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "WhatsappMessageTemplate" ("id", "createdAt", "name", "integrationWhatsappId", "sourceId", "language", "category", "status", "components")
                   VALUES (9, 1, 'hello', 101, 'src-9', 'en', 'MARKETING', 'APPROVED',
                           '[{"type":"HEADER","format":"IMAGE"},{"type":"BODY","text":"Hi {{1}}"}]'),
                          (10, 2, 'bye', 101, 'src-10', 'en', 'UTILITY', 'REJECTED', '[]'),
                          (20, 3, 'other', 201, 'src-20', 'en', 'UTILITY', 'APPROVED', '[]')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

#[tokio::test]
async fn public_and_deprecated_lists_forward_the_status_filter() {
    let f = fixture().await;
    seed(&f).await;
    for route in ["whatsappTemplates.list", "templateMessages.list"] {
        let (status, body) = call(&f, route, WS, &[], Some("status=APPROVED"), None).await;
        assert_eq!(status, 200, "{body}");
        let rows = body.as_array().unwrap();
        assert_eq!(rows.len(), 1, "{route}");
        assert_eq!(rows[0]["id"], "9");
        assert_eq!(rows[0]["integrationWhatsapp"]["id"], "101");
        assert_eq!(rows[0]["integrationWhatsapp"]["inboxId"], "6");
    }
    let (_, body) = call(&f, "whatsappTemplates.list", WS, &[], None, None).await;
    let ids: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["9", "10"]);
}

#[tokio::test]
async fn get_resolves_the_template_inside_the_workspace_only() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "whatsappTemplates.get", WS, &[("id", "20")], None, None).await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
}

#[tokio::test]
async fn get_lists_the_keys_to_fill_as_parameters() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "whatsappTemplates.get", WS, &[("id", "9")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["parameters"],
        json!([
            { "key": "header", "component": "header", "kind": "image", "required": true },
            { "key": "body.1", "component": "body", "kind": "text", "required": true, "placeholder": "{{1}}" },
        ])
    );
}

async fn graph_templates(
    State(paths): State<Arc<Mutex<Vec<String>>>>,
    uri: axum::http::Uri,
) -> Json<Value> {
    paths.lock().unwrap().push(uri.path().to_owned());
    Json(json!({ "data": [
        { "id": "src-9", "name": "hello2", "language": "en", "category": "MARKETING", "status": "APPROVED", "components": [] },
        { "id": "src-new", "name": "fresh", "language": "de", "category": "UTILITY", "status": "PENDING", "components": [] },
    ], "paging": {} }))
}

#[tokio::test]
async fn sync_pulls_from_meta_for_a_number_of_this_workspace_else_404() {
    let mut f = fixture().await;
    seed(&f).await;
    let paths = Arc::new(Mutex::new(Vec::new()));
    let app = axum::Router::new()
        .route("/v23.0/waba-1/message_templates", get(graph_templates))
        .with_state(Arc::clone(&paths));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.state.graph_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (status, body) = call(
        &f,
        "whatsappTemplates.sync",
        WS,
        &[("id", "101")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(paths.lock().unwrap().len(), 1);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "name" FROM "WhatsappMessageTemplate" WHERE "id" = ?1"#,
            9
        )
        .await,
        Some(json!("hello2"))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "WhatsappMessageTemplate" WHERE "id" = ?1"#,
            10
        )
        .await,
        Some(json!(0))
    );

    let (status, body) = call(
        &f,
        "whatsappTemplates.sync",
        WS,
        &[("id", "201")],
        None,
        None,
    )
    .await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
    assert_eq!(paths.lock().unwrap().len(), 1);
}
