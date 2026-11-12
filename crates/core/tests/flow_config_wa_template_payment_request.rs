//! Port of `packages/flow-config/__tests__/wa-template-payment-request.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::CollectPaymentRequestIssuesOptions;

fn pix_button() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Copy Pix code", "payment_setting": { "type": "pix_dynamic_code" } })
}

fn boleto_button() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Copy Boleto code" })
}

fn quick_reply() -> Value {
    json!({ "type": "QUICK_REPLY", "text": "Later" })
}

#[test]
fn resolvewapaymentrequestmethod_j_s() {
    let cases = [
        (
            json!({ "text": "Copy Pix code", "payment_setting": { "type": "pix_dynamic_code" } }),
            "pix",
        ),
        (
            json!({ "text": "Copy Boleto code", "payment_setting": { "type": "boleto" } }),
            "boleto",
        ),
        (
            json!({ "text": "Open payment link", "payment_setting": { "type": "payment_link" } }),
            "payment_link",
        ),
        (
            json!({ "text": "Review payment", "payment_setting": { "type": "offsite_card_pay" } }),
            "card_on_file",
        ),
        // type absent → the fixed label decides
        (json!({ "text": "Copy Pix code" }), "pix"),
        (json!({ "text": "  copy boleto CODE " }), "boleto"),
        // Meta returns the type upper-case when a template is read
        (
            json!({ "text": "Copy Pix code", "payment_setting": { "type": "PIX_DYNAMIC_CODE" } }),
            "pix",
        ),
        (
            json!({ "text": "Copy Boleto code", "payment_setting": { "type": "BOLETO" } }),
            "boleto",
        ),
        (
            json!({ "text": "Open payment link", "payment_setting": { "type": "PAYMENT_LINK" } }),
            "payment_link",
        ),
        (
            json!({ "text": "Review payment", "payment_setting": { "type": "OFFSITE_CARD_PAY" } }),
            "card_on_file",
        ),
        // known type decides; a localized or missing label is not a contradiction
        (
            json!({ "text": "", "payment_setting": { "type": "boleto" } }),
            "boleto",
        ),
        (
            json!({ "text": "Copiar código Pix", "payment_setting": { "type": "pix_dynamic_code" } }),
            "pix",
        ),
        (
            json!({ "text": "Abrir link de pagamento", "payment_setting": { "type": "payment_link" } }),
            "payment_link",
        ),
    ];
    for (button, expected) in cases {
        assert_eq!(
            fc::resolve_wa_payment_request_method(&button),
            Some(expected),
            "{button}"
        );
    }
}

#[test]
fn resolvewapaymentrequestmethod_j_unsupported() {
    let cases = [
        // unknown type never falls back to the label
        json!({ "text": "Copy Pix code", "payment_setting": { "type": "future_method" } }),
        // known type conflicting with another method's label
        json!({ "text": "Copy Boleto code", "payment_setting": { "type": "pix_dynamic_code" } }),
        // known type conflicting with a different method's documented label
        json!({ "text": "Open payment link", "payment_setting": { "type": "offsite_card_pay" } }),
        // no type: only the fixed English labels are trusted
        json!({ "text": "Copiar código Pix" }),
        json!({ "text": "Pay now" }),
        json!({ "text": "" }),
    ];
    for button in cases {
        assert_eq!(
            fc::resolve_wa_payment_request_method(&button),
            None,
            "{button}"
        );
    }
}

#[test]
fn extracttemplateparams_payment_request_seeds_a_blank_request_of_the_resolved_method_keeping_the_button_index()
 {
    let components = [
        json!({ "type": "BODY", "text": "Pay" }),
        json!({
            "type": "BUTTONS",
            "buttons": [{ "type": "URL", "text": "Static", "url": "https://a.test" }, pix_button(), quick_reply()],
        }),
    ];
    assert_eq!(
        fc::extract_template_params(&components)["button"],
        json!([{ "sub_type": "payment_request", "index": 1, "payment_request": { "method": "pix", "code": "" } }])
    );
}

#[test]
fn extracttemplateparams_payment_request_an_unresolvable_payment_button_is_not_seeded() {
    let components = [
        json!({ "type": "BUTTONS", "buttons": [{ "type": "PAYMENT_REQUEST", "text": "Pay now" }] }),
    ];
    assert!(
        fc::extract_template_params(&components)
            .get("button")
            .is_none()
    );
}

