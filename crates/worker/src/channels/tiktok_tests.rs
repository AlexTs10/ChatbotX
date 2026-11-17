//! `integrations/tiktok/__tests__`: `webhook`, `comment-webhook`,
//! `high-intent-comment-webhook`, `incoming-message`,
//! `quick-replies-attachment` and the error mapper. The HTTP-mocked API
//! tests (`direct-reply-api`, `message-send`, comment API) are n-a: the
//! payload shapes they pin are asserted on the builders here.
use serde_json::{Value, json};

use super::*;

const SECRET: &str = "test-client-secret";
const NOW: i64 = 1_800_000_000;

fn sign(body: &str, timestamp: i64, secret: &str) -> String {
    format!(
        "t={timestamp},s={}",
        hmac_sha256_hex(secret, &format!("{timestamp}.{body}"))
    )
}

fn payload(event: &str) -> Value {
    json!({
        "client_key": "client-key",
        "event": event,
        "create_time": 1_700_000_000,
        "user_openid": "sender-open-id",
        "content": r#"{"message":"hello"}"#,
    })
}

fn handle(body: &Value, signature: Option<String>, secret: &str) -> Result<Option<Job>, String> {
    let body = body.to_string();
    handle_request(
        Some(secret),
        Some("account-open-id"),
        signature.as_deref(),
        &body,
        NOW,
    )
}

fn signed(body: &Value) -> Result<Option<Job>, String> {
    handle(body, Some(sign(&body.to_string(), NOW, SECRET)), SECRET)
}

#[test]
fn enqueues_dm_events_as_incoming_tiktok_messages() {
    for (event, name, delay) in [
        ("im_receive_msg", "incomingMessage", None),
        ("im_send_msg", "incomingMessageEcho", Some(2000)),
    ] {
        let body = payload(event);
        let job = signed(&body).unwrap().unwrap();
        assert_eq!(job.name, name);
        assert_eq!(
            job.data,
            json!({ "type": name, "data": {
                "integrationType": "tiktok",
                "integrationIdentifier": "account-open-id",
                "payload": body,
            }})
        );
        assert_eq!(job.options.delay_ms, delay);
    }
}

#[test]
fn does_not_enqueue_removal_unsupported_or_schema_invalid_events() {
    assert_eq!(signed(&payload("authorization.removed")), Ok(None));
    assert_eq!(signed(&payload("profile.updated")), Ok(None));
    let invalid = json!({ "client_key": "client-key", "create_time": "not-a-number",
        "event": "im_receive_msg", "user_openid": "sender-open-id" });
    assert_eq!(signed(&invalid), Ok(None));
}

#[test]
fn rejects_missing_forged_replayed_or_future_signatures() {
    let body = payload("im_receive_msg");
    let text = body.to_string();
    for signature in [
        None,
        Some(sign(&text, NOW, "wrong-secret")),
        Some(sign(&text, NOW - 301, SECRET)),
        Some(sign(&text, NOW + 3, SECRET)),
    ] {
        let err = handle(&body, signature, SECRET).unwrap_err();
        assert!(
            err.contains("Invalid or missing webhook signature"),
            "{err}"
        );
    }
    assert!(handle(&body, Some(sign(&text, NOW - 300, SECRET)), SECRET).is_ok());
}

#[test]
fn rejects_an_empty_payload_and_a_missing_client_secret() {
    let err =
        handle_request(Some(SECRET), None, Some(&sign("", NOW, SECRET)), "", NOW).unwrap_err();
    assert!(err.contains("Empty webhook payload"));
    let body = payload("im_receive_msg").to_string();
    let err = handle_request(Some(""), None, Some(&sign(&body, NOW, "")), &body, NOW).unwrap_err();
    assert!(err.contains("Missing client secret for webhook verification"));
}

// ---- comment.update ----

const COMMENT_ID: &str = "7247303576418566913";
const VIDEO_ID: &str = "7203946942097902849";
const PARENT_ID: &str = "7235861947622916866";

