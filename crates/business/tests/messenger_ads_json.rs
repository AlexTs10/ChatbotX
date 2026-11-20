//! Port of `integrations/messenger/__tests__/messenger-ads-json.test.ts`
//! (Q.P6-U2). Adapted: `JSON.stringify` key order is serde_json's (sorted),
//! so the phone-button check reads the fields.

use chatbotx_business::messenger_ads_json::{AdsJson, Mode, convert};
use chatbotx_core::flow_config as fc;
use serde_json::{Value, json};

const FLOW_ID: &str = "1756191592047";
const FLOW_VERSION_ID: &str = "1782439145669";
const IMAGE_URL: &str = "https://cdn.example.com/a.jpg";

/// `{ ...base, ...extra }`.
fn spread(base: Value, extra: Value) -> Value {
    let mut out = base;
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        out[k] = v;
    }
    out
}

fn make_start_node(steps: Vec<Value>, quick_replies: Vec<Value>) -> Value {
    let mut node =
        fc::send_message_node_default_fn(&json!({ "dataProps": { "isStartNode": true } }));
    node["data"]["details"]["steps"] = json!(steps);
    node["data"]["details"]["quickReplies"] = json!(quick_replies);
    node
}

fn strict(node: &Value) -> AdsJson {
    convert(node, FLOW_ID, Some(FLOW_VERSION_ID), Mode::Strict)
}

fn lenient(node: &Value) -> AdsJson {
    convert(node, FLOW_ID, Some(FLOW_VERSION_ID), Mode::Lenient)
}

fn text_step(text: &str) -> Value {
    fc::send_text_step_default_fn(&json!({ "text": text }))
}

fn button(label: &str) -> Value {
    fc::button_step_default_fn(&json!({ "label": label }))
}

fn user_data() -> Value {
    fc::get_user_data_step_default_fn(&json!({}))
}

fn image(url: &str) -> Value {
    spread(
        fc::send_image_step_default_fn(),
        json!({ "mode": "url", "url": url }),
    )
}

fn card(title: &str, subtitle: &str) -> Value {
    spread(
        fc::send_card_step_default_fn(),
        json!({ "title": title, "subtitle": subtitle, "buttons": [] }),
    )
}

fn carousel(cards: Vec<Value>) -> Value {
    spread(
        fc::send_carousel_step_default_fn(),
        json!({ "cards": cards }),
    )
}

fn valid_quick_reply() -> Value {
    spread(
        button("Yes"),
        json!({
            "buttonType": "sendMessage",
            "beforeStep": fc::start_another_node_step_default_fn(&json!({ "nodeId": "1782439145670" })),
        }),
    )
}

fn image_message(url: &str) -> Value {
    json!({ "message": { "attachment": { "type": "image", "payload": { "url": url, "is_reusable": true } } } })
}

fn payload(result: &AdsJson, index: usize) -> Value {
    result.as_ref().expect("ok")[index]["message"]["attachment"]["payload"].clone()
}

// ---- strict ------------------------------------------------------------------

#[test]
fn plain_text_step_is_wrapped_and_allowed_variables_get_the_user_prefix() {
    let node = make_start_node(vec![text_step("Hi {{first_name}} {{full_name}}")], vec![]);
    assert_eq!(
        strict(&node),
        Ok(vec![
            json!({ "message": { "text": "Hi {{user_first_name}} {{user_full_name}}" } })
        ])
    );
}

#[test]
fn an_unrecognized_variable_trips_the_facebook_limitation_error() {
    let node = make_start_node(vec![text_step("Hello {{email}}")], vec![]);
    assert_eq!(strict(&node), Err("invalidVariable"));
}

#[test]
fn a_disallowed_step_type_get_user_data_is_rejected() {
    assert_eq!(
        strict(&make_start_node(vec![user_data()], vec![])),
        Err("invalidStepType")
    );
}

#[test]
fn a_non_send_message_start_node_is_rejected() {
    let node = json!({ "type": "landingPage", "id": "1", "data": {} });
    assert_eq!(strict(&node), Err("invalidStepType"));
}

#[test]
fn an_empty_starting_step_yields_an_empty_array() {
    assert_eq!(strict(&make_start_node(vec![], vec![])), Ok(vec![]));
}

#[test]
fn a_carousel_becomes_a_generic_template_with_rewritten_element_text() {
    let mut c = card("Card {{full_name}}", "Sub");
    c["image"] = spread(
        fc::send_image_step_default_fn(),
        json!({ "url": IMAGE_URL }),
    );
    let result = strict(&make_start_node(vec![carousel(vec![c])], vec![]));
    let payload = payload(&result, 0);
    assert_eq!(payload["template_type"], "generic");
    assert_eq!(payload["elements"][0]["title"], "Card {{user_full_name}}");
    assert_eq!(payload["elements"][0]["subtitle"], "Sub");
    assert_eq!(payload["elements"][0]["image_url"], IMAGE_URL);
}

