//! Port of `apps/builder/__tests__/mini-apps-public-api.test.ts`. Adapted: the
//! mocked services become `handlers::dispatch` calls over a real migrated
//! database; "the service was called with" becomes the stored rows / the
//! response; the token tests (scope / read_only) check the route table entry
//! the P3.5 auth runs on. The WhatsApp draft / publish / preview test is in
//! `mini_apps_whatsapp_public_api.rs` (local Graph server).

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id, scopes_of_module};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

fn flow_json() -> Value {
    json!({ "version": "7.3", "screens": [{ "id": "WELCOME", "title": "Welcome", "terminal": true, "layout": {
        "type": "SingleColumnLayout", "children": [
            { "type": "TextInput", "name": "full_name", "label": "Full name" },
            { "type": "Footer", "label": "Done", "on-click-action": { "name": "complete", "payload": {} } }
        ] } }] })
}

async fn create(f: &Fixture, ws: i64, name: &str) -> Value {
    let body = json!({ "name": name, "flowJson": flow_json() });
    let (status, created) = call(f, "miniApps.create", ws, &[], None, Some(body)).await;
    assert_eq!(status, 201, "{created}");
    created
}

fn id_of(app: &Value) -> String {
    app["id"].as_str().expect("id").to_owned()
}

async fn count(f: &Fixture, ws: i64) -> i64 {
    scalar(
        f,
        r#"SELECT COUNT(*) FROM "MiniApp" WHERE "workspaceId" = ?1"#,
        ws,
    )
    .await
    .and_then(|v| v.as_i64())
    .unwrap_or_default()
}

#[tokio::test]
async fn a_token_without_the_mini_apps_scope_is_denied() {
    assert_eq!(
        scopes_of_module("features/mini-apps/api/public"),
        vec!["mini-apps"]
    );
    let route = route_by_id("miniApps.list").expect("route");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["mini-apps"]));
}

#[tokio::test]
async fn list_maps_the_name_filter_and_scopes_to_the_tokens_workspace() {
    let f = fixture().await;
    let survey = create(&f, WS, "Survey").await;
    create(&f, WS, "Other").await;
    create(&f, OTHER_WS, "Survey elsewhere").await;
    let (status, list) = call(&f, "miniApps.list", WS, &[], Some("name=Sur"), None).await;
    assert_eq!(status, 200, "{list}");
    let data = list["data"].as_array().expect("data");
    assert_eq!(data.len(), 1, "{list}");
    assert_eq!(data[0]["id"], survey["id"]);
    let url = data[0]["publicUrl"].as_str().unwrap_or_default();
    assert!(
        url.contains(&format!("/mini-apps?id={}&token=", id_of(&survey))),
        "{url}"
    );
    assert_eq!(list["pageCount"], 1);
}

#[tokio::test]
async fn create_passes_the_flow_json_to_the_service_and_returns_validation() {
    let f = fixture().await;
    let body = json!({ "name": "Survey", "flowJson": flow_json(), "customFieldMappings": { "full_name": "77" } });
    let (status, created) = call(&f, "miniApps.create", WS, &[], None, Some(body)).await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["name"], "Survey");
    assert_eq!(created["validation"]["valid"], true, "{created}");
    assert_eq!(created["customFieldMappings"], json!({ "full_name": "77" }));
    assert_eq!(created["flowJson"]["screens"][0]["id"], "WELCOME");
}

