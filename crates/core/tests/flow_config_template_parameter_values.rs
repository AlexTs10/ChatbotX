//! Port of `packages/flow-config/__tests__/template-parameter-values.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use std::collections::BTreeMap;

fn positional() -> Vec<Value> {
    vec![
        json!({ "type": "HEADER", "format": "IMAGE" }),
        json!({ "type": "BODY", "text": "Hi {{1}}, order {{2}} is ready" }),
        json!({
            "type": "BUTTONS",
            "buttons": [
                { "type": "PHONE_NUMBER", "text": "Call", "phone_number": "+84" },
                { "type": "URL", "text": "Track", "url": "https://x.io/{{1}}" },
                { "type": "COPY_CODE", "text": "Copy" },
            ],
        }),
    ]
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn keys(specs: &[fc::TemplateParameterSpec]) -> Vec<String> {
    specs.iter().map(|s| s.key.clone()).collect()
}

fn spec(key: &str, component: &str, kind: &str, required: bool) -> fc::TemplateParameterSpec {
    fc::TemplateParameterSpec {
        key: key.to_string(),
        component: component.to_string(),
        kind: kind.to_string(),
        required,
        placeholder: None,
    }
}

fn wa(components: &[Value], pairs: &[(&str, &str)]) -> fc::AppliedTemplateParameters {
    fc::apply_wa_template_parameter_values(components, &values(pairs))
}

fn messenger(
    components: &[Value],
    format: &str,
    pairs: &[(&str, &str)],
) -> fc::AppliedTemplateParameters {
    fc::apply_messenger_template_parameter_values(components, format, &values(pairs))
}

const NONE: &[String] = &[];

// --- WhatsApp template parameters ---

#[test]
fn whatsapp_template_parameters_lists_one_key_per_value_to_fill() {
    assert_eq!(
        keys(&fc::describe_wa_template_parameters(&positional())),
        vec!["header", "body.1", "body.2", "button.1", "button.2"]
    );
}

#[test]
fn whatsapp_template_parameters_builds_the_nested_params_with_dense_button_slots() {
    let result = wa(
        &positional(),
        &[
            ("header", "https://cdn.x.io/a.jpg"),
            ("body.1", "Ann"),
            ("body.2", "#42"),
            ("button.1", "abc"),
            ("button.2", "SAVE10"),
        ],
    );

    assert_eq!(result.missing, NONE);
    assert_eq!(result.unknown, NONE);
    assert_eq!(
        result.params,
        json!({
            "header": [{ "type": "image", "image": { "link": "https://cdn.x.io/a.jpg" } }],
            "body": [{ "type": "text", "text": "Ann" }, { "type": "text", "text": "#42" }],
            "button": [
                { "sub_type": "url", "index": 1, "text": "abc" },
                { "sub_type": "copy_code", "index": 2, "coupon_code": "SAVE10" },
            ],
        })
    );
}

#[test]
fn whatsapp_template_parameters_named_placeholders_keep_parameter_name_and_are_keyed_by_name() {
    let result = wa(
        &[json!({ "type": "BODY", "text": "Order {{order_id}} for {{name}}" })],
        &[("body.order_id", "42"), ("body.name", "Ann")],
    );

    assert_eq!(
        result.params["body"],
        json!([
            { "type": "text", "text": "42", "parameter_name": "order_id" },
            { "type": "text", "text": "Ann", "parameter_name": "name" },
        ])
    );
}

#[test]
fn whatsapp_template_parameters_reports_missing_unknown_and_empty_keys() {
    let result = wa(
        &positional(),
        &[
            ("header", "https://cdn.x.io/a.jpg"),
            ("body.1", "  "),
            ("nope", "x"),
        ],
    );

    assert_eq!(
        result.missing,
        vec!["body.1", "body.2", "button.1", "button.2"]
    );
    assert_eq!(result.unknown, vec!["nope"]);
}

#[test]
fn whatsapp_template_parameters_location_headers_take_latitude_longitude_name_and_address_optional()
{
    let result = wa(
        &[json!({ "type": "HEADER", "format": "LOCATION" })],
        &[("header.latitude", "10.7"), ("header.longitude", "106.7")],
    );

    assert_eq!(result.missing, NONE);
    assert_eq!(
        result.params["header"][0]["location"],
        json!({ "latitude": "10.7", "longitude": "106.7", "name": "", "address": "" })
    );
}

#[test]
fn whatsapp_template_parameters_carousel_cards_are_keyed_by_card_index() {
    let result = wa(
        &[json!({
            "type": "CAROUSEL",
            "cards": [{
                "card_index": 0,
                "components": [
                    { "type": "HEADER", "format": "IMAGE" },
                    { "type": "BODY", "text": "Card {{1}}" },
                ],
            }],
        })],
        &[
            ("card.0.header", "https://cdn.x.io/c.jpg"),
            ("card.0.body.1", "one"),
        ],
    );

    assert_eq!(result.missing, NONE);
    assert_eq!(
        result.params["carousel"],
        json!([{
            "card_index": 0,
            "header": [{ "type": "image", "image": { "link": "https://cdn.x.io/c.jpg" } }],
            "body": [{ "type": "text", "text": "one" }],
        }])
    );
}

#[test]
fn whatsapp_template_parameters_a_limited_time_offer_needs_a_positive_millisecond_timestamp() {
    let components =
        [json!({ "type": "LIMITED_TIME_OFFER", "limited_time_offer": { "has_expiration": true } })];

    assert_eq!(
        wa(&components, &[("offer.expiration_ms", "soon")]).invalid,
        vec!["offer.expiration_ms"]
    );
    assert_eq!(
        wa(&components, &[("offer.expiration_ms", "1767225600000")]).params["limited_time_offer"],
        json!({ "expiration_time_ms": 1_767_225_600_000_u64 })
    );
}

#[test]
fn whatsapp_template_parameters_a_multi_product_button_is_reported_as_unsupported() {
    let result = wa(
        &[json!({ "type": "BUTTONS", "buttons": [{ "type": "MPM", "text": "View" }] })],
        &[],
    );

    assert_eq!(result.unsupported, vec!["button.0"]);
}

// --- Messenger template parameters ---

fn messenger_components() -> Vec<Value> {
    vec![
        json!({ "type": "HEADER", "format": "TEXT", "text": "Hello {{1}}" }),
        json!({ "type": "BODY", "text": "Your code is {{1}}" }),
        json!({ "type": "BUTTONS", "buttons": [{ "type": "URL", "text": "Open", "url": "https://x.io/{{1}}" }] }),
    ]
}

#[test]
fn messenger_template_parameters_lists_header_body_and_url_button_keys() {
    assert_eq!(
        keys(&fc::describe_messenger_template_parameters(
            &messenger_components()
        )),
        vec!["header.1", "body.1", "button.0"]
    );
}

#[test]
fn messenger_template_parameters_builds_positional_params_without_parameter_name() {
    let result = messenger(
        &messenger_components(),
        "POSITIONAL",
        &[("header.1", "Ann"), ("body.1", "1234"), ("button.0", "abc")],
    );

    assert_eq!(result.missing, NONE);
    assert_eq!(
        result.params,
        json!({
            "header": [{ "type": "text", "text": "Ann" }],
            "body": [{ "text": "1234" }],
            "button": [{ "sub_type": "url", "index": 0, "text": "abc" }],
        })
    );
}

// --- template parameter edge cases ---

#[test]
fn template_parameter_edge_cases_values_are_sent_as_given_only_blank_counts_as_missing() {
    let result = wa(
        &[json!({ "type": "BODY", "text": "Hi {{1}}" })],
        &[("body.1", " Ann ")],
    );
    assert_eq!(
        result.params["body"],
        json!([{ "type": "text", "text": " Ann " }])
    );

    let result = messenger(
        &[json!({ "type": "BODY", "text": "Hi {{1}}" })],
        "POSITIONAL",
        &[("body.1", " Ann ")],
    );
    assert_eq!(result.params["body"], json!([{ "text": " Ann " }]));
}

#[test]
fn template_parameter_edge_cases_a_placeholder_used_twice_is_one_key_filling_both_slots() {
    let components = [json!({ "type": "BODY", "text": "{{1}} and again {{1}}" })];

    assert_eq!(fc::describe_wa_template_parameters(&components).len(), 1);
    assert_eq!(
        wa(&components, &[("body.1", "Ann")]).params["body"],
        json!([{ "type": "text", "text": "Ann" }, { "type": "text", "text": "Ann" }])
    );
}

#[test]
fn template_parameter_edge_cases_a_template_without_variables_needs_no_keys() {
    let components = [
        json!({ "type": "BODY", "text": "Thanks for your order" }),
        json!({ "type": "BUTTONS", "buttons": [{ "type": "QUICK_REPLY", "text": "OK" }] }),
    ];

    assert!(fc::describe_wa_template_parameters(&components).is_empty());
    let result = wa(&components, &[]);
    assert_eq!(result.params, json!({}));
    assert_eq!(result.missing, NONE);
    assert_eq!(result.unknown, NONE);
}

#[test]
fn template_parameter_edge_cases_catalog_is_optional_flow_buttons_keep_the_builder_skeleton() {
    let components = [json!({
        "type": "BUTTONS",
        "buttons": [
            { "type": "CATALOG", "text": "Shop" },
            { "type": "FLOW", "text": "Book", "flow_id": 123, "navigate_screen": "S1" },
        ],
    })];

    assert_eq!(
        fc::describe_wa_template_parameters(&components),
        vec![spec("button.0", "button", "catalog_product", false)]
    );
    let result = wa(&components, &[]);
    assert_eq!(result.missing, NONE);
    assert_eq!(
        result.params["button"],
        json!([
            { "sub_type": "catalog", "index": 0, "thumbnail_product_retailer_id": "" },
            { "sub_type": "flow", "index": 1, "flowSourceId": "123", "navigateScreenId": "S1", "fieldMappings": [] },
        ])
    );
}

#[test]
fn template_parameter_edge_cases_carousel_card_buttons_are_keyed_by_card_and_button_index() {
    let result = wa(
        &[json!({
            "type": "CAROUSEL",
            "cards": [{
                "card_index": 1,
                "components": [{
                    "type": "BUTTONS",
                    "buttons": [
                        { "type": "QUICK_REPLY", "text": "Hi" },
                        { "type": "URL", "text": "Go", "url": "https://x.io/{{1}}" },
                    ],
                }],
            }],
        })],
        &[("card.1.button.1", "sku-9")],
    );

    assert_eq!(result.missing, NONE);
    assert_eq!(
        result.params["carousel"],
        json!([{ "card_index": 1, "button": [{ "sub_type": "url", "index": 1, "text": "sku-9" }] }])
    );
}

#[test]
fn template_parameter_edge_cases_a_messenger_image_header_with_a_variable_asks_for_the_header_text()
{
    let components = [json!({ "type": "HEADER", "format": "IMAGE", "text": "Hello {{1}}" })];

    assert_eq!(
        keys(&fc::describe_messenger_template_parameters(&components)),
        vec!["header.1"]
    );
    assert_eq!(
        messenger(&components, "POSITIONAL", &[("header.1", "Ann")]).params["header"],
        json!([{ "type": "text", "text": "Ann" }])
    );
}

#[test]
fn template_parameter_edge_cases_every_described_key_is_accepted_by_apply_and_nothing_else() {
    let mut components = positional();
    components.push(
        json!({ "type": "LIMITED_TIME_OFFER", "limited_time_offer": { "has_expiration": true } }),
    );
    components.push(json!({
        "type": "CAROUSEL",
        "cards": [{
            "card_index": 0,
            "components": [{ "type": "HEADER", "format": "VIDEO" }, { "type": "BODY", "text": "{{1}}" }],
        }],
    }));
    let values: BTreeMap<String, String> = fc::describe_wa_template_parameters(&components)
        .into_iter()
        .map(|s| {
            let value = if s.kind == "expiration_ms" {
                "1893456000000"
            } else {
                "v"
            };
            (s.key, value.to_string())
        })
        .collect();

    let result = fc::apply_wa_template_parameter_values(&components, &values);

    assert_eq!(result.missing, NONE);
    assert_eq!(result.unknown, NONE);
    assert_eq!(result.invalid, NONE);
    assert_eq!(result.unsupported, NONE);
}

// --- payment request template parameters ---

fn pix() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Copy Pix code", "payment_setting": { "type": "pix_dynamic_code" } })
}
fn boleto() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Copy Boleto code" })
}
fn link() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Open payment link" })
}
fn card() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Review payment" })
}
fn with_buttons(buttons: &[Value]) -> Vec<Value> {
    vec![
        json!({ "type": "BODY", "text": "Pay" }),
        json!({ "type": "BUTTONS", "buttons": buttons }),
    ]
}

