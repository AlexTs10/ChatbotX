//! Port of `packages/flow-config/__tests__/messenger-template.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn params(components: Value, format: &str) -> Value {
    fc::extract_messenger_template_params(components.as_array().unwrap(), format)
}

fn infos(components: Value, format: &str) -> Vec<Value> {
    fc::extract_messenger_parameter_infos(components.as_array().unwrap(), format)
}

fn flow_buttons(components: Value) -> Vec<Value> {
    fc::extract_messenger_flow_buttons(components.as_array().unwrap(), false)
}

fn buttons(list: Value) -> Value {
    json!([{ "type": "BUTTONS", "buttons": list }])
}

fn normalized(url: &str, variable: &'static str) -> Option<(String, &'static str)> {
    Some((url.to_string(), variable))
}

// ─── extractMessengerTemplateParams ───

#[test]
fn extractmessengertemplateparams_empty_components_returns() {
    assert_eq!(params(json!([]), "POSITIONAL"), json!({}));
}

#[test]
fn extractmessengertemplateparams_header_text_positional_no_parameter_name() {
    let component = json!({ "type": "HEADER", "format": "TEXT", "text": "Hello {{1}}" });
    let result = params(json!([component]), "POSITIONAL");
    assert_eq!(result["header"], json!([{ "type": "text", "text": "" }]));
}

#[test]
fn extractmessengertemplateparams_header_text_named_includes_parameter_name() {
    let named = json!({ "type": "HEADER", "format": "TEXT", "text": "Hello {{name}}" });
    let result = params(json!([named]), "NAMED");
    assert_eq!(
        result["header"],
        json!([{ "type": "text", "text": "", "parameter_name": "name" }])
    );
}

#[test]
fn extractmessengertemplateparams_header_text_no_variables_in_text_header_stays_undefined() {
    let no_vars = json!({ "type": "HEADER", "format": "TEXT", "text": "Static header" });
    let result = params(json!([no_vars]), "POSITIONAL");
    assert!(result.get("header").is_none());
}

#[test]
fn extractmessengertemplateparams_header_image_returns_no_header_params_image_is_fixed_at_template_creation_not_parameterized_at_send_time()
 {
    let component = json!({ "type": "HEADER", "format": "IMAGE" });
    assert!(
        params(json!([component.clone()]), "POSITIONAL")
            .get("header")
            .is_none()
    );
    assert!(params(json!([component]), "NAMED").get("header").is_none());
}

#[test]
fn extractmessengertemplateparams_header_image_image_static_text_no_header_params() {
    let component = json!({ "type": "HEADER", "format": "IMAGE", "text": "Order update" });
    assert!(
        params(json!([component]), "POSITIONAL")
            .get("header")
            .is_none()
    );
}

// Meta rejects the send with (#100 - 1893029) "Missing one or more header
// params" when an image header's text variable is not passed.
#[test]
fn extractmessengertemplateparams_header_image_image_text_variable_positional_collects_a_header_text_param()
 {
    let component = json!({ "type": "HEADER", "format": "IMAGE", "text": "{{1}}" });
    assert_eq!(
        params(json!([component]), "POSITIONAL")["header"],
        json!([{ "type": "text", "text": "" }])
    );
}

#[test]
fn extractmessengertemplateparams_header_image_image_text_variable_named_includes_parameter_name() {
    let component = json!({ "type": "HEADER", "format": "IMAGE", "text": "{{order_type}} Update" });
    assert_eq!(
        params(json!([component]), "NAMED")["header"],
        json!([{ "type": "text", "text": "", "parameter_name": "order_type" }])
    );
}

#[test]
fn extractmessengertemplateparams_stored_param_shape_positional_params_carry_no_parameter_name_key_at_all()
 {
    let result = params(
        json!([
            { "type": "HEADER", "format": "IMAGE", "text": "{{1}}" },
            { "type": "BODY", "text": "Hi {{1}}" },
        ]),
        "POSITIONAL",
    );
    assert_eq!(result["header"][0], json!({ "type": "text", "text": "" }));
    assert_eq!(result["body"][0], json!({ "text": "" }));
}

#[test]
fn extractmessengertemplateparams_stored_param_shape_components_without_text_produce_no_header_or_body_params()
 {
    let result = params(
        json!([{ "type": "HEADER", "format": "IMAGE" }, { "type": "BODY" }]),
        "POSITIONAL",
    );
    assert_eq!(result, json!({}));
}

