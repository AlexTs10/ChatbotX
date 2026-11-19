//! Port of the WhatsApp cases of `apps/builder/__tests__/mini-apps-public-api.test.ts`
//! ("a WhatsApp draft is pushed without publishing, then published, previewed
//! and deleted", "delete removes the WhatsApp Flows only when asked"). Adapted:
//! the mocked publication functions become the real ones against a local Graph
//! server (`ApiState.graph_url`) that holds one Flow.

mod handlers_common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const FLOW: &str = "F1";

/// The Flow's status on "Meta" (`None` = gone) and the calls made.
#[derive(Default)]
struct Meta {
    status: Option<String>,
    /// `/publish` answers with Meta's business-verification refusal.
    refuse_publish: bool,
    calls: Vec<String>,
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, body.to_string()).into_response()
}

async fn graph(meta: Arc<Mutex<Meta>>, request: Request<Body>) -> Response {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let method = request.method().to_string();
    let mut m = meta.lock().unwrap();
    m.calls.push(format!("{method} {path}"));
    let flow = |status: &str| json!({ "id": FLOW, "name": "Survey", "status": status, "categories": ["OTHER"], "validation_errors": [] });
    match (method.as_str(), path.as_str()) {
        ("POST", "/v23.0/waba-1/flows") => {
            m.status = Some("DRAFT".into());
            reply(
                StatusCode::OK,
                json!({ "id": FLOW, "success": true, "validation_errors": [] }),
            )
        }
        ("GET", "/v23.0/F1") if query.contains("preview") => reply(
            StatusCode::OK,
            json!({ "id": FLOW, "preview": { "preview_url": "https://business.facebook.com/wa/manage/flows/F1/preview", "expires_at": null } }),
        ),
        ("GET", "/v23.0/F1") => match m.status.clone() {
            Some(status) => reply(StatusCode::OK, flow(&status)),
            None => reply(
                StatusCode::BAD_REQUEST,
                json!({ "error": { "code": 100, "message": "Object with ID 'F1' does not exist" } }),
            ),
        },
        ("POST", "/v23.0/F1/publish") if m.refuse_publish => reply(
            StatusCode::BAD_REQUEST,
            json!({ "error": { "code": 139002, "message": "Publishing failed", "error_user_msg": "Verify your business before publishing." } }),
        ),
        ("POST", "/v23.0/F1/publish") => {
            m.status = Some("PUBLISHED".into());
            reply(StatusCode::OK, json!({ "success": true }))
        }
        ("POST", "/v23.0/F1/deprecate") => {
            m.status = Some("DEPRECATED".into());
            reply(StatusCode::OK, json!({ "success": true }))
        }
        ("DELETE", "/v23.0/F1") if m.status.as_deref() == Some("DRAFT") => {
            m.status = None;
            reply(StatusCode::OK, json!({ "success": true }))
        }
        ("DELETE", "/v23.0/F1") => reply(
            StatusCode::BAD_REQUEST,
            json!({ "error": { "code": 139004, "message": "Can't delete published Flow" } }),
        ),
        _ => reply(StatusCode::NOT_FOUND, json!({})),
    }
}