#[test]
fn payment_request_template_parameters_lists_the_per_customer_keys_of_each_method_never_the_method_itself()
 {
    assert_eq!(
        fc::describe_wa_template_parameters(&with_buttons(&[pix()])),
        vec![
            spec("button.0", "button", "pix_code", true),
            spec("button.0.amount", "button", "payment_amount", false),
        ]
    );
    let kinds = |b: Value| -> Vec<String> {
        fc::describe_wa_template_parameters(&with_buttons(&[b]))
            .into_iter()
            .map(|s| s.kind)
            .collect()
    };
    assert_eq!(kinds(boleto()), vec!["boleto_digitable_line"]);
    assert_eq!(kinds(link()), vec!["payment_link_url"]);
    let triples: Vec<(String, String, bool)> =
        fc::describe_wa_template_parameters(&with_buttons(&[card()]))
            .into_iter()
            .map(|s| (s.key, s.kind, s.required))
            .collect();
    assert_eq!(
        triples,
        vec![
            (
                "button.0".to_string(),
                "card_last_four_digits".to_string(),
                true
            ),
            (
                "button.0.amount".to_string(),
                "payment_amount".to_string(),
                true
            ),
            (
                "button.0.credential_id".to_string(),
                "card_credential_id".to_string(),
                false
            ),
        ]
    );
}