#[test]
fn extractmessengertemplateparams_body_positional_two_vars_two_empty_text_items_no_parameter_name()
{
    let component = json!({ "type": "BODY", "text": "Hi {{1}} and {{2}}" });
    assert_eq!(
        params(json!([component]), "POSITIONAL")["body"],
        json!([{ "text": "" }, { "text": "" }])
    );
}

#[test]
fn extractmessengertemplateparams_body_named_each_item_has_parameter_name() {
    let component =
        json!({ "type": "BODY", "text": "Hi {{first_name}} your order {{order_id}} is ready" });
    assert_eq!(
        params(json!([component]), "NAMED")["body"],
        json!([
            { "text": "", "parameter_name": "first_name" },
            { "text": "", "parameter_name": "order_id" },
        ])
    );
}

#[test]
fn extractmessengertemplateparams_body_body_with_no_variables_body_stays_undefined() {
    let component = json!({ "type": "BODY", "text": "Static body" });
    assert!(
        params(json!([component]), "POSITIONAL")
            .get("body")
            .is_none()
    );
}

#[test]
fn extractmessengertemplateparams_buttons_url_button_with_1_sub_type_url() {
    let result = params(
        buttons(json!([{ "type": "URL", "text": "Track", "url": "https://example.com/{{1}}" }])),
        "POSITIONAL",
    );
    assert_eq!(
        result["button"],
        json!([{ "sub_type": "url", "index": 0, "text": "" }])
    );
}

#[test]
fn extractmessengertemplateparams_buttons_url_button_without_1_in_url_uses_fixed_url_as_send_param()
{
    let result = params(
        buttons(json!([{ "type": "URL", "text": "Go", "url": "https://example.com/fixed" }])),
        "POSITIONAL",
    );
    assert_eq!(
        result["button"],
        json!([{ "sub_type": "url", "index": 0, "text": "https://example.com/fixed" }])
    );
}

// Meta's NAMED templates keep a named URL suffix ({{url_suffix}}); the
// send-time URL parameter is still just the suffix value.
#[test]
fn extractmessengertemplateparams_buttons_named_url_button_with_url_suffix_suffix_left_empty_to_fill()
 {
    let result = params(
        buttons(json!([{
            "type": "URL",
            "text": "Track Order",
            "url": "http://www.example.com/orders/{{url_suffix}}",
        }])),
        "NAMED",
    );
    assert_eq!(
        result["button"],
        json!([{ "sub_type": "url", "index": 0, "text": "" }])
    );
}

#[test]
fn extractmessengertemplateparams_buttons_postback_with_fixed_payload_no_not_added() {
    let result = params(
        buttons(json!([{ "type": "POSTBACK", "text": "Confirm", "payload": "CONFIRM" }])),
        "POSITIONAL",
    );
    assert!(result.get("button").is_none());
}

#[test]
fn extractmessengertemplateparams_buttons_postback_with_n_in_payload_not_added_becomes_a_flow_button_not_a_param()
 {
    let result = params(
        buttons(json!([{ "type": "POSTBACK", "text": "Track", "payload": "order_id_{{1}}" }])),
        "POSITIONAL",
    );
    assert!(result.get("button").is_none());
}

#[test]
fn extractmessengertemplateparams_buttons_quick_reply_with_fixed_payload_not_added() {
    let result = params(
        buttons(json!([{ "type": "QUICK_REPLY", "text": "Yes", "payload": "YES" }])),
        "POSITIONAL",
    );
    assert!(result.get("button").is_none());
}

#[test]
fn extractmessengertemplateparams_buttons_mixed_url_postback_buttons_only_url_button_produces_a_param()
 {
    let result = params(
        buttons(json!([
            { "type": "POSTBACK", "text": "Confirm", "payload": "CONFIRM" },
            { "type": "URL", "text": "Track", "url": "https://example.com/{{1}}" },
        ])),
        "POSITIONAL",
    );
    assert_eq!(result["button"].as_array().unwrap().len(), 1);
    assert_eq!(result["button"][0]["sub_type"], "url");
    assert_eq!(result["button"][0]["index"], 1);
}

