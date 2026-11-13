//! Port of the `productMessageService.listSendable` tests of
//! `packages/business/__tests__/product-message-service.test.ts`. Adapted:
//! the mocked connection lookup and catalog query become real
//! `IntegrationMetaCatalog` / `MetaCatalogItem` rows.

mod common;

use chatbotx_business::product_message::service::{ListSendable, list_sendable};
use chatbotx_db::rusqlite::{Connection, params};
use common::{WS, setup, two_workspaces};
use serde_json::json;

fn connection(tx: &Connection, catalog_id: Option<&str>) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (500, ?1, 'metaCatalog')"#,
        params![WS],
    )?;
    tx.execute(
        r#"INSERT INTO "IntegrationMetaCatalog" ("id", "workspaceId", "integrationId", "catalogId") VALUES (501, ?1, 500, ?2)"#,
        params![WS, catalog_id],
    )?;
    Ok(())
}

fn linked_product(tx: &Connection, id: i64, name: &str) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "Product" ("id", "workspaceId", "name", "price", "images") VALUES (?1, ?2, ?3, 12.5, ?4)"#,
        params![id, WS, name, format!(r#"[{{"url":"https://img/{id}.png","type":"link"}}]"#)],
    )?;
    tx.execute(
        r#"INSERT INTO "MetaCatalogItem" ("id", "integrationMetaCatalogId", "productId", "catalogId", "retailerId") VALUES (?1, 501, ?2, 'cat-1', ?3)"#,
        params![id + 1000, id, format!("sku-{id}")],
    )?;
    Ok(())
}

fn input(channel: &str) -> ListSendable {
    ListSendable {
        workspace_id: WS,
        channel: channel.to_string(),
        ..ListSendable::default()
    }
}

#[tokio::test]
async fn product_message_service_list_sendable_lists_the_connection_s_sendable_products_for_a_supporting_channel()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        connection(tx, Some("cat-1"))?;
        linked_product(tx, 1, "Shoe 1")?;
        linked_product(tx, 2, "Shoe 2")?;
        linked_product(tx, 3, "Shoe 3")?;
        linked_product(tx, 4, "Bag")
    })
    .await;
    let result = list_sendable(
        &db,
        ListSendable {
            keyword: Some("shoe".into()),
            page: Some(2),
            per_page: Some(2),
            ..input("whatsapp")
        },
    )
    .await
    .expect("list");
    assert_eq!(
        (result.ready, result.reason, result.page_count),
        (true, None, 2)
    );
    assert_eq!(
        result.data,
        vec![
            json!({ "id": "3", "name": "Shoe 3", "imageUrl": "https://img/3.png", "price": 12.5, "currency": "USD" })
        ]
    );
}

#[tokio::test]
async fn product_message_service_list_sendable_is_not_ready_without_a_catalog_connection() {
    let db = two_workspaces().await;
    let result = list_sendable(&db, input("whatsapp")).await.expect("list");
    assert_eq!(
        (
            result.ready,
            result.reason,
            result.data.len(),
            result.page_count
        ),
        (false, Some("catalogNotConnected"), 0, 0)
    );
}

#[tokio::test]
async fn product_message_service_list_sendable_is_not_ready_when_the_connection_is_not_bound_to_a_catalog_yet()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| connection(tx, None)).await;
    let result = list_sendable(&db, input("whatsapp")).await.expect("list");
    assert_eq!(result.reason, Some("catalogNotConnected"));
}

#[tokio::test]
async fn product_message_service_list_sendable_does_not_touch_the_catalog_for_channel_s() {
    let db = two_workspaces().await;
    setup(&db, |tx| connection(tx, Some("cat-1"))).await;
    for channel in ["telegram", "omnichannel", "unknown"] {
        let result = list_sendable(&db, input(channel)).await.expect("list");
        assert_eq!(
            (
                result.ready,
                result.reason,
                result.data.len(),
                result.page_count
            ),
            (false, Some("channelUnsupported"), 0, 0),
            "{channel}"
        );
    }
}
