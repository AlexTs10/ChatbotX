//! Ports of `integrations/messenger/__tests__`: `error-mapper`,
//! `outgoing-message-policy`, `send-message-quick-replies`,
//! `send-message-multiple-images`, `send-multiple-images`,
//! `quick-replies-attachment`, `send-carousel-layout`, `persona`,
//! `send-messenger-template`, `incoming-message-template-echo` and
//! `incoming-message-referral` (the pure parts; the HTTP mocks become
//! payload checks).

use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::ContactInbox;
use serde_json::{Value, json};

use super::*;
use crate::channel::{OutgoingContact, SendFlowStep};

fn graph_error(
    status: u16,
    code: Value,
    sub: Option<i64>,
    kind: Option<&str>,
    message: &str,
) -> GraphError {
    let mut error = json!({ "message": message, "code": code });
    if let Some(sub) = sub {
        error["error_subcode"] = json!(sub);
    }
    if let Some(kind) = kind {
        error["type"] = json!(kind);
    }
    GraphError::from_body(status, &json!({ "error": error }))
}

#[test]
fn code_551_and_200_with_1545041_map_to_user_blocked() {
    let blocked = map_error(
        &graph_error(
            400,
            json!(551),
            None,
            None,
            "This person isn't available right now",
        ),
        "e",
    );
    assert_eq!(blocked.category, Category::UserBlocked);
    assert!(blocked.is_permanent() && !blocked.is_retryable());
    let opted_out = map_error(
        &graph_error(
            403,
            json!(200),
            Some(1_545_041),
            None,
            "User opted out of messages",
        ),
        "e",
    );
    assert_eq!(opted_out.category, Category::UserBlocked);
    let plain = map_error(
        &graph_error(403, json!(200), None, None, "Permission error"),
        "e",
    );
    assert_eq!(plain.category, Category::PermissionDenied);
}

#[test]
fn disconnect_safe_errors_follow_the_table() {
    let e = |code: i64, sub: Option<i64>, message: &str| {
        graph_error(400, json!(code), sub, Some("OAuthException"), message)
    };
    assert!(is_disconnect_safe_error(&e(
        190,
        None,
        "(#190) Error validating application. Invalid application ID."
    )));
    assert!(!is_disconnect_safe_error(&e(
        190,
        None,
        "(#190) Error validating client secret."
    )));
    assert!(!is_disconnect_safe_error(&e(
        190,
        None,
        "(#190) Invalid OAuth access token - Cannot parse access token"
    )));
    assert!(is_disconnect_safe_error(&e(
        100,
        None,
        "(#100) App is not installed: 419370077795677"
    )));
    assert!(is_disconnect_safe_error(&e(
        100,
        Some(33),
        "Unsupported get request. Object with ID '1' does not exist"
    )));
    assert!(is_disconnect_safe_error(&e(
        803,
        None,
        "(#803) Some of the aliases you requested do not exist: 1"
    )));
    for code in [10, 200, 210] {
        assert!(is_disconnect_safe_error(&e(
            code,
            None,
            "Permissions error"
        )));
    }
    assert!(is_disconnect_safe_error(&e(
        190,
        Some(458),
        "Error validating access token"
    )));
    assert!(!is_disconnect_safe_error(&e(
        100,
        None,
        "(#100) Invalid parameter"
    )));
    for code in [1, 2, 4, 17, 32, 613] {
        assert!(!is_disconnect_safe_error(&e(code, None, "Transient")));
    }
    assert!(!is_disconnect_safe_error(&e(190, None, "Bad token")));
}