#[test]
fn extractmessengertemplateparams_buttons_postback_phone_number_static_url_phone_and_url_produce_send_params()
 {
    let result = params(
        buttons(json!([
            { "type": "POSTBACK", "text": "Detail", "payload": "{{1}}" },
            { "type": "PHONE_NUMBER", "text": "Phone", "phone_number": "+84349566551" },
            { "type": "URL", "text": "Visit website", "url": "https://ahachat.com/" },
        ])),
        "POSITIONAL",
    );
    assert_eq!(
        result["button"],
        json!([
            { "sub_type": "phone_number", "index": 1 },
            { "sub_type": "url", "index": 2, "text": "https://ahachat.com/" },
        ])
    );
}

// ─── suggestMessengerUrlSuffixVariable ───

#[test]
fn suggestmessengerurlsuffixvariable_mini_app_link_suggests_mini_app_token() {
    assert_eq!(
        fc::suggest_messenger_url_suffix_variable(Some(
            "https://app.example.com/mini-apps?id=12&token={{1}}"
        )),
        "{{mini_app_token}}"
    );
}

#[test]
fn suggestmessengerurlsuffixvariable_minigame_link_suggests_minigame_play_token() {
    assert_eq!(
        fc::suggest_messenger_url_suffix_variable(Some(
            "https://app.example.com/minigames?minigameId=abc&token={{1}}"
        )),
        "{{minigame_play_token}}"
    );
}

#[test]
fn suggestmessengerurlsuffixvariable_other_links_suggest_nothing() {
    assert_eq!(
        fc::suggest_messenger_url_suffix_variable(Some("https://example.com/{{1}}")),
        ""
    );
    assert_eq!(fc::suggest_messenger_url_suffix_variable(None), "");
}

#[test]
fn suggestmessengerurlsuffixvariable_extractmessengertemplateparams_defaults_a_mini_app_suffix_to_the_token()
 {
    let result = params(
        buttons(json!([{
            "type": "URL",
            "text": "Open",
            "url": "https://app.example.com/mini-apps?id=12&token={{1}}",
        }])),
        "POSITIONAL",
    );
    assert_eq!(
        result["button"],
        json!([{ "sub_type": "url", "index": 0, "text": "{{mini_app_token}}" }])
    );
}

// ─── normalizeMessengerTokenUrl / findMessengerTokenVariable ───

#[test]
fn normalizemessengertokenurl_swaps_a_mini_app_token_suffix_for_1() {
    assert_eq!(
        fc::normalize_messenger_token_url(
            "https://app.example.com/mini-apps?id=12&token={{mini_app_token}}"
        ),
        normalized(
            "https://app.example.com/mini-apps?id=12&token={{1}}",
            "mini_app_token"
        )
    );
}

#[test]
fn normalizemessengertokenurl_swaps_a_minigame_token_suffix_for_1() {
    assert_eq!(
        fc::normalize_messenger_token_url(
            "https://app.example.com/minigames?minigameId=abc&token={{ minigame_play_token }}"
        ),
        normalized(
            "https://app.example.com/minigames?minigameId=abc&token={{1}}",
            "minigame_play_token"
        )
    );
}

#[test]
fn normalizemessengertokenurl_every_normalized_link_gets_its_token_back_at_send_time() {
    let normalized = fc::normalize_messenger_token_url(
        "https://app.example.com/mini-apps?id=12&token={{mini_app_token}}",
    );
    assert_eq!(
        fc::suggest_messenger_url_suffix_variable(normalized.as_ref().map(|(url, _)| url.as_str())),
        "{{mini_app_token}}"
    );
}

#[test]
fn normalizemessengertokenurl_returns_null_for_s() {
    for (label, url) in [
        (
            "a non Mini App page",
            "https://shop.com/go?t={{mini_app_token}}",
        ),
        ("a path suffix", "https://x.com/app/{{minigame_play_token}}"),
        (
            "a token that is not last",
            "https://app.example.com/mini-apps?id=12&token={{mini_app_token}}&utm=fb",
        ),
        (
            "the other page's token",
            "https://app.example.com/minigames?minigameId=abc&token={{mini_app_token}}",
        ),
    ] {
        assert_eq!(fc::normalize_messenger_token_url(url), None, "{label}");
    }
}

