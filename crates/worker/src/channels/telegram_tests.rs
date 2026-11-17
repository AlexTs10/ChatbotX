//! `integrations/telegram/__tests__`: `group-routing`, `copy-text-button`,
//! `call-phone-number-button`, `quick-replies-attachment`,
//! `send-multiple-images`, `send-message-multiple-images`, `error-mapper`,
//! `connection` (describe) and the parse half of `incoming-message` (the
//! download cases need the file store: the copy code is WhatsApp's, tested
//! there).
use serde_json::{Value, json};

use super::*;

const SECRET: &str = "s3cret-token";

fn user() -> Value {
    json!({ "id": 100, "is_bot": false, "first_name": "Ada", "username": "ada" })
}

fn run(
    update: Value,
    secret: Option<&str>,
    configured: bool,
) -> Result<Option<(&'static str, Value)>, String> {
    let mut body = json!({ "update_id": 1 });
    for (k, v) in update.as_object().unwrap() {
        body[k] = v.clone();
    }
    route_update(
        "bot-1",
        configured.then_some(SECRET),
        secret,
        &body.to_string(),
    )
}

fn ok(update: Value) -> Option<(&'static str, Value)> {
    run(update, Some(SECRET), true).unwrap()
}

fn message(chat: Value, extra: Value) -> Value {
    let mut m = json!({ "message_id": 55, "from": user(), "chat": chat, "date": 1_760_000_000, "text": "hello group" });
    for (k, v) in extra.as_object().unwrap() {
        m[k] = v.clone();
    }
    json!({ "message": m })
}

fn iso_of(ms: i64) -> String {
    chatbotx_core::js::to_iso_string(ms).unwrap()
}

#[test]
fn a_group_message_goes_to_the_audience_job_never_to_incoming_message() {
    let job = ok(message(
        json!({ "id": -1001, "type": "supergroup", "title": "Fans" }),
        json!({}),
    ));
    assert_eq!(
        job,
        Some((
            "channelAudienceEvent",
            json!({ "type": "channelAudienceEvent", "data": {
                "integrationType": "telegram",
                "integrationIdentifier": "bot-1",
                "event": {
                    "kind": "groupMessage",
                    "user": { "sourceId": "100", "firstName": "Ada", "username": "ada" },
                    "chatId": "-1001",
                    "messageId": "55",
                    "textLength": "hello group".len(),
                    "occurredAt": iso_of(1_760_000_000_000),
                },
            }})
        ))
    );
}

#[test]
fn a_group_message_without_a_scorable_sender_is_dropped() {
    let mut bot = user();
    bot["is_bot"] = json!(true);
    assert_eq!(
        ok(message(
            json!({ "id": -1001, "type": "group" }),
            json!({ "from": bot })
        )),
        None
    );
}

#[test]
fn a_private_message_still_goes_to_incoming_message() {
    let (name, data) = ok(message(json!({ "id": 100, "type": "private" }), json!({}))).unwrap();
    assert_eq!(name, "incomingMessage");
    assert_eq!(data["type"], "incomingMessage");
}

#[test]
fn chat_member_join_leave_become_group_join_group_leave() {
    let update = |old: &str, new: &str| {
        json!({ "chat_member": {
            "chat": { "id": -1001, "type": "channel", "title": "News" },
            "from": user(),
            "date": 1_760_000_000,
            "old_chat_member": { "user": user(), "status": old },
            "new_chat_member": { "user": user(), "status": new },
        }})
    };
    let (_, joined) = ok(update("left", "member")).unwrap();
    assert_eq!(joined["data"]["event"]["kind"], "groupJoin");
    assert_eq!(joined["data"]["event"]["chatId"], "-1001");
    let (_, left) = ok(update("member", "kicked")).unwrap();
    assert_eq!(left["data"]["event"]["kind"], "groupLeave");
    assert_eq!(ok(update("member", "administrator")), None);
}

fn bot_user() -> Value {
    let mut u = user();
    u["is_bot"] = json!(true);
    u
}

