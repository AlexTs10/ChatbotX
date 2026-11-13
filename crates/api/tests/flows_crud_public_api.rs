//! Port of `apps/builder/__tests__/flows-public-api.test.ts`, the
//! `GET /v1/flows`, `GET /v1/flows/{id}`, `GET /v1/flows/{id}/nodes`,
//! `PATCH` / `DELETE /v1/flows/{id}` and `POST /v1/flows/{id}/duplicate`
//! blocks (P3.6-D2). Adapted: the mocked `flowService` /
//! `messengerWelcomeMessageFlowService` become a real migrated database;
//! the asserted service calls become the responses and stored rows. Not
//! applicable: the two mobile-member cases of `GET /v1/flows —
//! enableInInbox` (no mobile member auth, `context.member`, in the Rust API).

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

async fn create(f: &Fixture, ws: i64, body: Value) -> String {
    let (status, out) = call(f, "flows.create", ws, &[], None, Some(body)).await;
    assert_eq!(status, 201, "{out}");
    out["id"].as_str().expect("id").to_string()
}

fn text_spec(name: &str, text: &str) -> Value {
    json!({ "formatVersion": 1, "name": name, "steps": [{ "type": "send", "text": text }] })
}

async fn published(f: &Fixture, ws: i64, name: &str) -> String {
    create(
        f,
        ws,
        json!({ "name": name, "folderId": null, "spec": text_spec(name, "hi"), "publish": true }),
    )
    .await
}

async fn list(f: &Fixture, query: &str) -> Value {
    let (status, out) = call(f, "flows.list", WS, &[], Some(query), None).await;
    assert_eq!(status, 200, "{out}");
    out
}

fn ids(page: &Value) -> Vec<String> {
    page["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|f| f["id"].as_str().expect("id").to_string())
        .collect()
}

async fn set_active(f: &Fixture, id: &str, active: bool) {
    let (status, out) = call(
        f,
        "flows.update",
        WS,
        &[("id", id)],
        None,
        Some(json!({ "active": active })),
    )
    .await;
    assert_eq!(status, 204, "{out}");
}

fn num(id: &str) -> i64 {
    id.parse().expect("numeric id")
}

// describe("GET /v1/flows/{id}/nodes")

#[tokio::test]
async fn lists_the_published_nodes_of_the_flow_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let id = published(&f, WS, "Welcome").await;
    let (status, out) = call(&f, "flows.listNodes", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{out}");
    let data = out["data"].as_array().expect("data");
    assert!(!data.is_empty());
    for node in data {
        let keys: Vec<&str> = node
            .as_object()
            .expect("node")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 4, "{node}");
        assert_eq!(node["flowId"], json!(id));
        assert!(node["nodeId"].is_string());
        assert!(node["label"].is_string());
    }
    assert_eq!(data[0]["nodeType"], json!("sendMessage"));
    let (status, _) = call(&f, "flows.listNodes", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
    // A draft-only flow has no published nodes.
    let draft = create(&f, WS, json!({ "name": "Draft", "folderId": null })).await;
    let (status, out) = call(&f, "flows.listNodes", WS, &[("id", &draft)], None, None).await;
    assert_eq!((status, out), (200, json!({ "data": [] })));
}

#[tokio::test]
async fn tells_the_agent_which_operation_supplies_the_flow_id() {
    let route = route_by_id("flows.listNodes").expect("route");
    assert_eq!((route.method, route.path), ("GET", "/v1/flows/{id}/nodes"));
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["automation"]));
}

#[tokio::test]
async fn surfaces_a_missing_flow_instead_of_returning_an_empty_list() {
    let f = fixture().await;
    let (status, out) = call(&f, "flows.listNodes", WS, &[("id", "404")], None, None).await;
    assert_eq!(status, 404, "{out}");
    assert_eq!(out["code"], json!("notFound"));
    assert_eq!(out["message"], json!("Flow does not exists."));
}

// describe("GET /v1/flows")

