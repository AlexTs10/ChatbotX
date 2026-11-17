//! `integrations/zalo/__tests__`: `error-mapper`, `call-phone-number-button`,
//! `quick-replies-attachment`, `send-multiple-images`, `outgoing-message-ids`,
//! `outgoing-text-with-images`, `incoming-gif-message` (parse half),
//! `webhook-routing` and `follow-event`. The API mocks become a local HTTP
//! stub. `connection-oauth`, `refresh-access-token`, `get-user-profile`,
//! `tag`, `google-click-partner-samples` and `incoming-image-compression`
//! are n-a here (OAuth, tags, profile and the image store are other units).
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;

// ---- a tiny HTTP stub ------------------------------------------------------

/// Every request the stub saw: `(path, body)`.
type Seen = Arc<Mutex<Vec<(String, String)>>>;

/// Serves `reply(path, call index)` -> `(status, content-type, body)` on a
/// local port; the base URL ends with `/`.
fn stub(
    reply: impl Fn(&str, usize) -> (u16, &'static str, Vec<u8>) + Send + 'static,
) -> (String, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for (index, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split(' ').nth(1).unwrap_or("").to_owned();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                if let Some(v) = header.to_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            log.lock()
                .unwrap()
                .push((path.clone(), String::from_utf8_lossy(&body).into_owned()));
            let (status, content_type, out) = reply(&path, index);
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                out.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&out);
        }
    });
    (base, seen)
}

fn ok_json(value: Value) -> (u16, &'static str, Vec<u8>) {
    (200, "application/json", value.to_string().into_bytes())
}

fn sent_messages(seen: &Seen) -> Vec<Value> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(path, _)| path.ends_with(SEND_MESSAGE))
        .map(|(_, body)| serde_json::from_str::<Value>(body).unwrap()["message"].clone())
        .collect()
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---- error-mapper.test.ts --------------------------------------------------

fn exception(message: &str, status: u16, code: i64) -> ApiError {
    ApiError {
        http_status: status,
        code: Some(code),
        message: message.into(),
    }
}

#[test]
fn code_216_maps_to_user_blocked() {
    let mapped = map_to_channel_error(&exception("User blocked OA", 400, -216));
    assert_eq!(mapped.category, Category::UserBlocked);
    assert!(mapped.is_permanent());
}

#[test]
fn code_139_maps_to_user_blocked() {
    let mapped = map_to_channel_error(&exception("Refused template", 400, -139));
    assert_eq!(mapped.category, Category::UserBlocked);
}

#[test]
fn recognizes_the_invalid_access_token_code() {
    assert!(is_revoked_token_error(&exception(
        "Invalid token",
        400,
        -124
    )));
}

#[test]
fn does_not_treat_unrelated_provider_failures_as_revoked() {
    assert!(!is_revoked_token_error(&exception("Unavailable", 503, -1)));
}

#[test]
fn an_api_envelope_error_carries_its_numeric_code() {
    let error = check_api_error(&json!({ "error": -124, "message": "Invalid token" })).unwrap_err();
    assert_eq!(error, exception("Invalid token", 400, -124));
    assert!(check_api_error(&json!({ "error": 0, "data": {} })).is_ok());
    let oauth = check_api_error(&json!({ "error_name": "Invalid", "error_reason": "bad code" }));
    assert_eq!(oauth.unwrap_err().message, "bad code");
}

// ---- call-phone-number-button.test.ts --------------------------------------

#[test]
fn a_step_button_becomes_oa_open_phone_with_digits_and_no_plus() {
    let button = json!({
        "id": "1",
        "label": "Call us",
        "buttonType": "callPhoneNumber",
        "beforeStep": { "id": "2", "stepType": "callPhoneNumber", "phoneNumber": "+442079460958" },
        "steps": [],
    });
    let ctx = ButtonContext {
        flow_id: "40".into(),
        ..ButtonContext::default()
    };
    assert_eq!(
        button_template(&ctx, &button),
        json!({ "type": "oa.open.phone", "title": "Call us", "payload": { "phone_code": "442079460958" } })
    );
}

#[test]
fn a_canonical_phone_number_button_becomes_oa_open_phone() {
    let button = json!({
        "id": "1",
        "label": "Call us",
        "buttonType": "phoneNumber",
        "phoneNumber": "+442079460958",
        "postback": "payload",
    });
    assert_eq!(
        canonical_button_template(&button),
        json!({ "type": "oa.open.phone", "title": "Call us", "payload": { "phone_code": "442079460958" } })
    );
}