#[test]
fn extracttemplateparams_payment_request_carousel_card_payment_buttons_are_not_supported() {
    let components = [json!({
        "type": "CAROUSEL",
        "cards": [{ "card_index": 0, "components": [{ "type": "BUTTONS", "buttons": [pix_button()] }] }],
    })];
    assert_eq!(
        fc::extract_template_params(&components)["carousel"][0]["button"],
        json!([])
    );
}

#[test]
fn extracttemplateparams_payment_request_payment_buttons_coexist_with_limited_time_offer_and_document_header()
 {
    let params = fc::extract_template_params(&[
        json!({ "type": "HEADER", "format": "DOCUMENT" }),
        json!({ "type": "BODY", "text": "Hi" }),
        json!({ "type": "LIMITED_TIME_OFFER", "limited_time_offer": { "has_expiration": true } }),
        json!({ "type": "BUTTONS", "buttons": [boleto_button()] }),
    ]);
    assert_eq!(params["header"][0]["type"], "document");
    assert_eq!(
        params["limited_time_offer"],
        json!({ "expiration_time_ms": 0 })
    );
    assert_eq!(params["button"][0]["payment_request"]["method"], "boleto");
}

fn parse(params: Value) -> SafeParse {
    fc::wa_template_params_schema().safe_parse(&params)
}

#[test]
fn watemplateparamsschema_payment_request_shape_accepts_a_payment_button_with_its_data() {
    assert!(
        parse(json!({
            "button": [{ "sub_type": "payment_request", "index": 0, "payment_request": { "method": "pix", "code": "{{pix}}" } }],
        }))
        .success
    );
}

#[test]
fn watemplateparamsschema_payment_request_shape_rejects_s() {
    let cases = [
        (
            "payment data on a non-payment sub_type",
            json!({ "sub_type": "url", "index": 0, "text": "x", "payment_request": { "method": "pix", "code": "x" } }),
        ),
        (
            "payment sub_type without data",
            json!({ "sub_type": "payment_request", "index": 0 }),
        ),
        (
            "payment sub_type without an index",
            json!({ "sub_type": "payment_request", "payment_request": { "method": "pix", "code": "x" } }),
        ),
        (
            "unknown method",
            json!({ "sub_type": "payment_request", "index": 0, "payment_request": { "method": "crypto", "code": "x" } }),
        ),
    ];
    for (name, button) in cases {
        assert!(!parse(json!({ "button": [button] })).success, "{name}");
    }
}

#[test]
fn watemplateparamsschema_payment_request_shape_rejects_a_payment_request_inside_a_carousel_card() {
    let result = parse(json!({
        "carousel": [{
            "card_index": 0,
            "button": [{ "sub_type": "payment_request", "index": 0, "payment_request": { "method": "pix", "code": "x" } }],
        }],
    }));
    assert!(!result.success);
    assert_eq!(
        result.issues()[0].message,
        "waTemplatePaymentRequestMisplaced"
    );
}

#[test]
fn watemplateparamsschema_payment_request_shape_params_saved_before_payment_support_null_holes_no_payment_data_still_parse()
 {
    let result = parse(json!({ "button": [null, { "sub_type": "url", "index": 1, "text": "x" }] }));
    assert!(result.success);
    assert_eq!(
        result.data.unwrap()["button"],
        json!([{ "sub_type": "url", "index": 1, "text": "x" }])
    );
}

fn issues_for(params: Value) -> Vec<String> {
    let refined_step_schema = fc::send_wa_template_message_validator().base();
    refined_step_schema
        .safe_parse(&json!({
            "id": "1",
            "nodeId": "n1",
            "stepType": "sendWaTemplateMessage",
            "template": { "id": "t", "name": "t", "language": "en", "inboxId": null, "params": params },
            "buttons": [],
        }))
        .messages()
}

fn with_request(payment_request: Value) -> Value {
    fc::wa_template_params_schema()
        .parse(&json!({ "button": [{ "sub_type": "payment_request", "index": 0, "payment_request": payment_request }] }))
        .unwrap()
}

#[test]
fn publish_validation_of_payment_requests_a_blank_seeded_request_blocks_publish() {
    assert_eq!(
        issues_for(with_request(json!({ "method": "pix", "code": "" }))),
        vec!["waTemplatePaymentRequestIncomplete"]
    );
}

#[test]
fn publish_validation_of_payment_requests_a_malformed_value_blocks_publish() {
    assert_eq!(
        issues_for(with_request(
            json!({ "method": "payment_link", "url": "nope" })
        )),
        vec!["waTemplatePaymentRequestInvalid"]
    );
}

