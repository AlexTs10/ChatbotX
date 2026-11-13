//! Port of `packages/business/src/ecommerce-settings/__tests__/ecommerce-settings-service.test.ts`
//! and `apps/builder/__tests__/ecommerce-settings-legacy-keys.test.ts`.
//! Adapted: the mocked repositories become a real migrated database (the
//! asserted `upsert` values are the stored columns; "not called" = the row
//! is unchanged); a stored row is set up with SQL.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::ecommerce_settings::{self, defaults, parse_patch};
use common::{OTHER_WS, WS, flow, open_hours, scalar_text, setup, two_workspaces};
use serde_json::{Value, json};

async fn update(db: &chatbotx_db::Db, patch: Value) -> Result<Value, BusinessError> {
    ecommerce_settings::update(db, WS, patch).await
}

fn field_of(error: &BusinessError) -> Option<String> {
    match error {
        BusinessError::Exception { field, .. } => field.clone(),
        BusinessError::Repo(_) => None,
    }
}

fn data_of(error: &BusinessError) -> Option<Value> {
    match error {
        BusinessError::Exception { data, .. } => data.clone(),
        BusinessError::Repo(_) => None,
    }
}

async fn stored(db: &chatbotx_db::Db, column: &'static str) -> Option<String> {
    let sql: &'static str = match column {
        "currency" => r#"SELECT "currency" FROM "EcommerceSettings" WHERE "workspaceId" = ?1"#,
        "paymentMethods" => {
            r#"SELECT "paymentMethods" FROM "EcommerceSettings" WHERE "workspaceId" = ?1"#
        }
        "checkoutMode" => {
            r#"SELECT "checkoutMode" FROM "EcommerceSettings" WHERE "workspaceId" = ?1"#
        }
        _ => panic!("column {column}"),
    };
    scalar_text(db, sql, WS).await
}