// ---- quick-replies-attachment.test.ts --------------------------------------

fn flow_ctx() -> ButtonContext {
    ButtonContext {
        flow_id: "flow-1".into(),
        contact_inbox_id: Some("ci-1".into()),
        ..ButtonContext::default()
    }
}

fn quick_replies() -> Value {
    json!([
        { "id": "qr-1", "label": "Yes", "buttonType": "postback", "postback": "flow-1::qr-1" },
        { "id": "qr-2", "label": "Open", "buttonType": "url", "url": "https://example.com?code=flow-1::qr-2", "postback": "flow-1::qr-2" },
    ])
}

fn plain_button(id: &str, label: &str) -> Value {
    json!({ "id": id, "label": label, "buttonType": null, "beforeStep": null, "steps": [] })
}

#[test]
fn merges_node_quick_replies_into_the_text_template_buttons() {
    let step = json!({ "id": "step-1", "stepType": "sendText", "text": "Choose", "buttons": [plain_button("btn-1", "Existing")] });
    let payload = convert_flow_step_text(&flow_ctx(), &step, Some(&quick_replies()));
    let buttons = payload["attachment"]["payload"]["buttons"]
        .as_array()
        .unwrap();
    assert_eq!(buttons.len(), 3);
    assert_eq!(buttons[0]["title"], "Existing");
    assert_eq!(buttons[1]["title"], "Yes");
    assert_eq!(buttons[1]["payload"], "postback_flow-1::qr-1");
    assert_eq!(buttons[2]["title"], "Open");
    assert_eq!(
        buttons[2]["payload"],
        json!({ "url": "https://example.com?code=flow-1::qr-2" })
    );
}

