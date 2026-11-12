//! Port of `packages/flow-config/__tests__/wa-template.test.ts`.

mod flow_config_support;
use flow_config_support::*;

// Meta placeholders are positional ({{1}}) or named ({{order_id}}). A purely
// numeric token is positional; anything else is a named parameter that must be
// echoed back to Meta as `parameter_name` on every send-time parameter.
#[test]
fn isnamedtemplatetoken_token_s_named_s() {
    for (token, expected) in [
        ("1", false),
        ("2", false),
        ("12", false),
        ("user_name", true),
        ("order_id", true),
        ("first_name", true),
    ] {
        assert_eq!(fc::is_named_template_token(token), expected, "{token}");
    }
}

#[test]
fn extracttemplateparams_body_positional_1_2_two_text_params_without_parameter_name() {
    let components = [json!({ "type": "BODY", "text": "Hi {{1}} and {{2}}" })];
    assert_eq!(
        fc::extract_template_params(&components)["body"],
        json!([{ "type": "text", "text": "" }, { "type": "text", "text": "" }])
    );
}

#[test]
fn extracttemplateparams_body_named_single_user_name_one_text_param_with_parameter_name() {
    let components = [json!({ "type": "BODY", "text": "Happy Birthday, {{user_name}}!" })];
    assert_eq!(
        fc::extract_template_params(&components)["body"],
        json!([{ "type": "text", "text": "", "parameter_name": "user_name" }])
    );
}

#[test]
fn extracttemplateparams_body_named_multiple_placeholders_keep_their_own_names() {
    let components =
        [json!({ "type": "BODY", "text": "Hi {{first_name}}, order {{order_id}} is ready" })];
    assert_eq!(
        fc::extract_template_params(&components)["body"],
        json!([
            { "type": "text", "text": "", "parameter_name": "first_name" },
            { "type": "text", "text": "", "parameter_name": "order_id" },
        ])
    );
}

#[test]
fn extracttemplateparams_body_no_variables_body_stays_undefined() {
    let components = [json!({ "type": "BODY", "text": "Static body with no variables" })];
    assert!(
        fc::extract_template_params(&components)
            .get("body")
            .is_none()
    );
}

#[test]
fn extracttemplateparams_header_text_positional_1_no_parameter_name() {
    let components = [json!({ "type": "HEADER", "format": "TEXT", "text": "Order {{1}}" })];
    assert_eq!(
        fc::extract_template_params(&components)["header"],
        json!([{ "type": "text", "text": "" }])
    );
}

#[test]
fn extracttemplateparams_header_text_named_store_name_parameter_name() {
    let components = [json!({ "type": "HEADER", "format": "TEXT", "text": "From {{store_name}}" })];
    assert_eq!(
        fc::extract_template_params(&components)["header"],
        json!([{ "type": "text", "text": "", "parameter_name": "store_name" }])
    );
}

// Meta delivers a template FLOW button's `flow_id` as a JSON number. Both
// extractors must coerce it to a string.
fn flow_components() -> Vec<Value> {
    vec![
        json!({ "type": "BODY", "text": "Hello world!" }),
        json!({
            "type": "BUTTONS",
            "buttons": [{
                "type": "FLOW",
                "text": "View Flow",
                "flow_id": 1_690_702_985_711_558_i64,
                "flow_action": "NAVIGATE",
                "navigate_screen": "RECOMMEND",
            }],
        }),
    ]
}

#[test]
fn flow_button_flow_id_coercion_extracttemplateparams_yields_a_string_flowsourceid() {
    let params = fc::extract_template_params(&flow_components());
    assert_eq!(params["button"][0]["flowSourceId"], "1690702985711558");
}

#[test]
fn flow_button_flow_id_coercion_extractparameterinfos_yields_a_string_flowsourceid() {
    let infos = fc::extract_parameter_infos(&flow_components());
    let flow_param = infos
        .iter()
        .find(|param| param["buttonSubType"] == "flow")
        .unwrap();
    assert_eq!(flow_param["flowSourceId"], "1690702985711558");
}

// The editor picks a WhatsApp channel via template.inboxId and filters templates
// by it. It must survive publish/reload, and flows saved before the field existed
// must still parse.
fn base_step() -> Value {
    json!({
        "id": "11612473309626370",
        "stepType": "sendWaTemplateMessage",
        "template": { "id": "11637903697428480", "name": "get_feedback", "language": "en", "params": {} },
        "buttons": [],
    })
}

#[test]
fn sendwatemplatemessagestepschema_inboxid_persistence_persists_the_selected_channel_inboxid() {
    let step = base_step();
    let parsed = fc::send_wa_template_message_step_schema()
        .parse(&spread(&step, json!({ "template": spread(&step["template"], json!({ "inboxId": "11637902280048640" })) })))
        .unwrap();
    assert_eq!(parsed["template"]["inboxId"], "11637902280048640");
}

#[test]
fn sendwatemplatemessagestepschema_inboxid_persistence_stays_loadable_inboxid_undefined_for_flows_saved_before_the_field_existed()
 {
    let parsed = fc::send_wa_template_message_step_schema()
        .parse(&base_step())
        .unwrap();
    assert!(parsed["template"].get("inboxId").is_none());
}

fn pix() -> Value {
    json!({ "type": "PAYMENT_REQUEST", "text": "Copy Pix code", "payment_setting": { "type": "pix_dynamic_code" } })
}

#[test]
fn extractparameterinfos_payment_request_reads_the_payment_method_from_the_template_and_keeps_dense_slots()
 {
    let infos = fc::extract_parameter_infos(&[json!({
        "type": "BUTTONS",
        "buttons": [
            { "type": "URL", "text": "Static", "url": "https://a.test" },
            { "type": "URL", "text": "Track", "url": "https://a.test/{{1}}" },
            pix(),
        ],
    })]);

    assert_eq!(infos.len(), 2);
    assert_match_object(
        &infos[0],
        &json!({ "buttonSubType": "url", "paramIndex": 0 }),
    );
    assert_match_object(
        &infos[1],
        &json!({ "buttonSubType": "payment_request", "paymentMethod": "pix", "buttonIndex": 2, "paramIndex": 1 }),
    );
}

#[test]
fn extractparameterinfos_payment_request_a_payment_button_of_unknown_method_offers_no_field() {
    assert!(
        fc::extract_parameter_infos(&[json!({
            "type": "BUTTONS",
            "buttons": [{ "type": "PAYMENT_REQUEST", "text": "Pay now" }],
        })])
        .is_empty()
    );
}

#[test]
fn extractparameterinfos_payment_request_a_payment_button_inside_a_carousel_card_offers_no_field() {
    assert!(
        fc::extract_parameter_infos(&[json!({
            "type": "CAROUSEL",
            "cards": [{ "card_index": 0, "components": [{ "type": "BUTTONS", "buttons": [pix()] }] }],
        })])
        .is_empty()
    );
}