#[tokio::test]
async fn get_v1_flows_delegates_to_flow_service_list() {
    let f = fixture().await;
    let first = create(&f, WS, json!({ "name": "First", "folderId": null })).await;
    let second = create(&f, WS, json!({ "name": "Second", "folderId": null })).await;
    let off = create(&f, WS, json!({ "name": "Off", "folderId": null })).await;
    set_active(&f, &off, false).await;
    create(&f, OTHER_WS, json!({ "name": "Foreign", "folderId": null })).await;
    let page = list(&f, "page=1&perPage=50&active=true").await;
    // Active flows of the workspace, newest first.
    assert_eq!(ids(&page), vec![second, first]);
    assert_eq!(page["pageCount"], json!(1));
}

#[tokio::test]
async fn forwards_name_folder_id_and_sort_filters() {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "workspaceId", "name", "folderType", "paths") VALUES (70, ?1, 'F', 'flow', '[]')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("folder");
    let b = create(&f, WS, json!({ "name": "welcome b", "folderId": null })).await;
    let a = create(&f, WS, json!({ "name": "Welcome a", "folderId": null })).await;
    create(
        &f,
        WS,
        json!({ "name": "welcome in folder", "folderId": "70" }),
    )
    .await;
    create(&f, WS, json!({ "name": "other", "folderId": null })).await;
    let page = list(
        &f,
        "page=1&perPage=10&active=true&name=welcome&folderId=0&sort[0][id]=name&sort[0][desc]=false",
    )
    .await;
    assert_eq!(ids(&page), vec![a, b]);
    let page = list(
        &f,
        "page=2&perPage=1&name=welcome&folderId=0&sort[0][id]=name&sort[0][desc]=false",
    )
    .await;
    assert_eq!(page["pageCount"], json!(2));
    assert_eq!(page["data"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn passes_start_type_and_integration_whatsapp_id_to_the_service_with_the_page() {
    let f = fixture().await;
    published(&f, WS, "Text only").await;
    let page = list(
        &f,
        "page=2&perPage=2&active=true&startType=sendWaTemplateMessage",
    )
    .await;
    assert_eq!(page, json!({ "data": [], "pageCount": 0 }));
    let (status, out) = call(
        &f,
        "flows.list",
        WS,
        &[],
        Some("page=2&perPage=2&startType=sendWaTemplateMessage&integrationWhatsappId=42"),
        None,
    )
    .await;
    assert_eq!(status, 404, "{out}");
    assert_eq!(out["message"], json!("WhatsApp channel not found"));
}

#[tokio::test]
async fn returns_id_name_folder_id_and_active_per_flow_not_the_full_resource() {
    let f = fixture().await;
    let id = published(&f, WS, "Flow 1").await;
    let page = list(&f, "page=1&perPage=50&active=true").await;
    assert_eq!(
        page,
        json!({
            "data": [{ "id": id, "name": "Flow 1", "folderId": null, "active": true, "enableInInbox": false }],
            "pageCount": 1,
        })
    );
}

// describe("GET /v1/flows — enableInInbox")

#[tokio::test]
async fn forwards_the_enable_in_inbox_filter() {
    let f = fixture().await;
    let inbox = create(&f, WS, json!({ "name": "Inbox", "folderId": null })).await;
    let not_inbox = published(&f, WS, "Not inbox").await;
    assert_eq!(ids(&list(&f, "enableInInbox=true").await), vec![inbox]);
    assert_eq!(ids(&list(&f, "enableInInbox=false").await), vec![not_inbox]);
}

#[tokio::test]
async fn omitting_the_filter_lists_every_flow_as_before() {
    let f = fixture().await;
    let inbox = create(&f, WS, json!({ "name": "Inbox", "folderId": null })).await;
    let not_inbox = published(&f, WS, "Not inbox").await;
    assert_eq!(
        ids(&list(&f, "page=1&perPage=50").await),
        vec![not_inbox, inbox]
    );
}

#[tokio::test]
async fn returns_enable_in_inbox_on_every_listed_flow() {
    let f = fixture().await;
    published(&f, WS, "Flow 1").await;
    create(&f, WS, json!({ "name": "Flow 2", "folderId": null })).await;
    let page = list(&f, "").await;
    let flags: Vec<Value> = page["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|f| f["enableInInbox"].clone())
        .collect();
    assert_eq!(flags, vec![json!(true), json!(false)]);
}

// describe("GET /v1/flows/{id}")

#[tokio::test]
async fn get_v1_flows_id_delegates_to_flow_service_find_by_id() {
    let f = fixture().await;
    let id = published(&f, WS, "Flow 1").await;
    let (status, out) = call(&f, "flows.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{out}");
    let mut keys: Vec<&str> = out
        .as_object()
        .expect("flow")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "active",
            "createdAt",
            "currentVersionId",
            "draftVersionId",
            "enableInInbox",
            "flowVersions",
            "folderId",
            "id",
            "name",
            "updatedAt",
            "welcomeMessageFlows",
            "workspaceId",
        ]
    );
    assert_eq!(out["id"], json!(id));
    assert_eq!(out["workspaceId"], json!(WS.to_string()));
    assert_eq!(out["flowVersions"].as_array().map(Vec::len), Some(2));
    let mut version_keys: Vec<&str> = out["flowVersions"][0]
        .as_object()
        .expect("version")
        .keys()
        .map(String::as_str)
        .collect();
    version_keys.sort_unstable();
    assert_eq!(
        version_keys,
        vec![
            "createdAt",
            "edges",
            "flowId",
            "id",
            "isDraft",
            "isLatest",
            "nodes",
            "startNodeId",
            "workspaceId"
        ]
    );
    assert_eq!(out["welcomeMessageFlows"], json!([]));
    let (status, out) = call(&f, "flows.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404, "{out}");
}

#[tokio::test]
async fn returns_the_flow_s_current_meta_welcome_message_flows() {
    let f = fixture().await;
    let id = published(&f, WS, "Flow 1").await;
    let flow = num(&id);
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (30, ?1, 'Inbox', 'messenger', 'src-30')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters",
                     "persistentMenus", "personas", "workspaceId", "inboxId")
                   VALUES (3, '{}', 'p', 'Page', '[]', '[]', '[]', ?1, 30)"#,
                params![WS],
            )?;
            for (row, status) in [(1_i64, "current"), (2, "superseded")] {
                tx.execute(
                    r#"INSERT INTO "MessengerWelcomeMessageFlow" ("id", "workspaceId", "flowId", "integrationMessengerId", "pageId", "metaFlowId", "status")
                       VALUES (?1, ?2, ?3, 3, 'p', ?4, ?5)"#,
                    params![row, WS, flow, format!("m{row}"), status],
                )?;
            }
            Ok(())
        })
        .await
        .expect("welcome rows");
    let (status, out) = call(&f, "flows.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        out["welcomeMessageFlows"],
        json!([{ "integrationMessengerId": "3", "pageId": "p", "welcomeMessageFlowId": "m1" }])
    );
}

