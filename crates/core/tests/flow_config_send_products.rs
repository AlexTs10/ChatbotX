//! Port of `packages/flow-config/__tests__/send-products.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn whatsapp_capability() -> Option<Value> {
    fc::channel_flow_policies()["whatsapp"]
        .product_messages
        .clone()
}

fn product_ids(count: usize, from: usize) -> Vec<String> {
    (0..count).map(|index| (from + index).to_string()).collect()
}

fn build_step(props: Value) -> Value {
    fc::send_products_step_default_fn(&props)
}

fn issue_codes(channel: &str, step: &Value) -> Vec<String> {
    let validators = fc::channel_aware_step_validators();
    let validator = validators
        .get("sendProducts")
        .expect("sendProducts validator is not registered");
    fc::resolve_step_validator(validator, channel)
        .safe_parse(step)
        .messages()
}

fn valid_on_whatsapp(step: Value) {
    assert!(
        issue_codes("whatsapp", &step).is_empty(),
        "{:?}",
        issue_codes("whatsapp", &step)
    );
}

#[test]
fn sendproducts_content_schema_parses_the_message_fields_a_sender_outside_a_flow_provides() {
    let value =
        json!({ "display": "products", "body": "Look", "sections": [{ "productIds": ["1"] }] });
    assert_eq!(
        fc::send_products_content_schema().parse(&value).unwrap(),
        value
    );
}

#[test]
fn sendproducts_content_schema_carries_nothing_that_belongs_to_a_flow_step() {
    let parsed = fc::send_products_content_schema()
        .parse(&json!({
            "id": "step-1",
            "stepType": "sendProducts",
            "display": "carousel",
            "sections": [{ "productIds": ["1", "2"] }],
        }))
        .unwrap();

    assert!(parsed.get("id").is_none());
    assert!(parsed.get("stepType").is_none());
}

#[test]
fn sendproducts_content_schema_rejects_a_message_without_a_display_or_sections() {
    assert!(
        !fc::send_products_content_schema()
            .safe_parse(&json!({}))
            .success
    );
}

#[test]
fn sendproducts_content_schema_the_step_schema_accepts_everything_the_content_schema_does() {
    let step = build_step(json!({ "body": "Look", "sections": [{ "productIds": ["1"] }] }));

    assert!(fc::send_products_step_schema().safe_parse(&step).success);
}

#[test]
fn sendproducts_content_schema_a_sendproducts_step_still_parses_inside_the_send_message_node() {
    let step = build_step(json!({ "body": "Look", "sections": [{ "productIds": ["1"] }] }));

    let defaults = fc::send_message_node_default_fn(&json!({
        "detailProps": { "beforeStep": fc::choose_channel_step_default_fn(&json!({ "channel": "whatsapp" })) },
    }));
    let node = fc::send_message_node_schema().safe_parse(&spread(
        &defaults,
        json!({ "data": spread(&defaults["data"], json!({ "details": spread(&defaults["data"]["details"], json!({ "steps": [step] })) })) }),
    ));

    assert!(node.success);
}

#[test]
fn sendproducts_channel_support_only_whatsapp_declares_the_capability() {
    let channels: Vec<&str> = fc::channel_flow_policies()
        .iter()
        .filter(|(_, policy)| policy.product_messages.is_some())
        .map(|(channel, _)| *channel)
        .collect();

    assert_eq!(channels, vec!["whatsapp"]);
}

#[test]
fn sendproducts_channel_support_is_supported_on_whatsapp_and_unsupported_everywhere_else_omnichannel_included()
 {
    let support = fc::step_support();
    for (channel, policy) in fc::channel_flow_policies() {
        let expected = if *channel == "whatsapp" {
            &support["full"]
        } else {
            &support["unsupported"]
        };
        assert_eq!(
            policy.steps["sendProducts"],
            expected.as_str().unwrap(),
            "{channel}"
        );
    }
}

#[test]
fn sendproducts_on_whatsapp_accepts_one_product_with_no_text() {
    valid_on_whatsapp(build_step(json!({ "sections": [{ "productIds": ["1"] }] })));
}

#[test]
fn sendproducts_on_whatsapp_accepts_a_multi_product_message_with_header_body_and_titled_sections() {
    valid_on_whatsapp(build_step(json!({
        "header": "Our picks",
        "body": "Have a look",
        "footer": "Free delivery",
        "sections": [
            { "title": "Shoes", "productIds": ["1", "2"] },
            { "title": "Bags", "productIds": ["3"] },
        ],
    })));
}

#[test]
fn sendproducts_on_whatsapp_accepts_exactly_30_products_across_10_sections() {
    let sections: Vec<Value> = (0..10)
        .map(|index| json!({ "title": format!("s{index}"), "productIds": product_ids(3, index * 3 + 1) }))
        .collect();
    valid_on_whatsapp(build_step(
        json!({ "header": "h", "body": "b", "sections": sections }),
    ));
}

#[test]
fn sendproducts_on_whatsapp_rejects_a_header_on_a_single_product() {
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(json!({ "header": "h", "sections": [{ "productIds": ["1"] }] }))
        ),
        vec!["sendProductsFieldNotAllowed"]
    );
}

#[test]
fn sendproducts_on_whatsapp_requires_header_and_body_once_there_are_2_products() {
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(json!({ "sections": [{ "productIds": ["1", "2"] }] }))
        ),
        vec!["sendProductsFieldRequired", "sendProductsFieldRequired"]
    );
}