#[test]
fn findmessengertokenvariable_finds_a_token_variable_anywhere_in_the_text() {
    assert_eq!(
        fc::find_messenger_token_variable(Some("https://x/?t={{mini_app_token}}&a=1")),
        Some("mini_app_token")
    );
    assert_eq!(
        fc::find_messenger_token_variable(Some("{{minigame_play_token}}")),
        Some("minigame_play_token")
    );
}

#[test]
fn findmessengertokenvariable_returns_null_without_a_token_variable() {
    assert_eq!(
        fc::find_messenger_token_variable(Some("https://x/{{1}}")),
        None
    );
    assert_eq!(fc::find_messenger_token_variable(None), None);
}

// ─── extractMessengerParameterInfos ───

#[test]
fn extractmessengerparameterinfos_empty_components_returns() {
    assert!(infos(json!([]), "POSITIONAL").is_empty());
}

#[test]
fn extractmessengerparameterinfos_header_text_positional_paramname_is_positional_number_string() {
    let result = infos(
        json!([{ "type": "HEADER", "format": "TEXT", "text": "Hello {{foo}}" }]),
        "POSITIONAL",
    );
    assert_eq!(
        result,
        vec![json!({ "type": "header", "index": 0, "paramName": "1", "format": "text" })]
    );
}

#[test]
fn extractmessengerparameterinfos_header_text_named_paramname_is_raw_variable_name() {
    let result = infos(
        json!([{ "type": "HEADER", "format": "TEXT", "text": "Hello {{customer_name}}" }]),
        "NAMED",
    );
    assert_eq!(
        result,
        vec![
            json!({ "type": "header", "index": 0, "paramName": "customer_name", "format": "text" })
        ]
    );
}

#[test]
fn extractmessengerparameterinfos_header_image_returns_empty_array_image_fixed_at_template_creation_no_send_time_parameter_needed()
 {
    assert!(
        infos(
            json!([{ "type": "HEADER", "format": "IMAGE" }]),
            "POSITIONAL"
        )
        .is_empty()
    );
}

#[test]
fn extractmessengerparameterinfos_header_image_image_static_text_no_parameter() {
    assert!(
        infos(
            json!([{ "type": "HEADER", "format": "IMAGE", "text": "Order update" }]),
            "POSITIONAL"
        )
        .is_empty()
    );
}

#[test]
fn extractmessengerparameterinfos_header_image_image_text_variable_positional_header_text_param_is_collected()
 {
    let result = infos(
        json!([{ "type": "HEADER", "format": "IMAGE", "text": "{{1}}" }]),
        "POSITIONAL",
    );
    assert_eq!(
        result,
        vec![json!({ "type": "header", "index": 0, "paramName": "1", "format": "text" })]
    );
}

#[test]
fn extractmessengerparameterinfos_header_image_image_text_variable_named_paramname_is_raw_variable_name()
 {
    let result = infos(
        json!([{ "type": "HEADER", "format": "IMAGE", "text": "{{order_type}} Update" }]),
        "NAMED",
    );
    assert_eq!(
        result,
        vec![json!({ "type": "header", "index": 0, "paramName": "order_type", "format": "text" })]
    );
}

#[test]
fn extractmessengerparameterinfos_body_two_vars_positional_indexes_0_1_paramnames_1_2() {
    let result = infos(
        json!([{ "type": "BODY", "text": "Hi {{1}} your order {{2}} is confirmed" }]),
        "POSITIONAL",
    );
    assert_eq!(
        result,
        vec![
            json!({ "type": "body", "index": 0, "paramName": "1" }),
            json!({ "type": "body", "index": 1, "paramName": "2" }),
        ]
    );
}

#[test]
fn extractmessengerparameterinfos_body_two_vars_named_paramnames_are_raw_variable_names() {
    let result = infos(
        json!([{ "type": "BODY", "text": "Hi {{first_name}} your order {{order_id}} confirmed" }]),
        "NAMED",
    );
    assert_eq!(
        result,
        vec![
            json!({ "type": "body", "index": 0, "paramName": "first_name" }),
            json!({ "type": "body", "index": 1, "paramName": "order_id" }),
        ]
    );
}

#[test]
fn extractmessengerparameterinfos_buttons_url_button_with_1_buttonsubtype_url() {
    let result = infos(
        buttons(json!([{ "type": "URL", "text": "Track", "url": "https://x.com/{{1}}" }])),
        "POSITIONAL",
    );
    assert_eq!(
        result,
        vec![
            json!({ "type": "button", "index": 0, "paramName": "1", "buttonIndex": 0, "buttonSubType": "url" })
        ]
    );
}

