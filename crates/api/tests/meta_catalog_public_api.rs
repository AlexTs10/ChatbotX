//! Ports of the Meta Catalog route tests of
//! `apps/builder/__tests__/products-public-api.test.ts` and the two
//! "meta catalog state" tests of `meta-catalog-operations.test.ts`. Adapted:
//! the captured procedures and mocked services become `handlers::dispatch`
//! calls over real rows; "service called with" becomes the answer.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const CONNECTION: i64 = 501;

async fn connected(f: &Fixture, catalog_id: Option<&'static str>) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (500, ?1, 'metaCatalog')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationMetaCatalog" ("id", "workspaceId", "integrationId", "catalogId", "encryptedAuth")
                   VALUES (?1, ?2, 500, ?3, '{"v":1,"iv":"x","text":"secret","tag":"y"}')"#,
                params![CONNECTION, WS, catalog_id],
            )?;
            for (id, name) in [(1_i64, "Shoe"), (2, "Bag")] {
                tx.execute(
                    r#"INSERT INTO "Product" ("id", "workspaceId", "name", "images") VALUES (?1, ?2, ?3, '[]')"#,
                    params![id, WS, name],
                )?;
                tx.execute(
                    r#"INSERT INTO "MetaCatalogItem" ("id", "integrationMetaCatalogId", "productId", "catalogId", "retailerId") VALUES (?1, ?2, ?3, '9', ?4)"#,
                    params![id + 100, CONNECTION, id, format!("sku-{id}")],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "MetaCatalogSyncRun" ("id", "workspaceId", "integrationMetaCatalogId", "scope", "status", "handles", "submissionLeaseId")
                   VALUES (700, ?1, ?2, 'all', 'succeeded', '["h1"]', 'lease-1')"#,
                params![WS, CONNECTION],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

fn keys(value: &Value) -> Vec<String> {
    let mut out: Vec<String> = value.as_object().expect("object").keys().cloned().collect();
    out.sort();
    out
}

// ---- meta catalog state ----------------------------------------------------

#[tokio::test]
async fn meta_catalog_state_never_carries_the_encrypted_credential() {
    let f = fixture().await;
    connected(&f, Some("9")).await;
    let (status, body) = call(&f, "products.getMetaCatalog", WS, &[], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["connection"]["id"], CONNECTION.to_string());
    assert!(!body.to_string().contains("secret"));
    assert!(body["connection"].get("encryptedAuth").is_none());
    let (_, other) = call(&f, "products.getMetaCatalog", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "connection": null, "history": [] }));
}

#[tokio::test]
async fn meta_catalog_state_the_public_response_schema_drops_credential_and_bookkeeping_columns() {
    let f = fixture().await;
    connected(&f, Some("9")).await;
    let (_, body) = call(&f, "products.getMetaCatalog", WS, &[], None, None).await;
    let connection = keys(&body["connection"]);
    for dropped in ["encryptedAuth", "integrationId", "workspaceId", "deletedAt"] {
        assert!(!connection.contains(&dropped.to_string()), "{dropped}");
    }
    let run = keys(&body["history"][0]);
    for dropped in ["workspaceId", "handles", "submissionLeaseId"] {
        assert!(!run.contains(&dropped.to_string()), "{dropped}");
    }
    assert!(run.contains(&"integrationMetaCatalogId".to_string()));
}

// ---- DELETE /v1/products/meta-catalog --------------------------------------

#[tokio::test]
async fn delete_v1_products_meta_catalog_disconnects_the_token_workspace_s_catalog() {
    let f = fixture().await;
    connected(&f, Some("9")).await;
    let (status, body) = call(&f, "products.disconnectMetaCatalog", WS, &[], None, None).await;
    assert_eq!((status, body), (204, Value::Null));
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "deletedAt" IS NOT NULL FROM "IntegrationMetaCatalog" WHERE "id" = ?1"#,
            CONNECTION
        )
        .await,
        Some(json!(1))
    );
}

#[tokio::test]
async fn delete_v1_products_meta_catalog_a_running_sync_refusal_is_passed_through_for_the_declared_409()
 {
    let f = fixture().await;
    connected(&f, Some("9")).await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "MetaCatalogSyncRun" SET "status" = 'running'"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("run");
    let (status, body) = call(&f, "products.disconnectMetaCatalog", WS, &[], None, None).await;
    assert_eq!(status, 409);
    assert_eq!(body["code"], "metaCatalogSyncAlreadyRunning");
    assert_eq!(body["defined"], true);
}

// ---- GET /v1/products/meta-catalog/items -----------------------------------

#[tokio::test]
async fn get_v1_products_meta_catalog_items_searches_the_token_workspace_s_catalog_by_keyword_and_returns_ids_an_agent_can_use()
 {
    let f = fixture().await;
    connected(&f, Some("9")).await;
    let (status, body) = call(
        &f,
        "products.searchMetaCatalogItems",
        WS,
        &[],
        Some("keyword=shoe"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({ "connected": true, "items": [{ "retailerId": "sku-1", "name": "Shoe", "imageUrl": null }] })
    );
}

#[tokio::test]
async fn get_v1_products_meta_catalog_items_looks_one_item_up_by_its_exact_retailer_id() {
    let f = fixture().await;
    connected(&f, Some("9")).await;
    let (_, body) = call(
        &f,
        "products.searchMetaCatalogItems",
        WS,
        &[],
        Some("retailerId=sku-2"),
        None,
    )
    .await;
    assert_eq!(
        body["items"],
        json!([{ "retailerId": "sku-2", "name": "Bag", "imageUrl": null }])
    );
}

#[tokio::test]
async fn get_v1_products_meta_catalog_items_reports_a_workspace_without_a_catalog_as_not_connected_instead_of_failing()
 {
    let f = fixture().await;
    connected(&f, None).await;
    let (status, body) = call(&f, "products.searchMetaCatalogItems", WS, &[], None, None).await;
    assert_eq!(
        (status, body),
        (200, json!({ "connected": false, "items": [] }))
    );
}

#[tokio::test]
async fn get_v1_products_meta_catalog_items_never_lets_the_caller_pick_another_workspace() {
    let f = fixture().await;
    connected(&f, Some("9")).await;
    let (status, body) = call(
        &f,
        "products.searchMetaCatalogItems",
        OTHER_WS,
        &[],
        Some(&format!("keyword=shoe&workspaceId={WS}")),
        None,
    )
    .await;
    assert_eq!(
        (status, body),
        (200, json!({ "connected": false, "items": [] }))
    );
}

// ---- input checks (Rust only: the routes' zod schemas) ---------------------

#[tokio::test]
async fn sync_meta_catalog_checks_the_scope_and_the_bound_catalog() {
    let f = fixture().await;
    connected(&f, None).await;
    let (status, body) = call(
        &f,
        "products.syncMetaCatalog",
        WS,
        &[],
        None,
        Some(json!({ "scope": "selected", "selectedProductIds": [] })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["data"]["issues"][0]["path"], json!(["scope"]));
    let (status, body) = call(
        &f,
        "products.syncMetaCatalog",
        WS,
        &[],
        None,
        Some(json!({ "scope": "all" })),
    )
    .await;
    assert_eq!(
        (status, &body["code"]),
        (422, &json!("validation")),
        "{body}"
    );
    let (status, _) = call(
        &f,
        "products.selectMetaCatalog",
        WS,
        &[],
        None,
        Some(json!({ "catalogId": "x1" })),
    )
    .await;
    assert_eq!(status, 422);
}
