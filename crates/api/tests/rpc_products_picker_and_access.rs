//! Q.P6-U5: `productsAPI.listProductsAuthorizedAPI` (the product picker,
//! `features/products/api/authorized.ts`) and the
//! `assertCurrentUserCanAccessChatbot` server function (`lib/auth/utils.ts`).
//! No original TS test covers them; these check the ported behaviour.

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_db::rusqlite::params;
use rpc_common::{OUTSIDER_TOKEN, OWNER_TOKEN, WS, call, fixture};
use serde_json::{Value, json};

fn ids(body: &Value) -> Vec<String> {
    body["json"]["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|p| p["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn the_product_picker_lists_by_name_and_by_ids() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            for (id, name) in [(1, "Mug"), (2, "Cup"), (3, "Mug XL")] {
                tx.execute(
                    r#"INSERT INTO "Product" ("id", "workspaceId", "name", "images") VALUES (?1, ?2, ?3, '[]')"#,
                    params![id, WS, name],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    let path = "/rpc/productsAPI/listProductsAuthorizedAPI";
    let ws = WS.to_string();
    let (status, body) = call(
        &fx,
        "GET",
        path,
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws, "name": "mug", "page": 1, "perPage": 10 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut found = ids(&body);
    found.sort();
    assert_eq!(found, ["1", "3"]);
    assert_eq!(body["json"]["pageCount"], 1);
    assert_eq!(body["json"]["data"][0]["category"], Value::Null);

    let (_, body) = call(
        &fx,
        "GET",
        path,
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws, "ids": ["2"], "perPage": 1 }),
    )
    .await;
    assert_eq!(ids(&body), ["2"]);

    // `ids` is 1..=50 ids.
    let (status, _) = call(
        &fx,
        "GET",
        path,
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws, "ids": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Membership (`workspaceAuthorizedMidddleware`).
    let (status, _) = call(
        &fx,
        "GET",
        path,
        Some(OUTSIDER_TOKEN),
        json!({ "workspaceId": ws }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn assert_current_user_can_access_chatbot_needs_a_membership() {
    let fx = fixture().await;
    let path = "/rpc/actions/assertCurrentUserCanAccessChatbot";
    let ws = WS.to_string();
    let (status, body) = call(
        &fx,
        "POST",
        path,
        Some(OWNER_TOKEN),
        json!({ "args": [ws] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["json"], Value::Null);

    for (token, args) in [
        (OUTSIDER_TOKEN, json!([ws])),
        (OWNER_TOKEN, json!(["999"])),
        (OWNER_TOKEN, json!([])),
    ] {
        let (status, body) = call(&fx, "POST", path, Some(token), json!({ "args": args })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            body["json"]["message"],
            "User is not associated with this workspace"
        );
    }
}