#[test]
fn extractmessengerparameterinfos_buttons_named_url_button_with_url_suffix_paramname_is_the_variable_name()
 {
    let result = infos(
        buttons(json!([{
            "type": "URL",
            "text": "Track Order",
            "url": "http://www.example.com/orders/{{url_suffix}}",
        }])),
        "NAMED",
    );
    assert_eq!(
        result,
        vec![json!({
            "type": "button",
            "index": 0,
            "paramName": "url_suffix",
            "buttonIndex": 0,
            "buttonSubType": "url",
        })]
    );
}

#[test]
fn extractmessengerparameterinfos_buttons_static_url_button_no_parameterinfo_nothing_for_the_user_to_fill()
 {
    assert!(
        infos(
            buttons(json!([{ "type": "URL", "text": "Go", "url": "https://x.com/jobs/" }])),
            "NAMED"
        )
        .is_empty()
    );
}

#[test]
fn extractmessengerparameterinfos_buttons_postback_with_fixed_payload_no_empty_result() {
    let result = infos(
        buttons(json!([{ "type": "POSTBACK", "text": "Confirm", "payload": "CONFIRM" }])),
        "POSITIONAL",
    );
    assert!(result.is_empty());
}

#[test]
fn extractmessengerparameterinfos_buttons_postback_with_n_payload_empty_result_becomes_flow_button_not_parameterinfo()
 {
    let result = infos(
        buttons(json!([{ "type": "POSTBACK", "text": "Track", "payload": "order_id_{{1}}" }])),
        "POSITIONAL",
    );
    assert!(result.is_empty());
}

#[test]
fn extractmessengerparameterinfos_buttons_quick_reply_with_fixed_payload_no_param_entry() {
    let result = infos(
        buttons(json!([{ "type": "QUICK_REPLY", "text": "Yes", "payload": "YES" }])),
        "POSITIONAL",
    );
    assert!(result.is_empty());
}

// ─── extractMessengerFlowButtons ───

#[test]
fn extractmessengerflowbuttons_empty_components_returns() {
    assert!(flow_buttons(json!([])).is_empty());
}

#[test]
fn extractmessengerflowbuttons_postback_with_n_in_payload_one_buttonstepprops_with_label_button_text_buttontype_null()
 {
    let result = flow_buttons(buttons(
        json!([{ "type": "POSTBACK", "text": "Confirm", "payload": "order_id_{{1}}" }]),
    ));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["label"], "Confirm");
    assert!(result[0]["buttonType"].is_null());
    assert!(result[0]["beforeStep"].is_null());
    assert_eq!(result[0]["steps"], json!([]));
    assert!(result[0]["id"].is_string());
    assert!(!result[0]["id"].as_str().unwrap().is_empty());
}

#[test]
fn extractmessengerflowbuttons_quick_reply_with_n_in_payload_one_buttonstepprops_with_label_button_text()
 {
    let result = flow_buttons(buttons(
        json!([{ "type": "QUICK_REPLY", "text": "Yes", "payload": "confirm_{{1}}" }]),
    ));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["label"], "Yes");
    assert!(result[0]["buttonType"].is_null());
}

#[test]
fn extractmessengerflowbuttons_postback_without_in_payload_excluded() {
    let result = flow_buttons(buttons(
        json!([{ "type": "POSTBACK", "text": "Confirm", "payload": "CONFIRM" }]),
    ));
    assert!(result.is_empty());
}

#[test]
fn extractmessengerflowbuttons_url_button_excluded_url_buttons_are_params_not_flow_buttons() {
    let result = flow_buttons(buttons(
        json!([{ "type": "URL", "text": "Track", "url": "https://example.com/{{1}}" }]),
    ));
    assert!(result.is_empty());
}

#[test]
fn extractmessengerflowbuttons_mixed_postback_with_postback_without_url_only_postback_with_included()
 {
    let result = flow_buttons(buttons(json!([
        { "type": "POSTBACK", "text": "Routable", "payload": "track_{{1}}" },
        { "type": "POSTBACK", "text": "Fixed", "payload": "FIXED" },
        { "type": "URL", "text": "Link", "url": "https://example.com/{{1}}" },
    ])));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["label"], "Routable");
}