#[test]
fn sendproducts_on_whatsapp_requires_a_title_on_every_section_once_there_are_2_sections() {
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(json!({
                "header": "h",
                "body": "b",
                "sections": [{ "title": "Shoes", "productIds": ["1"] }, { "productIds": ["2"] }],
            }))
        ),
        vec!["sendProductsFieldRequired"]
    );
}

#[test]
fn sendproducts_on_whatsapp_rejects_text_over_the_channel_limits() {
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(json!({
                "header": rep("h", 61),
                "body": rep("b", 1025),
                "footer": rep("f", 61),
                "sections": [
                    { "title": rep("t", 25), "productIds": ["1"] },
                    { "title": "ok", "productIds": ["2"] },
                ],
            }))
        ),
        vec![
            "sendProductsFieldTooLong",
            "sendProductsFieldTooLong",
            "sendProductsFieldTooLong",
            "sendProductsFieldTooLong",
        ]
    );
}

#[test]
fn sendproducts_on_whatsapp_rejects_31_products_and_11_sections() {
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(
                json!({ "header": "h", "body": "b", "sections": [{ "title": "t", "productIds": product_ids(31, 1) }] })
            )
        ),
        vec!["sendProductsProductCountOutOfRange"]
    );

    let sections: Vec<Value> = (0..11)
        .map(|index| json!({ "title": format!("s{index}"), "productIds": [(index + 1).to_string()] }))
        .collect();
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(json!({ "header": "h", "body": "b", "sections": sections }))
        ),
        vec!["sendProductsSectionCountOutOfRange"]
    );
}

#[test]
fn sendproducts_on_whatsapp_rejects_duplicate_products_and_empty_sections_on_every_channel() {
    let duplicated = build_step(json!({
        "header": "h",
        "body": "b",
        "sections": [{ "title": "a", "productIds": ["1", "2"] }, { "title": "b", "productIds": ["2"] }],
    }));
    assert_eq!(
        issue_codes("whatsapp", &duplicated),
        vec!["sendProductsDuplicateProduct"]
    );
    assert_eq!(
        issue_codes("omnichannel", &duplicated),
        vec!["sendProductsDuplicateProduct"]
    );

    assert!(
        issue_codes(
            "whatsapp",
            &build_step(json!({ "sections": [{ "productIds": [] }] }))
        )
        .contains(&"sendProductsProductCountOutOfRange".to_owned())
    );
}

#[test]
fn sendproducts_on_whatsapp_counts_an_emoji_once_like_the_editor_does() {
    valid_on_whatsapp(build_step(json!({
        "header": rep("😀", 60),
        "body": "b",
        "sections": [{ "productIds": ["1", "2"] }],
    })));
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(
                json!({ "header": rep("😀", 61), "body": "b", "sections": [{ "productIds": ["1", "2"] }] })
            )
        ),
        vec!["sendProductsFieldTooLong"]
    );
}

#[test]
fn sendproducts_on_whatsapp_treats_whitespace_only_text_as_absent() {
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(
                json!({ "display": "carousel", "body": "   ", "sections": [{ "productIds": ["1", "2"] }] })
            )
        ),
        vec!["sendProductsFieldRequired"]
    );
}

#[test]
fn sendproducts_carousel_on_whatsapp_accepts_2_to_10_products_with_a_body_only() {
    valid_on_whatsapp(build_step(json!({
        "display": "carousel",
        "body": "Look",
        "sections": [{ "productIds": product_ids(10, 1) }],
    })));
}

#[test]
fn sendproducts_carousel_on_whatsapp_rejects_1_or_11_products() {
    for count in [1, 11] {
        assert_eq!(
            issue_codes(
                "whatsapp",
                &build_step(
                    json!({ "display": "carousel", "body": "Look", "sections": [{ "productIds": product_ids(count, 1) }] })
                )
            ),
            vec!["sendProductsProductCountOutOfRange"],
            "{count}"
        );
    }
}

#[test]
fn sendproducts_carousel_on_whatsapp_rejects_header_footer_titles_and_a_second_section() {
    assert_eq!(
        issue_codes(
            "whatsapp",
            &build_step(json!({
                "display": "carousel",
                "header": "h",
                "body": "Look",
                "footer": "f",
                "sections": [{ "title": "t", "productIds": ["1", "2"] }, { "productIds": ["3"] }],
            }))
        ),
        vec![
            "sendProductsSectionCountOutOfRange",
            "sendProductsFieldNotAllowed",
            "sendProductsFieldNotAllowed",
            "sendProductsFieldNotAllowed",
        ]
    );
}

fn shape(props: Value) -> Value {
    spread(
        &json!({ "display": "products", "sections": [{ "productCount": 1 }] }),
        props,
    )
}

#[test]
fn evaluateproductmessage_reports_an_unsupported_display_instead_of_throwing() {
    assert_eq!(
        fc::evaluate_product_message(
            &json!({ "displays": {} }),
            &shape(json!({ "display": "carousel" }))
        ),
        vec![json!({ "code": "sendProductsDisplayUnsupported", "path": ["display"] })]
    );
}

#[test]
fn evaluateproductmessage_points_each_issue_at_the_offending_field() {
    let capability = whatsapp_capability().expect("whatsapp capability missing");

    assert_eq!(
        fc::evaluate_product_message(
            &capability,
            &shape(json!({
                "header": "h",
                "body": "b",
                "sections": [{ "title": "a", "productCount": 1 }, { "productCount": 1 }],
            })),
        ),
        vec![json!({ "code": "sendProductsFieldRequired", "path": ["sections", 1, "title"] })]
    );
}
