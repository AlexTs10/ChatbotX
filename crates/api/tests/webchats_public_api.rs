//! Port of `apps/builder/__tests__/webchats-public-api.test.ts`.
//! Adapted: the mocked `integrationWebchatService` becomes a real migrated
//! database; the procedures are called through `handlers::dispatch` and the
//! asserted service calls become the stored rows and the responses.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// `baseInput` of the POST test.
fn base_input() -> Value {
    json!({
        "name": "My Webchat",
        "persistentMenus": [],
        "authorizedDomains": ["example.com"],
        "conversationStarters": [],
        "brandColor": "#112233",
        "hideHeader": false,
        "showLogo": true,
        "hideMessageInput": false,
        "enable": true,
    })
}

async fn create(f: &Fixture) -> Value {
    let (status, body) = call(f, "webchats.create", WS, &[], None, Some(base_input())).await;
    assert_eq!(status, 201, "{body}");
    body
}

#[tokio::test]
async fn registers_the_webchats_public_router_under_the_channels_scope() {
    for route_id in [
        "webchats.list",
        "webchats.get",
        "webchats.create",
        "webchats.update",
        "webchats.delete",
    ] {
        let route = route_by_id(route_id).expect("route");
        assert_eq!(
            route.auth,
            RouteAuth::WorkspaceToken(&["channels"]),
            "{route_id}"
        );
    }
}

#[tokio::test]
async fn webchat_public_resource_never_exposes_workspace_id_or_the_auth_credential_blob() {
    let f = fixture().await;
    let created = create(&f).await;
    let object = created.as_object().expect("object");
    assert!(!object.contains_key("workspaceId"));
    assert!(!object.contains_key("auth"));
    assert!(object.contains_key("inboxId"));
}

#[tokio::test]
async fn post_v1_webchats_creates_from_the_already_parsed_request_without_revalidating_it() {
    let f = fixture().await;
    let created = create(&f).await;
    let id = created["id"].as_str().expect("id").to_string();
    // The response is the `findByIdForWorkspace` read of the new row.
    let (status, found) = call(&f, "webchats.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(found, created);
    assert_eq!(created["name"], "My Webchat");
    assert_eq!(created["authorizedDomains"], json!(["example.com"]));
    assert_eq!(created["brandColor"], "#112233");
    assert_eq!(created["customCss"], Value::Null);
    assert_eq!(created["welcomeFlowId"], Value::Null);
    // `auth: {}` and the workspace are stored.
    let id: i64 = id.parse().expect("id");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "auth" FROM "IntegrationWebchat" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!("{}"))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "IntegrationWebchat" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn patch_v1_webchats_id_delegates_supplied_menus_to_the_service_for_branding_and_persistence()
{
    let f = fixture().await;
    let created = create(&f).await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, updated) = call(
        &f,
        "webchats.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "persistentMenus": [] })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    // Community edition: the branding menu is appended to the supplied menus.
    assert_eq!(
        updated["persistentMenus"],
        json!([{
            "label": "⚡ Built with chatbotx.io",
            "type": "url",
            "url": "http://app/?ref=selfhosted&channel=webchat",
        }])
    );
    assert_eq!(updated["name"], created["name"]);
}

#[tokio::test]
async fn patch_v1_webchats_id_leaves_omitted_menus_absent_from_the_service_update() {
    let f = fixture().await;
    let created = create(&f).await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, updated) = call(
        &f,
        "webchats.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["persistentMenus"], created["persistentMenus"]);
    assert_eq!(
        updated["conversationStarters"],
        created["conversationStarters"]
    );
}

#[tokio::test]
async fn update_webchat_public_request_omitting_a_defaulted_boolean_field_leaves_it_undefined_instead_of_resetting_it_to_the_create_time_default()
 {
    let f = fixture().await;
    let mut input = base_input();
    input["hideHeader"] = json!(true);
    input["showLogo"] = json!(false);
    input["hideMessageInput"] = json!(true);
    input["enable"] = json!(false);
    let (status, created) = call(&f, "webchats.create", WS, &[], None, Some(input)).await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("id").to_string();
    let (_, updated) = call(
        &f,
        "webchats.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(updated["hideHeader"], true);
    assert_eq!(updated["showLogo"], false);
    assert_eq!(updated["hideMessageInput"], true);
    assert_eq!(updated["enable"], false);
}

#[tokio::test]
async fn update_webchat_public_request_an_explicitly_supplied_boolean_field_still_parses_through() {
    let f = fixture().await;
    let created = create(&f).await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, updated) = call(
        &f,
        "webchats.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({
            "hideHeader": true,
            "showLogo": false,
            "hideMessageInput": true,
            "enable": false,
        })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["hideHeader"], true);
    assert_eq!(updated["showLogo"], false);
    assert_eq!(updated["hideMessageInput"], true);
    assert_eq!(updated["enable"], false);
}

#[tokio::test]
async fn delete_v1_webchats_id_delegates_to_integration_webchat_service_delete() {
    let f = fixture().await;
    let created = create(&f).await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, body) = call(&f, "webchats.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204, "{body}");
    let (status, _) = call(&f, "webchats.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
    // The connection of the webchat is disconnected.
    let inbox_id: i64 = created["inboxId"]
        .as_str()
        .expect("inboxId")
        .parse()
        .expect("id");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "status" FROM "Connection" WHERE "inboxId" = ?1"#,
            inbox_id
        )
        .await,
        Some(json!("disconnected"))
    );
}
