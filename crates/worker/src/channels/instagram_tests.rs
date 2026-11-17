//! Ports of `integrations/instagram/__tests__` (the pure parts; the HTTP
//! mocks become payload checks): `send-message-multiple-images`,
//! `send-flow-step-media`, `quick-replies-attachment`, `story-mention`,
//! `incoming-message-referral`, `message-echo-webhook`,
//! `http-client-policy-error` (mapper) and the webhook batching rules.

use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::ContactInbox;
use serde_json::{Value, json};

use super::*;
use crate::channel::{OutgoingContact, SendFlowStep};

fn contact_inbox(source_id: &str) -> ContactInbox {
    let conn = chatbotx_db::rusqlite::Connection::open_in_memory().unwrap();
    let columns: Vec<String> = ContactInbox::COLUMNS
        .iter()
        .map(|c| match *c {
            "id"
            | "createdAt"
            | "updatedAt"
            | "originalContactId"
            | "contactId"
            | "inboxId"
            | "consecutiveFailedReply" => "1".into(),
            "channel" => "'instagram'".into(),
            "source" => "'direct'".into(),
            "sourceId" => format!("'{source_id}'"),
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
            contact_inbox: contact_inbox("igsid-1"),
            source_conversation_id: None,
        },
        flow_id: "flow-1".into(),
        flow_version_id: Some("flow-version-1".into()),
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

const IMAGE_URL: &str = "https://example.com/a.png";
const NOW: i64 = 1_780_963_200_000;
const HOUR: i64 = 60 * 60 * 1000;

#[test]
fn multiple_images_batch_into_one_call_and_one_image_stays_singular() {
    let many = convert_message(&json!({ "contentType": "text", "attachments": [
        { "fileType": "image", "url": "https://x/1.png" }, { "fileType": "image", "url": "https://x/2.png" } ] }));
    assert_eq!(many.len(), 1);
    assert_eq!(
        many[0]["attachments"],
        json!([
            { "type": "image", "payload": { "url": "https://x/1.png", "is_reusable": true } },
            { "type": "image", "payload": { "url": "https://x/2.png", "is_reusable": true } },
        ])
    );
    assert!(many[0].get("attachment").is_none());
    let one = convert_message(
        &json!({ "contentType": "text", "attachments": [{ "fileType": "image", "url": "https://x/1.png" }] }),
    );
    assert_eq!(
        one[0]["attachment"],
        json!({ "type": "image", "payload": { "url": "https://x/1.png", "is_reusable": true } })
    );
}

#[test]
fn media_steps_drop_their_buttons_and_never_use_a_template() {
    let buttons = json!([
        { "id": "btn-1", "label": "Visit", "buttonType": "openWebsite", "beforeStep": { "url": "https://example.com" } },
        { "id": "btn-2", "label": "Chat", "buttonType": "sendMessage", "beforeStep": {} },
    ]);
    for (step_type, url, kind) in [
        ("sendImage", IMAGE_URL, "image"),
        ("sendVideo", "https://example.com/a.mp4", "video"),
    ] {
        let messages = ready(
            convert_flow_step(&step_data(
                json!({ "stepType": step_type, "url": url, "buttons": buttons }),
                None,
            ))
            .unwrap(),
        );
        assert_eq!(
            messages,
            vec![
                json!({ "attachment": { "type": kind, "payload": { "url": url, "is_reusable": true } } })
            ]
        );
        let text = messages[0].to_string();
        assert!(!text.contains("template") && !text.contains("Visit"));
    }
}

#[test]
fn quick_replies_ride_on_media_text_and_gif_steps() {
    let quick = Some(
        json!([{ "id": "qr-1", "label": "Yes", "buttonType": "postback", "postback": "qr-payload" }]),
    );
    for step in [
        json!({ "stepType": "sendImage", "url": IMAGE_URL, "buttons": [] }),
        json!({ "stepType": "sendText", "text": "hi", "buttons": [] }),
        json!({ "stepType": "sendGif", "url": IMAGE_URL }),
    ] {
        let messages = ready(convert_flow_step(&step_data(step, quick.clone())).unwrap());
        assert_eq!(
            messages[0]["quick_replies"],
            json!([{ "content_type": "text", "title": "Yes", "payload": "qr-payload" }])
        );
    }
}

#[test]
fn file_steps_upload_first_and_text_buttons_become_a_button_template() {
    let upload = convert_flow_step(&step_data(
        json!({ "stepType": "sendFile", "url": "https://x/a.pdf" }),
        None,
    ))
    .unwrap();
    assert_eq!(
        upload,
        vec![StepMessage::Upload {
            url: "https://x/a.pdf".into(),
            media_type: "file",
            quick_replies: vec![]
        }]
    );
    let text = ready(convert_flow_step(&step_data(json!({ "stepType": "sendText", "text": "Pick", "buttons": [
        { "id": "b1", "label": "Go", "buttonType": "sendMessage" }, { "id": "b2", "label": "2", "buttonType": "sendMessage" },
        { "id": "b3", "label": "3", "buttonType": "sendMessage" }, { "id": "b4", "label": "4", "buttonType": "sendMessage" } ] }), None)).unwrap());
    let payload = &text[0]["attachment"]["payload"];
    assert_eq!(payload["template_type"], "button");
    assert_eq!(payload["buttons"].as_array().unwrap().len(), 3);
    assert_eq!(payload["buttons"][0]["payload"], "flow-1:flow-version-1:b1");
}

#[test]
fn send_payload_carries_metadata_and_the_policy() {
    let payload = build_message_payload(
        "igsid-1",
        &json!({ "text": "hi" }),
        Some(("MESSAGE_TAG", Some("HUMAN_AGENT"))),
    )
    .unwrap();
    assert_eq!(
        payload,
        json!({ "recipient": { "id": "igsid-1" }, "message": { "text": "hi", "metadata": MESSAGE_METADATA },
        "messaging_type": "MESSAGE_TAG", "tag": "HUMAN_AGENT" })
    );
    assert!(build_message_payload("", &json!({}), None).is_err());
}

#[test]
fn messaging_policy_windows() {
    assert_eq!(
        resolve_messaging_policy(Some(NOW - HOUR), None, NOW).unwrap(),
        ("RESPONSE", None)
    );
    let expired = resolve_messaging_policy(Some(NOW - 25 * HOUR), None, NOW).unwrap_err();
    assert_eq!(
        expired.code.as_deref(),
        Some("instagram_response_window_expired")
    );
    assert_eq!(
        resolve_messaging_policy(None, Some("inbox"), NOW).unwrap(),
        ("RESPONSE", None)
    );
    assert_eq!(
        resolve_messaging_policy(Some(NOW - 48 * HOUR), Some("inbox"), NOW).unwrap(),
        ("MESSAGE_TAG", Some("HUMAN_AGENT"))
    );
    let late = resolve_messaging_policy(Some(NOW - 8 * 24 * HOUR), Some("inbox"), NOW).unwrap_err();
    assert_eq!(
        late.code.as_deref(),
        Some("instagram_human_agent_window_expired")
    );
}

#[test]
fn error_mapper_categories() {
    let error = |code: Value, sub: Option<Value>, kind: Option<&str>| GraphError {
        http_status: 400,
        code: Some(code),
        sub_code: sub,
        kind: kind.map(str::to_owned),
        message: Some("m".into()),
        transport: false,
    };
    assert_eq!(
        map_error(&error(json!(10), Some(json!(2_018_028)), None)).category,
        Category::QuotaExceeded
    );
    assert_eq!(
        map_error(&error(json!(100), Some(json!(2_018_001)), None)).category,
        Category::InvalidRecipient
    );
    assert_eq!(
        map_error(&error(json!(230), None, None)).category,
        Category::PermissionDenied
    );
    assert_eq!(
        map_error(&error(json!(-1), None, None)).category,
        Category::NetworkError
    );
    assert_eq!(
        map_error(&error(json!(999), None, Some("OAuthException"))).category,
        Category::AuthFailed
    );
    // A string subcode is not a number: no override.
    assert_eq!(
        map_error(&error(json!(100), Some(json!("2018001")), None)).category,
        Category::PayloadInvalid
    );
    assert!(is_revoked_token_error(&error(
        json!(190),
        Some(json!(463)),
        Some("OAuthException")
    )));
    assert!(!is_revoked_token_error(&error(
        json!(190),
        None,
        Some("OAuthException")
    )));
}

fn incoming(message: Value) -> Value {
    json!({ "object": "instagram", "entry": [{ "id": "ig-1", "time": 1, "messaging": [
        { "sender": { "id": "igsid-1" }, "recipient": { "id": "ig-1" }, "timestamp": 5, "message": message } ] }] })
}

#[test]
fn story_mentions_and_story_replies_set_content_attributes() {
    let mention = parse_incoming(
        "ig-1",
        &incoming(json!({ "mid": "mid-1", "attachments": [
        { "type": "story_mention", "payload": { "url": "https://lookaside.fbsbx.com/s" } } ] })),
    )
    .unwrap();
    let message = mention.message.unwrap();
    assert_eq!(
        message.content_attributes,
        Some(json!({ "type": "story_mention" }))
    );
    assert_eq!(message.source_id.as_deref(), Some("mid-1"));
    let plain = parse_incoming("ig-1", &incoming(json!({ "mid": "m", "text": "hi" }))).unwrap();
    assert_eq!(plain.message.unwrap().content_attributes, None);
    let reply = parse_incoming("ig-1", &incoming(json!({ "mid": "m", "text": "hi", "reply_to": { "story": { "id": "s1", "url": "https://x" } } }))).unwrap();
    assert_eq!(
        reply.message.unwrap().content_attributes,
        Some(json!({ "type": "story_reply", "story": { "id": "s1", "url": "https://x" } }))
    );
}

#[test]
fn referral_precedence_and_message_referral_tolerance() {
    let with_message_referral = parse_incoming("ig-1", &incoming(json!({ "mid": "m", "text": "hi",
        "referral": { "source": "ADS", "type": "OPEN_THREAD", "ad_id": "ad-1", "referer_uri": "https://www.instagram.com/p/x" } }))).unwrap();
    assert_eq!(
        with_message_referral.referral_source.as_deref(),
        Some("ADS")
    );
    assert_eq!(
        with_message_referral.referral.unwrap()["sourcePlatform"],
        "instagram"
    );
    // A message referral missing `source` / `type` is dropped; the message stays.
    let broken = parse_incoming(
        "ig-1",
        &incoming(json!({ "mid": "m", "text": "hi", "referral": { "ad_id": "1" } })),
    )
    .unwrap();
    assert!(broken.referral.is_none() && broken.message.is_some());
}

#[test]
fn an_echo_from_the_account_is_outgoing_to_the_recipient() {
    let payload = json!({ "object": "instagram", "entry": [{ "id": "ig-1", "time": 1, "messaging": [
        { "sender": { "id": "ig-1" }, "recipient": { "id": "igsid-1" }, "timestamp": 5, "message": { "mid": "e", "text": "x", "is_echo": true } } ] }] });
    let received = parse_incoming("ig-1", &payload).unwrap();
    assert_eq!(received.contact.source_id, "igsid-1");
    assert_eq!(received.message.unwrap().message_type, "outgoing");
}

fn names(jobs: &[WebhookJob]) -> Vec<&'static str> {
    jobs.iter().map(|j| j.name).collect()
}

#[test]
fn webhook_echoes_reads_reactions_and_comments() {
    let ev = |sender: &str, recipient: &str, extra: Value| {
        let mut e =
            json!({ "sender": { "id": sender }, "recipient": { "id": recipient }, "timestamp": 5 });
        e.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        e
    };
    let body = json!({ "object": "instagram", "entry": [
        { "id": "ig-1", "time": 1, "messaging": [
            ev("igsid", "ig-1", json!({ "message": { "mid": "m1", "text": "a" } })),
            ev("ig-1", "igsid", json!({ "message": { "mid": "e1", "is_echo": true } })),
            ev("ig-1", "igsid", json!({ "message": { "mid": "e2", "is_echo": true, "metadata": MESSAGE_METADATA } })),
            ev("igsid", "ig-1", json!({ "read": { "watermark": 1 } })),
            ev("igsid", "ig-1", json!({ "reaction": { "mid": "m1", "action": "react", "emoji": "x" } })),
            ev("igsid", "ig-1", json!({ "message": { "mid": "m1", "is_deleted": true } })),
        ] },
        { "id": "ig-1", "time": 2, "changes": [
            { "field": "comments", "value": { "id": "c1", "text": "hey", "from": { "id": "u1", "username": "bob" }, "media": { "id": "p1" } } },
            { "field": "live_comments", "value": { "id": "c2", "from": { "id": "u2" }, "media": { "id": "p2" } } },
            { "field": "comments", "value": { "id": "c3", "from": { "id": "u3" } } },
        ] },
    ]})
    .to_string();
    let jobs = webhook_jobs(&body).unwrap();
    assert_eq!(
        names(&jobs),
        vec![
            "incomingMessage",
            "incomingMessageEcho",
            "contactMarkAsRead",
            "messageReaction",
            "deleteIncomingMessage",
            "incomingComment",
            "incomingComment"
        ]
    );
    assert_eq!(jobs[1].data["data"]["integrationIdentifier"], "ig-1");
    assert_eq!(jobs[5].data["data"]["commentData"]["fromName"], "bob");
    assert_eq!(jobs[6].data["data"]["commentData"]["fromName"], "u2");
    assert_eq!(jobs[6].data["data"]["commentData"]["isLive"], true);
    assert!(jobs[5].data["data"]["commentData"].get("isLive").is_none());
}

#[test]
fn one_malformed_event_skips_the_whole_delivery() {
    let body = json!({ "object": "instagram", "entry": [{ "id": "ig-1", "time": 1, "messaging": [
        { "sender": { "id": "a" }, "recipient": { "id": "ig-1" }, "timestamp": 5, "message": { "mid": "m1" } },
        { "sender": { "id": "a" }, "recipient": { "id": "ig-1" }, "timestamp": 5, "reaction": { "mid": "m1", "action": "love" } },
    ] }] })
    .to_string();
    assert!(webhook_jobs(&body).unwrap().is_empty());
    assert!(
        webhook_jobs(r#"{"object":"page","entry":[]}"#)
            .unwrap()
            .is_empty()
    );
}
