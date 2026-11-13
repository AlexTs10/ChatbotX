//! Port of `packages/business/__tests__/product-message-resolve.test.ts`
//! (pure function; steps and items as JSON).

use std::collections::HashMap;

use chatbotx_business::product_message::resolve::{Resolution, resolve_product_message};
use chatbotx_core::flow_config::channel_rules::get_channel_flow_policy;
use serde_json::{Value, json};

fn capability() -> Value {
    get_channel_flow_policy(Some("whatsapp"))
        .and_then(|policy| policy.product_messages.clone())
        .expect("whatsapp must declare productMessages")
}

fn item(product_id: &str) -> Value {
    json!({
        "productId": product_id,
        "name": format!("Product {product_id}"),
        "imageUrl": null,
        "price": 10,
        "currency": "USD",
        "catalogReference": { "source": "metaCatalog", "catalogId": "cat-1", "retailerId": format!("sku-{product_id}") },
    })
}

fn resolve(props: Value, ids: &[&str]) -> Resolution {
    let mut content =
        json!({ "id": "1", "stepType": "sendProducts", "display": "products", "sections": [] });
    for (key, value) in props.as_object().expect("props").clone() {
        content[key] = value;
    }
    let sendable: HashMap<String, Value> =
        ids.iter().map(|id| (id.to_string(), item(id))).collect();
    resolve_product_message(&capability(), &content, &sendable)
}

fn ok(result: Resolution) -> (Value, Vec<String>) {
    match result {
        Resolution::Ok { message, missing } => (message, missing),
        other => panic!("expected ok, got {other:?}"),
    }
}

fn item_ids(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["productId"].as_str().expect("productId").to_string())
        .collect()
}

#[test]
fn resolve_product_message_products_display_keeps_the_author_s_sections_and_order() {
    let (message, missing) = ok(resolve(
        json!({ "header": "Picks", "body": "Look", "sections": [
            { "title": "Shoes", "productIds": ["2", "1"] },
            { "title": "Bags", "productIds": ["3"] },
        ] }),
        &["1", "2", "3"],
    ));
    assert!(missing.is_empty());
    assert_eq!(
        (&message["display"], &message["header"], &message["body"]),
        (&json!("products"), &json!("Picks"), &json!("Look"))
    );
    let sections = message["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 2);
    assert_eq!(
        (&sections[0]["title"], item_ids(&sections[0]["items"])),
        (&json!("Shoes"), vec!["2".to_string(), "1".to_string()])
    );
    assert_eq!(
        (&sections[1]["title"], item_ids(&sections[1]["items"])),
        (&json!("Bags"), vec!["3".to_string()])
    );
}

#[test]
fn resolve_product_message_products_display_drops_products_that_are_not_sendable_and_reports_them()
{
    let (message, missing) = ok(resolve(
        json!({ "header": "Picks", "body": "Look", "sections": [{ "title": "All", "productIds": ["1", "9", "2"] }] }),
        &["1", "2"],
    ));
    assert_eq!(missing, vec!["9".to_string()]);
    assert_eq!(
        item_ids(&message["sections"][0]["items"]),
        vec!["1".to_string(), "2".to_string()]
    );
}

#[test]
fn resolve_product_message_products_display_drops_a_section_whose_products_are_all_gone() {
    let (message, _) = ok(resolve(
        json!({ "header": "Picks", "body": "Look", "sections": [
            { "title": "Gone", "productIds": ["9"] },
            { "title": "Kept", "productIds": ["1", "2"] },
        ] }),
        &["1", "2"],
    ));
    let sections = message["sections"].as_array().expect("sections");
    assert_eq!((sections.len(), &sections[0]["title"]), (1, &json!("Kept")));
}

#[test]
fn resolve_product_message_products_display_omits_the_header_once_the_message_shrinks_to_one_product()
 {
    let (message, _) = ok(resolve(
        json!({ "header": "Picks", "body": "Look", "sections": [{ "title": "All", "productIds": ["1", "9"] }] }),
        &["1"],
    ));
    assert_eq!(message["display"], json!("products"));
    assert!(message.get("header").is_none_or(Value::is_null));
}

#[test]
fn resolve_product_message_products_display_fails_with_no_products_sendable_when_nothing_resolves()
{
    assert_eq!(
        resolve(
            json!({ "header": "Picks", "body": "Look", "sections": [{ "title": "x", "productIds": ["8", "9"] }] }),
            &[],
        ),
        Resolution::Failed("noProductsSendable")
    );
}

#[test]
fn resolve_product_message_products_display_fails_an_invalid_single_product_step_instead_of_dropping_its_header()
 {
    assert_eq!(
        resolve(
            json!({ "header": "Picks", "sections": [{ "productIds": ["1"] }] }),
            &["1"]
        ),
        Resolution::Failed("invalidMessage")
    );
}

#[test]
fn resolve_product_message_products_display_rejects_over_limit_text_instead_of_clamping_it() {
    assert_eq!(
        resolve(
            json!({ "header": "h".repeat(61), "body": "b", "sections": [{ "title": "t", "productIds": ["1", "2"] }] }),
            &["1", "2"],
        ),
        Resolution::Failed("invalidMessage")
    );
}

#[test]
fn resolve_product_message_products_display_treats_whitespace_only_text_as_absent() {
    assert_eq!(
        resolve(
            json!({ "header": "   ", "body": "   ", "sections": [{ "productIds": ["1", "2"] }] }),
            &["1", "2"],
        ),
        Resolution::Failed("invalidMessage")
    );
}

fn carousel(ids: &[&str]) -> Value {
    json!({ "display": "carousel", "body": "Look", "sections": [{ "productIds": ids }] })
}

#[test]
fn resolve_product_message_carousel_display_flattens_the_single_section_into_cards() {
    let (message, _) = ok(resolve(carousel(&["1", "2", "3"]), &["1", "2", "3"]));
    assert_eq!(
        (&message["display"], &message["body"]),
        (&json!("carousel"), &json!("Look"))
    );
    assert_eq!(
        item_ids(&message["items"]),
        vec!["1".to_string(), "2".to_string(), "3".to_string()]
    );
}

#[test]
fn resolve_product_message_carousel_display_fails_as_no_products_sendable_when_too_few_cards_remain()
 {
    assert_eq!(
        resolve(carousel(&["1", "9"]), &["1"]),
        Resolution::Failed("noProductsSendable")
    );
    assert_eq!(
        resolve(carousel(&["1", "8", "9"]), &["1"]),
        Resolution::Failed("noProductsSendable")
    );
}

#[test]
fn resolve_product_message_carousel_display_fails_an_invalid_step_instead_of_repairing_it() {
    let mut step = carousel(&["1", "2"]);
    step["footer"] = json!("not allowed");
    assert_eq!(
        resolve(step, &["1", "2"]),
        Resolution::Failed("invalidMessage")
    );
}
