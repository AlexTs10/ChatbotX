//! Port of `packages/business/__tests__/product-message-service.test.ts`.
//! Adapted: the mocked connection lookup and catalog query become real
//! `IntegrationMetaCatalog` / `MetaCatalogItem` rows, so the mock call-count
//! checks (`toHaveBeenCalledTimes`, `not.toHaveBeenCalled`) are dropped; a
//! failing lookup is a dropped `MetaCatalogItem` table.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::product_message::service::{
    ListSendable, PreparedProductMessage, list_sendable, prepare_outgoing, resolve_inbound_products,
};
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

// ---- prepareOutgoing ------------------------------------------------------------

/// `step(props)`.
fn step(props: serde_json::Value) -> serde_json::Value {
    let mut step =
        json!({ "id": "1", "stepType": "sendProducts", "display": "products", "sections": [] });
    for (key, value) in props.as_object().unwrap() {
        step[key] = value.clone();
    }
    step
}

/// The `CONNECTION` fixture with linked products `ids`.
async fn catalog_with(ids: &[i64]) -> common::TestDb {
    let db = two_workspaces().await;
    let ids = ids.to_vec();
    setup(&db, move |tx| {
        connection(tx, Some("cat-1"))?;
        for id in &ids {
            linked_product(tx, *id, &format!("Product {id}"))?;
        }
        Ok(())
    })
    .await;
    db
}

async fn prepare(
    db: &chatbotx_db::Db,
    content: serde_json::Value,
    reject: bool,
    channel: &str,
) -> Result<PreparedProductMessage, BusinessError> {
    prepare_outgoing(db, WS, channel, &step(content), reject).await
}

/// `rejects.toMatchObject({ reason, data: { reason } })` plus the code.
fn assert_unresolvable(error: &BusinessError, reason: &str) {
    let BusinessError::Exception {
        code, status, data, ..
    } = error
    else {
        panic!("not an exception: {error}");
    };
    assert_eq!(
        (code.as_str(), *status),
        ("productMessageUnresolvable", 422)
    );
    assert_eq!(data.as_ref().unwrap()["reason"], reason);
}

#[tokio::test]
async fn prepare_outgoing_resolution_resolves_every_section_with_one_catalog_query() {
    let db = catalog_with(&[1, 2, 3]).await;
    let result = prepare(
        &db,
        json!({ "header": "Picks", "body": "Look", "sections": [
            { "title": "A", "productIds": ["1", "2"] },
            { "title": "B", "productIds": ["3"] },
        ] }),
        false,
        "whatsapp",
    )
    .await
    .expect("prepared");
    assert!(result.missing.is_empty());
    let message = &result.message;
    assert_eq!(message["display"], "products");
    assert_eq!(message["sections"][0]["title"], "A");
    assert_eq!(message["sections"][0]["items"][0]["productId"], "1");
    assert_eq!(
        message["sections"][0]["items"][0]["catalogReference"]["retailerId"],
        "sku-1"
    );
    assert_eq!(message["sections"][0]["items"][1]["productId"], "2");
    assert_eq!(message["sections"][1]["title"], "B");
    assert_eq!(message["sections"][1]["items"][0]["productId"], "3");
}

#[tokio::test]
async fn prepare_outgoing_resolution_reports_products_that_are_no_longer_sendable() {
    let db = catalog_with(&[1, 2]).await;
    let content = json!({ "header": "h", "body": "b", "sections": [{ "title": "A", "productIds": ["1", "7", "2"] }] });
    let result = prepare(&db, content, false, "whatsapp")
        .await
        .expect("prepared");
    assert_eq!(result.missing, vec!["7".to_string()]);
}

#[tokio::test]
async fn prepare_outgoing_resolution_fails_permanently_when_nothing_is_sendable() {
    let db = catalog_with(&[]).await;
    let content = json!({ "header": "h", "body": "b", "sections": [{ "title": "A", "productIds": ["7", "8"] }] });
    let error = prepare(&db, content, false, "whatsapp").await.unwrap_err();
    assert_unresolvable(&error, "noProductsSendable");
}

#[tokio::test]
async fn prepare_outgoing_resolution_fails_when_the_workspace_has_no_catalog_and_products_are_needed()
 {
    let db = two_workspaces().await;
    let error = prepare(
        &db,
        json!({ "sections": [{ "productIds": ["1"] }] }),
        false,
        "whatsapp",
    )
    .await
    .unwrap_err();
    assert_unresolvable(&error, "catalogNotConnected");
}