#[test]
fn a_carousel_with_more_than_10_cards_is_chunked_into_multiple_messages() {
    let cards = (0..23).map(|i| card(&format!("Card {i}"), "")).collect();
    let result = strict(&make_start_node(vec![carousel(cards)], vec![]));
    let lengths: Vec<usize> = result
        .expect("ok")
        .iter()
        .map(|m| {
            m["message"]["attachment"]["payload"]["elements"]
                .as_array()
                .unwrap()
                .len()
        })
        .collect();
    assert_eq!(lengths, vec![10, 10, 3]);
}

#[test]
fn a_text_step_with_buttons_becomes_a_button_template_with_rewritten_labels() {
    let step = spread(
        text_step("Pick one"),
        json!({ "buttons": [button("Hi {{first_name}}")] }),
    );
    let result = strict(&make_start_node(vec![step], vec![]));
    let payload = payload(&result, 0);
    assert_eq!(payload["template_type"], "button");
    assert_eq!(payload["buttons"][0]["type"], "postback");
    assert_eq!(payload["buttons"][0]["title"], "Hi {{user_first_name}}");
}

#[test]
fn media_steps_emit_a_direct_url_attachment() {
    let node = make_start_node(vec![image("https://cdn.example.com/photo.jpg")], vec![]);
    assert_eq!(
        strict(&node),
        Ok(vec![image_message("https://cdn.example.com/photo.jpg")])
    );
}

#[test]
fn a_send_multiple_images_step_is_allowed_and_emits_a_batched_attachments_message() {
    let step = spread(
        fc::send_multiple_images_step_default_fn(),
        json!({ "images": [
            { "id": "img-1", "mode": "url", "url": "https://cdn.example.com/a.jpg" },
            { "id": "img-2", "mode": "url", "url": "https://cdn.example.com/b.jpg" },
        ] }),
    );
    let at = |url: &str| json!({ "type": "image", "payload": { "url": url, "is_reusable": true } });
    assert_eq!(
        strict(&make_start_node(vec![step], vec![])),
        Ok(vec![json!({ "message": { "attachments": [
            at("https://cdn.example.com/a.jpg"),
            at("https://cdn.example.com/b.jpg"),
        ] } })])
    );
}

// ---- lenient -----------------------------------------------------------------

#[test]
fn lenient_skips_disallowed_steps_instead_of_failing() {
    let node = make_start_node(vec![user_data(), text_step("Hi")], vec![]);
    assert_eq!(
        lenient(&node),
        Ok(vec![json!({ "message": { "text": "Hi" } })])
    );
}

#[test]
fn lenient_blanks_unknown_variables_and_keeps_the_allowed_ones() {
    let node = make_start_node(
        vec![text_step("Hi {{first_name}}, code {{coupon}}")],
        vec![],
    );
    assert_eq!(
        lenient(&node),
        Ok(vec![
            json!({ "message": { "text": "Hi {{user_first_name}}, code " } })
        ])
    );
}

#[test]
fn lenient_drops_a_text_message_that_is_empty_after_stripping_variables() {
    let node = make_start_node(vec![text_step(" {{phone}} "), image(IMAGE_URL)], vec![]);
    assert_eq!(lenient(&node), Ok(vec![image_message(IMAGE_URL)]));
}

#[test]
fn lenient_drops_an_empty_text_step_even_when_it_has_buttons() {
    let step = fc::send_text_step_default_fn(&json!({
        "text": "{{phone}}",
        "buttons": [spread(button("Go"), json!({ "buttonType": "sendMessage" }))],
    }));
    assert_eq!(lenient(&make_start_node(vec![step], vec![])), Ok(vec![]));
}

#[test]
fn lenient_send_gif_becomes_an_image_attachment() {
    let gif = spread(fc::send_gif_step_default_fn(), json!({ "url": IMAGE_URL }));
    assert_eq!(
        lenient(&make_start_node(vec![gif], vec![])),
        Ok(vec![image_message(IMAGE_URL)])
    );
}

#[test]
fn lenient_a_non_send_message_start_node_yields_no_messages() {
    let node = json!({ "type": "landingPage", "id": "1", "data": {} });
    assert_eq!(lenient(&node), Ok(vec![]));
}