#[tokio::test]
async fn create_without_flow_json_starts_from_the_builders_starter_screen() {
    let f = fixture().await;
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
    let stored = scalar(
        &f,
        r#"SELECT "definition" FROM "MiniApp" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .and_then(|v| v.as_str().map(str::to_owned))
    .expect("definition");
    let stored: Value = serde_json::from_str(&stored).expect("json");
    assert_eq!(
        without_ids(stored),
        without_ids(chatbotx_core::mini_app::starter_definition().to_value())
    );
}

/// The definition without its random screen keys and node ids.
fn without_ids(mut definition: Value) -> Value {
    for screen in definition["screens"].as_array_mut().into_iter().flatten() {
        if let Some(s) = screen.as_object_mut() {
            s.remove("key");
        }
        for node in screen["children"].as_array_mut().into_iter().flatten() {
            if let Some(n) = node.as_object_mut() {
                n.remove("id");
            }
        }
    }
    definition
}

#[tokio::test]
async fn validate_reports_issues_with_flow_json_paths() {
    let f = fixture().await;
    let mut broken = flow_json();
    broken["screens"][0]["layout"]["children"][0]["label"] =
        json!("This label is far too long for Meta");
    let (status, result) = call(
        &f,
        "miniApps.validate",
        WS,
        &[],
        None,
        Some(json!({ "flowJson": broken })),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["valid"], false);
    assert_eq!(result["issues"][0]["code"], "property_too_long", "{result}");
    assert_eq!(result["issues"][0]["path"], "screens[0].layout.children[0]");
}

#[tokio::test]
async fn a_read_only_token_cannot_create_or_publish() {
    for (id, method) in [
        ("miniApps.create", "POST"),
        ("miniApps.publishWhatsapp", "POST"),
    ] {
        let route = route_by_id(id).expect("route");
        assert_eq!(
            (route.method, route.auth),
            (method, RouteAuth::WorkspaceToken(&["mini-apps"]))
        );
    }
}

#[tokio::test]
async fn update_hands_the_flow_json_and_mappings_to_the_service_then_reloads() {
    let f = fixture().await;
    let body = json!({ "name": "Survey", "flowJson": flow_json(), "customFieldMappings": { "full_name": "77" } });
    let (_, created) = call(&f, "miniApps.create", WS, &[], None, Some(body)).await;
    let id = id_of(&created);
    let update = json!({ "flowJson": flow_json(), "customFieldMappings": { "full_name": null } });
    let (status, updated) = call(
        &f,
        "miniApps.update",
        WS,
        &[("id", id.as_str())],
        None,
        Some(update),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["name"], "Survey");
    assert_eq!(updated["customFieldMappings"], json!({}));
}

#[tokio::test]
async fn flow_json_returns_only_the_stored_flow_json_of_the_tokens_workspace() {
    let f = fixture().await;
    let id = id_of(&create(&f, WS, "Survey").await);
    let (status, flow) = call(
        &f,
        "miniApps.getFlowJson",
        WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{flow}");
    assert_eq!(flow["screens"][0]["id"], "WELCOME");
    let (status, _) = call(
        &f,
        "miniApps.getFlowJson",
        OTHER_WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn bulk_delete_is_scoped_to_the_tokens_workspace_and_denied_to_read_only_tokens() {
    let f = fixture().await;
    let a = id_of(&create(&f, WS, "A").await);
    let b = id_of(&create(&f, OTHER_WS, "B").await);
    let (status, out) = call(
        &f,
        "miniApps.deleteMany",
        WS,
        &[],
        None,
        Some(json!({ "ids": [a, b] })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["deletedCount"], 1);
    assert_eq!((count(&f, WS).await, count(&f, OTHER_WS).await), (0, 1));
    let route = route_by_id("miniApps.deleteMany").expect("route");
    assert_eq!(route.method, "POST");
}

#[tokio::test]
async fn delete_removes_the_whatsapp_flows_only_when_asked() {
    let f = fixture().await;
    let id = id_of(&create(&f, WS, "Survey").await);
    let (status, out) = call(
        &f,
        "miniApps.delete",
        WS,
        &[("id", id.as_str())],
        Some("deleteWhatsappFlows=true"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out, json!({ "deletedCount": 1, "whatsappFlows": [] }));
    assert_eq!(count(&f, WS).await, 0);
}

#[tokio::test]
async fn submissions_check_the_mini_app_belongs_to_the_tokens_workspace_first() {
    let f = fixture().await;
    let id = id_of(&create(&f, OTHER_WS, "Survey").await);
    let (status, _) = call(
        &f,
        "miniApps.listSubmissions",
        WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, list) = call(
        &f,
        "miniApps.listSubmissions",
        OTHER_WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["data"], json!([]));
}

#[tokio::test]
async fn duplicate_copies_the_mini_app_within_the_tokens_workspace() {
    let f = fixture().await;
    let id = id_of(&create(&f, WS, "Survey").await);
    let (status, copy) = call(
        &f,
        "miniApps.duplicate",
        WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 201, "{copy}");
    assert_eq!(copy["name"], "Survey - Copy");
    assert_ne!(copy["id"], json!(id));
    assert_eq!(count(&f, WS).await, 2);
    let (status, _) = call(
        &f,
        "miniApps.duplicate",
        OTHER_WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

// Parity `50-mini-apps`: `enabled` is applied by update and kept by duplicate.
#[tokio::test]
async fn update_disables_and_a_duplicate_stays_disabled() {
    let f = fixture().await;
    let id = id_of(&create(&f, WS, "Toggle").await);
    let params = [("id", id.as_str())];
    let body = json!({ "enabled": false });
    let (status, updated) = call(&f, "miniApps.update", WS, &params, None, Some(body)).await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["enabled"], false);
    let (status, copy) = call(&f, "miniApps.duplicate", WS, &params, None, Some(json!({}))).await;
    assert_eq!(status, 201, "{copy}");
    assert_eq!(copy["enabled"], false);
    // `possibleErrorsOnMutatingResource`: an unknown id is a defined 404.
    let unknown = [("id", "999999")];
    let (status, error) = call(
        &f,
        "miniApps.duplicate",
        WS,
        &unknown,
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 404, "{error}");
    assert_eq!(error["defined"], true);
}

// Parity `50-mini-apps`: a bad Flow JSON names the failing field.
#[tokio::test]
async fn a_bad_flow_json_reports_the_field_path() {
    let f = fixture().await;
    for (flow, path) in [
        (
            json!({ "version": "7.2", "screens": [] }),
            json!(["flowJson", "screens"]),
        ),
        (json!({ "screens": [] }), json!(["flowJson", "version"])),
        (json!("x"), json!(["flowJson"])),
    ] {
        let body = json!({ "name": "Bad", "flowJson": flow });
        let (status, error) = call(&f, "miniApps.create", WS, &[], None, Some(body)).await;
        assert_eq!(status, 422, "{error}");
        let paths: Vec<&Value> = error["data"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| &issue["path"])
            .collect();
        assert!(paths.contains(&&path), "{error}");
    }
    assert_eq!(count(&f, WS).await, 0);
}