#[tokio::test]
async fn prepare_outgoing_resolution_refuses_channel_s_before_reading_anything() {
    let db = catalog_with(&[1]).await;
    for channel in ["telegram", "omnichannel", "unknown"] {
        let error = prepare(
            &db,
            json!({ "sections": [{ "productIds": ["1"] }] }),
            false,
            channel,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.code(),
            Some("productMessageUnresolvable"),
            "{channel}"
        );
    }
}

#[tokio::test]
async fn prepare_outgoing_returns_the_message_its_text_and_the_entity_to_store() {
    let db = catalog_with(&[1, 2]).await;
    let prepared = prepare(
        &db,
        json!({ "header": "Picks", "body": "Look", "sections": [{ "productIds": ["1", "2"] }] }),
        true,
        "whatsapp",
    )
    .await
    .expect("prepared");
    assert!(prepared.missing.is_empty());
    assert_eq!(prepared.text.as_deref(), Some("Look"));
    assert_eq!(
        prepared.content_attributes,
        json!({ "type": "product_message", "payload": prepared.message })
    );
}

#[tokio::test]
async fn prepare_outgoing_has_no_text_when_the_message_carries_none() {
    let db = catalog_with(&[1]).await;
    let prepared = prepare(
        &db,
        json!({ "display": "products", "sections": [{ "productIds": ["1"] }] }),
        true,
        "whatsapp",
    )
    .await
    .expect("prepared");
    assert_eq!(prepared.text, None);
}

#[tokio::test]
async fn prepare_outgoing_refuses_a_message_that_breaks_the_channel_s_rules_before_any_lookup() {
    let db = catalog_with(&[1, 2]).await;
    let error = prepare(
        &db,
        json!({ "sections": [{ "productIds": ["1", "2"] }] }),
        true,
        "whatsapp",
    )
    .await
    .unwrap_err();
    let BusinessError::Exception {
        code,
        status,
        field,
        data,
        ..
    } = &error
    else {
        panic!("not an exception");
    };
    assert_eq!(
        (code.as_str(), *status, field.as_deref()),
        ("validation", 422, Some("products"))
    );
    assert!(
        data.as_ref().unwrap()["issues"]
            .as_str()
            .unwrap()
            .contains("header")
    );
}

#[tokio::test]
async fn prepare_outgoing_refuses_duplicate_products_as_a_structural_error() {
    let db = catalog_with(&[1]).await;
    let content = json!({ "header": "h", "body": "b", "sections": [{ "title": "A", "productIds": ["1", "1"] }] });
    let error = prepare(&db, content, true, "whatsapp").await.unwrap_err();
    assert_eq!(error.code(), Some("validation"));
}

#[tokio::test]
async fn prepare_outgoing_reports_channel_s_as_unsupported_not_as_invalid() {
    let db = catalog_with(&[1]).await;
    for channel in ["telegram", "omnichannel", "unknown"] {
        let error = prepare(
            &db,
            json!({ "sections": [{ "productIds": ["1", "1"] }] }),
            true,
            channel,
        )
        .await
        .unwrap_err();
        assert_unresolvable(&error, "channelUnsupported");
    }
}

#[tokio::test]
async fn prepare_outgoing_carries_the_reason_as_data_for_each_unresolvable_case() {
    let db = two_workspaces().await;
    let content = || json!({ "body": "b", "sections": [{ "productIds": ["1"] }] });
    let error = prepare(&db, content(), true, "whatsapp").await.unwrap_err();
    assert_unresolvable(&error, "catalogNotConnected");
    assert_eq!(
        error.message(),
        "Product message cannot be sent: catalogNotConnected"
    );

    setup(&db, |tx| connection(tx, Some("cat-1"))).await;
    let error = prepare(&db, content(), true, "whatsapp").await.unwrap_err();
    assert_unresolvable(&error, "noProductsSendable");
}

#[tokio::test]
async fn prepare_outgoing_rejects_when_some_products_are_not_sendable_and_says_which() {
    let db = catalog_with(&[1, 2]).await;
    let content = json!({ "header": "h", "body": "b", "sections": [{ "title": "A", "productIds": ["1", "7", "2", "8"] }] });
    let error = prepare(&db, content, true, "whatsapp").await.unwrap_err();
    assert_unresolvable(&error, "productsNotSendable");
    let BusinessError::Exception { data, .. } = &error else {
        unreachable!()
    };
    assert_eq!(data.as_ref().unwrap()["productIds"], "7,8");
    assert_eq!(
        error.message(),
        "Product message cannot be sent: productsNotSendable (7, 8)"
    );
}