#[test]
fn truncates_to_5_buttons_when_merged_buttons_exceed_zalo_max() {
    let step = json!({ "id": "step-1", "stepType": "sendText", "text": "Choose", "buttons": [plain_button("btn-1", "Existing")] });
    let quick: Vec<Value> = (0..5)
        .map(|i| json!({ "id": format!("qr-{i}"), "label": format!("QR {i}"), "buttonType": "postback", "postback": format!("flow-1::qr-{i}") }))
        .collect();
    let payload = convert_flow_step_text(&flow_ctx(), &step, Some(&Value::Array(quick)));
    assert_eq!(
        payload["attachment"]["payload"]["buttons"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn truncates_raw_step_buttons_that_alone_exceed_zalo_max() {
    let buttons: Vec<Value> = (0..6)
        .map(|i| plain_button(&format!("btn-{i}"), &format!("Button {i}")))
        .collect();
    let step =
        json!({ "id": "step-1", "stepType": "sendText", "text": "Choose", "buttons": buttons });
    let payload = convert_flow_step_text(&flow_ctx(), &step, None);
    assert_eq!(
        payload["attachment"]["payload"]["buttons"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn attaches_node_quick_replies_to_gif_media_template_buttons() {
    let (base, _) = stub(|_, _| {
        ok_json(
            json!({ "error": 0, "message": "ok", "data": { "attachment_id": "attachment-1", "width": 320, "height": 180 } }),
        )
    });
    let step = json!({ "id": "step-1", "stepType": "sendGif", "url": format!("{base}anim.gif") });
    let parts = flow_step_parts(&flow_ctx(), &step, Some(&quick_replies())).unwrap();
    let [part] = <[Part; 1]>::try_from(parts).unwrap();
    let payload =
        run(resolve(&client(), &base, "token", part)).unwrap()["attachment"]["payload"].clone();
    assert_eq!(payload["template_type"], "media");
    assert_eq!(payload["elements"][0]["media_type"], "gif");
    assert_eq!(payload["elements"][0]["attachment_id"], "attachment-1");
    let buttons = payload["buttons"].as_array().unwrap();
    assert_eq!(buttons[0]["payload"], "postback_flow-1::qr-1");
    assert_eq!(
        buttons[1]["payload"],
        json!({ "url": "https://example.com?code=flow-1::qr-2" })
    );
}

#[test]
fn encodes_the_skip_postback_with_the_postback_prefix() {
    let step = json!({ "id": "step-1", "stepType": "sendText", "text": "Choose", "buttons": [] });
    let quick = json!([{ "id": "skip-1", "label": "Skip", "buttonType": "postback", "postback": "STEP_INPUT_SKIP|11612473309626368" }]);
    let payload = convert_flow_step_text(&flow_ctx(), &step, Some(&quick));
    assert_eq!(
        payload["attachment"]["payload"]["buttons"],
        json!([{ "type": "oa.query.hide", "title": "Skip", "payload": "postback_STEP_INPUT_SKIP|11612473309626368" }])
    );
}

// ---- send-multiple-images.test.ts ------------------------------------------

#[test]
fn uploads_and_sends_one_single_image_template_per_url() {
    let (base, seen) = stub(|path, _| {
        if path.ends_with("upload/image") {
            ok_json(
                json!({ "error": 0, "message": "ok", "data": { "attachment_id": "att", "width": 100, "height": 100 } }),
            )
        } else if path.ends_with(".png") {
            (200, "image/png", vec![1, 2, 3])
        } else {
            ok_json(json!({ "error": 0, "message": "Success", "data": { "message_id": "m" } }))
        }
    });
    let step = json!({
        "id": "step-1",
        "stepType": "sendMultipleImages",
        "images": [
            { "id": "img-1", "mode": "url", "url": format!("{base}a.png") },
            { "id": "img-2", "mode": "url", "url": format!("{base}b.png") },
        ],
    });
    let parts = flow_step_parts(&flow_ctx(), &step, None).unwrap();
    let result = run(send_parts(
        &client(),
        &base,
        "tok",
        "zalo-uid-1",
        parts,
        false,
    ))
    .unwrap();
    assert_eq!(result.sent_count, 2);
    let uploads = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| p.ends_with("upload/image"))
        .count();
    assert_eq!(uploads, 2);
    let template = json!({ "attachment": { "type": "template", "payload": {
        "template_type": "media",
        "elements": [{ "media_type": "image", "attachment_id": "att", "width": 100, "height": 100 }],
    } } });
    assert_eq!(sent_messages(&seen), vec![template.clone(), template]);
}

// ---- outgoing-message-ids.test.ts ------------------------------------------

fn text_message(text: Option<&str>, attachments: Value) -> Value {
    json!({ "id": "msg-1", "contentType": "text", "messageType": "outgoing", "text": text, "attachments": attachments })
}

#[test]
fn send_message_returns_the_send_api_message_id() {
    let (base, seen) = stub(|_, _| {
        ok_json(
            json!({ "error": 0, "message": "Success", "data": { "message_id": "m_provider-1" } }),
        )
    });
    let parts = message_parts(&text_message(Some("hello"), json!([]))).unwrap();
    let result = run(send_parts(
        &client(),
        &base,
        "tok",
        "zalo-uid-1",
        parts,
        true,
    ))
    .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        result,
        SendResult {
            message_ids: vec!["m_provider-1".into()],
            sent_count: 1
        }
    );
    let body: Value = serde_json::from_str(&seen.lock().unwrap()[0].1).unwrap();
    assert_eq!(
        body,
        json!({ "recipient": { "user_id": "zalo-uid-1" }, "message": { "text": "hello" } })
    );
}

#[test]
fn send_flow_step_send_text_returns_the_send_api_message_id() {
    let (base, _) = stub(|_, _| {
        ok_json(
            json!({ "error": 0, "message": "Success", "data": { "message_id": "m_provider-1" } }),
        )
    });
    let step =
        json!({ "id": "step-1", "stepType": "sendText", "text": "automated reply", "buttons": [] });
    let parts = flow_step_parts(&flow_ctx(), &step, None).unwrap();
    let result = run(send_parts(
        &client(),
        &base,
        "tok",
        "zalo-uid-1",
        parts,
        false,
    ))
    .unwrap();
    assert_eq!(result.message_ids, vec!["m_provider-1".to_owned()]);
    assert_eq!(result.sent_count, 1);
}

#[test]
fn send_message_tolerates_a_response_without_a_message_id() {
    let (base, _) = stub(|_, _| ok_json(json!({ "error": 0, "message": "Success" })));
    let parts = message_parts(&text_message(Some("hello"), json!([]))).unwrap();
    let result = run(send_parts(
        &client(),
        &base,
        "tok",
        "zalo-uid-1",
        parts,
        true,
    ))
    .unwrap();
    assert_eq!(
        result,
        SendResult {
            message_ids: vec![],
            sent_count: 1
        }
    );
}

// ---- outgoing-text-with-images.test.ts -------------------------------------

fn image(url: String) -> Value {
    json!({ "fileType": "image", "mimeType": "image/png", "originPath": url, "size": 10, "url": url })
}

#[test]
fn sends_the_text_first_then_each_image_as_its_own_message() {
    let (base, seen) = stub(|path, _| {
        if path.ends_with(".png") {
            (200, "image/png", vec![1])
        } else if path.contains("upload") {
            ok_json(json!({ "error": 0, "message": "ok", "data": { "attachment_id": "att-1" } }))
        } else {
            ok_json(json!({ "error": 0, "data": { "message_id": "m-1" } }))
        }
    });
    let message = text_message(
        Some("hello"),
        json!([image(format!("{base}a.png")), image(format!("{base}b.png"))]),
    );
    let parts = message_parts(&message).unwrap();
    let result = run(send_parts(
        &client(),
        &base,
        "tok",
        "zalo-uid-1",
        parts,
        true,
    ))
    .unwrap();
    let sent = sent_messages(&seen);
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0], json!({ "text": "hello" }));
    assert_eq!(sent[1]["attachment"]["type"], "template");
    assert_eq!(result.sent_count, 3);
}

#[test]
fn sends_nothing_when_an_image_upload_fails() {
    let (base, seen) = stub(|path, _| {
        if path.ends_with("a.png") {
            (200, "image/png", vec![1])
        } else if path.ends_with("b.png") {
            (404, "text/plain", b"gone".to_vec())
        } else {
            ok_json(
                json!({ "error": 0, "message": "ok", "data": { "attachment_id": "att-1", "message_id": "m-1" } }),
            )
        }
    });
    let message = text_message(
        Some("hello"),
        json!([image(format!("{base}a.png")), image(format!("{base}b.png"))]),
    );
    let parts = message_parts(&message).unwrap();
    assert!(
        run(send_parts(
            &client(),
            &base,
            "tok",
            "zalo-uid-1",
            parts,
            true
        ))
        .is_err()
    );
    assert!(sent_messages(&seen).is_empty());
}

#[test]
fn still_sends_images_when_there_is_no_text() {
    let (base, seen) = stub(|path, _| {
        if path.ends_with(".png") {
            (200, "image/png", vec![1])
        } else {
            ok_json(
                json!({ "error": 0, "message": "ok", "data": { "attachment_id": "att-1", "message_id": "m-1" } }),
            )
        }
    });
    let parts = message_parts(&text_message(None, json!([image(format!("{base}a.png"))]))).unwrap();
    run(send_parts(
        &client(),
        &base,
        "tok",
        "zalo-uid-1",
        parts,
        true,
    ))
    .unwrap();
    assert_eq!(sent_messages(&seen).len(), 1);
}

// ---- incoming-gif-message.test.ts ------------------------------------------

#[test]
fn a_text_less_user_send_gif_keeps_the_gif_to_download() {
    let event = json!({
        "event_name": "user_send_gif",
        "app_id": "960762598645803452",
        "sender": { "id": "3234151001898787112" },
        "recipient": { "id": "4598245833493169062" },
        "message": {
            "attachments": [{ "payload": { "thumbnail": "https://zgif-v2.zdn.vn/8dd43c000745ee1bb754.png", "url": "https://zgif-v2.zdn.vn/833031e40aa1e3ffbab0.gif" }, "type": "gif" }],
            "msg_id": "0309b0c58bb4ecedb5a2",
        },
        "timestamp": "1790413469192",
    });
    let (received, files) = parse_event(&event).unwrap().unwrap();
    assert_eq!(received.contact.source_id, "3234151001898787112");
    assert_eq!(
        files,
        vec![(
            "gif".to_owned(),
            "https://zgif-v2.zdn.vn/833031e40aa1e3ffbab0.gif".to_owned()
        )]
    );
    // The stored fileType / mimeType / channel key come from the media copy
    // (`copy_to_storage`, covered by the WhatsApp media tests).
}

#[test]
fn strips_the_postback_echo_prefix_for_the_skip_payload() {
    let event = json!({
        "event_name": "user_send_text",
        "app_id": "960762598645803452",
        "sender": { "id": "3234151001898787112" },
        "recipient": { "id": "4598245833493169062" },
        "message": { "text": "postback_STEP_INPUT_SKIP|11612473309626368", "msg_id": "skip-msg-1" },
        "timestamp": "1790413469192",
    });
    let (received, _) = parse_event(&event).unwrap().unwrap();
    assert_eq!(
        received.postback_action.as_deref(),
        Some("STEP_INPUT_SKIP|11612473309626368")
    );
}

#[test]
fn a_location_becomes_a_maps_link_with_coordinates() {
    let event = json!({
        "event_name": "user_send_location",
        "app_id": "a",
        "sender": { "id": "u" },
        "recipient": { "id": "oa" },
        "message": { "msg_id": "m", "attachments": [{ "type": "location", "payload": { "coordinates": { "latitude": "10.1", "longitude": "106.2" } } }] },
    });
    let (received, _) = parse_event(&event).unwrap().unwrap();
    let message = received.message.unwrap();
    assert_eq!(message.content_type, "location");
    assert_eq!(
        message.text.as_deref(),
        Some("https://www.google.com/maps/search/?api=1&query=10.1,106.2")
    );
    assert_eq!(
        message.content_attributes,
        Some(json!({ "latitude": "10.1", "longitude": "106.2" }))
    );
}

#[test]
fn an_oa_send_echo_is_outgoing_for_the_recipient_and_msg_id_is_required() {
    let event = json!({ "event_name": "oa_send_text", "app_id": "a", "sender": { "id": "oa" }, "recipient": { "id": "u" }, "message": { "msg_id": "m", "text": "hi" } });
    let (received, _) = parse_event(&event).unwrap().unwrap();
    assert_eq!(received.contact.source_id, "u");
    assert_eq!(received.message.unwrap().message_type, "outgoing");
    let missing = json!({ "event_name": "user_send_text", "app_id": "a", "sender": { "id": "u" }, "recipient": { "id": "oa" }, "message": { "text": "hi" } });
    assert_eq!(
        parse_event(&missing).unwrap_err(),
        "Missing msg_id in message event"
    );
}

// ---- webhook-routing.test.ts -----------------------------------------------

const APP_ID: &str = "app-1";

fn route(body: &Value) -> Result<Option<Job>, String> {
    route_event(Some(APP_ID), None, None, &body.to_string())
}

#[test]
fn acks_user_received_message_delivery_receipts_without_enqueueing() {
    let body = json!({ "app_id": APP_ID, "event_name": "user_received_message", "sender": { "id": "oa-1" }, "recipient": { "id": "user-1" }, "message": { "msg_id": "m-1" } });
    assert_eq!(route(&body), Ok(None));
}

#[test]
fn acks_events_without_a_message_and_does_not_enqueue_them() {
    for name in ["user_submit_info", "some_future_event"] {
        let body = json!({ "app_id": APP_ID, "event_name": name, "sender": { "id": "user-1" }, "recipient": { "id": "oa-1" }, "info": { "name": "Jane", "phone": "0900000000" } });
        assert_eq!(route(&body), Ok(None), "{name}");
    }
}

#[test]
fn routes_user_send_text_to_the_incoming_message_queue() {
    let body = json!({ "app_id": APP_ID, "event_name": "user_send_text", "sender": { "id": "user-1" }, "recipient": { "id": "oa-1" }, "message": { "msg_id": "m-2", "text": "hello" } });
    let job = route(&body).unwrap().unwrap();
    assert_eq!(job.name, "incomingMessage");
    assert_eq!(job.data["type"], "incomingMessage");
    assert_eq!(job.data["data"]["integrationType"], "zalo");
    assert_eq!(job.data["data"]["integrationIdentifier"], "oa-1");
}

#[test]
fn routes_oa_send_text_echoes_to_the_incoming_message_echo_queue() {
    let body = json!({ "app_id": APP_ID, "event_name": "oa_send_text", "sender": { "id": "oa-1" }, "recipient": { "id": "user-1" }, "message": { "msg_id": "m-3", "text": "reply" } });
    let job = route(&body).unwrap().unwrap();
    assert_eq!(job.name, "incomingMessageEcho");
    assert_eq!(job.data["data"]["integrationIdentifier"], "oa-1");
}

#[test]
fn routes_user_seen_message_to_the_contact_mark_as_read_queue() {
    let body = json!({ "app_id": APP_ID, "event_name": "user_seen_message", "sender": { "id": "user-1" }, "recipient": { "id": "oa-1" }, "message": { "msg_ids": ["m-4"] } });
    let job = route(&body).unwrap().unwrap();
    assert_eq!(job.name, "contactMarkAsRead");
    assert_eq!(job.data["data"]["integrationIdentifier"], "oa-1");
    assert_eq!(job.data["data"]["sourceConversationId"], "user-1");
}

#[test]
fn tag_events_route_before_the_app_id_check_and_a_wrong_app_id_fails() {
    let tag = json!({ "app_id": "other", "oa_id": "oa-1", "event_name": "add_user_to_tag", "tag": { "name": "vip" } });
    assert_eq!(route(&tag).unwrap().unwrap().name, "channelLabelChange");
    let wrong = json!({ "app_id": "other", "event_name": "user_send_text", "sender": { "id": "u" }, "recipient": { "id": "oa" } });
    assert_eq!(
        route(&wrong),
        Err("Invalid app_id in webhook payload".into())
    );
    assert_eq!(
        route_event(Some(APP_ID), None, None, ""),
        Err("Empty webhook payload".into())
    );
}

// ---- follow-event.test.ts --------------------------------------------------

fn follow_payload(name: &str) -> Value {
    json!({
        "app_id": "app-1",
        "oa_id": "oa-1",
        "user_id_by_app": "app-user-9",
        "event_name": name,
        "timestamp": "1760000000000",
        "follower": { "id": "user-9" },
    })
}

#[test]
fn maps_a_follow_with_its_qr_source_to_a_follow_audience_job() {
    let mut payload = follow_payload("follow");
    payload["source"] = json!("qr");
    assert_eq!(
        parse_follow_event(&payload),
        Some(json!({
            "integrationType": "zalo",
            "integrationIdentifier": "oa-1",
            "event": {
                "kind": "follow",
                "user": { "sourceId": "user-9" },
                "occurredAt": "2025-10-09T08:53:20.000Z",
                "source": "qr",
            },
        }))
    );
}

#[test]
fn maps_an_unfollow_to_an_unfollow_audience_job() {
    let parsed = parse_follow_event(&follow_payload("unfollow")).unwrap();
    assert_eq!(parsed["event"]["kind"], "unfollow");
}

#[test]
fn returns_none_for_message_events_and_follows_missing_the_follower() {
    let message = json!({ "app_id": "app-1", "event_name": "user_send_text", "sender": { "id": "u" }, "recipient": { "id": "oa-1" } });
    assert_eq!(parse_follow_event(&message), None);
    let mut payload = follow_payload("follow");
    payload.as_object_mut().unwrap().remove("follower");
    assert_eq!(parse_follow_event(&payload), None);
}

const VERIFY: &str = "oa-secret";

fn follow_route(body: &Value, signature: Option<String>) -> Result<Option<Job>, String> {
    route_event(
        Some("app-1"),
        Some(VERIFY),
        signature.as_deref(),
        &body.to_string(),
    )
}

fn sign(raw: &str, body: &Value) -> String {
    format!(
        "mac={}",
        sha256_hex(&format!(
            "{}{raw}{}{VERIFY}",
            body["app_id"].as_str().unwrap(),
            body["timestamp"].as_str().unwrap()
        ))
    )
}

#[test]
fn drops_a_follow_with_a_missing_or_wrong_signature() {
    let body = follow_payload("follow");
    assert_eq!(follow_route(&body, None), Ok(None));
    assert_eq!(follow_route(&body, Some("mac=deadbeef".into())), Ok(None));
}

#[test]
fn queues_a_channel_audience_event_for_a_signed_follow() {
    let body = follow_payload("follow");
    let signature = sign(&body.to_string(), &body);
    let job = follow_route(&body, Some(signature)).unwrap().unwrap();
    assert_eq!(job.name, "channelAudienceEvent");
    assert_eq!(job.data["type"], "channelAudienceEvent");
    assert_eq!(job.data["data"]["integrationIdentifier"], "oa-1");
    assert_eq!(job.data["data"]["event"]["kind"], "follow");
}

#[test]
fn acks_a_malformed_follow_without_queuing() {
    let mut body = follow_payload("unfollow");
    body.as_object_mut().unwrap().remove("follower");
    let signature = sign(&body.to_string(), &body);
    assert_eq!(follow_route(&body, Some(signature)), Ok(None));
}

// ---- zalo-verifier ---------------------------------------------------------

#[test]
fn the_verifier_page_carries_the_escaped_verifier() {
    let page = verifier_page("abc123");
    assert_eq!(page.status, 200);
    assert!(page.body.contains(r#"content="abc123""#));
    assert!(
        verifier_page(r#""><script>"#)
            .body
            .contains("&quot;&gt;&lt;script&gt;")
    );
    assert_eq!(verifier_page("").status, 400);
}
