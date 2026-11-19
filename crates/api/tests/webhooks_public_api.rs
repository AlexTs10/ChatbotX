//! Port of `apps/builder/__tests__/webhooks-public-api.test.ts` (Q.D13a):
//! the TS test checks that each `/v1/webhooks` route delegates to
//! `webhookService` with the token workspace; here the routes run on a
//! migrated database and the responses and rows are checked. Not
//! applicable: the router scope / metadata asserts (the route table test).

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

#[tokio::test]
async fn webhooks_crud_is_scoped_to_the_token_workspace() {
    let fx = fixture().await;
    let conditions = json!([{ "type": "newContact" }, { "type": "tagApplied", "sourceId": "1" }]);
    let (status, created) = call(
        &fx,
        "webhooks.create",
        WS,
        &[],
        None,
        Some(json!({
            "name": "n8n trigger",
            "url": "https://1.1.1.1/webhook/abc",
            "conditions": conditions,
        })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("id").to_owned();

    let (status, list) = call(
        &fx,
        "webhooks.list",
        WS,
        &[],
        Some("page=1&perPage=50"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["data"][0]["id"], id.as_str(), "{list}");
    assert_eq!(list["pageCount"], 1);
    let (_, other) = call(&fx, "webhooks.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other["data"], json!([]), "{other}");

    // GET reads the webhook with its conditions in the token workspace.
    let (status, found) = call(&fx, "webhooks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{found}");
    assert_eq!(
        found["conditions"].as_array().map(Vec::len),
        Some(2),
        "{found}"
    );
    let (status, _) = call(&fx, "webhooks.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);

    // PUT replaces url and conditions; 404 outside the workspace.
    let body = json!({ "url": "https://1.1.1.1/new", "conditions": [{ "type": "newContact" }] });
    let (status, updated) = call(
        &fx,
        "webhooks.update",
        WS,
        &[("id", &id)],
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["url"], "https://1.1.1.1/new");
    assert_eq!(updated["conditions"].as_array().map(Vec::len), Some(1));
    let (status, out) = call(
        &fx,
        "webhooks.update",
        OTHER_WS,
        &[("id", &id)],
        None,
        Some(body),
    )
    .await;
    assert_eq!(
        (status, out["code"].clone()),
        (404, json!("notFound")),
        "{out}"
    );

    // PATCH settings changes active only.
    let (status, patched) = call(
        &fx,
        "webhooks.updateSettings",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "active": false })),
    )
    .await;
    assert_eq!(status, 200, "{patched}");
    assert_eq!(patched["active"], false);
    assert_eq!(patched["url"], "https://1.1.1.1/new");
    assert_eq!(patched["conditions"].as_array().map(Vec::len), Some(1));

    // bulk-delete in another workspace leaves the row; DELETE removes it.
    let (status, _) = call(
        &fx,
        "webhooks.deleteMany",
        OTHER_WS,
        &[],
        None,
        Some(json!({ "ids": [id] })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = call(&fx, "webhooks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200);
    let (status, _) = call(&fx, "webhooks.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&fx, "webhooks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn bulk_delete_removes_the_given_ids() {
    let fx = fixture().await;
    let mut ids = Vec::new();
    for name in ["a", "b"] {
        let (_, created) = call(
            &fx,
            "webhooks.create",
            WS,
            &[],
            None,
            Some(json!({ "name": name, "url": "https://1.1.1.1/h", "conditions": [{ "type": "newContact" }] })),
        )
        .await;
        ids.push(created["id"].clone());
    }
    let (status, _) = call(
        &fx,
        "webhooks.deleteMany",
        WS,
        &[],
        None,
        Some(json!({ "ids": ids })),
    )
    .await;
    assert_eq!(status, 204);
    let (_, list) = call(&fx, "webhooks.list", WS, &[], None, None).await;
    assert_eq!(list["data"], json!([]), "{list}");
}