#[tokio::test]
async fn prepare_outgoing_drops_the_missing_products_and_reports_them_when_told_to() {
    let db = catalog_with(&[1, 2]).await;
    let content = json!({ "header": "h", "body": "b", "sections": [{ "title": "A", "productIds": ["1", "7", "2"] }] });
    let prepared = prepare(&db, content, false, "whatsapp")
        .await
        .expect("prepared");
    assert_eq!(prepared.missing, vec!["7".to_string()]);
    assert_eq!(prepared.message["display"], "products");
}

#[tokio::test]
async fn prepare_outgoing_a_carousel_left_below_its_minimum_fails_instead_of_sending_one_card() {
    let db = catalog_with(&[1]).await;
    let content = json!({ "display": "carousel", "body": "Look", "sections": [{ "productIds": ["1", "2"] }] });
    let error = prepare(&db, content, false, "whatsapp").await.unwrap_err();
    assert_unresolvable(&error, "noProductsSendable");
}

// ---- resolveInboundProducts -------------------------------------------------------

fn retailer_ids() -> Vec<String> {
    ["sku-1", "sku-9", "sku-1"].map(String::from).to_vec()
}

fn unresolved(catalog_id: &str, retailer_id: &str) -> serde_json::Value {
    json!({ "catalogReference": { "source": "metaCatalog", "catalogId": catalog_id, "retailerId": retailer_id }, "product": null })
}

#[tokio::test]
async fn resolve_inbound_products_names_linked_products_once_keeps_unknown_ids_unresolved_in_first_seen_order()
 {
    let db = catalog_with(&[1]).await;
    let result = resolve_inbound_products(&db, WS, "whatsapp", "cat-1", &retailer_ids()).await;
    assert_eq!(
        result,
        vec![
            json!({
                "catalogReference": { "source": "metaCatalog", "catalogId": "cat-1", "retailerId": "sku-1" },
                "product": { "productId": "1", "name": "Product 1", "imageUrl": "https://img/1.png", "price": 12.5, "currency": "USD" },
            }),
            unresolved("cat-1", "sku-9"),
        ]
    );
}

#[tokio::test]
async fn resolve_inbound_products_reports_everything_unresolved_without_querying_products_when_no_catalog_is_connected()
 {
    let db = two_workspaces().await;
    let result = resolve_inbound_products(&db, WS, "whatsapp", "cat-1", &retailer_ids()).await;
    assert_eq!(
        result,
        vec![unresolved("cat-1", "sku-1"), unresolved("cat-1", "sku-9")]
    );
}

#[tokio::test]
async fn resolve_inbound_products_never_matches_against_a_catalog_other_than_the_connected_one() {
    let db = catalog_with(&[1]).await;
    let result = resolve_inbound_products(&db, WS, "whatsapp", "cat-other", &retailer_ids()).await;
    assert!(result.iter().all(|r| r["product"].is_null()));
    assert_eq!(result[0]["catalogReference"]["catalogId"], "cat-other");
}

#[tokio::test]
async fn resolve_inbound_products_still_answers_unresolved_when_the_product_lookup_fails() {
    let db = catalog_with(&[1]).await;
    setup(&db, |tx| {
        tx.execute(r#"DROP TABLE "MetaCatalogItem""#, [])?;
        Ok(())
    })
    .await;
    let result = resolve_inbound_products(&db, WS, "whatsapp", "cat-1", &retailer_ids()).await;
    assert_eq!(
        result,
        vec![unresolved("cat-1", "sku-1"), unresolved("cat-1", "sku-9")]
    );
}

#[tokio::test]
async fn resolve_inbound_products_has_nothing_to_name_for_a_channel_without_a_catalog_source() {
    let db = catalog_with(&[1]).await;
    assert!(
        resolve_inbound_products(&db, WS, "telegram", "cat-1", &retailer_ids())
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn resolve_inbound_products_skips_every_lookup_when_the_contact_referenced_no_products() {
    let db = catalog_with(&[1]).await;
    assert!(
        resolve_inbound_products(&db, WS, "whatsapp", "cat-1", &[])
            .await
            .is_empty()
    );
}