#[test]
fn categories_and_fallbacks() {
    assert_eq!(categorize(Some(613), None, None), Category::RateLimited);
    assert_eq!(categorize(Some(190), None, None), Category::AuthFailed);
    assert_eq!(categorize(Some(1), None, None), Category::NetworkError);
    assert_eq!(categorize(Some(506), None, None), Category::PayloadInvalid);
    assert_eq!(
        categorize(Some(9999), None, Some("OAuthException")),
        Category::AuthFailed
    );
    assert_eq!(
        categorize(None, None, Some("OAuthException")),
        Category::Unknown
    );
    // No Graph text: `rescue` falls back to the endpoint.
    let bare = map_error(
        &GraphError::from_body(502, &Value::Null),
        "v23.0/me/messages",
    );
    assert_eq!(
        (
            bare.message.as_str(),
            bare.code.as_deref(),
            bare.http_status_code
        ),
        ("v23.0/me/messages", Some("-1"), 502)
    );
}

const NOW: i64 = 1_780_963_200_000; // 2026-06-09T00:00:00.000Z
const HOUR: i64 = 3_600_000;

#[test]
fn messaging_policy_follows_the_meta_windows() {
    assert_eq!(
        resolve_messaging_policy(None, None, false, NOW),
        Ok(("RESPONSE", None))
    );
    assert_eq!(
        resolve_messaging_policy(Some(NOW - 23 * HOUR), Some("inbox"), false, NOW),
        Ok(("RESPONSE", None))
    );
    assert_eq!(
        resolve_messaging_policy(Some(NOW - 25 * HOUR), Some("inbox"), false, NOW),
        Ok(("MESSAGE_TAG", Some("HUMAN_AGENT")))
    );
    assert_eq!(
        resolve_messaging_policy(None, Some("inbox"), false, NOW),
        Ok(("RESPONSE", None))
    );
    let expired =
        resolve_messaging_policy(Some(NOW - 7 * 24 * HOUR - 1000), Some("inbox"), false, NOW)
            .unwrap_err();
    assert_eq!(
        expired.code.as_deref(),
        Some("messenger_human_agent_window_expired")
    );
    assert_eq!(
        resolve_messaging_policy(Some(NOW - 23 * HOUR), Some("inbox"), true, NOW),
        Ok(("MESSAGE_TAG", Some("HUMAN_AGENT")))
    );
    assert!(
        resolve_messaging_policy(Some(NOW - 7 * 24 * HOUR - 1000), Some("inbox"), true, NOW)
            .is_err()
    );
}

#[test]
fn the_send_payload_has_message_type_and_recipient_only() {
    let payload =
        build_message_payload("psid-1", &json!({ "text": "hi" }), ("RESPONSE", None), None);
    assert_eq!(
        payload,
        json!({ "recipient": { "id": "psid-1" }, "message": { "text": "hi", "metadata": MESSAGE_METADATA }, "messaging_type": "RESPONSE" })
    );
    let tagged = build_message_payload(
        "p",
        &json!({ "text": "x" }),
        ("MESSAGE_TAG", Some("HUMAN_AGENT")),
        Some("fb-a"),
    );
    assert_eq!(
        (tagged["tag"].as_str(), tagged["persona_id"].as_str()),
        (Some("HUMAN_AGENT"), Some("fb-a"))
    );
}

fn text_message(attributes: Value) -> Value {
    json!({ "contentType": "text", "text": "Choose an option", "contentAttributes": attributes })
}

fn button_template(buttons: Value) -> Value {
    json!({ "type": "template", "payload": { "templateType": "button", "buttons": buttons } })
}

fn postback_qr() -> Value {
    json!({ "id": "qr-1", "label": "Yes", "buttonType": "postback", "postback": "flow-1::qr-1" })
}

fn url_qr() -> Value {
    json!({ "id": "qr-2", "label": "Open", "buttonType": "url", "url": "https://example.com/open" })
}

fn booking() -> Value {
    json!({ "id": "booking-1", "label": "Book now", "buttonType": "url", "url": "https://example.com/booking/slug" })
}