#[test]
fn extractmessengerflowbuttons_multiple_postback_buttons_with_one_buttonstepprops_per_button() {
    let result = flow_buttons(buttons(json!([
        { "type": "POSTBACK", "text": "Accept", "payload": "accept_{{1}}" },
        { "type": "POSTBACK", "text": "Decline", "payload": "decline_{{1}}" },
    ])));
    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["label"], "Accept");
    assert_eq!(result[1]["label"], "Decline");
}

#[test]
fn extractmessengerflowbuttons_two_calls_produce_different_ids_per_button() {
    let component =
        buttons(json!([{ "type": "POSTBACK", "text": "Confirm", "payload": "c_{{1}}" }]));
    let a = flow_buttons(component.clone());
    let b = flow_buttons(component);
    assert_ne!(a[0]["id"], b[0]["id"]);
}

#[test]
fn extractmessengerflowbuttons_button_text_undefined_label_is_empty_string() {
    let result = flow_buttons(buttons(
        json!([{ "type": "POSTBACK", "text": "", "payload": "order_{{1}}" }]),
    ));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["label"], "");
}

#[test]
fn extractmessengerflowbuttons_non_buttons_components_are_ignored() {
    let result = flow_buttons(json!([
        { "type": "HEADER", "format": "TEXT", "text": "Hello {{1}}" },
        { "type": "BODY", "text": "Hi {{1}} your order is ready" },
    ]));
    assert!(result.is_empty());
}

// ─── sendMessengerTemplateMessageStepSchema ───

fn base() -> Value {
    json!({
        "id": "1",
        "stepType": "sendMessengerTemplateMessage",
        "nodeId": "node-1",
        "template": { "id": "tmpl-1", "name": "order_confirm", "language": "en", "params": {} },
    })
}

fn parse(value: Value) -> Value {
    let result = fc::send_messenger_template_message_step_schema().safe_parse(&value);
    assert!(result.success, "{:?}", result.error);
    result.data.unwrap()
}

#[test]
fn sendmessengertemplatemessagestepschema_parameterformat_defaults_to_positional() {
    let result = parse(spread(&base(), json!({ "buttons": [] })));
    assert_eq!(result["template"]["parameterFormat"], "POSITIONAL");
}

#[test]
fn sendmessengertemplatemessagestepschema_empty_buttons_stay_empty_linear_send_no_delivered_failed_branching()
 {
    let result = parse(spread(&base(), json!({ "buttons": [] })));
    assert_eq!(result["buttons"].as_array().unwrap().len(), 0);
}

#[test]
fn sendmessengertemplatemessagestepschema_buttons_default_to_when_omitted() {
    let without_buttons = without(&spread(&base(), json!({ "buttons": [] })), &["buttons"]);
    let result = parse(without_buttons);
    assert_eq!(result["buttons"], json!([]));
}

#[test]
fn sendmessengertemplatemessagestepschema_non_empty_buttons_are_preserved() {
    let result = parse(spread(
        &base(),
        json!({
            "buttons": [{ "id": "1", "label": "Accept", "steps": [], "beforeStep": null, "buttonType": null }],
        }),
    ));
    assert_eq!(result["buttons"].as_array().unwrap().len(), 1);
    assert_match_object(
        &result["buttons"][0],
        &json!({ "id": "1", "label": "Accept" }),
    );
}

#[test]
fn sendmessengertemplatemessagestepschema_empty_template_id_whitespace_parse_fails() {
    let result = fc::send_messenger_template_message_step_schema().safe_parse(&spread(
        &base(),
        json!({ "template": spread(&base()["template"], json!({ "id": "   " })), "buttons": [] }),
    ));
    assert!(!result.success);
}

// ─── sendMessengerTemplateMessageStepDefaultFn ───

fn default_step() -> Value {
    fc::send_messenger_template_message_step_default_fn(&json!({}))
}

#[test]
fn sendmessengertemplatemessagestepdefaultfn_steptype_is_sendmessengertemplatemessage() {
    assert_eq!(default_step()["stepType"], "sendMessengerTemplateMessage");
}

#[test]
fn sendmessengertemplatemessagestepdefaultfn_parameterformat_defaults_to_positional() {
    assert_eq!(default_step()["template"]["parameterFormat"], "POSITIONAL");
}

