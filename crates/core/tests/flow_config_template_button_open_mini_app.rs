//! Port of `packages/flow-config/__tests__/template-button-open-mini-app.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn mini_app_button() -> Value {
    spread(
        &fc::button_step_default_fn(&json!({ "label": "Form" })),
        json!({
            "buttonType": "openMiniApp",
            "beforeStep": fc::open_mini_app_step_default_fn(&json!({ "miniAppId": "7" })),
        }),
    )
}

fn template() -> Value {
    json!({ "id": "1", "name": "welcome", "language": "en", "params": {} })
}

#[test]
fn open_mini_app_buttons_on_template_steps_a_whatsapp_template_step_rejects_them() {
    let step = fc::send_wa_template_message_step_default_fn(&json!({ "template": template() }));

    assert!(
        fc::send_wa_template_message_step_schema()
            .safe_parse(&step)
            .success
    );

    let mut buttons = step["buttons"].as_array().unwrap().clone();
    buttons.push(mini_app_button());
    let result = fc::send_wa_template_message_step_schema()
        .safe_parse(&spread(&step, json!({ "buttons": buttons })));
    assert_eq!(
        result.issues()[0].message,
        "templateButtonOpenMiniAppUnsupported"
    );
}

#[test]
fn open_mini_app_buttons_on_template_steps_a_messenger_template_step_accepts_them() {
    let step =
        fc::send_messenger_template_message_step_default_fn(&json!({ "template": template() }));

    assert!(
        fc::send_messenger_template_message_step_schema()
            .safe_parse(&spread(&step, json!({ "buttons": [mini_app_button()] })))
            .success
    );
}

fn components() -> Vec<Value> {
    vec![
        json!({ "type": "BODY", "text": "Hi" }),
        json!({
            "type": "BUTTONS",
            "buttons": [
                { "type": "POSTBACK", "text": "Yes", "payload": "{{1}}" },
                { "type": "URL", "text": "Fill form", "url": "https://app.example.com/mini-apps?id=42&token={{1}}" },
                { "type": "URL", "text": "Site", "url": "https://example.com/{{1}}" },
            ],
        }),
    ]
}

#[test]
fn messenger_template_mini_app_buttons_reads_the_template_s_mini_app_url_buttons_with_their_position()
 {
    assert_eq!(
        fc::read_messenger_mini_app_buttons(&components()),
        vec![json!({ "index": 1, "label": "Fill form", "miniAppId": "42" })]
    );
}

#[test]
fn messenger_template_mini_app_buttons_only_a_flow_step_routes_mini_app_buttons_in_template_order()
{
    let types: Vec<Value> = fc::extract_messenger_flow_buttons(&components(), false)
        .iter()
        .map(|button| button["buttonType"].clone())
        .collect();
    assert_eq!(types, vec![Value::Null]);

    let flow_buttons = fc::extract_messenger_flow_buttons(&components(), true);
    let labels: Vec<&str> = flow_buttons.iter().map(|b| s(b, "label")).collect();
    assert_eq!(labels, vec!["Yes", "Fill form"]);
    assert_match_object(
        &flow_buttons[1],
        &json!({ "buttonType": "openMiniApp", "beforeStep": { "stepType": "openMiniApp", "miniAppId": "42" } }),
    );
}

#[test]
fn messenger_template_mini_app_buttons_keeps_an_existing_mini_app_button_s_id_and_follows_the_template_s_mini_app()
 {
    let existing = spread(
        &mini_app_button(),
        json!({ "id": "900", "beforeStep": fc::open_mini_app_step_default_fn(&json!({ "miniAppId": "7" })) }),
    );
    let component = components()[1].clone();
    let sliced = spread(&component, json!({ "buttons": [component["buttons"][1]] }));
    let template_button = fc::extract_messenger_flow_buttons(&[sliced], true)[0].clone();

    let merged = fc::merge_messenger_flow_buttons_with_existing(&[template_button], &[existing]);
    assert_eq!(merged[0]["id"], "900");
    assert_match_object(
        &merged[0],
        &json!({ "buttonType": "openMiniApp", "beforeStep": { "miniAppId": "42" } }),
    );
}

#[test]
fn messenger_template_mini_app_buttons_a_slot_that_switched_between_postback_and_mini_app_takes_the_template_s_button()
 {
    let postback = spread(
        &fc::button_step_default_fn(&json!({ "label": "Old" })),
        json!({ "id": "901" }),
    );
    let template_buttons = fc::extract_messenger_flow_buttons(&components(), true);

    let merged = fc::merge_messenger_flow_buttons_with_existing(
        &template_buttons,
        &[postback.clone(), postback],
    );
    assert_eq!(merged[0]["id"], "901");
    assert_eq!(merged[1], template_buttons[1]);
}