// describe("PATCH /v1/flows/{id}")

#[tokio::test]
async fn patch_v1_flows_id_delegates_to_flow_service_update() {
    let f = fixture().await;
    let id = create(&f, WS, json!({ "name": "Before", "folderId": null })).await;
    let (status, out) = call(
        &f,
        "flows.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!((status, out), (204, Value::Null));
    assert_eq!(
        scalar(&f, r#"SELECT "name" FROM "Flow" WHERE "id" = ?1"#, num(&id)).await,
        Some(json!("Renamed"))
    );
}

// describe("DELETE /v1/flows/{id}")

#[tokio::test]
async fn delete_v1_flows_id_delegates_to_flow_service_delete_many() {
    let f = fixture().await;
    let id = published(&f, WS, "Doomed").await;
    let (status, out) = call(&f, "flows.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, out), (204, Value::Null));
    assert_eq!(
        scalar(
            &f,
            r#"SELECT COUNT(*) FROM "Flow" WHERE "id" = ?1"#,
            num(&id)
        )
        .await,
        Some(json!(0))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT COUNT(*) FROM "FlowVersion" WHERE "flowId" = ?1"#,
            num(&id)
        )
        .await,
        Some(json!(0))
    );
}

// describe("POST /v1/flows/{id}/duplicate")

#[tokio::test]
async fn post_v1_flows_id_duplicate_delegates_to_flow_service_duplicate() {
    let f = fixture().await;
    let id = published(&f, WS, "Orig").await;
    let (status, out) = call(&f, "flows.duplicate", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 201, "{out}");
    let keys: Vec<&str> = out
        .as_object()
        .expect("body")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["id"]);
    let copy = num(out["id"].as_str().expect("id"));
    assert_eq!(
        scalar(&f, r#"SELECT "name" FROM "Flow" WHERE "id" = ?1"#, copy).await,
        Some(json!("Orig _copy"))
    );
}