/// A fixture with WhatsApp number 72 (WS, WABA `waba-1`), number 82
/// (`OTHER_WS`), a Graph server and Mini App "Survey"; returns its id.
async fn seeded() -> (Fixture, Arc<Mutex<Meta>>, String) {
    let mut f = fixture().await;
    let meta = Arc::new(Mutex::new(Meta::default()));
    let state = Arc::clone(&meta);
    let app =
        Router::new().fallback(move |request: Request<Body>| graph(Arc::clone(&state), request));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.state.graph_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    f.state
        .db
        .write(|tx| {
            let auth = json!({ "tokens": { "accessToken": "tok" }, "metadata": { "wabaId": "waba-1" } }).to_string();
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
                   VALUES (71, 'Number', 'whatsapp', 's71', ?1), (81, 'Far', 'whatsapp', 's81', ?2)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (72, ?1, 'p', 'waba-1', 'b', 'Number', ?2, 71), (82, ?1, 'p2', 'waba-2', 'b', 'Far', ?3, 81)"#,
                params![auth, WS, OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let (status, created) = call(
        &f,
        "miniApps.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Survey" })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    (f, meta, id)
}

async fn run(f: &Fixture, route: &str, id: &str, number: &str) -> (u16, Value) {
    match route {
        "miniApps.getWhatsappPreview" => {
            let query = format!("integrationWhatsappId={number}");
            call(f, route, WS, &[("id", id)], Some(&query), None).await
        }
        "miniApps.deleteWhatsappDraft" => {
            call(
                f,
                route,
                WS,
                &[("id", id), ("integrationWhatsappId", number)],
                None,
                None,
            )
            .await
        }
        _ => {
            call(
                f,
                route,
                WS,
                &[("id", id)],
                None,
                Some(json!({ "integrationWhatsappId": number })),
            )
            .await
        }
    }
}

async fn publication_status(f: &Fixture, id: &str) -> Option<Value> {
    scalar(
        f,
        r#"SELECT "status" FROM "MiniAppPublication" WHERE "miniAppId" = ?1"#,
        id.parse().unwrap(),
    )
    .await
}

#[tokio::test]
async fn a_whatsapp_draft_is_pushed_without_publishing_then_published_previewed_and_refused_delete()
{
    let (f, meta, id) = seeded().await;
    let (status, draft) = run(&f, "miniApps.pushWhatsappDraft", &id, "72").await;
    assert_eq!(status, 200, "{draft}");
    assert_eq!(
        (&draft["status"], &draft["published"]),
        (&json!("DRAFT"), &json!(false))
    );
    assert!(draft["whatsappFlowId"].is_string(), "{draft}");
    assert_eq!(publication_status(&f, &id).await, Some(json!("DRAFT")));

    let (status, preview) = run(&f, "miniApps.getWhatsappPreview", &id, "72").await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(
        preview,
        json!({ "previewUrl": "https://business.facebook.com/wa/manage/flows/F1/preview", "expiresAt": null })
    );

    let (status, published) = run(&f, "miniApps.publishWhatsappDraft", &id, "72").await;
    assert_eq!(status, 200, "{published}");
    assert_eq!(
        published,
        json!({ "status": "PUBLISHED", "published": true, "whatsappFlowId": draft["whatsappFlowId"], "validationErrors": [] })
    );

    // Live on Meta: a draft save and a draft delete are refused, nothing changes.
    let (status, refused) = run(&f, "miniApps.pushWhatsappDraft", &id, "72").await;
    assert_eq!(status, 422, "{refused}");
    let (status, refused) = run(&f, "miniApps.deleteWhatsappDraft", &id, "72").await;
    assert_eq!(status, 422, "{refused}");
    assert!(
        refused.to_string().contains("Only a draft can be deleted"),
        "{refused}"
    );
    assert_eq!(publication_status(&f, &id).await, Some(json!("PUBLISHED")));
    let calls = meta.lock().unwrap().calls.clone();
    assert!(!calls.iter().any(|c| c.starts_with("DELETE")), "{calls:?}");

    // Deleting the Mini App with its Flows deprecates the live Flow.
    let (status, out) = call(
        &f,
        "miniApps.delete",
        WS,
        &[("id", id.as_str())],
        None,
        Some(json!({ "deleteWhatsappFlows": true })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        out,
        json!({ "deletedCount": 1, "whatsappFlows": [{ "miniAppId": id, "integrationWhatsappId": "72", "flowId": FLOW, "outcome": "deprecated" }] })
    );
    assert_eq!(meta.lock().unwrap().status.as_deref(), Some("DEPRECATED"));
}

#[tokio::test]
async fn a_draft_is_deleted_on_meta_and_the_publication_dropped() {
    let (f, meta, id) = seeded().await;
    assert_eq!(
        run(&f, "miniApps.pushWhatsappDraft", &id, "72").await.0,
        200
    );
    let (status, out) = run(&f, "miniApps.deleteWhatsappDraft", &id, "72").await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out, json!({ "outcome": "deleted" }));
    assert_eq!(publication_status(&f, &id).await, None);
    assert_eq!(meta.lock().unwrap().status, None);
    let flows = scalar(
        &f,
        r#"SELECT COUNT(*) FROM "WhatsappFlow" WHERE "integrationWhatsappId" = ?1"#,
        72,
    )
    .await;
    assert_eq!(flows, Some(json!(0)));
}

#[tokio::test]
async fn another_workspaces_number_or_a_mini_app_not_on_it_is_not_found_before_meta() {
    let (f, meta, id) = seeded().await;
    let (status, out) = run(&f, "miniApps.publishWhatsapp", &id, "82").await;
    assert_eq!(status, 404, "{out}");
    assert!(
        out.to_string().contains("WhatsApp number not found"),
        "{out}"
    );
    for route in [
        "miniApps.getWhatsappPreview",
        "miniApps.publishWhatsappDraft",
        "miniApps.deleteWhatsappDraft",
    ] {
        let (status, out) = run(&f, route, &id, "72").await;
        assert_eq!(status, 404, "{route}: {out}");
        assert!(
            out.to_string().contains("not on this WhatsApp number"),
            "{out}"
        );
    }
    assert!(meta.lock().unwrap().calls.is_empty());
}

/// Port of "a refused publish records the draft and surfaces Meta's reason"
/// (`mini-app-contact-variables.test.ts`).
#[tokio::test]
async fn a_refused_publish_records_the_draft_and_surfaces_metas_reason() {
    let (f, meta, id) = seeded().await;
    meta.lock().unwrap().refuse_publish = true;
    let (status, out) = run(&f, "miniApps.publishWhatsapp", &id, "72").await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["status"], "DRAFT");
    assert_eq!(out["published"], false);
    assert_eq!(
        out["validationErrors"],
        json!([{ "message": "Verify your business before publishing." }])
    );
    assert_eq!(publication_status(&f, &id).await, Some(json!("DRAFT")));
}

/// Port of "reports Flows of a number that left the workspace as failed"
/// (`delete-mini-apps-whatsapp-flows.test.ts`): the Mini App is still deleted.
#[tokio::test]
async fn a_flow_on_a_number_that_left_the_workspace_is_reported_failed() {
    let (f, meta, id) = seeded().await;
    assert_eq!(
        run(&f, "miniApps.pushWhatsappDraft", &id, "72").await.0,
        200
    );
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "IntegrationWhatsapp" SET "workspaceId" = ?1 WHERE "id" = 72"#,
                params![OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("move the number");
    let calls_before = meta.lock().unwrap().calls.len();
    let body = json!({ "ids": [id], "deleteWhatsappFlows": true });
    let (status, out) = call(&f, "miniApps.deleteMany", WS, &[], None, Some(body)).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["deletedCount"], 1);
    assert_eq!(out["whatsappFlows"][0]["outcome"], "failed");
    assert_eq!(
        out["whatsappFlows"][0]["error"],
        "WhatsApp number is no longer in this workspace"
    );
    assert_eq!(meta.lock().unwrap().calls.len(), calls_before);
}