#[test]
fn payment_request_template_parameters_fills_the_payment_request_of_the_button_at_its_own_index() {
    let result = wa(
        &with_buttons(&[json!({ "type": "QUICK_REPLY", "text": "Later" }), pix()]),
        &[("button.1", "000201"), ("button.1.amount", "12.34")],
    );

    assert_eq!(result.missing, NONE);
    assert_eq!(result.invalid, NONE);
    assert_eq!(
        result.params["button"],
        json!([{
            "sub_type": "payment_request",
            "index": 1,
            "payment_request": { "method": "pix", "code": "000201", "amount": "12.34" },
        }])
    );
}

#[test]
fn payment_request_template_parameters_a_missing_required_value_is_reported_an_optional_one_is_not()
{
    assert_eq!(wa(&with_buttons(&[pix()]), &[]).missing, vec!["button.0"]);

    assert_eq!(
        wa(&with_buttons(&[card()]), &[("button.0", "4242")]).missing,
        vec!["button.0.amount"]
    );
}

#[test]
fn payment_request_template_parameters_malformed_literals_are_invalid_variables_are_accepted() {
    let result = wa(
        &with_buttons(&[link(), card()]),
        &[
            ("button.0", "not a url"),
            ("button.1", "42"),
            ("button.1.amount", "{{total}}"),
        ],
    );
    assert_eq!(result.invalid, vec!["button.0", "button.1"]);

    let ok = wa(&with_buttons(&[link()]), &[("button.0", "{{pay_url}}")]);
    assert_eq!(ok.invalid, NONE);
    assert_eq!(
        ok.params["button"][0]["payment_request"],
        json!({ "method": "payment_link", "url": "{{pay_url}}" })
    );
}

#[test]
fn payment_request_template_parameters_a_button_whose_method_cannot_be_told_is_unsupported() {
    let pay_now = json!({ "type": "PAYMENT_REQUEST", "text": "Pay now" });

    assert_eq!(
        wa(&with_buttons(std::slice::from_ref(&pay_now)), &[]).unsupported,
        vec!["button.0"]
    );
    assert!(fc::describe_wa_template_parameters(&with_buttons(&[pay_now])).is_empty());
}

#[test]
fn payment_request_template_parameters_a_payment_button_inside_a_carousel_card_is_unsupported() {
    let components = [json!({
        "type": "CAROUSEL",
        "cards": [{ "card_index": 0, "components": [{ "type": "BUTTONS", "buttons": [pix()] }] }],
    })];

    assert_eq!(wa(&components, &[]).unsupported, vec!["card.0.button.0"]);
}

#[test]
fn payment_request_template_parameters_keys_of_another_method_are_unknown() {
    let result = wa(
        &with_buttons(&[boleto()]),
        &[("button.0", "1234"), ("button.0.amount", "10")],
    );

    assert_eq!(result.unknown, vec!["button.0.amount"]);
}