#[test]
fn my_chat_member_records_the_bots_status_in_the_chat() {
    let (name, data) = ok(json!({ "my_chat_member": {
        "chat": { "id": -1001, "type": "supergroup", "title": "Fans" },
        "from": user(),
        "date": 1_760_000_000,
        "old_chat_member": { "user": bot_user(), "status": "member" },
        "new_chat_member": { "user": bot_user(), "status": "administrator" },
    }}))
    .unwrap();
    assert_eq!(name, "channelAudienceEvent");
    assert_eq!(
        data["data"]["event"],
        json!({
            "kind": "botChatStatus",
            "chat": { "id": "-1001", "type": "supergroup", "title": "Fans" },
            "status": "administrator",
            "oldStatus": "member",
            "user": { "sourceId": "100", "firstName": "Ada", "username": "ada" },
            "occurredAt": iso_of(1_760_000_000_000),
        })
    );
}

fn private_bot_status(old: &str, new: &str) -> Value {
    json!({ "my_chat_member": {
        "chat": { "id": 100, "type": "private", "first_name": "Ada" },
        "from": user(),
        "date": 1_760_000_000,
        "old_chat_member": { "user": bot_user(), "status": old },
        "new_chat_member": { "user": bot_user(), "status": new },
    }})
}

#[test]
fn blocking_and_unblocking_the_bot_in_a_private_chat_carries_the_user_and_both_statuses() {
    for (old, new) in [("member", "kicked"), ("kicked", "member")] {
        let (_, data) = ok(private_bot_status(old, new)).unwrap();
        assert_eq!(
            data["data"]["event"],
            json!({
                "kind": "botChatStatus",
                "chat": { "id": "100", "type": "private" },
                "status": new,
                "oldStatus": old,
                "user": { "sourceId": "100", "firstName": "Ada", "username": "ada" },
                "occurredAt": iso_of(1_760_000_000_000),
            })
        );
    }
}

#[test]
fn a_block_from_a_legacy_bot_without_a_secret_is_not_trusted() {
    assert_eq!(
        run(private_bot_status("member", "kicked"), None, false),
        Ok(None)
    );
}

#[test]
fn rejects_an_update_with_a_wrong_secret_when_the_bot_has_one() {
    let group = message(json!({ "id": -1001, "type": "supergroup" }), json!({}));
    for presented in [Some("forged"), None] {
        let err = run(group.clone(), presented, true).unwrap_err();
        assert!(err.contains("Invalid webhook secret token"));
    }
}

#[test]
fn a_legacy_bot_without_a_stored_secret_keeps_dms_but_never_scores_group_activity() {
    let group = message(json!({ "id": -1001, "type": "supergroup" }), json!({}));
    assert_eq!(run(group, None, false), Ok(None));
    let dm = run(
        message(json!({ "id": 100, "type": "private" }), json!({})),
        None,
        false,
    )
    .unwrap();
    assert_eq!(dm.unwrap().0, "incomingMessage");
}

// ---- buttons ----

const PIX_CODE: &str = "00020126580014br.gov.bcb.pix0136123e4567-e12b-12d1-a456-4266554400005204000053039865802BR5913Fulano de Tal6008BRASILIA62070503***63041D3D";

fn copy_button(text: &str) -> Value {
    json!({ "id": "1", "label": "Copiar PIX", "buttonType": "copyText",
            "beforeStep": { "id": "2", "stepType": "copyText", "text": text }, "steps": [] })
}

fn reply_button(id: &str) -> Value {
    json!({ "id": id, "label": format!("Plan {id}"), "buttonType": "sendMessage",
            "beforeStep": { "id": format!("{id}0"), "stepType": "startAnotherNode", "nodeId": null }, "steps": [] })
}

#[test]
fn copy_text_is_sent_as_a_copy_text_button_carrying_the_text() {
    assert_eq!(
        build_inline_button("40", &copy_button(PIX_CODE)),
        json!({ "text": "Copiar PIX", "copy_text": { "text": PIX_CODE } })
    );
}

#[test]
fn copy_text_falls_back_to_a_reply_button_when_the_text_is_too_long_or_empty() {
    let max = chatbotx_core::flow_config::steps::COPY_TEXT_MAX;
    for text in ["x".repeat(max + 1), String::new()] {
        let button = build_inline_button("40", &copy_button(&text));
        assert!(button.get("copy_text").is_none());
        assert!(button["callback_data"].is_string());
    }
}

#[test]
fn a_stored_copy_button_is_rebuilt_as_a_copy_text_button() {
    let stored = json!({ "id": "1", "label": "Copiar PIX", "buttonType": "copyText", "copyText": PIX_CODE, "postback": "payload" });
    assert_eq!(
        build_canonical_inline_button(&stored),
        json!({ "text": "Copiar PIX", "copy_text": { "text": PIX_CODE } })
    );
}

