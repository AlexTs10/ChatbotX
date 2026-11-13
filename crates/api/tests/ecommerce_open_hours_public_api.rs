//! Port of `apps/builder/__tests__/ecommerce-open-hours-public-api.test.ts`.
//! Adapted: the captured oRPC handlers become `handlers::dispatch` calls over a
//! real migrated database; "the service was called with" becomes the stored rows.

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

fn ranges() -> Value {
    json!([{ "day": 1, "from": "09:00", "to": "17:00" }])
}

async fn create(f: &handlers_common::Fixture, workspace_id: i64, name: &str) -> Value {
    let (status, body) = call(
        f,
        "openHours.create",
        workspace_id,
        &[],
        None,
        Some(json!({ "name": name, "ranges": ranges() })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

#[tokio::test]
async fn registers_the_open_hours_public_router_under_the_ecommerce_scope() {
    assert_eq!(
        scopes_of_module("features/ecommerce-open-hours/api/public"),
        vec!["ecommerce"]
    );
}

#[tokio::test]
async fn open_hours_public_api_get_v1_open_hours_lists_the_token_s_schedules() {
    let f = fixture().await;
    let weekdays = create(&f, WS, "Weekdays").await;
    create(&f, OTHER_WS, "Other").await;
    let (status, body) = call(&f, "openHours.list", WS, &[], None, None).await;
    assert_eq!((status, body), (200, json!({ "data": [weekdays] })));
}

#[tokio::test]
async fn open_hours_public_api_post_v1_open_hours_creates_in_the_token_s_workspace() {
    let f = fixture().await;
    let created = create(&f, WS, "Weekdays").await;
    assert_eq!(
        (created["name"].clone(), created["ranges"].clone()),
        (json!("Weekdays"), ranges())
    );
    let (_, other) = call(&f, "openHours.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [] }));
}

#[tokio::test]
async fn open_hours_public_api_patch_v1_open_hours_id_changes_only_what_it_is_sent() {
    let f = fixture().await;
    let created = create(&f, WS, "Weekdays").await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, body) = call(
        &f,
        "openHours.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Weekend" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "id": id, "name": "Weekend", "ranges": ranges() })
    );
}

#[tokio::test]
async fn open_hours_public_api_delete_v1_open_hours_id_deletes_in_the_token_s_workspace() {
    let f = fixture().await;
    let created = create(&f, WS, "Weekdays").await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, _) = call(&f, "openHours.delete", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
    let (status, body) = call(&f, "openHours.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, body), (204, Value::Null));
    let (_, list) = call(&f, "openHours.list", WS, &[], None, None).await;
    assert_eq!(list, json!({ "data": [] }));
}