fn comment_content(overrides: &[(&str, Option<&str>)]) -> String {
    let mut fields: Vec<(String, String)> = vec![
        ("comment_id".into(), COMMENT_ID.into()),
        ("video_id".into(), VIDEO_ID.into()),
        ("parent_comment_id".into(), "0".into()),
        ("comment_type".into(), r#""comment""#.into()),
        ("comment_action".into(), r#""insert""#.into()),
        ("unique_identifier".into(), r#""+ABc1D2/E0fGhijkl""#.into()),
        ("timestamp".into(), "1687394416109".into()),
        ("text".into(), r#""nice video""#.into()),
    ];
    for (key, value) in overrides {
        fields.retain(|(k, _)| k != key);
        if let Some(value) = value {
            fields.push(((*key).into(), (*value).into()));
        }
    }
    let inner: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!(r#""{k}":{v}"#))
        .collect();
    format!("{{{}}}", inner.join(","))
}

fn comment_job(content: &str, event: &str) -> Option<Job> {
    let event = json!({ "client_key": "key-1", "event": event, "create_time": 1_615_338_610,
        "user_openid": "open-1", "content": content });
    route_event(None, &event)
}

fn comment(overrides: &[(&str, Option<&str>)]) -> Option<Job> {
    comment_job(&comment_content(overrides), "comment.update")
}

#[test]
fn keeps_snowflake_ids_exact_and_the_timestamp_a_number() {
    let parsed = parse_comment_content(&comment_content(&[])).unwrap();
    assert_eq!(parsed["comment_id"], COMMENT_ID);
    assert_eq!(parsed["video_id"], VIDEO_ID);
    assert_eq!(parsed["timestamp"], 1_687_394_416_109_i64);
    let quoted = comment_content(&[("comment_id", Some(&format!(r#""{COMMENT_ID}""#)))]);
    assert_eq!(
        parse_comment_content(&quoted).unwrap()["comment_id"],
        COMMENT_ID
    );
    assert_eq!(parse_comment_content("{not json"), None);
    assert_eq!(
        parse_comment_content(&comment_content(&[("comment_action", Some(r#""pinned""#))])),
        None
    );
}

#[test]
fn enqueues_incoming_comment_for_a_new_comment_ids_intact() {
    let job = comment(&[]).unwrap();
    assert_eq!(job.name, "incomingComment");
    assert_eq!(
        job.data,
        json!({ "type": "incomingComment", "data": {
            "integrationType": "tiktok",
            "integrationIdentifier": "open-1",
            "commentData": {
                "commentId": COMMENT_ID,
                "postId": VIDEO_ID,
                "fromId": "+ABc1D2/E0fGhijkl",
                "message": "nice video",
                "createdTime": 1_687_394_416,
            },
        }})
    );
}

fn parent_of(job: Option<Job>) -> Value {
    job.unwrap().data["data"]["commentData"]["parentId"].clone()
}

#[test]
fn resolves_the_parent_comment_id() {
    assert_eq!(
        parent_of(comment(&[
            ("comment_type", Some(r#""reply""#)),
            ("parent_comment_id", Some(PARENT_ID))
        ])),
        PARENT_ID
    );
    // The `0` sentinel is no parent; `comment_type` beats a parent id.
    assert_eq!(parent_of(comment(&[])), Value::Null);
    assert_eq!(
        parent_of(comment(&[("parent_comment_id", Some(PARENT_ID))])),
        Value::Null
    );
    assert_eq!(parent_of(comment(&[("comment_type", None)])), Value::Null);
    assert_eq!(
        parent_of(comment(&[
            ("comment_type", None),
            ("parent_comment_id", Some(PARENT_ID))
        ])),
        PARENT_ID
    );
    assert_eq!(
        parent_of(comment(&[("parent_comment_id", Some("null"))])),
        Value::Null
    );
}

#[test]
fn falls_back_to_the_envelope_create_time() {
    let job = comment(&[("timestamp", None)]).unwrap();
    assert_eq!(
        job.data["data"]["commentData"]["createdTime"],
        1_615_338_610
    );
}

#[test]
fn routes_deletions_and_drops_visibility_changes_and_anonymous_comments() {
    let job = comment(&[("comment_action", Some(r#""delete""#))]).unwrap();
    assert_eq!(job.name, "deleteIncomingComment");
    assert_eq!(
        job.data,
        json!({ "type": "deleteIncomingComment", "data": {
            "integrationType": "tiktok", "integrationIdentifier": "open-1", "commentId": COMMENT_ID,
        }})
    );
    for action in ["set_to_hidden", "set_to_friends_only", "set_to_public"] {
        assert_eq!(
            comment(&[("comment_action", Some(&format!(r#""{action}""#)))]),
            None
        );
    }
    assert_eq!(comment_job("{not json", "comment.update"), None);
    assert_eq!(comment(&[("unique_identifier", None)]), None);
    let delete = comment(&[
        ("comment_action", Some(r#""delete""#)),
        ("unique_identifier", None),
    ]);
    assert_eq!(delete.unwrap().name, "deleteIncomingComment");
    let dm = r#"{"from_user":{"id":"user-1"},"conversation_id":"conv-1","type":"text","text":{"body":"hi"}}"#;
    assert_eq!(
        comment_job(dm, "im_receive_msg").unwrap().name,
        "incomingMessage"
    );
}

// ---- im_receive_high_intent_comment ----

#[test]
fn enqueues_high_intent_comments_with_the_id_intact_and_a_backoff() {
    let content = format!(
        r#"{{"comment_id":{COMMENT_ID},"comment_text":"how much?","unique_identifier":"+ABc","is_follower":true,"timestamp":1687394416109}}"#
    );
    let job = comment_job(&content, "im_receive_high_intent_comment").unwrap();
    assert_eq!(job.name, "tiktokHighIntentComment");
    assert_eq!(
        job.data["data"],
        json!({
            "integrationType": "tiktok",
            "integrationIdentifier": "open-1",
            "commentId": COMMENT_ID,
            "commentText": "how much?",
            "uniqueIdentifier": "+ABc",
            "isFollower": true,
            "commentedAt": 1_687_394_416,
        })
    );
    assert_eq!(job.options.attempts, Some(6));
    assert_eq!(job.options.backoff, Some(Backoff::Exponential(30_000)));
    let no_time = format!(r#"{{"comment_id":"{COMMENT_ID}"}}"#);
    let job = comment_job(&no_time, "im_receive_high_intent_comment").unwrap();
    assert_eq!(job.data["data"]["commentedAt"], 1_615_338_610);
    let event = json!({ "client_key": "k", "event": "im_receive_high_intent_comment", "create_time": 1,
        "user_openid": "open-1", "content": no_time });
    assert_eq!(
        route_event(Some("configured"), &event).unwrap().data["data"]["integrationIdentifier"],
        "configured"
    );
    assert_eq!(
        comment_job("{not json", "im_receive_high_intent_comment"),
        None
    );
}

// ---- receiveMessage ----

const BUSINESS: &str = "-000m4JH_b1JiHPxoOSKii9FmQCeIA2fZ7Jw";
const CUSTOMER: &str = "+ZMM6I2kBFMYfuEhZjEEt6o0FEebXi68VHcHvO0WgQlZMkbqHafwNm7D4qoyqm52";
const VIDEO: &str = "7685373940801228040";

fn inbound(overrides: Value) -> Value {
    let mut content = json!({
        "timestamp": 1_789_814_497_488_i64,
        "unique_identifier": CUSTOMER,
        "from_user": { "id": CUSTOMER, "role": "personal_account" },
        "to_user": { "id": BUSINESS, "role": "business_account" },
        "from": "wthzu_nb",
        "to": "hunghero32",
        "conversation_id": "u5JhpselBAhoPOlnH0tZ9y07kg==",
        "message_id": "u5Buqc6lAghuOelnHUZV88DK2A==",
        "type": "text",
        "text": { "body": "hello" },
    });
    for (k, v) in overrides.as_object().unwrap() {
        if v.is_null() {
            content.as_object_mut().unwrap().remove(k);
        } else {
            content[k] = v.clone();
        }
    }
    content
}

fn receive(event: &str, content: Value) -> Received {
    parse_event(
        &json!({ "client_key": "7623241219121414161", "event": event, "create_time": 1_789_814_497,
        "user_openid": BUSINESS, "content": content.to_string() }),
    )
    .unwrap()
}

#[test]
fn renders_a_shared_video_as_its_link() {
    let embed = format!("https://www.tiktok.com/player/v1/{VIDEO}?music_info=1");
    let message = receive(
        "im_receive_msg",
        inbound(json!({ "type": "share_post", "text": null,
        "share_post": { "video_id": VIDEO, "embed_url": embed } })),
    )
    .message
    .unwrap();
    assert_eq!(message.text.as_deref(), Some(embed.as_str()));
    assert!(message.attachments.is_empty());
    assert_eq!(
        message.content_attributes,
        Some(json!({ "type": "shared_post", "sharedPost": { "postId": VIDEO, "url": embed } }))
    );
    let bare = receive(
        "im_receive_msg",
        inbound(json!({ "type": "share_post", "text": null,
        "share_post": { "video_id": VIDEO } })),
    )
    .message
    .unwrap();
    assert_eq!(bare.text.as_deref(), Some(VIDEO));
    assert_eq!(
        bare.content_attributes.unwrap()["sharedPost"]["url"],
        Value::Null
    );
    let odd = receive(
        "im_receive_msg",
        inbound(json!({ "type": "share_post", "text": null,
        "share_post": { "video_id": 7_685_373_940_801_228_000_u64 } })),
    )
    .message
    .unwrap();
    assert_eq!(
        odd.source_id.as_deref(),
        Some("u5Buqc6lAghuOelnHUZV88DK2A==")
    );
    assert_eq!((odd.text, odd.content_attributes), (None, None));
}

#[test]
fn leaves_text_untouched_and_unknown_types_empty() {
    let plain = receive("im_receive_msg", inbound(json!({})))
        .message
        .unwrap();
    assert_eq!(plain.text.as_deref(), Some("hello"));
    assert_eq!(plain.content_attributes, None);
    let future = receive(
        "im_receive_msg",
        inbound(json!({ "type": "some_future_type", "text": null })),
    )
    .message
    .unwrap();
    assert_eq!(future.text, None);
    assert!(future.attachments.is_empty());
}

#[test]
fn keys_inbound_contacts_by_from_user_and_echoes_by_to_user() {
    let inbound_received = receive("im_receive_msg", inbound(json!({})));
    assert_eq!(inbound_received.contact.source_id, CUSTOMER);
    assert_eq!(inbound_received.message.unwrap().message_type, "incoming");
    assert_eq!(
        inbound_received.contact.channel_conversation_id.as_deref(),
        Some("u5JhpselBAhoPOlnH0tZ9y07kg==")
    );
    let echo = receive(
        "im_send_msg",
        inbound(json!({
            "text": { "body": "reply from the business" },
            "unique_identifier": BUSINESS,
            "from_user": { "id": BUSINESS, "role": "business_account" },
            "to_user": { "id": CUSTOMER, "role": "personal_account" },
            "from": "hunghero32",
            "to": "wthzu_nb",
        })),
    );
    assert_eq!(echo.contact.source_id, CUSTOMER);
    assert_eq!(echo.message.unwrap().message_type, "outgoing");
}

#[test]
fn hands_the_reply_source_unique_id_over_as_the_postback() {
    let received = receive(
        "im_receive_msg",
        inbound(json!({ "reply_source_payload": {
            "reply_source_msg_id": "u5Buqc6lAghuOelnHUZV88DK2A==",
            "reply_source_unique_id": "STEP_INPUT_SKIP|11612473309626368",
        }})),
    );
    assert_eq!(
        received.postback_action.as_deref(),
        Some("STEP_INPUT_SKIP|11612473309626368")
    );
}

#[test]
fn an_image_dm_keeps_the_media_url_with_its_mime_type() {
    let message = receive(
        "im_receive_msg",
        inbound(json!({ "type": "image", "text": null,
        "media_url": "https://cdn.example.com/a.PNG?x=1" })),
    )
    .message
    .unwrap();
    assert_eq!(message.attachments[0].mime_type, "image/png");
    assert_eq!(
        message.attachments[0].origin_path,
        "https://cdn.example.com/a.PNG?x=1"
    );
    assert_eq!(detect_image_mime_type("https://x/y"), "image/jpeg");
}

// ---- quick replies (`convertFlowStepText`) ----

fn button_ctx() -> ButtonContext {
    ButtonContext {
        flow_id: "flow-1".into(),
        contact_inbox_id: Some("ci-1".into()),
        ..ButtonContext::default()
    }
}

fn text_step(text: &str, buttons: Value) -> Value {
    json!({ "id": "step-1", "stepType": "sendText", "text": text, "buttons": buttons })
}

fn qr() -> Value {
    json!({ "id": "qr-1", "label": "Yes", "buttonType": "postback", "postback": "flow-1::qr-1" })
}

fn url_qr() -> Value {
    json!({ "id": "qr-2", "label": "Open", "buttonType": "url",
            "url": "https://example.com?code=flow-1::qr-2", "postback": "flow-1::qr-2" })
}

fn convert(step: &Value, quick: Value) -> Vec<Value> {
    convert_flow_step_text("business-1", "conv-1", &button_ctx(), step, Some(&quick))
}

#[test]
fn turns_text_plus_quick_replies_into_template_cards() {
    let payloads = convert(&text_step("Choose", json!([])), json!([qr()]));
    assert_eq!(payloads[0]["message_type"], "TEMPLATE");
    assert_eq!(payloads[0]["template"]["type"], "QA_BUTTON_CARD");
    assert_eq!(payloads[0]["template"]["title"], "Choose");
    assert_eq!(
        payloads[0]["template"]["buttons"],
        json!([{ "type": "REPLY", "id": "flow-1::qr-1", "title": "Yes" }])
    );
    let plain = convert_flow_step_text(
        "business-1",
        "conv-1",
        &button_ctx(),
        &text_step("Hi", json!([])),
        None,
    );
    assert_eq!(plain[0]["message_type"], "TEXT");
    assert_eq!(plain[0]["text"]["body"], "Hi");
}

#[test]
fn merges_step_buttons_with_quick_replies() {
    let existing = json!([{ "id": "btn-1", "label": "Existing", "buttonType": null, "beforeStep": null, "steps": [] }]);
    let buttons =
        convert(&text_step("Choose", existing), json!([qr()]))[0]["template"]["buttons"].clone();
    assert_eq!(buttons[0]["title"], "Existing");
    assert_eq!(
        buttons[1],
        json!({ "type": "REPLY", "id": "flow-1::qr-1", "title": "Yes" })
    );
}

#[test]
fn keeps_url_buttons_and_url_quick_replies_in_link_cards() {
    let site = json!([{ "id": "btn-1", "label": "Open site", "buttonType": "openWebsite",
        "beforeStep": { "url": "https://example.com" }, "steps": [] }]);
    let payloads = convert(&text_step("Choose", site), json!([qr()]));
    let types: Vec<&Value> = payloads.iter().map(|p| &p["template"]["type"]).collect();
    assert_eq!(types, ["QA_BUTTON_CARD", "QA_LINK_CARD"]);
    assert_eq!(payloads[0]["template"]["buttons"][0]["id"], "flow-1::qr-1");
    assert!(
        payloads[1]["template"]["buttons"][0]["id"]
            .as_str()
            .unwrap()
            .contains("https://example.com")
    );
    let link = convert(&text_step("Choose", json!([])), json!([url_qr()]));
    assert_eq!(link[0]["template"]["type"], "QA_LINK_CARD");
    assert_eq!(
        link[0]["template"]["buttons"][0]["id"],
        "https://example.com?code=flow-1::qr-2"
    );
    let mixed = convert(&text_step("Choose", json!([])), json!([qr(), url_qr()]));
    assert_eq!(mixed.len(), 2);
    assert_eq!(mixed[1]["template"]["buttons"][0]["title"], "Open");
}

#[test]
fn clamps_a_long_text_to_the_card_title_limit() {
    let long = "x".repeat(TIKTOK_CARD_TITLE_MAX + 50);
    for quick in [qr(), url_qr()] {
        let payloads = convert(&text_step(&long, json!([])), json!([quick]));
        assert_eq!(
            payloads[0]["template"]["title"],
            long[..TIKTOK_CARD_TITLE_MAX]
        );
    }
}

#[test]
fn encodes_the_skip_postback_unchanged_as_the_reply_button_id() {
    let skip = json!([{ "id": "skip-1", "label": "Skip", "buttonType": "postback", "postback": "STEP_INPUT_SKIP|11612473309626368" }]);
    let payloads = convert(&text_step("Choose", json!([])), skip);
    assert_eq!(
        payloads[0]["template"]["buttons"][0],
        json!({ "type": "REPLY", "id": "STEP_INPUT_SKIP|11612473309626368", "title": "Skip" })
    );
}

#[test]
fn chunks_more_than_three_buttons_into_several_cards() {
    let many: Vec<Value> = (0..4)
        .map(|i| json!({ "id": format!("q{i}"), "label": format!("L{i}"), "buttonType": "postback", "postback": format!("p{i}") }))
        .collect();
    let payloads = convert(&text_step("Choose", json!([])), json!(many));
    assert_eq!(payloads.len(), 2);
    assert_eq!(
        payloads[0]["template"]["buttons"].as_array().unwrap().len(),
        3
    );
}

// ---- errors ----

#[test]
fn maps_descriptions_and_statuses_to_categories() {
    assert_eq!(
        categorize(400, "access_token_expired"),
        Category::AuthFailed
    );
    assert_eq!(
        categorize(200, "DM_DISABLED for user"),
        Category::UserBlocked
    );
    assert_eq!(categorize(403, ""), Category::PermissionDenied);
    assert_eq!(categorize(404, ""), Category::InvalidRecipient);
    assert_eq!(categorize(503, ""), Category::NetworkError);
    assert_eq!(categorize(418, ""), Category::Unknown);
    let revoked = |status, message: &str| ApiError {
        http_status: status,
        code: None,
        message: message.into(),
    };
    assert!(is_revoked_token_error(&revoked(401, "x")));
    assert!(is_revoked_token_error(&revoked(
        400,
        "Token revoked by user"
    )));
    assert!(!is_revoked_token_error(&revoked(500, "x")));
    let mapped = map_to_channel_error(&ApiError::new("bad_request"));
    assert_eq!(
        (mapped.category, mapped.code.as_deref()),
        (Category::PayloadInvalid, Some("tiktokError"))
    );
}