#[test]
fn keyboard_keeps_up_to_three_buttons_on_one_row_and_one_a_row_past_three() {
    let three = inline_keyboard(
        ["1", "2", "3"]
            .iter()
            .map(|i| build_inline_button("40", &reply_button(i)))
            .collect(),
    );
    assert_eq!(three["inline_keyboard"].as_array().unwrap().len(), 1);
    assert_eq!(three["inline_keyboard"][0].as_array().unwrap().len(), 3);
    let ten = inline_keyboard(
        (1..=10)
            .map(|i| build_inline_button("40", &reply_button(&i.to_string())))
            .collect(),
    );
    let rows = ten["inline_keyboard"].as_array().unwrap();
    assert_eq!(rows.len(), 10);
    assert!(rows.iter().all(|r| r.as_array().unwrap().len() == 1));
    let prebuilt = (0..4)
        .map(|i| json!({ "text": format!("Plan {i}"), "callback_data": format!("p{i}") }))
        .collect();
    assert_eq!(
        inline_keyboard(prebuilt)["inline_keyboard"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn a_call_phone_number_button_is_an_ordinary_reply_button_without_the_number() {
    let call = json!({ "id": "1", "label": "Call us", "buttonType": "callPhoneNumber",
        "beforeStep": { "id": "2", "stepType": "callPhoneNumber", "phoneNumber": "+14155552671" }, "steps": [] });
    let button = build_inline_button("40", &call);
    assert_eq!(button["text"], "Call us");
    assert!(button["callback_data"].is_string());
    assert!(
        !button["callback_data"]
            .as_str()
            .unwrap()
            .contains("14155552671")
    );
}

fn quick() -> Value {
    json!([
        { "id": "qr-1", "label": "Yes", "buttonType": "postback", "postback": "flow-1::qr-1" },
        { "id": "qr-2", "label": "Open", "buttonType": "url", "url": "https://example.com?code=flow-1::qr-2", "postback": "flow-1::qr-2" },
    ])
}

#[test]
fn merges_node_quick_replies_into_the_inline_keyboard() {
    let step = json!({ "id": "step-1", "stepType": "sendText", "text": "Choose",
        "buttons": [{ "id": "btn-1", "label": "Existing", "buttonType": null, "beforeStep": null, "steps": [] }] });
    let requests = convert_flow_step("chat-1", "flow-1", &step, Some(&quick())).unwrap();
    let keyboard: Vec<Value> = requests[0].1["reply_markup"]["inline_keyboard"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r.as_array().unwrap().clone())
        .collect();
    assert_eq!(keyboard[0]["text"], "Existing");
    assert_eq!(
        keyboard[1],
        json!({ "text": "Yes", "callback_data": "flow-1::qr-1" })
    );
    assert_eq!(
        keyboard[2],
        json!({ "text": "Open", "url": "https://example.com?code=flow-1::qr-2" })
    );
}

#[test]
fn adds_node_quick_replies_to_image_and_file_payloads() {
    for (kind, field) in [("sendImage", "photo"), ("sendFile", "document")] {
        let step = json!({ "id": "step-1", "stepType": kind, "url": "https://example.com/x", "buttons": [] });
        let (_, payload) =
            &convert_flow_step("chat-1", "flow-1", &step, Some(&quick())).unwrap()[0];
        assert_eq!(payload[field], "https://example.com/x");
        assert_eq!(
            payload["reply_markup"],
            json!({ "inline_keyboard": [[
                { "text": "Yes", "callback_data": "flow-1::qr-1" },
                { "text": "Open", "url": "https://example.com?code=flow-1::qr-2" },
            ]] })
        );
    }
}

#[test]
fn encodes_the_skip_postback_unchanged_as_the_inline_keyboard_callback_data() {
    let step = json!({ "id": "step-1", "stepType": "sendText", "text": "Choose", "buttons": [] });
    let skip = json!([{ "id": "skip-1", "label": "Skip", "buttonType": "postback", "postback": "STEP_INPUT_SKIP|11612473309626368" }]);
    let (_, payload) = &convert_flow_step("chat-1", "flow-1", &step, Some(&skip)).unwrap()[0];
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][0],
        json!({ "text": "Skip", "callback_data": "STEP_INPUT_SKIP|11612473309626368" })
    );
}

fn images_step() -> Value {
    json!({ "id": "step-1", "stepType": "sendMultipleImages", "images": [
        { "id": "img-1", "mode": "url", "url": "https://example.com/a.png" },
        { "id": "img-2", "mode": "url", "url": "https://example.com/b.png" },
        { "id": "img-3", "mode": "url", "url": "https://example.com/c.png" },
    ]})
}

fn photos() -> Value {
    json!([
        { "type": "photo", "media": "https://example.com/a.png" },
        { "type": "photo", "media": "https://example.com/b.png" },
        { "type": "photo", "media": "https://example.com/c.png" },
    ])
}

#[test]
fn multiple_images_yield_exactly_one_send_media_group_payload() {
    let requests = convert_flow_step("chat-1", "f", &images_step(), None).unwrap();
    assert_eq!(
        requests,
        vec![(
            "sendMediaGroup",
            json!({ "chat_id": "chat-1", "media": photos() })
        )]
    );
}

#[test]
fn multiple_images_yield_nothing_without_a_chat_id() {
    assert!(
        convert_flow_step("", "f", &images_step(), None)
            .unwrap()
            .is_empty()
    );
}

fn image(url: &str) -> Value {
    json!({ "fileType": "image", "mimeType": "image/png", "url": url })
}

#[test]
fn send_message_batches_multiple_image_attachments_into_one_media_group() {
    let message = json!({ "id": "msg-1", "contentType": "text", "text": null, "attachments": [
        image("https://example.com/a.png"), image("https://example.com/b.png"), image("https://example.com/c.png"),
    ]});
    assert_eq!(
        convert_message("chat-1", &message, None),
        vec![(
            "sendMediaGroup",
            json!({ "chat_id": "chat-1", "media": photos() })
        )]
    );
}

#[test]
fn send_message_with_a_single_image_still_uses_send_photo() {
    let message = json!({ "id": "msg-1", "contentType": "text", "text": null, "attachments": [image("https://example.com/a.png")] });
    let requests = convert_message("chat-1", &message, None);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "sendPhoto");
}

