//! Port of `packages/flow-config/__tests__/template-structure.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn base() -> Value {
    json!({ "name": "promo", "language": "vi", "category": "MARKETING", "parameterFormat": "POSITIONAL" })
}

fn with_components(components: Value) -> Value {
    spread(&base(), json!({ "components": components }))
}

fn whatsapp_components(header_handle: &str) -> Value {
    json!([
        { "type": "HEADER", "format": "IMAGE", "example": { "header_handle": [header_handle] } },
        { "type": "BODY", "text": "Hi {{1}}, code {{2}}", "example": { "body_text": [["Ada", "X"]] } },
        {
            "type": "BUTTONS",
            "buttons": [
                { "type": "URL", "text": "Shop", "url": "https://shop.test/{{1}}", "example": ["a"] },
                { "type": "FLOW", "text": "Book", "flow_id": 1_690_702_985_711_558_i64, "navigate_screen": "WELCOME" },
                { "type": "COPY_CODE", "text": "Copy", "example": ["SALE"] },
            ],
        },
    ])
}

fn key(v: &Value) -> String {
    fc::template_structure_key(v)
}

#[test]
fn templatestructurekey_ignores_page_specific_example_data_such_as_uploaded_header_handles_and_samples()
 {
    let a = key(&with_components(whatsapp_components("4:handle-page-a")));
    let b = key(&with_components(whatsapp_components("4:handle-page-b")));
    assert_eq!(a, b);
}

#[test]
fn templatestructurekey_is_independent_of_object_key_order() {
    let ordered = key(&with_components(
        json!([{ "type": "BODY", "text": "Hi {{1}}" }]),
    ));
    let reordered = key(&json!({
        "components": [{ "text": "Hi {{1}}", "type": "BODY" }],
        "parameterFormat": "POSITIONAL",
        "category": "MARKETING",
        "language": "vi",
        "name": "promo",
    }));
    assert_eq!(ordered, reordered);
}

#[test]
fn templatestructurekey_differs_when_the_body_placeholders_differ() {
    let one = key(&with_components(
        json!([{ "type": "BODY", "text": "Hi {{1}}" }]),
    ));
    let two = key(&with_components(
        json!([{ "type": "BODY", "text": "Hi {{1}} {{2}}" }]),
    ));
    assert_ne!(one, two);
}

#[test]
fn templatestructurekey_differs_between_a_static_and_a_dynamic_url_button() {
    let button = |url: &str| {
        with_components(
            json!([{ "type": "BUTTONS", "buttons": [{ "type": "URL", "text": "Go", "url": url }] }]),
        )
    };
    assert_ne!(
        key(&button("https://a.test")),
        key(&button("https://a.test/{{1}}"))
    );
}

#[test]
fn templatestructurekey_differs_between_a_static_and_a_dynamic_messenger_postback_payload() {
    let postback = |payload: &str| {
        with_components(
            json!([{ "type": "BUTTONS", "buttons": [{ "type": "POSTBACK", "text": "Go", "payload": payload }] }]),
        )
    };
    assert_ne!(key(&postback("FIXED")), key(&postback("{{flow}}")));
}

#[test]
fn templatestructurekey_differs_when_a_whatsapp_flow_button_points_at_another_flow_or_screen() {
    let flow = |flow_id: i64, screen: &str| {
        with_components(json!([{
            "type": "BUTTONS",
            "buttons": [{ "type": "FLOW", "text": "Book", "flow_id": flow_id, "navigate_screen": screen }],
        }]))
    };
    assert_ne!(key(&flow(1, "A")), key(&flow(2, "A")));
    assert_ne!(key(&flow(1, "A")), key(&flow(1, "B")));
}

#[test]
fn templatestructurekey_differs_across_button_kinds_order_header_format_parameter_format_and_lto_flag()
 {
    let with_buttons =
        |buttons: Value| with_components(json!([{ "type": "BUTTONS", "buttons": buttons }]));
    assert_ne!(
        key(&with_buttons(
            json!([{ "type": "COPY_CODE", "text": "Copy" }])
        )),
        key(&with_buttons(
            json!([{ "type": "CATALOG", "text": "Copy" }])
        ))
    );
    assert_ne!(
        key(&with_buttons(
            json!([{ "type": "QUICK_REPLY", "text": "A" }, { "type": "QUICK_REPLY", "text": "B" }])
        )),
        key(&with_buttons(
            json!([{ "type": "QUICK_REPLY", "text": "B" }, { "type": "QUICK_REPLY", "text": "A" }])
        ))
    );
    assert_ne!(
        key(&with_components(
            json!([{ "type": "HEADER", "format": "IMAGE" }])
        )),
        key(&with_components(
            json!([{ "type": "HEADER", "format": "VIDEO" }])
        ))
    );
    assert_ne!(
        key(&with_components(json!([]))),
        key(&spread(
            &base(),
            json!({ "parameterFormat": "NAMED", "components": [] })
        ))
    );
    assert_ne!(
        key(&with_components(
            json!([{ "type": "LIMITED_TIME_OFFER", "limited_time_offer": { "has_expiration": true } }])
        )),
        key(&with_components(
            json!([{ "type": "LIMITED_TIME_OFFER", "limited_time_offer": { "has_expiration": false } }])
        ))
    );
}

#[test]
fn templatestructurekey_normalises_carousel_cards_recursively_and_drops_card_examples() {
    let carousel = |handle: &str| {
        with_components(json!([{
            "type": "CAROUSEL",
            "cards": [{
                "card_index": 0,
                "components": [
                    { "type": "HEADER", "format": "IMAGE", "example": { "header_handle": [handle] } },
                    { "type": "BODY", "text": "Card {{1}}" },
                ],
            }],
        }]))
    };
    assert_eq!(key(&carousel("h1")), key(&carousel("h2")));
    assert_match_object(
        &fc::build_template_structure(&carousel("h1"))["components"][0]["cards"][0],
        &json!({
            "cardIndex": 0,
            "components": [{ "type": "HEADER", "format": "IMAGE" }, { "type": "BODY", "text": "Card {{1}}" }],
        }),
    );
}

#[test]
fn templatestructurekey_tolerates_malformed_component_payloads_without_throwing() {
    let _ = key(&spread(&base(), json!({ "components": "not-an-array" })));
    assert_eq!(
        fc::build_template_structure(&spread(
            &base(),
            json!({ "components": [null, 1, { "type": "BODY" }] })
        ))["components"],
        json!([{ "type": "BODY" }])
    );
}
