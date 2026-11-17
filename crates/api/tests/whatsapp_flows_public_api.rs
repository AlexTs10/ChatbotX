//! Port of `apps/builder/__tests__/whatsapp-flows-public-api.test.ts` (Q.D12).
//! Adapted: the mocked `whatsappFlowService` / operations become real rows and a
//! local Graph server (`ApiState.graph_url`).

mod handlers_common;

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::get;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Integration 3 (inbox 6) with Flow 1; integration 4 of the other workspace with Flow 2.
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
                   VALUES (3, '{"tokens":{"accessToken":"secret"},"metadata":{"wabaId":"waba-1"}}', 'p1', 'waba-1', 'b', 'Wa', ?1, 6),
                          (4, '{}', 'p2', 'waba-2', 'b', 'Wa2', ?2, 7)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "WhatsappFlow" ("id", "name", "integrationWhatsappId", "sourceId", "status")
                   VALUES (1, 'Survey', 3, 'meta-1', 'PUBLISHED'), (2, 'Other', 4, 'meta-2', 'DRAFT')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

#[tokio::test]
async fn list_is_scoped_to_the_tokens_workspace_and_drops_the_integration_row() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "whatsappFlows.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "1");
    assert_eq!(rows[0]["completedCount"], "0");
    assert!(rows[0].get("integrationWhatsapp").is_none());
    assert!(!body.to_string().contains("secret"));
    let (_, body) = call(
        &f,
        "whatsappFlows.list",
        WS,
        &[],
        Some("integrationWhatsappId=4"),
        None,
    )
    .await;
    assert_eq!(body, json!([]));
}

fn reply(status: u16, value: Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::from_u16(status).unwrap(),
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

async fn graph(
    State(paths): State<Arc<Mutex<Vec<String>>>>,
    uri: axum::http::Uri,
) -> axum::response::Response {
    paths.lock().unwrap().push(uri.path().to_owned());
    match uri.path() {
        "/v23.0/waba-1/flows" => reply(
            200,
            json!({ "data": [
            { "id": "meta-1", "name": "Survey 2", "status": "PUBLISHED", "categories": ["SURVEY"], "validation_errors": [] },
            { "id": "meta-new", "name": "Fresh", "status": "DRAFT", "categories": [], "validation_errors": [] },
        ] }),
        ),
        // A download URL that is not public is never fetched.
        _ => reply(
            200,
            json!({ "data": [
            { "asset_type": "FLOW_JSON", "download_url": "http://127.0.0.1:1/flow.json" },
        ] }),
        ),
    }
}

async fn start(f: &mut Fixture) -> Arc<Mutex<Vec<String>>> {
    let paths = Arc::new(Mutex::new(Vec::new()));
    let app = axum::Router::new()
        .route("/v23.0/waba-1/flows", get(graph))
        .route("/v23.0/meta-1/assets", get(graph))
        .with_state(Arc::clone(&paths));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.state.graph_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    paths
}

#[tokio::test]
async fn screens_use_the_tokens_workspace_and_a_private_asset_url_reads_as_none() {
    let mut f = fixture().await;
    let paths = start(&mut f).await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "whatsappFlows.getScreens",
        WS,
        &[("flowId", "1")],
        None,
        None,
    )
    .await;
    assert_eq!((status, body), (200, json!({ "screens": [] })));
    assert_eq!(*paths.lock().unwrap(), ["/v23.0/meta-1/assets"]);

    let (status, body) = call(
        &f,
        "whatsappFlows.getScreens",
        WS,
        &[("flowId", "2")],
        None,
        None,
    )
    .await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
}

#[tokio::test]
async fn sync_pulls_the_numbers_flows_and_a_foreign_number_is_a_404() {
    let mut f = fixture().await;
    let paths = start(&mut f).await;
    seed(&f).await;
    let (status, body) = call(&f, "whatsappFlows.sync", WS, &[("id", "3")], None, None).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "name" FROM "WhatsappFlow" WHERE "id" = ?1"#,
            1
        )
        .await,
        Some(json!("Survey 2"))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "WhatsappFlow" WHERE "integrationWhatsappId" = ?1"#,
            3
        )
        .await,
        Some(json!(2))
    );

    let (status, body) = call(&f, "whatsappFlows.sync", WS, &[("id", "4")], None, None).await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
    assert_eq!(paths.lock().unwrap().len(), 1);
}