#[test]
fn publish_validation_of_payment_requests_variables_and_valid_values_pass() {
    assert!(
        issues_for(with_request(
            json!({ "method": "pix", "code": "{{pix_code}}" })
        ))
        .is_empty()
    );
    assert!(
        issues_for(with_request(
            json!({ "method": "payment_link", "url": "https://pay.test/1" })
        ))
        .is_empty()
    );
}

fn mismatch_components() -> Vec<Value> {
    vec![
        json!({ "type": "BODY", "text": "Pay" }),
        json!({ "type": "BUTTONS", "buttons": [quick_reply(), pix_button(), boleto_button()] }),
    ]
}

fn entry(index: usize, method: &str) -> Value {
    let request = match method {
        "pix" => json!({ "method": "pix", "code": "x" }),
        "boleto" => json!({ "method": "boleto", "digitableLine": "1" }),
        "payment_link" => json!({ "method": "payment_link", "url": "https://a.test" }),
        other => panic!("{other}"),
    };
    json!({ "sub_type": "payment_request", "index": index, "payment_request": request })
}

fn params(buttons: Vec<Value>) -> Value {
    json!({ "button": buttons })
}

#[test]
fn findpaymentrequestmismatches_exact_one_to_one_match_in_any_order_has_no_mismatch() {
    assert!(
        fc::find_payment_request_mismatches(
            &params(vec![entry(2, "boleto"), entry(1, "pix")]),
            &mismatch_components()
        )
        .is_empty()
    );
}

#[test]
fn findpaymentrequestmismatches_reports_s() {
    let cases = [
        (
            "missing entry",
            params(vec![entry(1, "pix")]),
            json!([{ "index": 2, "reason": "missing" }]),
        ),
        (
            "duplicate entry",
            params(vec![entry(1, "pix"), entry(1, "pix"), entry(2, "boleto")]),
            json!([{ "index": 1, "reason": "duplicate" }]),
        ),
        (
            "wrong method",
            params(vec![entry(1, "boleto"), entry(2, "boleto")]),
            json!([{ "index": 1, "reason": "method_mismatch" }]),
        ),
        (
            "extra entry",
            params(vec![entry(1, "pix"), entry(2, "boleto"), entry(0, "pix")]),
            json!([{ "index": 0, "reason": "extra" }]),
        ),
    ];
    for (name, sent, expected) in cases {
        assert_eq!(
            Value::Array(fc::find_payment_request_mismatches(
                &sent,
                &mismatch_components()
            )),
            expected,
            "{name}"
        );
    }
}

#[test]
fn findpaymentrequestmismatches_a_template_payment_button_of_unknown_method_can_never_be_sent() {
    assert_eq!(
        fc::find_payment_request_mismatches(
            &params(vec![]),
            &[
                json!({ "type": "BUTTONS", "buttons": [{ "type": "PAYMENT_REQUEST", "text": "Pay now" }] })
            ],
        ),
        vec![json!({ "index": 0, "reason": "unsupported_template_button" })]
    );
}

#[test]
fn findpaymentrequestmismatches_a_template_without_payment_buttons_only_flags_stray_entries() {
    let body = [json!({ "type": "BODY", "text": "Hi" })];
    assert!(fc::find_payment_request_mismatches(&params(vec![]), &body).is_empty());
    assert_eq!(
        fc::find_payment_request_mismatches(&params(vec![entry(0, "pix")]), &body),
        vec![json!({ "index": 0, "reason": "extra" })]
    );
}

fn key(button: Value) -> String {
    fc::template_structure_key(&json!({
        "name": "pay",
        "language": "pt_BR",
        "category": "UTILITY",
        "components": [{ "type": "BUTTONS", "buttons": [button] }],
    }))
}

#[test]
fn templatestructurekey_payment_buttons_templates_with_different_payment_methods_never_share_a_structure()
 {
    assert_ne!(key(pix_button()), key(boleto_button()));
}

#[test]
fn templatestructurekey_payment_buttons_same_method_with_or_without_payment_setting_shares_a_structure()
 {
    assert_eq!(
        key(pix_button()),
        key(json!({ "type": "PAYMENT_REQUEST", "text": "Copy Pix code" }))
    );
}

#[test]
fn templatestructurekey_payment_buttons_non_payment_buttons_keep_their_structure_key() {
    assert!(!key(json!({ "type": "QUICK_REPLY", "text": "Hi" })).contains("paymentMethod"));
}