#[test]
fn quick_replies_and_the_button_template() {
    let both = chat_messages(
        &text_message(button_template(json!([booking()]))),
        Some(&json!([postback_qr()])),
    );
    assert_eq!(both.len(), 1);
    assert_eq!(both[0]["attachment"]["payload"]["template_type"], "button");
    assert_eq!(
        both[0]["quick_replies"],
        json!([{ "content_type": "text", "title": "Yes", "payload": "flow-1::qr-1" }])
    );
    let no_url = chat_messages(
        &text_message(Value::Null),
        Some(&json!([postback_qr(), url_qr()])),
    );
    assert_eq!(no_url[0]["quick_replies"].as_array().unwrap().len(), 1);
    let only_url = chat_messages(&text_message(Value::Null), Some(&json!([url_qr()])));
    assert!(only_url[0].get("quick_replies").is_none());
    let booking_and_qr = chat_messages(
        &text_message(button_template(json!([booking()]))),
        Some(&json!([booking(), postback_qr()])),
    );
    assert_eq!(
        booking_and_qr[0]["attachment"]["payload"]["buttons"][0]["type"],
        "web_url"
    );
    assert_eq!(
        booking_and_qr[0]["attachment"]["payload"]["buttons"][0]["url"],
        "https://example.com/booking/slug"
    );
    assert_eq!(booking_and_qr[0]["quick_replies"][0]["title"], "Yes");
    let skip = chat_messages(
        &text_message(Value::Null),
        Some(
            &json!([{ "id": "skip-1", "label": "Skip", "buttonType": "postback", "postback": "STEP_INPUT_SKIP|11612473309626368" }]),
        ),
    );
    assert_eq!(
        skip[0]["quick_replies"][0]["payload"],
        "STEP_INPUT_SKIP|11612473309626368"
    );
}

#[test]
fn native_quick_replies_swap_in_email_and_phone() {
    let replies = canonical_quick_replies(&[
        postback_qr(),
        json!({ "id": "e", "label": "Email", "buttonType": "postback", "postback": NATIVE_USER_EMAIL }),
        json!({ "id": "p", "label": "Phone", "buttonType": "postback", "postback": NATIVE_USER_PHONE_NUMBER }),
    ]);
    assert_eq!(
        replies,
        vec![
            json!({ "content_type": "text", "title": "Yes", "payload": "flow-1::qr-1" }),
            json!({ "content_type": "user_email" }),
            json!({ "content_type": "user_phone_number" }),
        ]
    );
}

#[test]
fn multiple_images_go_in_one_attachments_array() {
    let image = |n: u8| json!({ "fileType": "image", "url": format!("https://x/{n}.png") });
    let many =
        convert_message(&json!({ "contentType": "text", "attachments": [image(1), image(2)] }));
    assert_eq!(many.len(), 1);
    assert_eq!(
        many[0]["attachments"],
        json!([
            { "type": "image", "payload": { "url": "https://x/1.png", "is_reusable": true } },
            { "type": "image", "payload": { "url": "https://x/2.png", "is_reusable": true } },
        ])
    );
    let one = convert_message(&json!({ "contentType": "text", "attachments": [image(1)] }));
    assert_eq!(
        one[0]["attachment"],
        json!({ "type": "image", "payload": { "url": "https://x/1.png", "is_reusable": true } })
    );
    let with_file = convert_message(
        &json!({ "contentType": "text", "attachments": [image(1), image(2), { "fileType": "file", "url": "https://x/a.pdf" }] }),
    );
    assert_eq!(with_file.len(), 2);
    assert_eq!(
        with_file[1]["attachment"],
        json!({ "type": "file", "payload": { "url": "https://x/a.pdf", "is_reusable": true } })
    );
}

/// A contact inbox row (every nullable column null).
fn contact_inbox(id: i64, source_id: &str, persona: Option<&str>) -> ContactInbox {
    let conn = chatbotx_db::rusqlite::Connection::open_in_memory().unwrap();
    let columns: Vec<String> = ContactInbox::COLUMNS
        .iter()
        .map(|c| match *c {
            "id" => id.to_string(),
            "createdAt"
            | "updatedAt"
            | "originalContactId"
            | "contactId"
            | "inboxId"
            | "consecutiveFailedReply" => "1".into(),
            "channel" => "'messenger'".into(),
            "source" => "'direct'".into(),
            "sourceId" => format!("'{source_id}'"),
            "personaId" => persona.map_or("NULL".into(), |p| format!("'{p}'")),
            _ => "NULL".into(),
        })
        .collect();
    conn.query_row(&format!("SELECT {}", columns.join(", ")), [], |r| {
        ContactInbox::from_row_at(r, 0)
    })
    .unwrap()
}