/// A stored row with `assignments` applied (`flattenSettings(DEFAULTS)` + overrides).
async fn stored_row(db: &chatbotx_db::Db, assignments: &'static str) {
    update(db, json!({})).await.expect("default row");
    setup(db, move |tx| {
        tx.execute(
            &format!(r#"UPDATE "EcommerceSettings" SET {assignments} WHERE "workspaceId" = ?1"#),
            [WS],
        )?;
        Ok(())
    })
    .await;
}

// ts:125 describe("ecommerceSettingsService")

#[tokio::test]
async fn ecommerce_settings_service_answers_with_the_defaults_until_the_workspace_saves() {
    let db = two_workspaces().await;
    assert_eq!(
        ecommerce_settings::get(&db, WS).await.expect("get"),
        defaults()
    );
}

#[tokio::test]
async fn ecommerce_settings_service_merges_a_partial_group_into_the_defaults_and_writes_every_column()
 {
    let db = two_workspaces().await;
    let saved = update(
        &db,
        json!({ "payments": { "currency": "vnd", "currencySymbol": "₫" } }),
    )
    .await
    .expect("update");
    let mut payments = defaults()["payments"].clone();
    payments["currency"] = json!("VND");
    payments["currencySymbol"] = json!("₫");
    assert_eq!(saved["payments"], payments);
    assert_eq!(stored(&db, "currency").await.as_deref(), Some("VND"));
    let read = ecommerce_settings::get(&db, WS).await.expect("get");
    assert_eq!(read["general"]["openHoursId"], Value::Null);
    assert_eq!(read["general"]["abandonedCartDelayMinutes"], Value::Null);
    assert_eq!(
        read["general"]["productButtons"],
        defaults()["general"]["productButtons"]
    );
    assert_eq!(read["shipping"]["shippingCountries"], json!([]));
    assert_eq!(read["checkout"]["requireName"], true);
}

#[tokio::test]
async fn ecommerce_settings_service_needs_a_link_for_the_external_link_checkout() {
    let db = two_workspaces().await;
    let error = update(
        &db,
        json!({ "checkout": { "checkoutMode": "externalLink" } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.code(), Some("validation"));
    assert_eq!(field_of(&error).as_deref(), Some("checkoutLink"));
    assert_eq!(stored(&db, "checkoutMode").await, None);
}

#[tokio::test]
async fn ecommerce_settings_service_needs_a_flow_of_this_workspace_for_the_flow_checkout() {
    let db = two_workspaces().await;
    // Orchestrator fix (P3.6-D2): the real reference guard runs first and gives 404 "Flow not
    // found" for a flow of another workspace (parity p1-08 02-ecommerce-settings.test.ts:180-188).
    setup(&db, |tx| flow(tx, 99, OTHER_WS)).await;
    let error = update(
        &db,
        json!({ "checkout": { "checkoutMode": "flow", "checkoutFlowId": "99" } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Flow not found");
}

#[tokio::test]
async fn ecommerce_settings_service_needs_a_flow_for_the_flow_order_confirmation() {
    let db = two_workspaces().await;
    let error = update(
        &db,
        json!({ "checkout": { "orderConfirmationMode": "flow" } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.code(), Some("validation"));
    assert_eq!(field_of(&error).as_deref(), Some("orderConfirmationFlowId"));
    assert_eq!(stored(&db, "checkoutMode").await, None);
}

#[tokio::test]
async fn ecommerce_settings_service_needs_a_flow_of_this_workspace_for_the_order_confirmation() {
    let db = two_workspaces().await;
    // Orchestrator fix (P3.6-D2): the real reference guard runs first and gives 404 "Flow not
    // found" for a flow of another workspace (parity p1-08 02-ecommerce-settings.test.ts:180-188).
    setup(&db, |tx| flow(tx, 99, OTHER_WS)).await;
    let error = update(
        &db,
        json!({ "checkout": { "orderConfirmationMode": "flow", "orderConfirmationFlowId": "99" } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Flow not found");
}

#[tokio::test]
async fn ecommerce_settings_service_keeps_the_default_shipping_country_inside_the_shipping_countries()
 {
    let db = two_workspaces().await;
    let error = update(
        &db,
        json!({ "shipping": { "shippingCountries": ["vn", "th"], "defaultShippingCountry": "us" } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(field_of(&error).as_deref(), Some("defaultShippingCountry"));
}

#[tokio::test]
async fn ecommerce_settings_service_needs_an_open_hours_schedule_of_this_workspace() {
    let db = two_workspaces().await;
    let error = update(&db, json!({ "general": { "openHoursId": "7" } }))
        .await
        .expect_err("refused");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(stored(&db, "checkoutMode").await, None);
    // A schedule of this workspace is accepted.
    setup(&db, |tx| open_hours(tx, 7, WS)).await;
    let saved = update(&db, json!({ "general": { "openHoursId": "7" } }))
        .await
        .expect("update");
    assert_eq!(saved["general"]["openHoursId"], "7");
}

#[tokio::test]
async fn ecommerce_settings_service_keeps_product_buttons_in_one_order_whatever_order_they_come_in()
{
    let db = two_workspaces().await;
    let saved = update(
        &db,
        json!({ "general": { "productButtons": ["save", "addToCart"] } }),
    )
    .await
    .expect("update");
    assert_eq!(
        saved["general"]["productButtons"],
        json!(["addToCart", "save"])
    );
}

#[tokio::test]
async fn ecommerce_settings_service_refuses_an_abandoned_cart_delay_the_settings_page_does_not_offer()
 {
    let db = two_workspaces().await;
    update(
        &db,
        json!({ "general": { "abandonedCartDelayMinutes": 7 } }),
    )
    .await
    .expect_err("refused");
}

#[tokio::test]
async fn ecommerce_settings_service_needs_a_store_address_for_pickup() {
    let db = two_workspaces().await;
    let error = update(&db, json!({ "shipping": { "shippingType": "pickup" } }))
        .await
        .expect_err("refused");
    assert_eq!(field_of(&error).as_deref(), Some("storeAddress"));
}

#[tokio::test]
async fn ecommerce_settings_service_needs_a_store_address_for_delivery_and_pickup() {
    let db = two_workspaces().await;
    let error = update(
        &db,
        json!({ "shipping": { "shippingType": "deliveryAndPickup" } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(field_of(&error).as_deref(), Some("storeAddress"));
}

#[tokio::test]
async fn ecommerce_settings_service_guards_the_flows_it_references_against_a_template_uninstall_inside_the_save()
 {
    // Adapted: no template locks in the port; the referenced flow of this
    // workspace is accepted and stored.
    let db = two_workspaces().await;
    setup(&db, |tx| flow(tx, 7, WS)).await;
    let saved = update(
        &db,
        json!({ "checkout": {
            "checkoutMode": "flow",
            "checkoutFlowId": "7",
            "orderConfirmationMode": "flow",
            "orderConfirmationFlowId": "7",
        } }),
    )
    .await
    .expect("update");
    assert_eq!(saved["checkout"]["checkoutFlowId"], "7");
    assert_eq!(saved["checkout"]["orderConfirmationFlowId"], "7");
    assert_eq!(stored(&db, "checkoutMode").await.as_deref(), Some("flow"));
}

#[tokio::test]
async fn ecommerce_settings_service_saves_another_group_when_the_stored_checkout_flow_was_deleted()
{
    let db = two_workspaces().await;
    stored_row(
        &db,
        r#""checkoutMode" = 'flow', "checkoutFlowId" = NULL, "orderConfirmationMode" = 'flow', "orderConfirmationFlowId" = NULL"#,
    )
    .await;
    let saved = update(&db, json!({ "payments": { "currency": "usd" } }))
        .await
        .expect("update");
    assert_eq!(saved["payments"]["currency"], "USD");
}

#[tokio::test]
async fn ecommerce_settings_service_still_refuses_a_flow_mode_patch_that_leaves_the_flow_empty() {
    let db = two_workspaces().await;
    stored_row(&db, r#""checkoutMode" = 'flow', "checkoutFlowId" = NULL"#).await;
    let error = update(&db, json!({ "checkout": { "checkoutMode": "flow" } }))
        .await
        .expect_err("refused");
    assert_eq!(field_of(&error).as_deref(), Some("checkoutFlowId"));
}

#[tokio::test]
async fn ecommerce_settings_service_creates_and_locks_the_row_before_merging_in_one_transaction() {
    let db = two_workspaces().await;
    assert_eq!(stored(&db, "currency").await, None);
    update(&db, json!({ "payments": { "currency": "usd" } }))
        .await
        .expect("update");
    assert_eq!(stored(&db, "currency").await.as_deref(), Some("USD"));
}

// ts:344 describe("ecommerceSettingsService — online payment methods")

#[tokio::test]
async fn online_payment_methods_stripe_or_paypal_can_only_be_offered_once_connected() {
    let db = two_workspaces().await;
    let error = update(
        &db,
        json!({ "payments": { "paymentMethods": ["cod", "stripe"] } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.code(), Some("paymentProviderNotConnected"));
    assert_eq!(data_of(&error), Some(json!({ "provider": "stripe" })));
    assert_eq!(stored(&db, "paymentMethods").await, None);
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "EcommercePaymentProvider" ("id", "workspaceId", "provider", "livemode")
               VALUES (1, ?1, 'stripe', 0)"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    update(
        &db,
        json!({ "payments": { "paymentMethods": ["cod", "stripe"] } }),
    )
    .await
    .expect("update");
    assert_eq!(
        stored(&db, "paymentMethods").await.as_deref(),
        Some(r#"["cod","stripe"]"#)
    );
}

#[tokio::test]
async fn online_payment_methods_paypal_is_refused_in_a_currency_it_does_not_take_whichever_field_changes()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "EcommercePaymentProvider" ("id", "workspaceId", "provider", "livemode")
               VALUES (1, ?1, 'paypal', 0)"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let error = update(
        &db,
        json!({ "payments": { "paymentMethods": ["paypal"], "currency": "VND" } }),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.code(), Some("paymentCurrencyUnsupported"));
    assert_eq!(
        data_of(&error),
        Some(json!({ "provider": "paypal", "currency": "VND" }))
    );
    stored_row(&db, r#""paymentMethods" = '["paypal"]'"#).await;
    let error = update(&db, json!({ "payments": { "currency": "VND" } }))
        .await
        .expect_err("refused");
    assert_eq!(error.code(), Some("paymentCurrencyUnsupported"));
}

#[tokio::test]
async fn online_payment_methods_dropping_a_disconnected_provider_brings_cash_on_delivery_back_when_nothing_is_left()
 {
    let db = two_workspaces().await;
    stored_row(&db, r#""paymentMethods" = '["stripe"]'"#).await;
    ecommerce_settings::drop_payment_method(&db, WS, "stripe")
        .await
        .expect("drop");
    assert_eq!(
        stored(&db, "paymentMethods").await.as_deref(),
        Some(r#"["cod"]"#)
    );
}

// ts:408 describe("selling from an external store")

#[tokio::test]
async fn selling_from_an_external_store_switching_to_a_store_that_is_not_connected_is_refused() {
    let db = two_workspaces().await;
    let error = update(&db, json!({ "general": { "storeSource": "woocommerce" } }))
        .await
        .expect_err("refused");
    assert_eq!(error.code(), Some("externalStoreNotConnected"));
    assert_eq!(data_of(&error), Some(json!({ "store": "woocommerce" })));
    assert_eq!(stored(&db, "currency").await, None);
}

// legacy keys (apps/builder/__tests__/ecommerce-settings-legacy-keys.test.ts)

#[test]
fn the_deprecated_shopify_checkout_keys_still_update_the_shared_settings() {
    let patch = parse_patch(&json!({
        "checkout": { "shopifyCheckoutMethod": "inApp", "shopifyAutoApplyDiscounts": false }
    }))
    .expect("parse");
    assert_eq!(
        patch["checkout"],
        json!({ "externalCheckoutMethod": "inApp", "externalAutoApplyDiscounts": false })
    );
}

#[test]
fn the_new_keys_win_over_the_deprecated_ones() {
    let patch = parse_patch(&json!({
        "checkout": { "externalCheckoutMethod": "webCheckout", "shopifyCheckoutMethod": "inApp" }
    }))
    .expect("parse");
    assert_eq!(patch["checkout"]["externalCheckoutMethod"], "webCheckout");
}

#[test]
fn a_patch_without_checkout_is_left_as_it_is() {
    let patch = parse_patch(&json!({ "general": { "orderingAvailable": true } })).expect("parse");
    assert_eq!(patch, json!({ "general": { "orderingAvailable": true } }));
}