#[test]
fn sendmessengertemplatemessagestepdefaultfn_buttons_default_to_empty_linear_send_no_branching() {
    assert_eq!(default_step()["buttons"].as_array().unwrap().len(), 0);
}

#[test]
fn sendmessengertemplatemessagestepdefaultfn_id_is_always_generated_non_empty_string() {
    let step = default_step();
    assert!(step["id"].is_string());
    assert!(!step["id"].as_str().unwrap().is_empty());
}

#[test]
fn sendmessengertemplatemessagestepdefaultfn_two_calls_produce_different_ids() {
    assert_ne!(default_step()["id"], default_step()["id"]);
}

#[test]
fn sendmessengertemplatemessagestepdefaultfn_props_override_template_fields() {
    let step = fc::send_messenger_template_message_step_default_fn(&json!({
        "template": {
            "id": "tmpl-99",
            "name": "custom",
            "language": "vi",
            "parameterFormat": "NAMED",
            "params": { "body": [{ "text": "hello" }] },
        },
    }));
    assert_eq!(step["template"]["id"], "tmpl-99");
    assert_eq!(step["template"]["parameterFormat"], "NAMED");
}

#[test]
fn sendmessengertemplatemessagestepschema_inboxid_integrationmessengerid_preserves_inboxid_through_schema_parse()
 {
    let step = default_step();
    let with_meta = spread(
        &step,
        json!({ "template": spread(&step["template"], json!({ "id": "tmpl-1", "inboxId": "inbox-123" })) }),
    );

    let parsed = fc::send_messenger_template_message_step_schema().safe_parse(&with_meta);
    assert!(parsed.success);
    assert_eq!(parsed.data.unwrap()["template"]["inboxId"], "inbox-123");
}

#[test]
fn sendmessengertemplatemessagestepschema_inboxid_integrationmessengerid_preserves_integrationmessengerid_through_schema_parse()
 {
    let step = default_step();
    let with_meta = spread(
        &step,
        json!({
            "template": spread(&step["template"], json!({ "id": "tmpl-1", "integrationMessengerId": "intg-456" })),
        }),
    );

    let parsed = fc::send_messenger_template_message_step_schema().safe_parse(&with_meta);
    assert!(parsed.success);
    assert_eq!(
        parsed.data.unwrap()["template"]["integrationMessengerId"],
        "intg-456"
    );
}

#[test]
fn sendmessengertemplatemessagestepschema_inboxid_integrationmessengerid_messengertemplatebuttonparamschema_rejects_sub_type_quick_reply()
 {
    let result = fc::messenger_template_button_param_schema()
        .safe_parse(&json!({ "sub_type": "quick_reply" }));
    assert!(!result.success);
}

// ─── mergeMessengerFlowButtonsWithExisting ───

#[test]
fn mergemessengerflowbuttonswithexisting_keeps_existing_button_id_and_routing_config_by_index_while_updating_label()
 {
    let result = fc::merge_messenger_flow_buttons_with_existing(
        &[
            json!({ "id": "new-button", "label": "New template label", "buttonType": null, "beforeStep": null, "steps": [] }),
        ],
        &[json!({
            "id": "connected-button",
            "label": "Old template label",
            "buttonType": "startAnotherNode",
            "beforeStep": { "id": "before-step", "stepType": "startAnotherNode", "nodeId": "target-node", "viewOnly": true },
            "steps": [],
        })],
    );

    assert_eq!(
        result,
        vec![json!({
            "id": "connected-button",
            "label": "New template label",
            "buttonType": "startAnotherNode",
            "beforeStep": { "id": "before-step", "stepType": "startAnotherNode", "nodeId": "target-node", "viewOnly": true },
            "steps": [],
        })]
    );
}

#[test]
fn mergemessengerflowbuttonswithexisting_uses_new_template_button_when_there_is_no_existing_button_at_that_index()
 {
    let result = fc::merge_messenger_flow_buttons_with_existing(
        &[
            json!({ "id": "new-button", "label": "New template label", "buttonType": null, "beforeStep": null, "steps": [] }),
        ],
        &[],
    );

    assert_match_object(
        &result[0],
        &json!({ "id": "new-button", "label": "New template label", "buttonType": null }),
    );
}