fn step_data(step: Value, quick_replies: Option<Value>) -> SendFlowStep {
    SendFlowStep {
        contact: OutgoingContact {
            contact_inbox: contact_inbox(7, "psid-1", None),
            source_conversation_id: None,
        },
        flow_id: "flow-1".into(),
        flow_version_id: Some("fv-1".into()),
        step,
        product_message: None,
        quick_replies,
        metadata: None,
        rich_response: None,
        send_from: None,
        comment_anchor: None,
        message: Value::Null,
    }
}

fn ready(messages: Vec<StepMessage>) -> Vec<Value> {
    messages
        .into_iter()
        .map(|m| match m {
            StepMessage::Ready(v) => v,
            StepMessage::Upload { .. } => panic!("unexpected upload"),
        })
        .collect()
}

fn card(n: usize) -> Value {
    json!({ "id": format!("card-{n}"), "stepType": "sendCard", "title": format!("Card {n}"), "subtitle": format!("Subtitle {n}"),
        "image": { "url": format!("https://example.com/{n}.png") },
        "buttons": [{ "id": format!("btn-{n}"), "label": format!("Button {n}"), "buttonType": null, "beforeStep": null, "steps": [] }] })
}

fn carousel_step(layout: Option<&str>, count: usize) -> Value {
    let mut step = json!({ "id": "step-1", "stepType": "sendCarousel", "cards": (1..=count).map(card).collect::<Vec<_>>() });
    if let Some(layout) = layout {
        step["layout"] = json!(layout);
    }
    step
}