#[test]
fn lenient_quick_replies_ride_on_the_last_kept_message() {
    let node = make_start_node(vec![text_step("A"), user_data()], vec![valid_quick_reply()]);
    let messages = lenient(&node).expect("ok");
    assert_eq!(
        messages[0]["message"]["quick_replies"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn lenient_quick_replies_are_dropped_when_every_message_is_excluded() {
    let node = make_start_node(vec![user_data()], vec![valid_quick_reply()]);
    assert_eq!(lenient(&node), Ok(vec![]));
}

#[test]
fn lenient_drops_a_quick_reply_whose_label_is_blank_after_stripping_variables() {
    let blank = spread(valid_quick_reply(), json!({ "label": "{{city}}" }));
    let node = make_start_node(vec![text_step("Pick")], vec![blank, valid_quick_reply()]);
    let messages = lenient(&node).expect("ok");
    let titles: Vec<&str> = messages[0]["message"]["quick_replies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|qr| qr["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["Yes"]);
}

#[test]
fn lenient_omits_quick_replies_when_every_quick_reply_is_blank() {
    let blank = spread(valid_quick_reply(), json!({ "label": "{{city}}" }));
    let node = make_start_node(vec![text_step("Pick")], vec![blank]);
    assert_eq!(
        lenient(&node),
        Ok(vec![json!({ "message": { "text": "Pick" } })])
    );
}

#[test]
fn lenient_a_text_step_whose_only_button_is_blank_becomes_plain_text() {
    let step = spread(
        text_step("Pick one"),
        json!({ "buttons": [button("{{city}}")] }),
    );
    assert_eq!(
        lenient(&make_start_node(vec![step], vec![])),
        Ok(vec![json!({ "message": { "text": "Pick one" } })])
    );
}

#[test]
fn lenient_trims_a_button_label_left_with_whitespace_by_stripping() {
    let step = spread(
        text_step("Pick one"),
        json!({ "buttons": [button("Hi {{city}}")] }),
    );
    let result = lenient(&make_start_node(vec![step], vec![]));
    assert_eq!(payload(&result, 0)["buttons"][0]["title"], "Hi");
}

#[test]
fn lenient_drops_carousel_cards_with_a_blank_title_and_omits_a_blank_subtitle() {
    let node = make_start_node(
        vec![carousel(vec![
            card("{{product}}", ""),
            card("Shoes", "{{price}}"),
        ])],
        vec![],
    );
    let result = lenient(&node);
    let elements = payload(&result, 0)["elements"].clone();
    assert_eq!(elements.as_array().map(Vec::len), Some(1));
    assert_eq!(elements[0]["title"], "Shoes");
    assert!(elements[0].get("subtitle").is_none());
}

#[test]
fn lenient_a_carousel_whose_every_card_is_blank_yields_no_message() {
    let node = make_start_node(vec![carousel(vec![card("{{product}}", "")])], vec![]);
    assert_eq!(lenient(&node), Ok(vec![]));
}

#[test]
fn strict_mode_still_rejects_an_unknown_variable_in_a_label() {
    let blank = spread(valid_quick_reply(), json!({ "label": "{{city}}" }));
    let node = make_start_node(vec![text_step("Pick")], vec![blank]);
    assert_eq!(strict(&node), Err("invalidVariable"));
}

#[test]
fn strict_mode_still_rejects_send_gif() {
    let gif = spread(fc::send_gif_step_default_fn(), json!({ "url": IMAGE_URL }));
    assert_eq!(
        strict(&make_start_node(vec![gif], vec![])),
        Err("invalidStepType")
    );
}

// ---- call phone number buttons -------------------------------------------------

fn text_node_with_call(phone: &str) -> Value {
    let call = spread(
        button("Call us"),
        json!({
            "buttonType": "callPhoneNumber",
            "beforeStep": { "id": "2", "stepType": "callPhoneNumber", "phoneNumber": phone },
            "steps": [],
        }),
    );
    make_start_node(
        vec![spread(
            text_step("Need help?"),
            json!({ "buttons": [call] }),
        )],
        vec![],
    )
}

#[test]
fn a_literal_number_becomes_a_native_phone_number_button() {
    let result = strict(&text_node_with_call("+14155552671"));
    assert_eq!(
        payload(&result, 0)["buttons"][0],
        json!({ "type": "phone_number", "title": "Call us", "payload": "+14155552671" })
    );
}

#[test]
fn strict_mode_rejects_a_number_that_still_holds_a_variable() {
    assert_eq!(
        strict(&text_node_with_call("{{contact.phone}}")),
        Err("invalidVariable")
    );
}

#[test]
fn lenient_mode_drops_a_number_that_still_holds_a_variable() {
    assert_eq!(
        lenient(&text_node_with_call("{{contact.phone}}")),
        Ok(vec![json!({ "message": { "text": "Need help?" } })])
    );
}