// ---- errors / describe ----

#[test]
fn recognizes_unauthorized_bot_token_responses_as_revoked() {
    let err = |status| ApiError {
        http_status: status,
        code: None,
        message: "x".into(),
    };
    assert!(is_revoked_token_error(&err(401)));
    assert!(!is_revoked_token_error(&err(503)));
}

#[test]
fn describe_rejects_malformed_auth_rather_than_exposing_the_token() {
    for secret in ["bot-secret", "bot:secret"] {
        assert_eq!(
            bot_id_of_token(secret),
            Err("Telegram auth has no bot identity")
        );
    }
    assert_eq!(bot_id_of_token("123:abc"), Ok("123"));
    assert_ne!(bot_id_of_token("123:abc"), bot_id_of_token("456:abc"));
}

#[test]
fn maps_bot_api_descriptions_and_statuses_to_categories() {
    assert_eq!(
        categorize(403, "forbidden: bot was blocked by the user"),
        Category::UserBlocked
    );
    assert_eq!(
        categorize(400, "bad request: chat not found"),
        Category::InvalidRecipient
    );
    assert_eq!(
        categorize(429, "too many requests: retry after 5"),
        Category::RateLimited
    );
    assert_eq!(categorize(502, ""), Category::NetworkError);
    assert_eq!(categorize(418, ""), Category::Unknown);
}

// ---- receive (parse half) ----

#[test]
fn stores_locale_from_message_sender_language_code() {
    let update = json!({ "update_id": 1, "message": { "message_id": 10,
        "from": { "id": 100, "is_bot": false, "first_name": "Ada", "last_name": "Lovelace", "language_code": "vi" },
        "chat": { "id": 100, "type": "private" }, "date": 1_765_440_000, "text": "hello" } });
    let (received, files) = parse_update(&update).unwrap();
    assert_eq!(received.contact.locale.as_deref(), Some("vi"));
    assert_eq!(received.contact.source_id, "100");
    assert!(files.is_empty());
}

#[test]
fn stores_locale_from_callback_query_sender_language_code() {
    let update = json!({ "update_id": 1, "callback_query": { "id": "callback-1",
        "from": { "id": 100, "is_bot": false, "first_name": "Ada", "language_code": "vi" }, "data": "button-1" } });
    assert_eq!(
        parse_update(&update).unwrap().0.contact.locale.as_deref(),
        Some("vi")
    );
}