#[test]
fn payment_request_mixed_with_quick_replies_the_payment_slot_keeps_its_template_index_and_the_quick_replies_bind_around_it()
 {
    let components = [json!({
        "type": "BUTTONS",
        "buttons": [{ "type": "QUICK_REPLY", "text": "Later" }, pix_button(), { "type": "QUICK_REPLY", "text": "Stop" }],
    })];
    let params = fc::extract_template_params(&components);
    assert_eq!(
        params["button"],
        json!([{ "sub_type": "payment_request", "index": 1, "payment_request": { "method": "pix", "code": "" } }])
    );

    let later_button = fc::button_step_default_fn(&json!({ "label": "Later" }));
    let stop_button = fc::button_step_default_fn(&json!({ "label": "Stop" }));
    let bound = fc::bind_wa_template_quick_reply_buttons(
        &components,
        &[
            fc::button_step_default_fn(&json!({ "label": "Delivered" })),
            fc::button_step_default_fn(&json!({ "label": "Failed" })),
            later_button.clone(),
            stop_button.clone(),
        ],
    );

    assert_eq!(
        bound,
        vec![
            json!({ "templateButtonIndex": 0, "stepButton": later_button }),
            json!({ "templateButtonIndex": 2, "stepButton": stop_button }),
        ]
    );
}

#[test]
fn payment_link_url_rule_accepts_exactly_what_zodurlwithvariables_accepts_j() {
    for url in [
        "https://pay.test/1",
        "myapp://pay/1",
        "{{pay_url}}",
        "https://pay.test/{{id}}",
        "not a url",
        "pay.test/1",
        "",
    ] {
        let issues = fc::collect_payment_request_issues(
            &json!({ "method": "payment_link", "url": url }),
            CollectPaymentRequestIssuesOptions {
                allow_variables: true,
            },
        );
        let is_url_format_valid = !issues
            .iter()
            .any(|issue| issue.field == "url" && issue.kind == "invalid");

        // A blank value is a "required" issue, not a format one.
        assert_eq!(
            is_url_format_valid,
            url.trim().is_empty()
                || fc::deps::zod_url_with_variables(None)
                    .safe_parse(&json!(url))
                    .success,
            "{url:?}"
        );
    }
}

fn synced_components() -> Vec<Value> {
    // Captured from a real approved pt_BR Pix template (sample code shortened):
    // English label, upper-case payment_setting.type.
    vec![
        json!({ "type": "BODY", "text": "Olá! Seu pedido está pronto." }),
        json!({
            "type": "BUTTONS",
            "buttons": [{ "type": "PAYMENT_REQUEST", "text": "Copy Pix code", "payment_setting": { "type": "PIX_DYNAMIC_CODE" } }],
        }),
    ]
}

#[test]
fn a_payment_template_exactly_as_meta_returned_it_when_synced_is_recognised_as_a_pix_button_and_seeded_with_a_blank_pix_request()
 {
    assert_eq!(
        fc::extract_template_params(&synced_components())["button"],
        json!([{ "sub_type": "payment_request", "index": 0, "payment_request": { "method": "pix", "code": "" } }])
    );
}

#[test]
fn a_payment_template_exactly_as_meta_returned_it_when_synced_matches_a_pix_request_one_to_one_and_rejects_another_method()
 {
    let pix = json!({
        "button": [{ "sub_type": "payment_request", "index": 0, "payment_request": { "method": "pix", "code": "x" } }],
    });
    assert!(fc::find_payment_request_mismatches(&pix, &synced_components()).is_empty());
    assert_eq!(
        fc::find_payment_request_mismatches(
            &json!({
                "button": [{ "sub_type": "payment_request", "index": 0, "payment_request": { "method": "boleto", "digitableLine": "1" } }],
            }),
            &synced_components(),
        ),
        vec![json!({ "index": 0, "reason": "method_mismatch" })]
    );
}

#[test]
fn a_payment_template_exactly_as_meta_returned_it_when_synced_shares_a_structure_key_with_the_same_template_written_in_lower_case()
 {
    let key = |kind: &str| {
        fc::template_structure_key(&json!({
            "name": "pagamento_pix_teste",
            "language": "pt_BR",
            "category": "UTILITY",
            "components": [{
                "type": "BUTTONS",
                "buttons": [{ "type": "PAYMENT_REQUEST", "text": "Copy Pix code", "payment_setting": { "type": kind } }],
            }],
        }))
    };

    assert_eq!(key("PIX_DYNAMIC_CODE"), key("pix_dynamic_code"));
}
