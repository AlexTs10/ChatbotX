//! Port of `packages/flow-config/__tests__/wa-template-mpm-rules.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn base_step() -> Value {
    json!({
        "id": "1",
        "nodeId": "node-1",
        "stepType": "sendWaTemplateMessage",
        "template": { "id": "tmpl-1", "name": "tpl", "language": "en", "inboxId": null, "params": {} },
        "buttons": [],
    })
}

fn issues_for(params: Value) -> Vec<String> {
    let refined_step_schema = fc::send_wa_template_message_validator().base();
    let step = base_step();
    let result = refined_step_schema.safe_parse(&spread(
        &step,
        json!({ "template": spread(&step["template"], json!({ "params": params })) }),
    ));
    result.messages()
}

fn has(issues: &[String], code: &str) -> bool {
    issues.iter().any(|issue| issue == code)
}

fn limit(key: &str) -> usize {
    fc::wa_template_mpm_limits()[key].as_u64().unwrap() as usize
}

#[test]
fn countmpmproducts_sums_product_items_across_every_section() {
    assert_eq!(
        fc::count_mpm_products(Some(&json!([
            { "title": "A", "product_items": [{ "product_retailer_id": "1" }] },
            { "title": "B", "product_items": [{ "product_retailer_id": "2" }, { "product_retailer_id": "3" }] },
        ]))),
        3
    );
}

#[test]
fn countmpmproducts_treats_missing_sections_product_items_as_zero() {
    assert_eq!(fc::count_mpm_products(None), 0);
    assert_eq!(
        fc::count_mpm_products(Some(&json!([{ "title": "Empty" }]))),
        0
    );
}

#[test]
fn mpm_publish_validation_top_level_button_an_mpm_button_with_zero_configured_products_is_invalid()
{
    assert!(has(
        &issues_for(json!({ "button": [{ "sub_type": "mpm", "index": 0, "sections": [] }] })),
        "waTemplateMpmNoProducts"
    ));
}

#[test]
fn mpm_publish_validation_top_level_button_an_mpm_button_with_no_sections_at_all_is_invalid() {
    assert!(has(
        &issues_for(json!({ "button": [{ "sub_type": "mpm", "index": 0 }] })),
        "waTemplateMpmNoProducts"
    ));
}

#[test]
fn mpm_publish_validation_top_level_button_an_mpm_button_with_at_least_one_product_is_valid() {
    assert!(
        issues_for(json!({
            "button": [{
                "sub_type": "mpm",
                "index": 0,
                "sections": [{ "title": "Best sellers", "product_items": [{ "product_retailer_id": "sku-1" }] }],
            }],
        }))
        .is_empty()
    );
}

#[test]
fn mpm_publish_validation_top_level_button_non_mpm_buttons_are_left_alone() {
    assert!(
        issues_for(json!({ "button": [{ "sub_type": "url", "index": 0, "text": "" }] })).is_empty()
    );
}

#[test]
fn mpm_publish_validation_top_level_button_unfilled_placeholder_rows_blank_retailer_id_never_count_as_products()
 {
    assert!(has(
        &issues_for(json!({
            "button": [{
                "sub_type": "mpm",
                "index": 0,
                "sections": [{
                    "title": "Best sellers",
                    "product_items": [{ "product_retailer_id": "" }, { "product_retailer_id": "   " }],
                }],
            }],
        })),
        "waTemplateMpmNoProducts"
    ));
}

#[test]
fn mpm_publish_validation_top_level_button_a_blank_row_next_to_a_real_product_is_invalid_would_send_an_empty_id_to_meta()
 {
    assert!(has(
        &issues_for(json!({
            "button": [{
                "sub_type": "mpm",
                "index": 0,
                "sections": [{
                    "title": "Best sellers",
                    "product_items": [{ "product_retailer_id": "sku-1" }, { "product_retailer_id": "" }],
                }],
            }],
        })),
        "waTemplateMpmIncompleteProducts"
    ));
}

#[test]
fn mpm_publish_validation_top_level_button_an_extra_section_with_no_products_is_invalid_even_when_another_section_is_complete()
 {
    assert!(has(
        &issues_for(json!({
            "button": [{
                "sub_type": "mpm",
                "index": 0,
                "sections": [
                    { "title": "Best sellers", "product_items": [{ "product_retailer_id": "sku-1" }] },
                    { "title": "Empty", "product_items": [] },
                ],
            }],
        })),
        "waTemplateMpmIncompleteProducts"
    ));
}

#[test]
fn mpm_publish_validation_top_level_button_exceeding_the_section_limit_is_invalid() {
    let sections: Vec<Value> = (0..limit("maxSections") + 1)
        .map(|i| json!({ "title": format!("Section {i}"), "product_items": [{ "product_retailer_id": format!("sku-{i}") }] }))
        .collect();

    assert!(has(
        &issues_for(json!({ "button": [{ "sub_type": "mpm", "index": 0, "sections": sections }] })),
        "waTemplateMpmTooManySections"
    ));
}

#[test]
fn mpm_publish_validation_top_level_button_exceeding_the_total_product_limit_is_invalid() {
    let product_items: Vec<Value> = (0..limit("maxProductsTotal") + 1)
        .map(|i| json!({ "product_retailer_id": format!("sku-{i}") }))
        .collect();

    assert!(has(
        &issues_for(json!({
            "button": [{
                "sub_type": "mpm",
                "index": 0,
                "sections": [{ "title": "Everything", "product_items": product_items }],
            }],
        })),
        "waTemplateMpmTooManyProducts"
    ));
}

#[test]
fn mpm_publish_validation_carousel_card_button_an_mpm_button_inside_a_carousel_card_with_zero_products_is_invalid()
 {
    assert!(has(
        &issues_for(json!({
            "carousel": [{ "card_index": 0, "button": [{ "sub_type": "mpm", "index": 0, "sections": [] }] }],
        })),
        "waTemplateMpmNoProducts"
    ));
}

#[test]
fn mpm_publish_validation_carousel_card_button_an_mpm_button_inside_a_carousel_card_with_a_product_is_valid()
 {
    assert!(
        issues_for(json!({
            "carousel": [{
                "card_index": 0,
                "button": [{
                    "sub_type": "mpm",
                    "index": 0,
                    "sections": [{ "title": "Card picks", "product_items": [{ "product_retailer_id": "sku-1" }] }],
                }],
            }],
        }))
        .is_empty()
    );
}