#[test]
fn hands_the_step_input_skip_callback_data_to_the_worker_unchanged() {
    let update = json!({ "update_id": 1, "callback_query": { "id": "callback-skip",
        "from": { "id": 100, "is_bot": false, "first_name": "Ada" }, "data": "STEP_INPUT_SKIP|11612473309626368" } });
    let (received, _) = parse_update(&update).unwrap();
    assert_eq!(
        received.postback_action.as_deref(),
        Some("STEP_INPUT_SKIP|11612473309626368")
    );
    assert_eq!(
        received.message.unwrap().text.as_deref(),
        Some("STEP_INPUT_SKIP|11612473309626368")
    );
}

#[test]
fn leaves_locale_undefined_when_message_sender_is_absent() {
    let update = json!({ "update_id": 1, "message": { "message_id": 10,
        "chat": { "id": 100, "type": "private" }, "date": 1_765_440_000, "text": "hello" } });
    assert_eq!(parse_update(&update).unwrap().0.contact.locale, None);
}

#[test]
fn start_command_carries_the_ref() {
    let update = json!({ "update_id": 1, "message": { "message_id": 10, "from": user(),
        "chat": { "id": 100, "type": "private" }, "date": 1, "text": "/start promo-1" } });
    assert_eq!(
        parse_update(&update).unwrap().0.r#ref.as_deref(),
        Some("promo-1")
    );
}

fn media_message(media: Value) -> Value {
    let mut m = json!({ "message_id": 11, "from": { "id": 100, "is_bot": false, "first_name": "Ada" },
        "chat": { "id": 100, "type": "private" }, "date": 1_765_440_000 });
    for (k, v) in media.as_object().unwrap() {
        m[k] = v.clone();
    }
    m
}

#[test]
fn attaches_the_largest_photo_with_its_width_height() {
    let files = media_requests(&media_message(json!({ "photo": [
        { "file_id": "small", "file_unique_id": "s", "width": 90, "height": 90, "file_size": 2366 },
        { "file_id": "largest", "file_unique_id": "l", "width": 700, "height": 700, "file_size": 51_569 },
    ]})));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_id, "largest");
    assert_eq!(
        (files[0].file_type.as_str(), files[0].mime_type.as_str()),
        ("image", "image/jpeg")
    );
    assert_eq!((files[0].width, files[0].height), (Some(700), Some(700)));
    assert!(files[0].is_photo_message);
}

#[test]
fn stores_an_animation_once_as_a_gif_ignoring_the_duplicate_document() {
    let gif = json!({ "file_id": "gif-file", "file_unique_id": "g", "mime_type": "video/mp4" });
    let files = media_requests(&media_message(json!({ "animation": gif, "document": gif })));
    assert_eq!(files.len(), 1);
    assert_eq!(
        (files[0].file_type.as_str(), files[0].mime_type.as_str()),
        ("gif", "video/mp4")
    );
}

#[test]
fn stickers_map_to_webp_images_gifs_or_their_thumbnail() {
    let sticker = |extra: Value| {
        let mut s = json!({ "file_id": "sticker-file", "file_unique_id": "u", "width": 512, "height": 512, "is_animated": false, "is_video": false });
        for (k, v) in extra.as_object().unwrap() {
            s[k] = v.clone();
        }
        media_requests(&media_message(json!({ "sticker": s })))
    };
    let static_sticker = sticker(json!({}));
    assert_eq!(
        (
            static_sticker[0].file_type.as_str(),
            static_sticker[0].mime_type.as_str()
        ),
        ("image", "image/webp")
    );
    assert_eq!(static_sticker[0].width, Some(512));
    let video = sticker(json!({ "is_video": true }));
    assert_eq!(
        (video[0].file_type.as_str(), video[0].mime_type.as_str()),
        ("gif", "video/webm")
    );
    let tgs = sticker(
        json!({ "is_animated": true, "thumbnail": { "file_id": "tgs-thumb", "file_unique_id": "t", "width": 128, "height": 128 } }),
    );
    assert_eq!(tgs[0].file_id, "tgs-thumb");
    assert_eq!(tgs[0].width, Some(128));
    assert!(sticker(json!({ "is_animated": true })).is_empty());
}