#[test]
fn carousel_layout_sets_the_aspect_ratio_per_chunk() {
    let horizontal =
        ready(convert_flow_step(&step_data(carousel_step(Some("horizontal"), 3), None)).unwrap());
    assert_eq!(horizontal.len(), 1);
    assert_eq!(
        horizontal[0]["attachment"]["payload"]["template_type"],
        "generic"
    );
    assert_eq!(
        horizontal[0]["attachment"]["payload"]["image_aspect_ratio"],
        "horizontal"
    );
    let vertical =
        ready(convert_flow_step(&step_data(carousel_step(Some("vertical"), 3), None)).unwrap());
    assert_eq!(
        vertical[0]["attachment"]["payload"]["image_aspect_ratio"],
        "square"
    );
    assert_eq!(
        vertical[0]["attachment"]["payload"]["elements"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let element = &horizontal[0]["attachment"]["payload"]["elements"][0];
    assert_eq!(element["subtitle"], "Subtitle 1");
    assert_eq!(element["image_url"], "https://example.com/1.png");
    assert_eq!(element["buttons"][0]["title"], "Button 1");
    // A postback button carries flow, version, button and contact inbox.
    assert_eq!(element["buttons"][0]["payload"], "flow-1:fv-1:btn-1:::7");
    let chunks =
        ready(convert_flow_step(&step_data(carousel_step(Some("vertical"), 12), None)).unwrap());
    assert_eq!(chunks.len(), 2);
    assert!(
        chunks
            .iter()
            .all(|c| c["attachment"]["payload"]["image_aspect_ratio"] == "square")
    );
    let legacy = ready(convert_flow_step(&step_data(carousel_step(None, 1), None)).unwrap());
    assert_eq!(
        legacy[0]["attachment"]["payload"]["image_aspect_ratio"],
        "horizontal"
    );
}

#[test]
fn flow_step_quick_replies_attach_to_text_template_and_gif() {
    let qrs = json!([postback_qr(), { "id": "qr-3", "label": "No", "buttonType": "postback", "postback": "flow-1::qr-3" }]);
    let text = ready(
        convert_flow_step(&step_data(
            json!({ "stepType": "sendText", "text": "Hello", "buttons": [] }),
            Some(qrs.clone()),
        ))
        .unwrap(),
    );
    assert_eq!(text[0]["text"], "Hello");
    assert_eq!(text[0]["quick_replies"].as_array().unwrap().len(), 2);
    let with_buttons = ready(
        convert_flow_step(&step_data(
            json!({ "stepType": "sendText", "text": "Hello", "buttons": [{ "id": "b1", "label": "Existing", "buttonType": "sendMessage" }] }),
            Some(qrs.clone()),
        ))
        .unwrap(),
    );
    assert_eq!(
        with_buttons[0]["attachment"]["payload"]["buttons"][0]["title"],
        "Existing"
    );
    assert_eq!(with_buttons[0]["quick_replies"][1]["title"], "No");
    let no_qr = ready(
        convert_flow_step(&step_data(
            json!({ "stepType": "sendText", "text": "Hello", "buttons": [{ "id": "b1", "label": "Existing", "buttonType": "sendMessage" }] }),
            None,
        ))
        .unwrap(),
    );
    assert!(no_qr[0].get("quick_replies").is_none());
    let gif = ready(
        convert_flow_step(&step_data(
            json!({ "stepType": "sendGif", "url": "https://x/a.gif" }),
            Some(qrs),
        ))
        .unwrap(),
    );
    assert_eq!(
        gif[0]["attachment"],
        json!({ "type": "image", "payload": { "url": "https://x/a.gif", "is_reusable": true } })
    );
    assert_eq!(gif[0]["quick_replies"].as_array().unwrap().len(), 2);
    let bare_gif = ready(
        convert_flow_step(&step_data(
            json!({ "stepType": "sendGif", "url": "https://x/a.gif" }),
            None,
        ))
        .unwrap(),
    );
    assert!(bare_gif[0].get("quick_replies").is_none());
}

#[test]
fn multiple_images_step_is_one_message() {
    let step = json!({ "stepType": "sendMultipleImages", "images": [{ "url": "https://x/1.png" }, { "url": "https://x/2.png" }] });
    let one = ready(convert_flow_step(&step_data(step.clone(), None)).unwrap());
    assert_eq!(one.len(), 1);
    assert_eq!(
        one[0]["attachments"],
        json!([{ "type": "image", "payload": { "url": "https://x/1.png" } }, { "type": "image", "payload": { "url": "https://x/2.png" } }])
    );
    assert!(one[0].get("attachment").is_none() && one[0].get("quick_replies").is_none());
    let with_qr = ready(convert_flow_step(&step_data(step, Some(json!([postback_qr()])))).unwrap());
    assert_eq!(
        with_qr[0]["quick_replies"],
        json!([{ "content_type": "text", "title": "Yes", "payload": "flow-1::qr-1" }])
    );
}

#[test]
fn media_with_buttons_uploads_then_uses_a_media_template() {
    let step = json!({ "stepType": "sendImage", "url": "https://x/1.png", "buttons": [{ "id": "b1", "label": "Go", "buttonType": "sendMessage" }] });
    let steps: [StepMessage; 1] = convert_flow_step(&step_data(step, None))
        .unwrap()
        .try_into()
        .unwrap();
    let [
        StepMessage::Upload {
            url,
            media_type,
            mut template,
        },
    ] = steps
    else {
        panic!("expected an upload")
    };
    assert_eq!((url.as_str(), media_type), ("https://x/1.png", "image"));
    fill_attachment_id(&mut template, "att-1");
    assert_eq!(
        template["attachment"]["payload"]["elements"][0]["attachment_id"],
        "att-1"
    );
    assert_eq!(template["attachment"]["payload"]["template_type"], "media");
    let inline = ready(
        convert_flow_step(&step_data(
            json!({ "stepType": "sendVideo", "url": "https://x/v.mp4", "buttons": [] }),
            None,
        ))
        .unwrap(),
    );
    assert_eq!(
        inline[0]["attachment"],
        json!({ "type": "video", "payload": { "url": "https://x/v.mp4", "is_reusable": true } })
    );
    assert!(convert_flow_step(&step_data(json!({ "stepType": "sendCalendar" }), None)).is_err());
}

#[test]
fn persona_resolution_prefers_the_registered_contact_persona() {
    let personas = json!([{ "id": "local-a", "facebookPersonaId": "fb-a" }, { "id": "local-b", "facebookPersonaId": "fb-b" }, { "id": "local-unregistered" }]);
    assert_eq!(
        resolve_persona_id(&personas, Some("fb-default"), None).as_deref(),
        Some("fb-default")
    );
    assert_eq!(
        resolve_persona_id(&personas, Some("fb-default"), Some("local-b")).as_deref(),
        Some("fb-b")
    );
    assert_eq!(
        resolve_persona_id(&personas, Some("fb-default"), Some("local-unregistered")).as_deref(),
        Some("fb-default")
    );
    assert_eq!(
        resolve_persona_id(&personas, Some("fb-default"), Some("local-gone")).as_deref(),
        Some("fb-default")
    );
    assert_eq!(
        resolve_persona_id(&personas, None, Some("local-gone")),
        None
    );
    assert_eq!(
        resolve_persona_id(&Value::Null, Some("fb-default"), Some("local-a")).as_deref(),
        Some("fb-default")
    );
}

#[test]
fn messenger_template_request_orders_button_parameters() {
    let step = json!({ "stepType": "sendMessengerTemplateMessage",
        "template": { "name": "promo", "language": "en_US", "parameterFormat": "NAMED", "params": {
            "header": [{ "type": "text", "text": "H", "parameter_name": "h" }, { "type": "image" }],
            "body": [{ "type": "text", "text": "B", "parameter_name": "b" }],
            "button": [{ "sub_type": "url", "text": "https://x", "index": 0 }] } },
        "buttons": [{ "id": "btn-1", "label": "Go", "buttonType": "sendMessage" }] });
    let request = template_send_request(&step_data(step, None), None);
    assert_eq!(request["messaging_type"], "UTILITY");
    assert_eq!(request["recipient"]["id"], "psid-1");
    assert_eq!(request["message"]["metadata"], MESSAGE_METADATA);
    assert_eq!(
        request["message"]["template"]["components"],
        json!([
            { "type": "header", "parameters": [{ "type": "text", "text": "H", "parameter_name": "h" }] },
            { "type": "body", "parameters": [{ "type": "text", "text": "B", "parameter_name": "b" }] },
            { "type": "buttons", "parameters": [{ "type": "URL", "url": "https://x" }, { "type": "POSTBACK", "payload": "flow-1:fv-1:btn-1" }] },
        ])
    );
}

fn echo(message: Value) -> Received {
    let mut m = json!({ "mid": "mid-1", "is_echo": true });
    m.as_object_mut()
        .unwrap()
        .extend(message.as_object().unwrap().clone());
    parse_incoming(
        "page-1",
        &json!({ "object": "page", "entry": [{ "id": "page-1", "time": 1, "messaging": [{
            "sender": { "id": "page-1" }, "recipient": { "id": "psid-1" }, "timestamp": 1, "message": m }] }] }),
        NOW,
    )
    .unwrap()
}

#[test]
fn echo_app_id_sets_the_origin() {
    assert_eq!(
        echo(json!({ "text": "hi", "app_id": 26_390_203_743_090_i64 }))
            .echo_origin
            .as_deref(),
        Some("firstParty")
    );
    assert_eq!(
        echo(json!({ "text": "hi", "app_id": 1_517_776_481_860_111_i64 }))
            .echo_origin
            .as_deref(),
        Some("thirdParty")
    );
    assert_eq!(
        echo(json!({ "text": "hi", "app_id": "26390203743090" }))
            .echo_origin
            .as_deref(),
        Some("firstParty")
    );
    assert_eq!(
        echo(json!({ "text": "hi", "app_id": 9_007_199_254_740_994_i64 })).echo_origin,
        None
    );
    let received = echo(json!({ "text": "hi" }));
    assert_eq!(received.echo_origin, None);
    let message = received.message.unwrap();
    assert_eq!(
        (
            message.message_type.as_str(),
            received.contact.source_id.as_str()
        ),
        ("outgoing", "psid-1")
    );
}

#[test]
fn echo_of_a_template_uses_its_title_as_text() {
    let received = echo(
        json!({ "attachments": [{ "type": "template", "payload": { "elements": [{ "title": " One " }, { "title": "Two" }] } }] }),
    );
    assert_eq!(received.message.unwrap().text.as_deref(), Some("One\nTwo"));
}

fn customer(event: Value) -> Received {
    let mut item = json!({ "sender": { "id": "psid-1" }, "recipient": { "id": "page-1" }, "timestamp": 1_780_000_000_123_i64 });
    item.as_object_mut()
        .unwrap()
        .extend(event.as_object().unwrap().clone());
    parse_incoming(
        "page-1",
        &json!({ "object": "page", "entry": [{ "id": "page-1", "time": 1, "messaging": [item] }] }),
        NOW,
    )
    .unwrap()
}

#[test]
fn postbacks_quick_replies_and_referrals_are_read() {
    let postback = customer(
        json!({ "postback": { "mid": "m-1", "title": "Start", "payload": "flow-1:fv:btn" } }),
    );
    assert_eq!(postback.postback_action.as_deref(), Some("flow-1:fv:btn"));
    assert_eq!(postback.button_title.as_deref(), Some("Start"));
    assert_eq!(
        postback.message.as_ref().unwrap().text.as_deref(),
        Some("Start")
    );
    assert_eq!(
        postback.thread_control.as_ref().unwrap()["delivery"],
        "owner"
    );
    let quick = customer(
        json!({ "message": { "mid": "m-2", "text": "Yes", "quick_reply": { "payload": "qr-pay", "title": "Yes" } } }),
    );
    assert_eq!(quick.quick_reply_action.as_deref(), Some("qr-pay"));
    let ad = customer(
        json!({ "message": { "mid": "m-3", "referral": { "source": "ADS", "type": "OPEN_THREAD", "ad_id": "ad-1", "text": "Ad text" } } }),
    );
    assert_eq!(
        ad.message.as_ref().unwrap().text.as_deref(),
        Some("Ad text")
    );
    assert_eq!(ad.referral_source.as_deref(), Some("ADS"));
    let referral = ad.referral.unwrap();
    assert_eq!(referral["adId"], "ad-1");
    assert!(referral["raw"].get("text").is_none());
    let shortlink = customer(
        json!({ "referral": { "ref": "promo", "source": "SHORTLINK", "type": "OPEN_THREAD" } }),
    );
    assert_eq!(shortlink.r#ref.as_deref(), Some("promo"));
    assert!(shortlink.message.is_none());
}

#[test]
fn location_and_attachments() {
    let located = customer(
        json!({ "message": { "mid": "m", "attachments": [{ "type": "location", "payload": { "coordinates": { "lat": 1.5, "long": 2.5 } } }] } }),
    );
    let message = located.message.unwrap();
    assert_eq!(message.content_type, "location");
    assert_eq!(
        message.content_attributes,
        Some(json!({ "latitude": "1.5", "longitude": "2.5" }))
    );
    let media = customer(json!({ "message": { "mid": "m", "attachments": [
        { "type": "image", "payload": { "url": "https://scontent.xx.fbcdn.net/a.jpg" } },
        { "type": "image", "payload": { "url": "https://scontent.xx.fbcdn.net/a.jpg" } },
        { "type": "video", "payload": { "url": "https://video.xx.fbcdn.net/v.mp4" } },
    ] } }));
    let attachments = media.message.unwrap().attachments;
    assert_eq!(attachments.len(), 2);
    assert_eq!(
        attachments[0].origin_path,
        "https://scontent.xx.fbcdn.net/a.jpg"
    );
    assert!(parse_incoming("page-1", &json!({ "entry": [{ "id": "page-1" }] }), NOW).is_err());
}
